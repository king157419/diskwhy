//! 阶段 D：带护栏的动作（清理清单 → 预检 → 执行 → 可撤销）。
//!
//! 流程：
//! 1. [`build_plan`]：GUI 的清理清单（只有路径）→ [`Plan`]。**档位、来源、规则都由后端用当前扫描结果和
//!    规则库重新算**，不信任前端传来的任何说明；被拦下的项不枚举内容。能进清单的项会重新枚举磁盘上的
//!    实际内容（扫描树里每个目录只留了前 50 个文件，不能拿来当删除清单），给每个文件记下指纹
//!    （文件 ID / 大小 / 修改时间）。
//! 2. [`preflight`]：逐项、逐条检查，给出阻止 / 跳过 / 警告（见 [`preflight`] 模块说明的 9 条护栏）。
//! 3. [`execute`]：只处理预检标记为“删”的条目；执行前每个条目再取一次指纹，对不上就跳过；
//!    去向按条目所属清单项决定（阶段 J，[`Plan::dest_for`]）：safe 档且 `safe_direct` → 直接删除
//!    （同一个句柄上重新校验后设删除标记）；其余按策略：回收站走 `IFileOperation`，暂存区走 `MoveFileExW`
//!    （跨卷：`CopyFileExW` 可中断 → 逐字节校验 → 删除），`Permanent` 直接删除；
//!    所有清单项的条目切成 256 条一块进同一个工作队列，`clamp(CPU 数, 4, 16)` 个线程；
//!    每条结果写进 `%LOCALAPPDATA%\DiskWhy\actions.log`（JSON lines）。失败不回滚已成功的项。
//! 4. 隔离区可以一键还原（[`quarantine::restore`]）或彻底清空（[`quarantine::purge`]）。
//!
//! `builtin_tool` / `migrate` 只给指引（[`guidance`]），**本阶段不自动运行任何系统命令**。

pub mod execute;
pub mod guidance;
pub mod log;
pub mod perf;
pub mod plan;
pub mod preflight;
pub mod protect;
pub mod quarantine;
pub mod refresh;
pub mod runstate;
pub mod winops;

pub use execute::{execute, ExecOptions, ExecProgress, ExecReport, ItemResult};
pub use perf::PerfModel;
pub use plan::{build_plan, build_plan_forced, build_plan_quick, build_plan_quick_forced, enumerate_plan};
pub use preflight::{preflight, preflight_with, quote, PreflightOptions, PreflightReport};

use crate::explain::{How, Source, Tier};
use serde::{Deserialize, Serialize};
use winops::{Fingerprint, LockingProcess};

/// 预览里每项最多列多少条。
pub const PREVIEW_LIMIT: usize = 200;
/// 30 分钟内修改过的文件视为“正在用”，跳过。
pub const HOT_WINDOW_SECS: i64 = 30 * 60;
/// 执行并发上限（阶段 J：所有条目切块进同一个队列，线程数 `clamp(CPU 数, 4, 16)`）。
pub const MAX_WORKERS: usize = 16;
/// 执行队列里一块的条目数。
pub const CHUNK: usize = 256;

/// 执行用几个线程：`clamp(CPU 数, 4, 16)`。
pub fn default_workers() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, MAX_WORKERS)
}
/// 单个清单项最多枚举多少条（超过就整项拒绝，让用户分批）。
pub const MAX_ENTRIES_PER_ITEM: usize = 500_000;
/// 回收站所在卷可用空间低于这个比例时提示“删了不会立刻腾空间”。
pub const LOW_SPACE_RATIO: f64 = 0.10;

/// 怎么删。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Strategy {
    /// 送进回收站（默认）。
    RecycleBin,
    /// 移到 `<dest_root>\DiskWhy-Quarantine\<时间戳>\<原路径映射>`，保留 7 天，可一键还原。
    Quarantine { dest_root: String },
    /// 直接删除（阶段 J）：不进回收站、不进暂存区。
    Permanent,
}

impl Strategy {
    pub fn label(&self) -> String {
        match self {
            Strategy::RecycleBin => "回收站".into(),
            Strategy::Quarantine { dest_root } => format!("暂存区（{dest_root}）"),
            Strategy::Permanent => "直接删除".into(),
        }
    }
}

/// 一个清单项实际去哪（阶段 J，见 [`Plan::dest_for`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dest {
    Permanent,
    Recycle,
    Quarantine,
}

/// 个数 + 字节。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountBytes {
    pub count: usize,
    pub bytes: u64,
}

impl CountBytes {
    pub fn add(&mut self, bytes: u64) {
        self.count += 1;
        self.bytes += bytes;
    }
    pub fn merge(&mut self, o: &CountBytes) {
        self.count += o.count;
        self.bytes += o.bytes;
    }
}

/// 按去向分别统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByDest {
    pub permanent: CountBytes,
    pub recycle: CountBytes,
    pub quarantine: CountBytes,
    /// 本该进回收站、却被 Windows 直接删掉的（回收站没收）。不算进 `permanent`，单独报告。
    #[serde(default)]
    pub nuked: CountBytes,
}

impl ByDest {
    pub fn get_mut(&mut self, d: Dest) -> &mut CountBytes {
        match d {
            Dest::Permanent => &mut self.permanent,
            Dest::Recycle => &mut self.recycle,
            Dest::Quarantine => &mut self.quarantine,
        }
    }
    pub fn merge(&mut self, o: &ByDest) {
        self.permanent.merge(&o.permanent);
        self.recycle.merge(&o.recycle);
        self.quarantine.merge(&o.quarantine);
        self.nuked.merge(&o.nuked);
    }
}

/// 清理清单里的一项（前端只传路径；`expect_rule` 是加入清单时看到的规则，用来发现“说明已经变了”）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    pub path: String,
    #[serde(default)]
    pub expect_rule: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    /// 联接 / 符号链接 / 挂载点：不跟随，只处理链接自身。
    Link,
    /// 云文件目录（OneDrive 等）：不进去，整个跳过。
    CloudDir,
}

/// 清单项里实际会被处理的一条（文件或链接）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub kind: EntryKind,
    pub attrs: u32,
    pub reparse_tag: u32,
    /// 逻辑大小。
    pub size: u64,
    /// 能腾出的字节：磁盘占用按簇取整；硬链接（还有别的名字）为 0。
    pub bytes: u64,
    pub mtime: i64,
    /// 生成清单时的指纹（云占位不打开，没有指纹）。
    pub fp: Option<Fingerprint>,
    /// 链接是否是目录型（联接 / 目录符号链接）。
    #[serde(default)]
    pub link_is_dir: bool,
}

/// 跳过的原因。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkipReason {
    /// 30 分钟内修改过。
    Hot { minutes_ago: i64 },
    /// 云占位 / 云文件（删了可能连云端一起删，或触发下载）。
    Cloud,
    /// 被进程占用（Restart Manager 报告，只报告不关进程）。
    Locked { processes: Vec<LockingProcess> },
    /// 没有删除权限。
    NoPermission { code: u32 },
    /// 受保护路径。
    Protected { why: String },
    /// 生成清单后被改过 / 替换过。
    Changed { what: String },
    /// 已经不在了。
    Missing,
    /// 读不到信息。
    Unreadable { code: u32 },
    /// 跨盘暂存时链接数 > 1 的文件（阶段 J）：整份拷到别的盘既占空间又不腾空间。
    Hardlink { links: u32 },
    /// 回收站收不下（比回收站上限还大 / 这个盘的回收站设成了直接删除）：送进去会被 Windows 直接永久删除，所以不送（阶段 J）。
    RecycleUnavailable { why: String },
    /// 有程序正从这个文件夹里运行（进程映像在里面）：整个文件夹不动，免得删掉一半。
    Running { processes: Vec<LockingProcess> },
    /// 按年龄挑的规则（min_age_days）：这一项里有文件是最近 N 天改的，整个子项先不删。
    Recent { days: u32 },
}

impl SkipReason {
    pub fn code(&self) -> &'static str {
        match self {
            SkipReason::Hot { .. } => "hot",
            SkipReason::Cloud => "cloud",
            SkipReason::Locked { .. } => "locked",
            SkipReason::NoPermission { .. } => "no_permission",
            SkipReason::Protected { .. } => "protected",
            SkipReason::Changed { .. } => "changed",
            SkipReason::Missing => "missing",
            SkipReason::Unreadable { .. } => "unreadable",
            SkipReason::Hardlink { .. } => "hardlink",
            SkipReason::RecycleUnavailable { .. } => "recycle_unavailable",
            SkipReason::Running { .. } => "running",
            SkipReason::Recent { .. } => "recent",
        }
    }

    pub fn text(&self) -> String {
        match self {
            SkipReason::Hot { minutes_ago } => format!("{minutes_ago} 分钟前刚改过（30 分钟内修改的文件可能正在用，跳过）"),
            SkipReason::Cloud => "云端文件 / 占位文件（OneDrive 等）：删了可能连云端的一起删，DiskWhy 不碰".into(),
            SkipReason::Locked { processes } if processes.is_empty() => "正在被别的程序占用（没查到是哪个进程）".into(),
            SkipReason::Locked { processes } => format!(
                "正在被占用：{}（DiskWhy 只报告，不会关掉它们）",
                processes.iter().map(|p| format!("{}（PID {}）", p.name, p.pid)).collect::<Vec<_>>().join("、")
            ),
            SkipReason::NoPermission { code } => format!("没有删除权限（Win32 错误 {code}）"),
            SkipReason::Protected { why } => why.clone(),
            SkipReason::Changed { what } => format!("生成清单之后被改过：{what}（防止删掉被替换的文件，跳过）"),
            SkipReason::Missing => "已经不在了".into(),
            SkipReason::Unreadable { code } => format!("读不到文件信息（Win32 错误 {code}）"),
            SkipReason::Hardlink { .. } => "硬链接，删了不腾空间".into(),
            SkipReason::RecycleUnavailable { why } => why.clone(),
            SkipReason::Running { processes } => format!(
                "正在运行：{}（整个文件夹不动，DiskWhy 不会关掉它们）",
                processes.iter().map(|p| format!("{}（PID {}）", p.name, p.pid)).collect::<Vec<_>>().join("、")
            ),
            SkipReason::Recent { days } => format!("最近 {days} 天内改过（只删 {days} 天以上没动过的，整个子文件夹先不删）"),
        }
    }
}

/// 预检对一条的决定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Decision {
    /// 还没预检。
    Pending,
    /// 删（进回收站 / 隔离区）。
    Delete,
    /// 链接：只删链接自身（同卷隔离时移动链接自身）。
    RemoveLink,
    Skip { reason: SkipReason },
}

/// 清单里的一项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub path: String,
    pub is_dir: bool,
    /// 扫描结果里的占用（给界面对照）。
    pub scan_allocated: u64,
    pub tier: Tier,
    /// 解释来源是 Unknown（界面上的灰色）。
    pub unknown: bool,
    pub source: Source,
    pub source_label: String,
    pub rule_id: Option<String>,
    pub what: String,
    pub owner: Option<String>,
    pub lose: String,
    pub how: Vec<How>,
    /// 被拦下的原因（档位 / 保护路径 / 规则不一致 / …）。拦下的项不会执行。
    pub blocked: Option<String>,
    /// 全部条目（文件 / 链接 / 云目录）。
    pub entries: Vec<Entry>,
    /// 与 `entries` 一一对应。
    pub decisions: Vec<Decision>,
    /// 子文件夹（最深的在前）；里面的东西处理完变空了就删掉。清单项本身的文件夹保留。
    pub dirs: Vec<String>,
    /// 枚举时出的错（列不出的子目录等）。
    pub enum_errors: Vec<String>,
    /// 所在卷的簇大小（预检取到指纹后按它把磁盘占用取整）。
    #[serde(default = "default_cluster")]
    pub cluster: u64,
    /// 所在卷是 NTFS（常驻 MFT 的小文件删了不腾空间，见 [`crate::fs::cluster_bytes`]）。
    #[serde(default)]
    pub ntfs: bool,
    /// 由“旧版本”（keep_latest）规则说明时：那个旧版本文件夹（这一项本身，或它所在的上层版本文件夹）。
    /// 预检和执行开始时，这个文件夹里只要有文件被占用 / 刚改过，整个跳过（不会只删一半，见
    /// [`preflight::skip_busy_versions`]）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_root: Option<String>,
    /// 由按年龄挑的规则（min_age_days）说明时：规则命中的那个文件夹（比如 %TEMP%）和天数。它下面每个直接子项是一个
    /// 单元：子项里有一个文件 N 天内改过 / 被占用 / 有程序在里面运行，整个子项跳过。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_age_days: Option<u32>,
    /// 商店应用重定向区里的项：(LocalCache 里的前缀, 程序看到的 AppData 前缀)。程序从重定向区运行时，
    /// 进程映像路径是后者，判断“有没有程序在里面运行”时两种写法都要看。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msix_alias: Option<(String, String)>,
    /// 用户二次确认后强制加入的（keep / 认不出来 / 规则说不建议直接删的）：档位门槛和规则一致性不拦，
    /// 保护路径、节点本身（系统保留 / 映射 / 读不了 / 云占位）、热文件、占用等其余护栏照旧。
    /// 这种项**永远不直接删**：去回收站或暂存区（见 [`Plan::dest_for`]），找得回来。
    #[serde(default)]
    pub forced: bool,
}

impl PlanItem {
    /// 同一个路径在商店应用重定向下的另一种写法（没有就是 None）。
    pub fn alias_of(&self, path: &str) -> Option<String> {
        let (real, alias) = self.msix_alias.as_ref()?;
        let (pl, rl) = (path.to_lowercase(), real.to_lowercase());
        if pl == rl {
            return Some(alias.clone());
        }
        let rest = pl.strip_prefix(&format!("{}\\", rl.trim_end_matches('\\')))?;
        let n = path.chars().count() - rest.chars().count();
        Some(format!("{}\\{}", alias.trim_end_matches('\\'), path.chars().skip(n).collect::<String>()))
    }
}

fn default_cluster() -> u64 {
    4096
}

/// 计划 + 预检的进度（阶段 G）。各阶段在工作线程里更新这些计数，界面层自己定时读（不在热路径上发消息）；
/// `cancel` 置位后枚举 / 预检尽快停下，计划不会标记为已预检，不能执行。
#[derive(Debug, Default)]
pub struct Progress {
    /// 0 = 生成清单，1 = 列文件，2 = 逐个检查，3 = 问是哪些程序在占用，4 = 完成
    pub phase: std::sync::atomic::AtomicU8,
    pub dirs: std::sync::atomic::AtomicUsize,
    pub entries: std::sync::atomic::AtomicUsize,
    pub checked: std::sync::atomic::AtomicUsize,
    pub to_check: std::sync::atomic::AtomicUsize,
    pub locked: std::sync::atomic::AtomicUsize,
    pub rm_done: std::sync::atomic::AtomicUsize,
    pub rm_total: std::sync::atomic::AtomicUsize,
    pub cancel: std::sync::atomic::AtomicBool,
}

/// [`Progress`] 某一刻的读数（发给界面）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProgressView {
    /// `plan` / `list` / `check` / `who` / `done`
    pub phase: String,
    pub dirs: usize,
    pub entries: usize,
    pub checked: usize,
    pub to_check: usize,
    pub locked: usize,
    pub rm_done: usize,
    pub rm_total: usize,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn set_phase(&self, p: u8) {
        self.phase.store(p, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn view(&self) -> ProgressView {
        use std::sync::atomic::Ordering::Relaxed;
        ProgressView {
            phase: match self.phase.load(Relaxed) {
                0 => "plan",
                1 => "list",
                2 => "check",
                3 => "who",
                _ => "done",
            }
            .into(),
            dirs: self.dirs.load(Relaxed),
            entries: self.entries.load(Relaxed),
            checked: self.checked.load(Relaxed),
            to_check: self.to_check.load(Relaxed),
            locked: self.locked.load(Relaxed),
            rm_done: self.rm_done.load(Relaxed),
            rm_total: self.rm_total.load(Relaxed),
        }
    }
}

impl PlanItem {
    /// 会被删掉的条目（预检之后才准）。
    pub fn will_delete(&self) -> impl Iterator<Item = (&Entry, &Decision)> {
        self.entries.iter().zip(self.decisions.iter()).filter(|(_, d)| matches!(d, Decision::Delete | Decision::RemoveLink))
    }
    pub fn entry_count(&self) -> usize {
        self.will_delete().count()
    }
    pub fn bytes(&self) -> u64 {
        self.will_delete().map(|(e, _)| e.bytes).sum()
    }
    /// 前 N 条实际会删的文件。
    pub fn entries_preview(&self, n: usize) -> Vec<EntryView> {
        self.will_delete().take(n).map(|(e, d)| EntryView::of(e, d)).collect()
    }
}

/// 一次清理的计划。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub created_at: i64,
    pub scan_root: String,
    pub strategy: Strategy,
    pub items: Vec<PlanItem>,
    /// 计划层面的提示（合并了重复项等）。
    pub warnings: Vec<String>,
    /// 预检做过了没有（没做过不许执行）。
    pub preflight_done: bool,
    /// 阶段 J：safe 档（可放心清理，会自动重建）直接删除，不管 `strategy`；false 时 safe 档也按 `strategy`
    /// （用户明确选了“回收站”时前端传 false）。默认 true。
    #[serde(default = "yes")]
    pub safe_direct: bool,
}

fn yes() -> bool {
    true
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().filter(|i| i.blocked.is_none()).map(|i| i.bytes()).sum()
    }

    /// 这一项的去向：safe 档且 `safe_direct` → 直接删除；否则按策略。9 条护栏和去向无关，都在去向之前执行。
    pub fn dest_for(&self, item: &PlanItem) -> Dest {
        // 强制加入的项不管档位、不管策略，都不直接删（策略是“直接删除”时也进回收站）
        if item.forced {
            return match self.strategy {
                Strategy::Quarantine { .. } => Dest::Quarantine,
                _ => Dest::Recycle,
            };
        }
        if item.tier == Tier::Safe && self.safe_direct {
            return Dest::Permanent;
        }
        match self.strategy {
            Strategy::RecycleBin => Dest::Recycle,
            Strategy::Quarantine { .. } => Dest::Quarantine,
            Strategy::Permanent => Dest::Permanent,
        }
    }

    /// 这次实际用了哪些方式（写进动作日志的 `method` 和 last-run.json）：全是直接删除时写“直接删除”，
    /// 可放心清理直接删、需你判断按策略时两段都写；否则就是策略本身。
    pub fn method_label(&self) -> String {
        let live = || self.items.iter().filter(|i| i.blocked.is_none() && (!self.preflight_done || i.will_delete().next().is_some()));
        let safe_direct = live().any(|i| self.dest_for(i) == Dest::Permanent && i.tier == Tier::Safe);
        let other = live().any(|i| self.dest_for(i) != Dest::Permanent);
        match (safe_direct, other, &self.strategy) {
            (true, false, _) | (_, _, Strategy::Permanent) => "直接删除".into(),
            (true, true, s) => format!("可放心清理直接删除，需你判断{}", s.label()),
            (false, _, s) => s.label(),
        }
    }

    /// 暂存区根目录（策略是暂存时）。
    pub fn quarantine_root(&self) -> Option<&str> {
        match &self.strategy {
            Strategy::Quarantine { dest_root } => Some(dest_root),
            _ => None,
        }
    }

    /// 某个条目进暂存区是否跨盘（盘符不同）。
    pub fn crosses_volume(&self, entry_path: &str) -> bool {
        match self.quarantine_root() {
            Some(d) => !same_volume(d, entry_path),
            None => false,
        }
    }

    /// 某一条（预检后的决定）实际去哪：链接在同盘暂存时整体改名进暂存区，其余链接直接删（算“直接删除”）。
    pub fn entry_dest(&self, item: &PlanItem, e: &Entry) -> Dest {
        let d = self.dest_for(item);
        if e.kind == EntryKind::Link && !(d == Dest::Quarantine && !self.crosses_volume(&e.path)) {
            return Dest::Permanent;
        }
        d
    }
}

/// 两个路径是否在同一个盘符上（按 `X:` 比较；挂载点之类真的跨卷时 `MoveFileExW` 会报 NOT_SAME_DEVICE，照样改走复制）。
pub fn same_volume(a: &str, b: &str) -> bool {
    match (crate::scan::volume_root_of(a), crate::scan::volume_root_of(b)) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(&y),
        _ => false,
    }
}

/// 给界面看的一条。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryView {
    pub path: String,
    pub kind: EntryKind,
    pub bytes: u64,
    pub mtime: i64,
    /// 附注（“链接：只删链接本身”“硬链接，删了不腾空间”）。
    pub note: Option<String>,
}

impl EntryView {
    pub fn of(e: &Entry, d: &Decision) -> Self {
        let note = match (e.kind, d) {
            (EntryKind::Link, _) => Some(format!(
                "这是{}：不跟进去，只删链接本身，它指向的内容不受影响",
                if e.link_is_dir { "目录联接 / 目录链接" } else { "文件符号链接" }
            )),
            _ if e.fp.is_some_and(|f| f.links > 1) => Some(format!("硬链接（共 {} 个名字）：删掉这个名字不腾空间", e.fp.unwrap().links)),
            _ => None,
        };
        EntryView { path: e.path.clone(), kind: e.kind, bytes: e.bytes, mtime: e.mtime, note }
    }
}

/// 生成一个短 id（时间 + 计数）。
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{prefix}{}-{:x}{:02x}", fmt_stamp(crate::explain::now_unix()), (nanos % 0xFFFF) as u32, n % 256)
}

/// Unix 秒 → `YYYYMMDD-HHMMSS`（本地时区不可靠地取不到，统一用 UTC，并在界面上标注）。
pub fn fmt_stamp(unix: i64) -> String {
    let (y, m, d, hh, mm, ss) = civil(unix);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// Unix 秒 → ISO 8601（UTC）。
pub fn fmt_iso(unix: i64) -> String {
    let (y, m, d, hh, mm, ss) = civil(unix);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Howard Hinnant 的 civil_from_days。
fn civil(unix: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, (secs / 3600) as u32, ((secs % 3600) / 60) as u32, (secs % 60) as u32)
}

#[cfg(test)]
mod tests {
    #[test]
    fn stamps() {
        assert_eq!(super::fmt_iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::fmt_iso(1_790_186_441), "2026-09-23T18:00:41Z");
        assert_eq!(super::fmt_stamp(951_782_400), "20000229-000000");
    }
}
