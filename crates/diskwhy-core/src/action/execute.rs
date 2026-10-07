//! 执行：只处理预检标记为“删 / 只删链接”的条目。
//!
//! - 每条执行前再取一次指纹（护栏 9）：文件 ID、大小、修改时间任何一项和生成清单时不同就跳过；
//!   同时再查一次保护路径、再看一次是不是 30 分钟内改过。**不管去向是什么，9 条护栏都在去向之前执行**；
//! - 去向（阶段 J，[`Plan::entry_dest`]）：safe 档且 `safe_direct` → 直接删除；其余按策略；
//! - 直接删除：`DeleteHandle`（DELETE 权限、不跟随链接）打开 → **同一个句柄上**取指纹重新校验 →
//!   设删除标记（FILE_DISPOSITION_INFO_EX），校验和删除之间文件换不掉；
//! - 回收站：工作线程进入 STA，用 `IFileOperation` 成批送进回收站（每批最多 256 个），之后逐个确认原路径已经
//!   不在了才算成功；整个子文件夹满足严格条件时整夹回收（见 [`DIR_RECYCLE`]），日志仍逐个文件记；
//! - 暂存区：`MoveFileExW` 同卷改名，跨卷 `CopyFileExW`（停止时能中断大文件）→ 逐字节校验 → 删除；
//!   跨卷时链接数 > 1 的文件跳过（硬链接，删了不腾空间）；每移进来一个就往 `manifest.journal` 追加一行（不缓冲），结束时整份写 `manifest.json`；
//! - 链接：同卷暂存时把链接自身改名进暂存区；否则 `RemoveDirectoryW` / `DeleteFileW` 只删链接本身；
//! - 所有清单项的条目切成 256 条一块，放进**同一个队列**（各项交错），`clamp(CPU 数, 4, 16)` 个线程取块；
//!   每项有剩余块计数，做完最后一块的线程删这一项变空的子文件夹（最深的先删）并保存 manifest；
//! - 停止（`cancel`）：不再取新块，当前文件做完 / 中断（跨盘大文件复制会中断），剩下的原样不动，
//!   报告 `cancelled = true`；
//! - 进度每 ~200 ms 由计时线程发一次（按估计成本加权），每 30 秒写一条 `heartbeat` 日志并更新
//!   `last-run.json`；
//! - 某条失败不回滚已经成功的，报告里写清楚；每条都写日志。

use super::log::{ActionLog, LogRecord};
use super::perf::{Measured, PerfModel};
use super::protect::Protector;
use super::quarantine::{self, Manifest, ManifestEntry};
use super::runstate::RunState;
use super::winops::{self, ComSta, DeleteHandle, Fingerprint};
use super::{
    new_id, ByDest, CountBytes, Decision, Dest, Entry, EntryKind, Plan, PlanItem, SkipReason, CHUNK, HOT_WINDOW_SECS, MAX_WORKERS,
    PREVIEW_LIMIT,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 回收站模式下整夹回收（默认开）。条件（全部满足才整夹，否则逐个文件）：
/// 子树里每一条都是普通文件、预检决定都是“删”、执行前逐条重新校验都通过；没有链接 / 云目录 / 列不出的子目录；
/// 回收前重新列一遍，文件和文件夹的名字集合（以及每个文件的大小 / 修改时间）和清单完全一样；子树逻辑大小 ≤ 1 GB；
/// 16 ~ 2000 个文件；子树里没有太长的路径（[`winops::BIN_MAX_PATH`]，太长的会被 Windows 连整个文件夹直接删掉）。
pub const DIR_RECYCLE: bool = true;
pub const DIR_RECYCLE_MAX_BYTES: u64 = 1 << 30;
pub const DIR_RECYCLE_MIN_FILES: usize = 16;
/// 整夹回收的子树最多这么多个文件：逐条重新校验到真正送进回收站之间的时间要短（护栏 4、9 的空档），
/// 更大的子树挑它下面的子文件夹。
pub const DIR_RECYCLE_MAX_FILES: usize = 2000;

/// 执行选项。
pub struct ExecOptions<'a> {
    pub log: &'a ActionLog,
    /// false 时（用户在预览里选了“取消”而不是“跳过被占用的”），有被占用的文件就整个不执行。
    pub skip_locked: bool,
    /// 停止：置位后不再取新块，当前文件做完 / 中断，返回 `cancelled = true` 的报告。
    pub cancel: Option<&'a AtomicBool>,
    pub hot_window_secs: i64,
    pub workers: usize,
    /// 进度条的成本权重用它（每种操作的单文件耗时 + 跨盘吞吐）。
    pub perf: PerfModel,
    /// Some 时执行完把实测写回（`%LOCALAPPDATA%\DiskWhy\perf.json`）。
    pub perf_path: Option<PathBuf>,
    /// Some 时写执行状态（`%LOCALAPPDATA%\DiskWhy\last-run.json`）。
    pub run_state_path: Option<PathBuf>,
    /// 回收站模式下是否整夹回收（见 [`DIR_RECYCLE`]）。
    pub dir_recycle: bool,
    /// 进度事件间隔（毫秒）。
    pub progress_ms: u64,
    /// `heartbeat` 日志 / 状态文件更新间隔（秒）。
    pub heartbeat_secs: u64,
    /// 已不用（阶段 J 复核起暂存记录改为逐条追加到 `manifest.journal`）；保留字段免得调用方改。
    pub manifest_flush_secs: u64,
}

impl<'a> ExecOptions<'a> {
    pub fn new(log: &'a ActionLog) -> Self {
        ExecOptions {
            log,
            skip_locked: true,
            cancel: None,
            hot_window_secs: HOT_WINDOW_SECS,
            workers: super::default_workers(),
            perf: PerfModel::default(),
            perf_path: None,
            run_state_path: None,
            dir_recycle: DIR_RECYCLE,
            progress_ms: 200,
            heartbeat_secs: 30,
            manifest_flush_secs: 10,
        }
    }
}

/// 进度（约每 200 ms 一次，最后再发一次）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecProgress {
    /// 已处理（成功 + 跳过 + 失败）的条目数 / 要处理的条目数。
    pub done: usize,
    pub total: usize,
    /// 已成功处理的字节 / 要处理的总字节。
    pub bytes_done: u64,
    pub total_bytes: u64,
    /// 按估计成本加权的进度（毫秒）：进度条 = cost_done / cost_total，单调。
    pub cost_done: f64,
    pub cost_total: f64,
    pub elapsed_ms: u64,
    pub current: String,
    /// 已经收到停止，正在收尾。
    #[serde(default)]
    pub cancelling: bool,
    /// 执行已经结束、报告已经可以取了（Tauri 层在重扫受影响的子树之前发这一条，界面马上显示完成页）。
    #[serde(default)]
    pub finished: bool,
}

/// 一条的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryResult {
    pub path: String,
    pub kind: EntryKind,
    pub bytes: u64,
    /// ok / skipped / failed
    pub result: String,
    pub reason: String,
    pub dest: Option<String>,
    /// 阶段 J：实际去向（成功时）。
    #[serde(default)]
    pub dest_kind: Option<Dest>,
}

/// 一个清单项的结果。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ItemResult {
    pub path: String,
    pub blocked: Option<String>,
    pub ok: usize,
    pub ok_bytes: u64,
    pub skipped: usize,
    pub failed: usize,
    pub links_removed: usize,
    pub dirs_removed: usize,
    /// 前 200 条非成功明细 + 前 50 条成功明细。
    pub details: Vec<EntryResult>,
    /// 阶段 J。
    #[serde(default)]
    pub skipped_bytes: u64,
    #[serde(default)]
    pub failed_bytes: u64,
    #[serde(default)]
    pub by_dest: ByDest,
    /// 停止时还没处理、原样没动的条目。
    #[serde(default)]
    pub not_processed: usize,
    #[serde(default)]
    pub not_processed_bytes: u64,
}

/// 执行报告。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecReport {
    pub batch_id: String,
    pub strategy_label: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub items: Vec<ItemResult>,
    pub ok: usize,
    pub ok_bytes: u64,
    pub skipped: usize,
    pub failed: usize,
    pub manifest_path: Option<String>,
    pub log_path: String,
    pub log_failures: u64,
    pub cancelled: bool,
    /// 受影响的文件夹（清单项本身；文件项为其父目录），界面据此重扫这些子树。
    pub affected_dirs: Vec<String>,
    pub notes: Vec<String>,
    /// 阶段 J：源盘执行前后剩余空间的实测差值之和（每个盘 ≥ 0）。
    #[serde(default)]
    pub freed_bytes: u64,
    /// 阶段 J：按去向分别成功了多少个 / 多少字节。
    #[serde(default)]
    pub by_dest: ByDest,
    #[serde(default)]
    pub skipped_bytes: u64,
    /// 跳过按原因（hot / cloud / locked / changed / missing / hardlink / …）分的个数 + 字节。
    #[serde(default)]
    pub skipped_by_reason: BTreeMap<String, CountBytes>,
    #[serde(default)]
    pub failed_bytes: u64,
    /// 停止时还没处理、原样没动的。
    #[serde(default)]
    pub not_processed: CountBytes,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub safe_direct: bool,
    #[serde(default)]
    pub workers: usize,
    /// 各种操作的实测忙碌时间（各线程累计，毫秒），用来标定 perf.json；界面不用。
    #[serde(default)]
    pub measured: Measured,
}

// ───────────────────────── 内部结构 ─────────────────────────

/// 一块的结果（合并进清单项的结果）。
#[derive(Default)]
struct Part {
    ok: usize,
    ok_bytes: u64,
    skipped: usize,
    skipped_bytes: u64,
    failed: usize,
    failed_bytes: u64,
    links_removed: usize,
    dirs_removed: usize,
    by_dest: ByDest,
    skipped_by_reason: BTreeMap<String, CountBytes>,
    not_processed: CountBytes,
    details: Vec<EntryResult>,
    ok_details: usize,
}

enum UnitKind {
    Files(Vec<usize>),
    Dir { dir: String, entries: Vec<usize>, subdirs: Vec<String> },
}

struct Unit {
    item: usize,
    kind: UnitKind,
}

impl Unit {
    fn indices(&self) -> &[usize] {
        match &self.kind {
            UnitKind::Files(v) => v,
            UnitKind::Dir { entries, .. } => entries,
        }
    }
}

struct Shared<'a> {
    plan: &'a Plan,
    batch_id: String,
    batch_dir: Option<String>,
    manifest: Mutex<Option<Manifest>>,
    manifest_path: Option<PathBuf>,
    manifest_io: Mutex<()>,
    /// manifest.json 旁边的追加式记录：每移进来一个写一行（见 [`quarantine::JOURNAL`]）。
    journal: Option<quarantine::Journal>,
    protector: &'a Protector,
    opts: &'a ExecOptions<'a>,
    done: AtomicUsize,
    bytes_done: AtomicU64,
    /// 成本（毫秒 × 1000）。
    cost_done: AtomicU64,
    total: usize,
    total_bytes: u64,
    cost_total: f64,
    /// 给 CopyFileExW 的 pbCancel（BOOL）。
    cancel_i32: AtomicI32,
    current: Mutex<String>,
    /// 回收站模式：各源盘（小写盘根）的回收站设置，送进去之前分流“放不下 / 关掉了”的（见 [`winops::RecycleBinInfo`]）。
    bins: Vec<(String, winops::RecycleBinInfo)>,
    /// 回收站没收、被 Windows 直接删掉的个数（正常不会有：送进去之前已经分流）。
    nuked: AtomicUsize,
    /// 已经开始的块数（`batch_begin` 日志用）。
    units_started: AtomicUsize,
    units_total: usize,
    /// 执行开始时 Restart Manager 查到正在被某个程序用着的 Chromium 缓存文件夹（小写，带结尾 `\`）：
    /// 里面的一律跳过（预检之后才打开的浏览器，见 [`super::preflight::live_chromium_cache_dirs`]）。
    live_caches: Vec<String>,
}

impl Shared<'_> {
    /// 这么大的条目送进它所在盘的回收站会不会被直接删掉：会 → Some(原因)。
    fn bin_refuse(&self, path: &str, size: u64) -> Option<String> {
        let v = crate::scan::volume_root_of(path)?.to_lowercase();
        self.bins.iter().find(|(r, _)| *r == v).and_then(|(_, b)| b.refuse_path(path, size))
    }

    /// 在执行开始时查到的“正在用”的 Chromium 缓存文件夹里？
    fn in_live_cache(&self, path: &str) -> bool {
        if self.live_caches.is_empty() {
            return false;
        }
        let l = path.to_lowercase();
        self.live_caches.iter().any(|d| l.starts_with(d.as_str()))
    }

    fn cancelled(&self) -> bool {
        let c = self.opts.cancel.is_some_and(|c| c.load(Ordering::SeqCst));
        if c {
            self.cancel_i32.store(1, Ordering::SeqCst);
        }
        c
    }

    fn tick(&self, ok_bytes: u64, cost_ms: f64) {
        self.done.fetch_add(1, Ordering::Relaxed);
        self.bytes_done.fetch_add(ok_bytes, Ordering::Relaxed);
        self.cost_done.fetch_add((cost_ms * 1000.0) as u64, Ordering::Relaxed);
    }

    fn snapshot(&self, t0: Instant) -> ExecProgress {
        ExecProgress {
            done: self.done.load(Ordering::Relaxed),
            total: self.total,
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            total_bytes: self.total_bytes,
            cost_done: (self.cost_done.load(Ordering::Relaxed) as f64 / 1000.0).min(self.cost_total),
            cost_total: self.cost_total,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            current: self.current.lock().map(|g| g.clone()).unwrap_or_default(),
            cancelling: self.opts.cancel.is_some_and(|c| c.load(Ordering::Relaxed)),
            finished: false,
        }
    }

    /// 估计成本（毫秒）：单文件常数 + 跨盘时按字节。
    fn cost(&self, item: &PlanItem, e: &Entry) -> f64 {
        let d = self.plan.entry_dest(item, e);
        let link = e.kind == EntryKind::Link;
        let x = d == Dest::Quarantine && !link && self.plan.crosses_volume(&e.path);
        let p = &self.opts.perf;
        p.op_ms(d, x, link) + if x { p.copy_ms(e.size, 1.0) } else { 0.0 }
    }

    fn save_manifest(&self) -> Result<(), String> {
        let Some(mp) = &self.manifest_path else { return Ok(()) };
        // 先拿写文件的锁再取快照：两次保存按顺序来，旧快照不会盖掉新的
        let _io = self.manifest_io.lock();
        let snap = self.manifest.lock().map(|g| g.clone()).unwrap_or(None);
        let Some(m) = snap else { return Ok(()) };
        m.save(mp)
    }

    fn run_state(&self, status: &str, started: i64) -> RunState {
        RunState {
            batch: self.batch_id.clone(),
            status: status.into(),
            pid: std::process::id(),
            started,
            updated: crate::explain::now_unix(),
            total: self.total,
            total_bytes: self.total_bytes,
            processed: self.done.load(Ordering::Relaxed),
            processed_bytes: self.bytes_done.load(Ordering::Relaxed),
            strategy: self.plan.method_label(),
            exe: std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
        }
    }
}

// ───────────────────────── 入口 ─────────────────────────

/// 执行计划。`progress` 在计时线程里被调用（要求 Send + Sync），约每 `progress_ms` 一次，结束时再调用一次。
pub fn execute<F>(plan: &Plan, protector: &Protector, opts: &ExecOptions, progress: F) -> Result<ExecReport, String>
where
    F: Fn(&ExecProgress) + Send + Sync,
{
    if !plan.preflight_done {
        return Err("还没有预检，不能执行".into());
    }
    let locked: usize = plan
        .items
        .iter()
        .flat_map(|i| i.decisions.iter())
        .filter(|d| matches!(d, Decision::Skip { reason: SkipReason::Locked { .. } }))
        .count();
    if locked > 0 && !opts.skip_locked {
        return Err(format!("有 {locked} 个文件被占用，你选择了不跳过，所以什么都没做"));
    }
    let t0 = Instant::now();
    let started_at = crate::explain::now_unix();
    let batch_id = new_id("");

    // 整项再过一次保护路径；拦下的项不进队列
    let item_blocked: Vec<Option<String>> = plan
        .items
        .iter()
        .map(|i| i.blocked.clone().or_else(|| protector.check(&i.path)))
        .collect();
    let active = |ii: usize| item_blocked[ii].is_none();
    let needs_q = plan
        .items
        .iter()
        .enumerate()
        .any(|(ii, it)| active(ii) && it.will_delete().any(|(e, _)| plan.entry_dest(it, e) == Dest::Quarantine));
    let needs_sta = plan.items.iter().enumerate().any(|(ii, it)| active(ii) && plan.dest_for(it) == Dest::Recycle && it.entry_count() > 0);

    // 预检之后才打开的浏览器 / Electron 程序：它们的磁盘缓存用“全共享（含删除）”打开，直接删会删在它们脚下
    // （PHASE_J_CRASH.md 隐患 5）。执行开始时再问一次 Restart Manager；问不出来按“正在用”处理。
    let live_caches: Vec<String> = super::preflight::live_chromium_cache_dirs(plan).into_iter().map(|(d, _)| format!("{d}\\")).collect();

    let (batch_dir, manifest, manifest_path) = match (plan.quarantine_root(), needs_q) {
        (Some(dest_root), true) => {
            if let Some(why) = super::preflight::quarantine_dest_problem(dest_root, plan) {
                return Err(why);
            }
            let dir = quarantine::batch_dir(dest_root, &batch_id);
            std::fs::create_dir_all(crate::win::long_path(&dir)).map_err(|e| format!("建不了暂存区文件夹 {dir}：{e}"))?;
            let mp = PathBuf::from(&dir).join(quarantine::MANIFEST);
            let m = Manifest::new(&batch_id, dest_root, &dir);
            m.save(&mp)?;
            (Some(dir), Some(m), Some(mp))
        }
        _ => (None, None, None),
    };

    // 工作队列：每项先放整夹回收的单元，再把其余条目按 256 条切块；各项轮流交错
    let mut per_item: Vec<Vec<UnitKind>> = Vec::with_capacity(plan.items.len());
    for (ii, item) in plan.items.iter().enumerate() {
        let mut list = Vec::new();
        if active(ii) && !item.entries.is_empty() {
            let mut used = vec![false; item.entries.len()];
            if opts.dir_recycle && item.is_dir && plan.dest_for(item) == Dest::Recycle {
                for (dir, entries, subdirs) in dir_units(item, protector) {
                    for &i in &entries {
                        used[i] = true;
                    }
                    list.push(UnitKind::Dir { dir, entries, subdirs });
                }
            }
            let rest: Vec<usize> = (0..item.entries.len()).filter(|&i| !used[i]).collect();
            for c in rest.chunks(CHUNK) {
                list.push(UnitKind::Files(c.to_vec()));
            }
        }
        per_item.push(list);
    }
    let remaining: Vec<AtomicUsize> = per_item.iter().map(|l| AtomicUsize::new(l.len())).collect();
    let mut units: Vec<Unit> = Vec::new();
    {
        let mut iters: Vec<std::vec::IntoIter<UnitKind>> = per_item.into_iter().map(|l| l.into_iter()).collect();
        loop {
            let mut any = false;
            for (ii, it) in iters.iter_mut().enumerate() {
                if let Some(k) = it.next() {
                    units.push(Unit { item: ii, kind: k });
                    any = true;
                }
            }
            if !any {
                break;
            }
        }
    }

    // 总量
    let mut total = 0usize;
    let mut total_bytes = 0u64;
    let mut src_vols: Vec<String> = Vec::new();
    for (ii, it) in plan.items.iter().enumerate() {
        if !active(ii) {
            continue;
        }
        for (e, _) in it.will_delete() {
            total += 1;
            total_bytes += e.bytes;
        }
        if let Some(v) = crate::scan::volume_root_of(&it.path) {
            if !src_vols.iter().any(|x| x.eq_ignore_ascii_case(&v)) {
                src_vols.push(v);
            }
        }
    }
    let free_before: Vec<Option<u64>> = src_vols.iter().map(|v| crate::win::volume_space(v).ok().map(|s| s.free_bytes)).collect();

    let mut shared = Shared {
        plan,
        batch_id: batch_id.clone(),
        batch_dir,
        manifest: Mutex::new(manifest),
        journal: manifest_path.as_deref().map(quarantine::Journal::open),
        manifest_path: manifest_path.clone(),
        manifest_io: Mutex::new(()),
        protector,
        opts,
        done: AtomicUsize::new(0),
        bytes_done: AtomicU64::new(0),
        cost_done: AtomicU64::new(0),
        total,
        total_bytes,
        cost_total: 0.0,
        cancel_i32: AtomicI32::new(0),
        current: Mutex::new(String::new()),
        bins: if needs_sta { src_vols.iter().map(|v| (v.to_lowercase(), winops::RecycleBinInfo::read(v))).collect() } else { Vec::new() },
        nuked: AtomicUsize::new(0),
        units_started: AtomicUsize::new(0),
        units_total: units.len(),
        live_caches,
    };
    shared.cost_total = plan
        .items
        .iter()
        .enumerate()
        .filter(|(ii, _)| active(*ii))
        .flat_map(|(_, it)| it.will_delete().map(move |(e, _)| (it, e)))
        .map(|(it, e)| shared.cost(it, e))
        .sum();
    let workers = opts.workers.clamp(1, MAX_WORKERS).min(units.len().max(1));

    // 取证：start 记录 + 状态文件
    let mut st = LogRecord::new(&batch_id, "start", &plan.id);
    st.bytes = total_bytes;
    st.method = plan.method_label();
    st.reason = format!(
        "计划 {total} 个，{} 块，{workers} 个线程，safe 直接删 = {}，进程 {}，父进程 {}，程序 {}",
        units.len(),
        plan.safe_direct,
        std::process::id(),
        crate::win::parent_pid().map(|p| p.to_string()).unwrap_or_else(|| "?".into()),
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
    );
    opts.log.write(&st);
    if let Some(p) = &opts.run_state_path {
        let _ = shared.run_state("running", started_at).save(p);
    }

    let sh = &shared;
    let parts: Vec<Mutex<Option<Part>>> = units.iter().map(|_| Mutex::new(None)).collect();
    // 每块是否被某个线程取走了（取走了却没有结果 = 那个线程 panic 了）
    let taken: Vec<AtomicBool> = units.iter().map(|_| AtomicBool::new(false)).collect();
    // 每项是否开始处理过（停止时没开始的项一点都不动，连空文件夹也不删）
    let item_started: Vec<AtomicBool> = plan.items.iter().map(|_| AtomicBool::new(false)).collect();
    let finalized: Vec<AtomicBool> = plan.items.iter().map(|_| AtomicBool::new(false)).collect();
    let dirs_removed: Vec<AtomicUsize> = plan.items.iter().map(|_| AtomicUsize::new(0)).collect();
    let measured: Mutex<Measured> = Mutex::new(Measured::default());
    let next = AtomicUsize::new(0);
    let all_done = AtomicBool::new(false);
    let (units_r, parts_r, taken_r, started_r, remaining_r, finalized_r, dirs_removed_r, measured_r, next_r, all_done_r, progress_r) =
        (&units, &parts, &taken, &item_started, &remaining, &finalized, &dirs_removed, &measured, &next, &all_done, &progress);
    let (spawn_err, panicked) = std::thread::scope(|s| {
        // 计时线程：进度 / 日志 flush / 状态文件 / heartbeat / 停止信号转给 CopyFileExW
        s.spawn(move || {
            let mut last_prog = Instant::now();
            let mut last_hb = Instant::now();
            let mut last_rs = Instant::now();
            while !all_done_r.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(20));
                sh.cancelled();
                if last_prog.elapsed() >= Duration::from_millis(sh.opts.progress_ms) {
                    last_prog = Instant::now();
                    progress_r(&sh.snapshot(t0));
                    sh.opts.log.flush();
                }
                // last-run.json 很小（临时文件再改名）：每 2 秒更新一次，程序被结束时“已处理 N 个”最多差 2 秒
                if last_rs.elapsed() >= Duration::from_secs(2) {
                    last_rs = Instant::now();
                    if let Some(p) = &sh.opts.run_state_path {
                        let _ = sh.run_state("running", started_at).save(p);
                    }
                }
                if last_hb.elapsed() >= Duration::from_secs(sh.opts.heartbeat_secs) {
                    last_hb = Instant::now();
                    let mut hb = LogRecord::new(&sh.batch_id, "heartbeat", &sh.plan.id);
                    hb.bytes = sh.bytes_done.load(Ordering::Relaxed);
                    hb.reason = format!("已处理 {}/{}，{} 毫秒", sh.done.load(Ordering::Relaxed), sh.total, t0.elapsed().as_millis());
                    sh.opts.log.write(&hb);
                }
            }
        });
        let mut handles = Vec::new();
        let mut spawn_err = None;
        for k in 0..workers {
            // 8 MB 栈（shell32 + 外壳扩展在默认 2 MB 上偏薄，PHASE_J_CRASH.md 隐患 6）；起名字，panic 记录里能看出是谁
            let h = std::thread::Builder::new().name(format!("exec-{k}")).stack_size(8 << 20).spawn_scoped(s, move || {
                let _com = needs_sta.then(ComSta::init);
                let mut meas = Measured::default();
                loop {
                    if sh.cancelled() {
                        break;
                    }
                    let u = next_r.fetch_add(1, Ordering::SeqCst);
                    if u >= units_r.len() {
                        break;
                    }
                    let unit = &units_r[u];
                    taken_r[u].store(true, Ordering::SeqCst);
                    started_r[unit.item].store(true, Ordering::SeqCst);
                    let part = run_unit(unit, sh, &mut meas);
                    *parts_r[u].lock().unwrap_or_else(|p| p.into_inner()) = Some(part);
                    if remaining_r[unit.item].fetch_sub(1, Ordering::SeqCst) == 1 && !finalized_r[unit.item].swap(true, Ordering::SeqCst) {
                        let n = finalize_item(&sh.plan.items[unit.item], sh, &mut meas);
                        dirs_removed_r[unit.item].store(n, Ordering::SeqCst);
                    }
                }
                measured_r.lock().unwrap_or_else(|p| p.into_inner()).merge(&meas);
            });
            match h {
                Ok(h) => handles.push(h),
                Err(_) if !handles.is_empty() => break, // 起不了更多线程就用已有的
                Err(e) => {
                    spawn_err = Some(format!("起不了工作线程：{e}"));
                    break;
                }
            }
        }
        // 某个工作线程 panic 了（panic hook 已经记进 crash.log）：不能当作“没处理”，下面单独记账
        let panicked = handles.into_iter().map(|h| h.join()).filter(|r| r.is_err()).count();
        // 一个工作线程都没起来时也要让计时线程停下，否则 scope 永远等它
        all_done_r.store(true, Ordering::SeqCst);
        (spawn_err, panicked)
    });
    if let Some(e) = spawn_err {
        // 一个文件都还没动
        let mut r = LogRecord::new(&batch_id, "summary", &plan.id);
        r.result = "failed".into();
        r.reason = e.clone();
        opts.log.write(&r);
        if let Some(p) = &opts.run_state_path {
            let _ = shared.run_state("failed", started_at).save(p);
        }
        return Err(e);
    }

    // 没有结果的块：没取到的（停止）——预检就跳过的照常记账，要删的记为“没处理”（原样没动）；
    // 取到了却没有结果的（线程 panic）——结果不确定，记成失败，不说“原样没动”
    let cancel_flag = opts.cancel.is_some_and(|c| c.load(Ordering::SeqCst));
    let mut main_meas = Measured::default();
    let mut uncertain = 0usize;
    for (u, unit) in units.iter().enumerate() {
        let mut g = parts[u].lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            let mut p = Part::default();
            if taken[u].load(Ordering::SeqCst) {
                uncertain += account_uncertain(&plan.items[unit.item], unit.indices(), &shared, &mut p);
            } else {
                account_untouched(&plan.items[unit.item], unit.indices(), &shared, &mut p);
            }
            *g = Some(p);
        }
    }
    // 还没收尾的项（停止 / 没有条目的项 / 线程出错）：只删真正空了的子文件夹。
    // 停止时一块都没开始的项原样不动（连原来就空着的子文件夹也不删）
    for (ii, item) in plan.items.iter().enumerate() {
        if !active(ii) || (cancel_flag && !item_started[ii].load(Ordering::SeqCst)) {
            continue;
        }
        if !finalized[ii].swap(true, Ordering::SeqCst) {
            let n = finalize_item(item, &shared, &mut main_meas);
            dirs_removed[ii].store(n, Ordering::SeqCst);
        }
    }

    // 合并
    let mut items: Vec<ItemResult> = plan
        .items
        .iter()
        .enumerate()
        .map(|(ii, it)| ItemResult { path: it.path.clone(), blocked: item_blocked[ii].clone(), ..Default::default() })
        .collect();
    let mut ok_details = vec![0usize; plan.items.len()];
    let mut skipped_by_reason: BTreeMap<String, CountBytes> = BTreeMap::new();
    for (u, unit) in units.iter().enumerate() {
        let p = parts[u].lock().unwrap_or_else(|p| p.into_inner()).take().unwrap_or_default();
        for (k, v) in &p.skipped_by_reason {
            skipped_by_reason.entry(k.clone()).or_default().merge(v);
        }
        merge_part(&mut items[unit.item], p, &mut ok_details[unit.item]);
    }
    for (ii, r) in items.iter_mut().enumerate() {
        r.dirs_removed += dirs_removed[ii].load(Ordering::SeqCst);
        // 明细里失败、跳过的排在前面（稳定排序，同类保持原顺序）
        r.details.sort_by_key(|d| match d.result.as_str() {
            "failed" => 0,
            "skipped" => 1,
            _ => 2,
        });
    }

    // 文件已经动过了：这里保存失败也不能返回 Err（否则界面会以为什么都没做），记进报告。
    // 先关掉 journal（保存成功后它就多余了，Manifest::save 会删掉它）
    if let Some(j) = &shared.journal {
        j.close();
    }
    let manifest_error = shared
        .save_manifest()
        .err()
        .map(|e| format!("暂存区清单保存失败：{e}。移走的文件还在批次文件夹里（另有 {} 记着），可以在暂存区里还原。", quarantine::JOURNAL));
    let freed_bytes: u64 = src_vols
        .iter()
        .zip(&free_before)
        .map(|(v, b)| match (b, crate::win::volume_space(v)) {
            (Some(b), Ok(a)) => a.free_bytes.saturating_sub(*b),
            _ => 0,
        })
        .sum();
    let mut by_dest = ByDest::default();
    let mut not_processed = CountBytes::default();
    for r in &items {
        by_dest.merge(&r.by_dest);
        not_processed.count += r.not_processed;
        not_processed.bytes += r.not_processed_bytes;
    }
    // 按了停止、但最后一个文件已经做完（什么都没剩下）：不算“已停止”
    let cancelled = cancel_flag && not_processed.count > 0;
    let mut rep = ExecReport {
        batch_id: batch_id.clone(),
        strategy_label: plan.strategy.label(),
        started_at,
        finished_at: crate::explain::now_unix(),
        ok: items.iter().map(|i| i.ok).sum(),
        ok_bytes: items.iter().map(|i| i.ok_bytes).sum(),
        skipped: items.iter().map(|i| i.skipped).sum(),
        failed: items.iter().map(|i| i.failed).sum(),
        skipped_bytes: items.iter().map(|i| i.skipped_bytes).sum(),
        failed_bytes: items.iter().map(|i| i.failed_bytes).sum(),
        manifest_path: manifest_path.map(|p| p.to_string_lossy().to_string()),
        log_path: opts.log.path().to_string_lossy().to_string(),
        log_failures: 0,
        cancelled,
        affected_dirs: plan
            .items
            .iter()
            .enumerate()
            .filter(|(ii, _)| active(*ii))
            .map(|(_, i)| {
                if i.is_dir {
                    i.path.clone()
                } else {
                    i.path.rsplit_once('\\').map(|(p, _)| if p.len() == 2 { format!("{p}\\") } else { p.to_string() }).unwrap_or_default()
                }
            })
            .collect(),
        items,
        notes: Vec::new(),
        freed_bytes,
        by_dest,
        skipped_by_reason,
        not_processed,
        duration_ms: 0,
        safe_direct: plan.safe_direct,
        workers,
        measured: Measured::default(),
    };
    // 隔离区一个都没放进去：删掉空批次
    if by_dest.quarantine.count == 0 {
        if let Some(mp) = &rep.manifest_path {
            let mp = std::path::Path::new(mp);
            let has_stored = Manifest::load(mp).map(|m| m.entries.iter().any(|e| e.status == "stored")).unwrap_or(true);
            if !has_stored {
                let _ = std::fs::remove_file(mp);
                if let Some(d) = mp.parent() {
                    quarantine::remove_empty_tree(&d.to_string_lossy());
                    if let Some(q) = d.parent() {
                        let _ = std::fs::remove_dir(q);
                    }
                }
                rep.manifest_path = None;
            }
        }
    }
    // 补充说明只写真正出了问题的（完成页上是一个橙色的“注意 N 条”）；直接删 / 回收站 / 暂存这些正常去向
    // 已经在完成页的小字里分行写了，不再重复
    if let Some(e) = manifest_error {
        rep.notes.push(e);
    }
    let nuked = shared.nuked.load(Ordering::Relaxed);
    if nuked > 0 {
        rep.notes.push(format!("有 {nuked} 个文件回收站没收，Windows 直接删除了，回收站里找不回。"));
    }
    if uncertain > 0 || panicked > 0 {
        rep.notes.push(format!("程序内部出错，有 {uncertain} 个文件的结果不确定（可能已经处理了），详情见动作日志。"));
    }
    let wall_ms = t0.elapsed().as_millis() as u64;
    rep.duration_ms = wall_ms;
    let mut sum = LogRecord::new(&batch_id, "summary", &plan.id);
    sum.bytes = rep.ok_bytes;
    sum.method = plan.method_label();
    sum.result = if rep.failed == 0 { "ok".into() } else { "failed".into() };
    sum.reason = format!(
        "成功 {}，跳过 {}，失败 {}{}；直接删除 {} / 回收站 {} / 暂存 {}；实测腾出 {} 字节，用时 {} 毫秒",
        rep.ok,
        rep.skipped,
        rep.failed,
        if cancelled { format!("，已停止（{} 个没处理）", rep.not_processed.count) } else { String::new() },
        by_dest.permanent.count,
        by_dest.recycle.count,
        by_dest.quarantine.count,
        freed_bytes,
        wall_ms
    );
    opts.log.write(&sum);
    if let Some(p) = &opts.run_state_path {
        let _ = shared.run_state(if cancelled { "cancelled" } else { "done" }, started_at).save(p);
    }
    let mut m = measured.into_inner().unwrap_or_default();
    m.merge(&main_meas);
    rep.measured = m;
    // 停止 / 出错的那次不学：墙钟时间里有一大段没做的活，会把预计时间带偏
    if let (Some(p), false) = (&opts.perf_path, cancel_flag || uncertain > 0) {
        let mut model = PerfModel::load(p);
        model.learn(&rep.measured, wall_ms as f64, workers);
        let _ = model.save(p);
    }
    progress(&shared.snapshot(t0));
    rep.log_failures = opts.log.failures.load(Ordering::Relaxed);
    Ok(rep)
}

// ───────────────────────── 整夹回收的候选 ─────────────────────────

/// 回收站模式下可以整夹回收的子文件夹（只看清单本身：静态条件；执行时还要逐条重新校验 + 重新列目录）。
/// 返回 (文件夹, 子树里的条目下标, 子树里的子文件夹)。只选最上层的（选了一个就不再选它下面的）。
fn dir_units(item: &PlanItem, protector: &Protector) -> Vec<(String, Vec<usize>, Vec<String>)> {
    let lp: Vec<String> = item.entries.iter().map(|e| e.path.to_lowercase()).collect();
    // 条目按小写路径排好序时，一个子树的条目是连续的一段（enumerate_plan 就是这么排的）
    if !lp.windows(2).all(|w| w[0] <= w[1]) {
        return Vec::new();
    }
    let ldirs: Vec<String> = item.dirs.iter().map(|d| d.to_lowercase()).collect();
    let errs: Vec<String> = item.enum_errors.iter().map(|e| e.to_lowercase()).collect();
    let mut order: Vec<usize> = (0..item.dirs.len()).collect();
    order.sort_by_key(|&i| item.dirs[i].len());
    let mut chosen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for di in order {
        let pre = format!("{}\\", ldirs[di]);
        if chosen.iter().any(|c| pre.starts_with(c.as_str())) {
            continue;
        }
        if errs.iter().any(|e| e.contains(&ldirs[di])) {
            continue;
        }
        let lo = lp.partition_point(|x| x.as_str() < pre.as_str());
        let hi = lo + lp[lo..].partition_point(|x| x.starts_with(pre.as_str()));
        if hi - lo < DIR_RECYCLE_MIN_FILES || hi - lo > DIR_RECYCLE_MAX_FILES {
            continue;
        }
        // 路径长度：子树里任何一条（原路径，或进了回收站之后 `$Recycle.Bin\<SID>\$Rxxxxxx\…` 下的新路径）
        // 超过回收站收得下的长度，Windows 会把**整个文件夹**直接永久删除（阶段 J 复核实测）
        let base = item.dirs[di].encode_utf16().count() + 1;
        let fits = |p: &str| {
            let n = p.encode_utf16().count();
            n <= winops::BIN_MAX_PATH && n.saturating_sub(base) + winops::BIN_DIR_PREFIX <= winops::BIN_MAX_PATH
        };
        if !(lo..hi).all(|i| fits(&item.entries[i].path)) || !item.dirs.iter().zip(&ldirs).filter(|(_, l)| l.starts_with(pre.as_str())).all(|(d, _)| fits(d)) {
            continue;
        }
        let mut size = 0u64;
        let ok = (lo..hi).all(|i| {
            size += item.entries[i].size;
            item.entries[i].kind == EntryKind::File && matches!(item.decisions.get(i), Some(Decision::Delete))
        });
        if !ok || size > DIR_RECYCLE_MAX_BYTES {
            continue;
        }
        let dir = &item.dirs[di];
        if !item.entries[lo].path.starts_with(&format!("{dir}\\")) {
            continue; // 大小写不一致之类：保守起见逐个文件
        }
        if protector.entry_blocked(dir).is_some() {
            continue;
        }
        let subdirs: Vec<String> = item.dirs.iter().zip(&ldirs).filter(|(_, l)| l.starts_with(pre.as_str())).map(|(d, _)| d.clone()).collect();
        chosen.push(pre);
        out.push((dir.clone(), (lo..hi).collect(), subdirs));
    }
    out
}

// ───────────────────────── 单元执行 ─────────────────────────

fn run_unit(u: &Unit, sh: &Shared, meas: &mut Measured) -> Part {
    let item = &sh.plan.items[u.item];
    let mut part = Part::default();
    if let Some(&i) = u.indices().first() {
        if let Ok(mut g) = sh.current.lock() {
            g.clone_from(&item.entries[i].path);
        }
        // 取证：每块开始前记一行（进程突然没了时，能看出哪些块正在做）
        let n = sh.units_started.fetch_add(1, Ordering::Relaxed) + 1;
        let mut b = LogRecord::new(&sh.batch_id, "batch_begin", &item.entries[i].path);
        b.item = item.path.clone();
        b.reason = format!(
            "第 {n}/{} 块，{} 条{}，线程 {}",
            sh.units_total,
            u.indices().len(),
            if matches!(u.kind, UnitKind::Dir { .. }) { "（整夹回收候选）" } else { "" },
            std::thread::current().name().unwrap_or("?")
        );
        sh.opts.log.write(&b);
    }
    match &u.kind {
        UnitKind::Dir { dir, entries, subdirs } => {
            if !(!sh.cancelled() && try_recycle_dir(item, dir, entries, subdirs, sh, &mut part, meas)) {
                run_files(item, entries, sh, &mut part, meas);
            }
        }
        UnitKind::Files(idx) => run_files(item, idx, sh, &mut part, meas),
    }
    part
}

fn tier_str(t: crate::explain::Tier) -> &'static str {
    quarantine::tier_str(t)
}

fn rec_for(sh: &Shared, item: &PlanItem, op: &str, e: &Entry) -> LogRecord {
    let mut r = LogRecord::new(&sh.batch_id, op, &e.path);
    r.bytes = e.bytes;
    r.item = item.path.clone();
    r.source = item.rule_id.clone().unwrap_or_else(|| item.source_label.clone());
    r.tier = tier_str(item.tier).into();
    r
}

fn er(e: &Entry, result: &str, reason: String, dest: Option<String>, dest_kind: Option<Dest>, bytes: u64) -> EntryResult {
    EntryResult { path: e.path.clone(), kind: e.kind, bytes, result: result.into(), reason, dest, dest_kind }
}

fn push(p: &mut Part, r: EntryResult, code: Option<&str>) {
    match r.result.as_str() {
        "ok" => {
            p.ok += 1;
            p.ok_bytes += r.bytes;
            if let Some(d) = r.dest_kind {
                p.by_dest.get_mut(d).add(r.bytes);
            }
            if p.ok_details < 50 {
                p.ok_details += 1;
                p.details.push(r);
            }
        }
        "skipped" => {
            p.skipped += 1;
            p.skipped_bytes += r.bytes;
            p.skipped_by_reason.entry(code.unwrap_or("other").to_string()).or_default().add(r.bytes);
            if p.details.len() < PREVIEW_LIMIT + 50 {
                p.details.push(r);
            }
        }
        _ => {
            p.failed += 1;
            p.failed_bytes += r.bytes;
            if p.details.len() < PREVIEW_LIMIT + 50 {
                p.details.push(r);
            }
        }
    }
}

fn merge_part(res: &mut ItemResult, p: Part, ok_details: &mut usize) {
    res.ok += p.ok;
    res.ok_bytes += p.ok_bytes;
    res.skipped += p.skipped;
    res.skipped_bytes += p.skipped_bytes;
    res.failed += p.failed;
    res.failed_bytes += p.failed_bytes;
    res.links_removed += p.links_removed;
    res.dirs_removed += p.dirs_removed;
    res.by_dest.merge(&p.by_dest);
    res.not_processed += p.not_processed.count;
    res.not_processed_bytes += p.not_processed.bytes;
    for d in p.details {
        if d.result == "ok" {
            if *ok_details < 50 {
                *ok_details += 1;
                res.details.push(d);
            }
        } else if res.details.len() < PREVIEW_LIMIT + 50 {
            res.details.push(d);
        }
    }
}

/// 生成清单时的指纹和现在的比（护栏 9）。
fn compare_fp(e: &Entry, old: &Fingerprint, now: &Fingerprint, hot_window_secs: i64) -> Option<SkipReason> {
    if now.volume != old.volume || now.id != old.id {
        return Some(SkipReason::Changed { what: "文件 ID 变了（被替换成了另一个文件）".into() });
    }
    // 护栏 5 再查一次：预检之后变成了云占位 / 云文件（只看属性，打开时只要了读属性权限，不会触发下载）
    if e.kind == EntryKind::File && now.attrs & crate::fs::attrs::CLOUD_PLACEHOLDER_MASK != 0 {
        return Some(SkipReason::Cloud);
    }
    if e.kind == EntryKind::File {
        if now.size != old.size {
            return Some(SkipReason::Changed { what: format!("大小从 {} 字节变成了 {} 字节", old.size, now.size) });
        }
        if now.mtime_ft != old.mtime_ft {
            return Some(SkipReason::Changed { what: "修改时间变了".into() });
        }
        let age = crate::explain::now_unix() - winops::ft_to_unix(now.mtime_ft);
        if age < hot_window_secs {
            return Some(SkipReason::Hot { minutes_ago: age.max(0) / 60 });
        }
    }
    None
}

/// 护栏 9：重新校验（按路径打开取指纹）。Err(原因) 表示要跳过。
fn reverify(e: &Entry, sh: &Shared) -> Result<Fingerprint, SkipReason> {
    if let Some(why) = sh.protector.entry_blocked(&e.path) {
        return Err(SkipReason::Protected { why });
    }
    if sh.in_live_cache(&e.path) {
        return Err(SkipReason::Locked { processes: vec![] });
    }
    let Some(old) = e.fp else {
        return Err(SkipReason::Changed { what: "生成清单时没取到文件信息".into() });
    };
    let now = match winops::fingerprint(&e.path, true) {
        Ok(f) => f,
        Err(2) | Err(3) => return Err(SkipReason::Missing),
        Err(c) => return Err(SkipReason::Unreadable { code: c }),
    };
    match compare_fp(e, &old, &now, sh.opts.hot_window_secs) {
        Some(r) => Err(r),
        None => Ok(now),
    }
}

enum PermFail {
    Skip(SkipReason),
    Failed(String),
}

/// 直接删除：保护路径 → DELETE 打开（不跟随）→ 同一个句柄上重新校验 → 设删除标记 → 关句柄（删掉）。
fn permanent(e: &Entry, sh: &Shared) -> Result<(), PermFail> {
    if let Some(why) = sh.protector.entry_blocked(&e.path) {
        return Err(PermFail::Skip(SkipReason::Protected { why }));
    }
    if sh.in_live_cache(&e.path) {
        return Err(PermFail::Skip(SkipReason::Locked { processes: vec![] }));
    }
    let Some(old) = e.fp else {
        return Err(PermFail::Skip(SkipReason::Changed { what: "生成清单时没取到文件信息".into() }));
    };
    let h = match DeleteHandle::open(&e.path, false) {
        Ok(h) => h,
        Err(2) | Err(3) => return Err(PermFail::Skip(SkipReason::Missing)),
        Err(c) if winops::is_sharing_violation(c) => return Err(PermFail::Skip(SkipReason::Locked { processes: vec![] })),
        Err(5) => return Err(PermFail::Failed("没有删除权限（Win32 错误 5）".into())),
        Err(c) => return Err(PermFail::Failed(format!("打不开（Win32 错误 {c}）"))),
    };
    let now = h.fingerprint().map_err(|c| PermFail::Skip(SkipReason::Unreadable { code: c }))?;
    if let Some(r) = compare_fp(e, &old, &now, sh.opts.hot_window_secs) {
        return Err(PermFail::Skip(r));
    }
    h.delete().map_err(|c| PermFail::Failed(format!("删除失败（Win32 错误 {c}）")))
}

/// 没处理的条目记账（停止时）：预检就跳过的照常记一行跳过，要删的记为“没处理”。
fn account_untouched(item: &PlanItem, idx: &[usize], sh: &Shared, part: &mut Part) {
    let mut n = CountBytes::default();
    for &i in idx {
        let (e, d) = (&item.entries[i], &item.decisions[i]);
        match d {
            Decision::Skip { reason } => log_preflight_skip(item, e, reason, sh, part),
            Decision::Delete | Decision::RemoveLink => n.add(e.bytes),
            Decision::Pending => {}
        }
    }
    if n.count > 0 {
        part.not_processed.merge(&n);
        let mut r = LogRecord::new(&sh.batch_id, "cancel", &item.path);
        r.item = item.path.clone();
        r.bytes = n.bytes;
        r.result = "skipped".into();
        r.reason = format!("已停止：{} 个没处理，原样没动", n.count);
        sh.opts.log.write(&r);
    }
}

/// 线程出错（panic）的块：这块里的文件可能已经处理了一部分，结果不确定——要删的记成失败（不说“原样没动”），
/// 预检就跳过的照常记账。返回不确定的个数。
fn account_uncertain(item: &PlanItem, idx: &[usize], sh: &Shared, part: &mut Part) -> usize {
    const WHY: &str = "程序内部出错，这一块的结果不确定（可能已经处理了）";
    let mut n = CountBytes::default();
    for &i in idx {
        let (e, d) = (&item.entries[i], &item.decisions[i]);
        match d {
            Decision::Skip { reason } => log_preflight_skip(item, e, reason, sh, part),
            Decision::Delete | Decision::RemoveLink => {
                n.add(e.bytes);
                push(part, er(e, "failed", WHY.into(), None, None, e.bytes), None);
            }
            Decision::Pending => {}
        }
    }
    if n.count > 0 {
        let mut r = LogRecord::new(&sh.batch_id, "unit_panic", &item.path);
        r.item = item.path.clone();
        r.bytes = n.bytes;
        r.result = "failed".into();
        r.reason = format!("{WHY}：{} 个", n.count);
        sh.opts.log.write(&r);
    }
    n.count
}

fn log_preflight_skip(item: &PlanItem, e: &Entry, reason: &SkipReason, sh: &Shared, part: &mut Part) {
    // 预检时就跳过的（热文件 / 云 / 被占用 / …）也记一行，日志是完整的记录。
    let mut r = rec_for(sh, item, "skip", e);
    r.result = "skipped".into();
    r.reason = reason.text();
    sh.opts.log.write(&r);
    push(part, er(e, "skipped", reason.text(), None, None, e.bytes), Some(reason.code()));
}

fn skip_entry(item: &PlanItem, e: &Entry, op: &str, reason: SkipReason, sh: &Shared, part: &mut Part, cost: f64) {
    let mut r = rec_for(sh, item, op, e);
    r.result = "skipped".into();
    r.reason = reason.text();
    sh.opts.log.write(&r);
    push(part, er(e, "skipped", reason.text(), None, None, e.bytes), Some(reason.code()));
    sh.tick(0, cost);
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn run_files(item: &PlanItem, idx: &[usize], sh: &Shared, part: &mut Part, meas: &mut Measured) {
    let mut recycle: Vec<(&Entry, f64)> = Vec::new();
    // 暂存：上一个成功放进去的文件所在的文件夹（同一个文件夹里的下一个就不用再建 / 再查）
    let mut last_parent: Option<String> = None;
    let mut recycle_ms = 0.0;
    let mut stop_at: Option<usize> = None;
    for (k, &i) in idx.iter().enumerate() {
        let (e, d) = (&item.entries[i], &item.decisions[i]);
        if !matches!(d, Decision::Delete | Decision::RemoveLink) {
            if let Decision::Skip { reason } = d {
                log_preflight_skip(item, e, reason, sh, part);
            }
            continue;
        }
        if sh.cancelled() {
            stop_at = Some(k);
            break;
        }
        let t = Instant::now();
        let cost = sh.cost(item, e);
        let dest = sh.plan.entry_dest(item, e);
        if matches!(d, Decision::RemoveLink) {
            match reverify(e, sh) {
                Err(reason) => skip_entry(item, e, "remove_link", reason, sh, part, cost),
                Ok(_) => {
                    let (result, reason, stored, method, kind) = do_link(e, item, dest, sh);
                    let mut r = rec_for(sh, item, "remove_link", e);
                    r.result = result.clone();
                    r.reason = reason.clone();
                    r.dest = stored.clone();
                    r.method = method.into();
                    sh.opts.log.write(&r);
                    if result == "ok" {
                        part.links_removed += 1;
                    }
                    push(part, er(e, &result, reason, stored, (result == "ok").then_some(kind), 0), None);
                    sh.tick(0, cost);
                }
            }
            meas.permanent.0 += 1;
            meas.permanent.1 += ms(t);
            continue;
        }
        match dest {
            Dest::Permanent => {
                let mut r = rec_for(sh, item, "delete", e);
                r.method = "delete_disposition".into();
                match permanent(e, sh) {
                    Ok(()) => {
                        sh.opts.log.write(&r);
                        push(part, er(e, "ok", String::new(), None, Some(Dest::Permanent), e.bytes), None);
                        sh.tick(e.bytes, cost);
                    }
                    Err(PermFail::Skip(reason)) => skip_entry(item, e, "delete", reason, sh, part, cost),
                    Err(PermFail::Failed(why)) => {
                        r.result = "failed".into();
                        r.reason = why.clone();
                        sh.opts.log.write(&r);
                        push(part, er(e, "failed", why, None, None, e.bytes), None);
                        sh.tick(0, cost);
                    }
                }
                meas.permanent.0 += 1;
                meas.permanent.1 += ms(t);
            }
            Dest::Recycle => {
                match reverify(e, sh) {
                    Err(reason) => skip_entry(item, e, "recycle", reason, sh, part, cost),
                    Ok(now) => match sh.bin_refuse(&e.path, now.size) {
                        // 回收站放不下 / 关掉了：送进去会被 Windows 直接删掉，不送，原样不动
                        Some(why) => skip_entry(item, e, "recycle", SkipReason::RecycleUnavailable { why }, sh, part, cost),
                        None => recycle.push((e, cost)),
                    },
                }
                recycle_ms += ms(t);
                if recycle.len() >= CHUNK {
                    let t2 = Instant::now();
                    let n = flush_recycle(&mut recycle, item, sh, part);
                    meas.recycle.0 += n;
                    meas.recycle.1 += recycle_ms + ms(t2);
                    recycle_ms = 0.0;
                }
            }
            Dest::Quarantine => {
                let xvol = sh.plan.crosses_volume(&e.path);
                let mut copied = false;
                match reverify(e, sh) {
                    Err(reason) => skip_entry(item, e, "quarantine", reason, sh, part, cost),
                    Ok(now) if xvol && now.links > 1 => skip_entry(item, e, "quarantine", SkipReason::Hardlink { links: now.links }, sh, part, cost),
                    Ok(_) => {
                        let batch = sh.batch_dir.as_deref().unwrap_or_default();
                        let dst = quarantine::map_path(batch, &e.path);
                        let mut r = rec_for(sh, item, "quarantine", e);
                        r.dest = Some(dst.clone());
                        let parent = dst.rsplit_once('\\').map(|(p, _)| p.to_string());
                        let ready = parent.is_some() && parent == last_parent;
                        match quarantine::move_file_with(&e.path, &dst, Some(&sh.cancel_i32), ready) {
                            Ok(method) => {
                                last_parent = parent;
                                copied = method == "copy_verify_delete";
                                r.method = method.into();
                                add_manifest(sh, item, e, Some(dst.clone()), method, "");
                                sh.opts.log.write(&r);
                                push(part, er(e, "ok", String::new(), Some(dst), Some(Dest::Quarantine), e.bytes), None);
                                sh.tick(e.bytes, cost);
                            }
                            Err(why) if sh.cancelled() && why.starts_with("已停止") => {
                                // 停止中断了这个文件的复制：复制品已删、原文件没动，算“没处理”
                                r.result = "skipped".into();
                                r.reason = why;
                                r.method = "copy_cancelled".into();
                                sh.opts.log.write(&r);
                                part.not_processed.add(e.bytes);
                            }
                            Err(why) => {
                                r.result = "failed".into();
                                r.reason = why.clone();
                                sh.opts.log.write(&r);
                                push(part, er(e, "failed", why, None, None, e.bytes), None);
                                sh.tick(0, cost);
                            }
                        }
                    }
                }
                if xvol {
                    meas.xvol.0 += 1;
                    meas.xvol.1 += ms(t);
                    // 只按真拷过去的字节学吞吐（跳过 / 失败 / 停止中断的不算）
                    if copied {
                        meas.xvol.2 += e.size;
                    }
                } else {
                    meas.rename.0 += 1;
                    meas.rename.1 += ms(t);
                }
            }
        }
    }
    if let Some(k) = stop_at {
        // 已经校验过、还没送进回收站的：原样没动
        for (e, _) in recycle.drain(..) {
            part.not_processed.add(e.bytes);
        }
        account_untouched(item, &idx[k..], sh, part);
    } else if !recycle.is_empty() {
        let t2 = Instant::now();
        let n = flush_recycle(&mut recycle, item, sh, part);
        meas.recycle.0 += n;
        meas.recycle.1 += recycle_ms + ms(t2);
    }
}

fn flush_recycle(batch: &mut Vec<(&Entry, f64)>, item: &PlanItem, sh: &Shared, part: &mut Part) -> usize {
    if batch.is_empty() {
        return 0;
    }
    let paths: Vec<String> = batch.iter().map(|(e, _)| e.path.clone()).collect();
    let results = winops::recycle_items(&paths);
    for ((e, cost), r) in batch.iter().zip(results) {
        let mut rec = rec_for(sh, item, "recycle", e);
        rec.method = "ifileoperation".into();
        rec.dest = Some("回收站".into());
        match r {
            winops::RecycleOutcome::Recycled => {
                push(part, er(e, "ok", String::new(), Some("回收站".into()), Some(Dest::Recycle), e.bytes), None);
                sh.tick(e.bytes, *cost);
            }
            winops::RecycleOutcome::Deleted => {
                // 回收站没收（正常不会发生：放不下的已经分流）——它已经不在了，如实记成“直接删除”
                sh.nuked.fetch_add(1, Ordering::Relaxed);
                rec.method = "ifileoperation_nuked".into();
                rec.dest = None;
                rec.reason = NUKED_TEXT.into();
                push(part, er(e, "ok", NUKED_TEXT.into(), None, None, e.bytes), None);
                part.by_dest.nuked.add(e.bytes);
                sh.tick(e.bytes, *cost);
            }
            winops::RecycleOutcome::Failed { msg, .. } => {
                // 还在原处：多半是预检之后又被程序占用了（例如正在运行的 Chromium 缓存）——自己探测一次，被占用就记“跳过·被占用”
                let skip = match winops::delete_probe(&e.path, false) {
                    Err(c) if winops::is_sharing_violation(c) => Some(SkipReason::Locked { processes: vec![] }),
                    Err(2) | Err(3) => Some(SkipReason::Missing),
                    _ => None,
                };
                rec.dest = None;
                match skip {
                    Some(reason) => {
                        rec.result = "skipped".into();
                        rec.reason = format!("{}（{msg}）", reason.text());
                        push(part, er(e, "skipped", reason.text(), None, None, e.bytes), Some(reason.code()));
                    }
                    None => {
                        rec.result = "failed".into();
                        rec.reason = msg.clone();
                        push(part, er(e, "failed", msg, None, None, e.bytes), None);
                    }
                }
                sh.tick(0, *cost);
            }
        }
        sh.opts.log.write(&rec);
    }
    let n = batch.len();
    batch.clear();
    n
}

/// 回收站没收、被 Windows 直接删掉时的说明。
const NUKED_TEXT: &str = "回收站没收，Windows 直接删除了";

/// 递归列目录，收集 相对路径（小写）→ (是不是文件夹, 大小, 修改时间 Unix 秒)。遇到链接 / 云文件 / 列不出 → false。
fn list_tree(dir: &str, rel: &str, found: &mut HashMap<String, (bool, u64, i64)>) -> bool {
    use crate::fs::attrs;
    let Ok(list) = winops::list_dir(dir) else { return false };
    for r in list {
        if r.attrs & attrs::REPARSE_POINT != 0 || r.attrs & attrs::CLOUD_PLACEHOLDER_MASK != 0 {
            return false;
        }
        let relp = if rel.is_empty() { r.name.clone() } else { format!("{rel}{SEP}{}", r.name) };
        let is_dir = r.attrs & attrs::DIRECTORY != 0;
        found.insert(relp.to_lowercase(), (is_dir, r.size, r.mtime));
        if is_dir && !list_tree(&crate::fs::join(dir, &r.name), &relp, found) {
            return false;
        }
    }
    true
}

const SEP: char = std::path::MAIN_SEPARATOR;

/// 整夹回收（见 [`DIR_RECYCLE`]）。条件不满足返回 false（什么都没动，调用方逐个文件处理）。
fn try_recycle_dir(item: &PlanItem, dir: &str, idx: &[usize], subdirs: &[String], sh: &Shared, part: &mut Part, meas: &mut Measured) -> bool {
    let t = Instant::now();
    if sh.protector.entry_blocked(dir).is_some() {
        return false;
    }
    // 逐条重新校验（护栏 9）：任何一条不过就整个退回逐个文件；停止了也马上退回（逐个文件那边会把剩下的记成“没处理”）
    let pre = dir.len() + 1;
    let mut expected: HashMap<String, (bool, u64, i64)> = HashMap::with_capacity(idx.len() + subdirs.len());
    for &i in idx {
        if sh.cancelled() {
            return false;
        }
        let e = &item.entries[i];
        let Ok(fp) = reverify(e, sh) else { return false };
        let Some(r) = e.path.get(pre..) else { return false };
        expected.insert(r.to_lowercase(), (false, fp.size, winops::ft_to_unix(fp.mtime_ft)));
    }
    for s in subdirs {
        match s.get(pre..) {
            Some(r) => expected.insert(r.to_lowercase(), (true, 0, 0)),
            None => return false,
        };
    }
    // 重新列一遍：名字集合必须和清单完全一样（多一个少一个都不行），每个文件的大小 / 修改时间也要和刚才校验时一样
    // （校验完到送进回收站之间又被写过的文件——护栏 4——不能跟着整夹走）
    let mut found = HashMap::with_capacity(expected.len());
    if !list_tree(dir, "", &mut found) || found.len() != expected.len() {
        return false;
    }
    for (k, (is_dir, size, mtime)) in &found {
        match expected.get(k) {
            Some((true, _, _)) if *is_dir => {}
            Some((false, s, m)) if !*is_dir && s == size && m == mtime => {}
            _ => return false,
        }
    }
    // 整个文件夹比回收站上限还大 / 回收站关了：不整夹送（逐个文件时每个再单独判断）
    let size: u64 = idx.iter().map(|&i| item.entries[i].size).sum();
    if sh.bin_refuse(dir, size).is_some() {
        return false;
    }
    if sh.cancelled() {
        return false;
    }
    let r = winops::recycle_items(&[dir.to_string()]);
    let nuked = match r.first() {
        Some(winops::RecycleOutcome::Recycled) => false,
        Some(winops::RecycleOutcome::Deleted) => true,
        _ => return false,
    };
    if nuked {
        sh.nuked.fetch_add(idx.len(), Ordering::Relaxed);
    }
    let mut bytes = 0u64;
    for &i in idx {
        let e = &item.entries[i];
        bytes += e.bytes;
        let mut rec = rec_for(sh, item, "recycle", e);
        if nuked {
            rec.method = "ifileoperation_dir_nuked".into();
            rec.reason = NUKED_TEXT.into();
            push(part, er(e, "ok", NUKED_TEXT.into(), None, None, e.bytes), None);
            part.by_dest.nuked.add(e.bytes);
        } else {
            rec.method = "ifileoperation_dir".into();
            rec.dest = Some("回收站".into());
            push(part, er(e, "ok", String::new(), Some("回收站".into()), Some(Dest::Recycle), e.bytes), None);
        }
        sh.opts.log.write(&rec);
        sh.tick(e.bytes, sh.cost(item, e));
    }
    let mut rd = LogRecord::new(&sh.batch_id, "recycle_dir", dir);
    rd.item = item.path.clone();
    rd.source = item.rule_id.clone().unwrap_or_else(|| item.source_label.clone());
    rd.tier = tier_str(item.tier).into();
    rd.bytes = bytes;
    rd.method = "ifileoperation_dir".into();
    rd.dest = Some("回收站".into());
    rd.reason = format!("整个文件夹进回收站：{} 个文件、{} 个子文件夹（上面逐个记了）", idx.len(), subdirs.len());
    sh.opts.log.write(&rd);
    part.dirs_removed += 1 + subdirs.len();
    meas.recycle.0 += idx.len();
    meas.recycle.1 += ms(t);
    true
}

/// 清单项收尾：只删变空的子文件夹（最深的先删），清单项本身保留；保存 manifest。返回删掉的个数。
fn finalize_item(item: &PlanItem, sh: &Shared, meas: &mut Measured) -> usize {
    let mut n = 0;
    for dir in &item.dirs {
        // 只删真正的空目录：目录如果在生成清单后被换成了链接，RemoveDirectoryW 也只会删链接本身。
        // 整夹回收掉的目录已经不在了，列不出来，直接跳过。
        match winops::list_dir(dir) {
            Ok(list) if list.is_empty() => {}
            _ => continue,
        }
        if sh.protector.entry_blocked(dir).is_some() {
            continue;
        }
        let t = Instant::now();
        let mut r = LogRecord::new(&sh.batch_id, "rmdir", dir);
        r.item = item.path.clone();
        r.source = item.rule_id.clone().unwrap_or_else(|| item.source_label.clone());
        r.tier = tier_str(item.tier).into();
        r.method = "remove_directory".into();
        match winops::remove_dir(dir) {
            Ok(()) => n += 1,
            Err(c) => {
                r.result = "failed".into();
                r.reason = format!("删不掉空文件夹（Win32 错误 {c}）");
            }
        }
        sh.opts.log.write(&r);
        meas.rmdir.0 += 1;
        meas.rmdir.1 += ms(t);
    }
    n
}

/// 链接：同盘暂存 → 改名进暂存区（链接对象本身）；否则只删链接本身。
/// 返回 (结果, 说明, 暂存位置, 方法, 实际去向)。
fn do_link(e: &Entry, item: &PlanItem, dest: Dest, sh: &Shared) -> (String, String, Option<String>, &'static str, Dest) {
    if let (Dest::Quarantine, Some(batch)) = (dest, sh.batch_dir.as_deref()) {
        let dst = quarantine::map_path(batch, &e.path);
        if let Some((parent, _)) = dst.rsplit_once('\\') {
            let _ = std::fs::create_dir_all(crate::win::long_path(parent));
        }
        if winops::rename(&e.path, &dst).is_ok() {
            add_manifest(sh, item, e, Some(dst.clone()), "rename", "链接自身被移进暂存区，指向的内容没动");
            return ("ok".into(), "链接自身移进暂存区，指向的内容没动".into(), Some(dst), "rename_link", Dest::Quarantine);
        }
    }
    let r = if e.link_is_dir { winops::remove_dir(&e.path) } else { winops::delete_file(&e.path) };
    match r {
        Ok(()) => {
            if sh.plan.dest_for(item) == Dest::Quarantine && sh.batch_dir.is_some() {
                add_manifest(sh, item, e, None, "link_removed", "链接本身不占空间，跨盘无法移动，已直接删除链接；指向的内容没动");
            }
            (
                "ok".into(),
                "只删了链接本身（链接不占空间，直接删除，不进回收站或暂存区），它指向的内容没动".into(),
                None,
                if e.link_is_dir { "remove_directory_link" } else { "delete_file_link" },
                Dest::Permanent,
            )
        }
        Err(c) => ("failed".into(), format!("删不掉链接（Win32 错误 {c}）"), None, "remove_link", Dest::Permanent),
    }
}

fn add_manifest(sh: &Shared, item: &PlanItem, e: &Entry, stored: Option<String>, method: &str, note: &str) {
    if let Ok(mut g) = sh.manifest.lock() {
        if let Some(m) = g.as_mut() {
            let me = ManifestEntry {
                original: e.path.clone(),
                stored,
                kind: e.kind,
                size: e.size,
                bytes: e.bytes,
                mtime: e.mtime,
                quarantined_at: crate::explain::now_unix(),
                item: item.path.clone(),
                source: item.rule_id.clone().unwrap_or_else(|| item.source_label.clone()),
                tier: item.tier,
                method: method.into(),
                status: "stored".into(),
                note: note.into(),
            };
            // 先写 journal（不缓冲，程序随时被结束也不丢），再进内存里的 manifest（结束时整份写一次）
            if let Some(j) = &sh.journal {
                j.append(&me);
            }
            m.entries.push(me);
        }
    }
}
