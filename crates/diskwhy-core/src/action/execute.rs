//! 执行：只处理预检标记为“删 / 只删链接”的条目。
//!
//! - 每条执行前再取一次指纹（护栏 9）：文件 ID、大小、修改时间任何一项和生成清单时不同就跳过；
//!   同时再查一次保护路径、再看一次是不是 30 分钟内改过；
//! - 回收站：每个工作线程进入 STA，用 `IFileOperation` 成批送进回收站（每批最多 256 个），
//!   之后逐个确认原路径已经不在了才算成功；
//! - 隔离区：`MoveFileExW` 同卷改名，跨卷复制 → 逐字节校验 → 删除；写 `manifest.json`；
//! - 链接：隔离且同卷时把链接自身改名进隔离区；否则 `RemoveDirectoryW` / `DeleteFileW` 只删链接本身；
//! - 清单项里的子文件夹在里面的东西处理完之后，**只删空的**（最深的先删），清单项本身的文件夹保留；
//! - 最多 4 个清单项并发；某条失败不回滚已经成功的，报告里写清楚；
//! - 每条都写日志。

use super::log::{ActionLog, LogRecord};
use super::quarantine::{self, Manifest, ManifestEntry};
use super::winops::{self, ComSta};
use super::{new_id, Decision, Entry, EntryKind, Plan, PlanItem, SkipReason, Strategy, HOT_WINDOW_SECS, MAX_WORKERS, PREVIEW_LIMIT};
use super::protect::Protector;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

/// 执行选项。
pub struct ExecOptions<'a> {
    pub log: &'a ActionLog,
    /// false 时（用户在预览里选了“取消”而不是“跳过被占用的”），有被占用的文件就整个不执行。
    pub skip_locked: bool,
    pub cancel: Option<&'a AtomicBool>,
    pub hot_window_secs: i64,
    pub workers: usize,
}

impl<'a> ExecOptions<'a> {
    pub fn new(log: &'a ActionLog) -> Self {
        ExecOptions { log, skip_locked: true, cancel: None, hot_window_secs: HOT_WINDOW_SECS, workers: MAX_WORKERS }
    }
}

/// 进度。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecProgress {
    pub done: usize,
    pub total: usize,
    pub bytes_done: u64,
    pub current: String,
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
}

struct Shared<'a> {
    strategy: &'a Strategy,
    batch_id: String,
    batch_dir: Option<String>,
    manifest: Mutex<Option<Manifest>>,
    manifest_path: Option<std::path::PathBuf>,
    protector: &'a Protector,
    opts: &'a ExecOptions<'a>,
    done: AtomicUsize,
    bytes_done: AtomicU64,
    total: usize,
}

/// 执行计划。`progress` 在工作线程里被调用（要求 Send + Sync）。
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
    let started_at = crate::explain::now_unix();
    let batch_id = new_id("");
    let (batch_dir, manifest, manifest_path) = match &plan.strategy {
        Strategy::RecycleBin => (None, None, None),
        Strategy::Quarantine { dest_root } => {
            if let Some(why) = super::preflight::quarantine_dest_problem(dest_root, plan) {
                return Err(why);
            }
            let dir = quarantine::batch_dir(dest_root, &batch_id);
            std::fs::create_dir_all(crate::win::long_path(&dir)).map_err(|e| format!("建不了隔离区文件夹 {dir}：{e}"))?;
            let mp = std::path::PathBuf::from(&dir).join(quarantine::MANIFEST);
            let m = Manifest::new(&batch_id, dest_root, &dir);
            m.save(&mp)?;
            (Some(dir), Some(m), Some(mp))
        }
    };
    let total: usize = plan.items.iter().filter(|i| i.blocked.is_none()).map(|i| i.entry_count()).sum();
    let shared = Shared {
        strategy: &plan.strategy,
        batch_id: batch_id.clone(),
        batch_dir,
        manifest: Mutex::new(manifest),
        manifest_path: manifest_path.clone(),
        protector,
        opts,
        done: AtomicUsize::new(0),
        bytes_done: AtomicU64::new(0),
        total,
    };
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, ItemResult)>> = Mutex::new(Vec::new());
    let workers = opts.workers.clamp(1, MAX_WORKERS).min(plan.items.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                let _com = matches!(plan.strategy, Strategy::RecycleBin).then(ComSta::init);
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= plan.items.len() {
                        break;
                    }
                    let r = run_item(&plan.items[i], &shared, &progress);
                    results.lock().unwrap().push((i, r));
                }
            });
        }
    });
    let mut items = results.into_inner().unwrap();
    items.sort_by_key(|x| x.0);
    let items: Vec<ItemResult> = items.into_iter().map(|x| x.1).collect();

    // 文件已经动过了：这里保存失败也不能返回 Err（否则界面会以为什么都没做），记进报告
    let mut manifest_error = None;
    if let (Some(mp), Some(m)) = (&manifest_path, shared.manifest.lock().unwrap().as_ref()) {
        if let Err(e) = m.save(mp) {
            manifest_error = Some(format!("隔离区清单保存失败：{e}。已移走的文件仍在批次文件夹里，按原路径映射可以手动放回。"));
        }
    }
    let mut rep = ExecReport {
        batch_id: batch_id.clone(),
        strategy_label: plan.strategy.label(),
        started_at,
        finished_at: crate::explain::now_unix(),
        ok: items.iter().map(|i| i.ok).sum(),
        ok_bytes: items.iter().map(|i| i.ok_bytes).sum(),
        skipped: items.iter().map(|i| i.skipped).sum(),
        failed: items.iter().map(|i| i.failed).sum(),
        manifest_path: manifest_path.map(|p| p.to_string_lossy().to_string()),
        log_path: opts.log.path().to_string_lossy().to_string(),
        log_failures: 0,
        cancelled: opts.cancel.is_some_and(|c| c.load(Ordering::SeqCst)),
        affected_dirs: plan
            .items
            .iter()
            .filter(|i| i.blocked.is_none())
            .map(|i| {
                if i.is_dir {
                    i.path.clone()
                } else {
                    i.path.rsplit_once('\\').map(|(p, _)| if p.len() == 2 { format!("{p}\\") } else { p.to_string() }).unwrap_or_default()
                }
            })
            .collect(),
        items,
        notes: Vec::new(),
    };
    // 隔离区一个都没放进去：删掉空批次
    if rep.ok == 0 {
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
    if let Some(e) = manifest_error {
        rep.notes.push(e);
    }
    if matches!(plan.strategy, Strategy::RecycleBin) {
        rep.notes.push("文件在回收站里，右键回收站 → 还原，就能放回原处；清空回收站之后才会真正腾出空间。".into());
    } else {
        rep.notes.push(format!("隔离区保留 {} 天（不会自动删），可以在“隔离区”里整批还原或彻底清空。", quarantine::KEEP_DAYS));
    }
    let mut sum = LogRecord::new(&batch_id, "summary", &plan.id);
    sum.bytes = rep.ok_bytes;
    sum.method = plan.strategy.label();
    sum.result = if rep.failed == 0 { "ok".into() } else { "failed".into() };
    sum.reason = format!("成功 {}，跳过 {}，失败 {}", rep.ok, rep.skipped, rep.failed);
    opts.log.write(&sum);
    rep.log_failures = opts.log.failures.load(Ordering::Relaxed);
    Ok(rep)
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

fn push(res: &mut ItemResult, er: EntryResult) {
    match er.result.as_str() {
        "ok" => {
            res.ok += 1;
            res.ok_bytes += er.bytes;
            if res.details.iter().filter(|d| d.result == "ok").count() < 50 {
                res.details.push(er);
            }
        }
        "skipped" => {
            res.skipped += 1;
            if res.details.len() < PREVIEW_LIMIT + 50 {
                res.details.push(er);
            }
        }
        _ => {
            res.failed += 1;
            if res.details.len() < PREVIEW_LIMIT + 50 {
                res.details.push(er);
            }
        }
    }
}

/// 护栏 9：重新校验。返回 Some(原因) 表示要跳过。
fn reverify(e: &Entry, sh: &Shared) -> Option<SkipReason> {
    if let Some(why) = sh.protector.entry_blocked(&e.path) {
        return Some(SkipReason::Protected { why });
    }
    let Some(old) = e.fp else {
        return Some(SkipReason::Changed { what: "生成清单时没取到文件信息".into() });
    };
    let now = match winops::fingerprint(&e.path, true) {
        Ok(f) => f,
        Err(2) | Err(3) => return Some(SkipReason::Missing),
        Err(c) => return Some(SkipReason::Unreadable { code: c }),
    };
    if now.volume != old.volume || now.id != old.id {
        return Some(SkipReason::Changed { what: "文件 ID 变了（被替换成了另一个文件）".into() });
    }
    if e.kind == EntryKind::File {
        if now.size != old.size {
            return Some(SkipReason::Changed { what: format!("大小从 {} 字节变成了 {} 字节", old.size, now.size) });
        }
        if now.mtime_ft != old.mtime_ft {
            return Some(SkipReason::Changed { what: "修改时间变了".into() });
        }
        let age = crate::explain::now_unix() - winops::ft_to_unix(now.mtime_ft);
        if age < sh.opts.hot_window_secs {
            return Some(SkipReason::Hot { minutes_ago: age.max(0) / 60 });
        }
    }
    None
}

fn run_item<F: Fn(&ExecProgress) + Send + Sync>(item: &PlanItem, sh: &Shared, progress: &F) -> ItemResult {
    let mut res = ItemResult { path: item.path.clone(), blocked: item.blocked.clone(), ..Default::default() };
    if item.blocked.is_some() {
        return res;
    }
    // 整项再过一次保护路径
    if let Some(why) = sh.protector.check(&item.path) {
        res.blocked = Some(why);
        return res;
    }
    let tick = |bytes: u64, cur: &str| {
        let done = sh.done.fetch_add(1, Ordering::Relaxed) + 1;
        let b = sh.bytes_done.fetch_add(bytes, Ordering::Relaxed) + bytes;
        if done % 50 == 0 || done == sh.total {
            progress(&ExecProgress { done, total: sh.total, bytes_done: b, current: cur.to_string() });
        }
    };
    let cancelled = || sh.opts.cancel.is_some_and(|c| c.load(Ordering::SeqCst));
    let mut recycle_batch: Vec<&Entry> = Vec::new();

    for (e, d) in item.entries.iter().zip(&item.decisions) {
        if !matches!(d, Decision::Delete | Decision::RemoveLink) {
            // 预检时就跳过的（热文件 / 云 / 被占用 / …）也记一行，日志是完整的记录。
            if let Decision::Skip { reason } = d {
                let mut r = rec_for(sh, item, "skip", e);
                r.result = "skipped".into();
                r.reason = reason.text();
                sh.opts.log.write(&r);
                push(&mut res, EntryResult { path: e.path.clone(), kind: e.kind, bytes: e.bytes, result: "skipped".into(), reason: reason.text(), dest: None });
            }
            continue;
        }
        if cancelled() {
            let er = EntryResult { path: e.path.clone(), kind: e.kind, bytes: e.bytes, result: "skipped".into(), reason: "已取消".into(), dest: None };
            let mut r = rec_for(sh, item, "cancel", e);
            r.result = "skipped".into();
            r.reason = "已取消".into();
            sh.opts.log.write(&r);
            push(&mut res, er);
            continue;
        }
        let op = match (d, sh.strategy) {
            (Decision::RemoveLink, _) => "remove_link",
            (_, Strategy::RecycleBin) => "recycle",
            (_, Strategy::Quarantine { .. }) => "quarantine",
        };
        if let Some(reason) = reverify(e, sh) {
            let mut r = rec_for(sh, item, op, e);
            r.result = "skipped".into();
            r.reason = reason.text();
            sh.opts.log.write(&r);
            push(&mut res, EntryResult { path: e.path.clone(), kind: e.kind, bytes: e.bytes, result: "skipped".into(), reason: reason.text(), dest: None });
            tick(0, &e.path);
            continue;
        }
        match (d, sh.strategy) {
            (Decision::RemoveLink, _) => {
                let (result, reason, dest, method) = do_link(e, item, sh);
                let mut r = rec_for(sh, item, op, e);
                r.result = result.clone();
                r.reason = reason.clone();
                r.dest = dest.clone();
                r.method = method.into();
                sh.opts.log.write(&r);
                if result == "ok" {
                    res.links_removed += 1;
                }
                push(&mut res, EntryResult { path: e.path.clone(), kind: e.kind, bytes: 0, result, reason, dest });
                tick(0, &e.path);
            }
            (_, Strategy::RecycleBin) => {
                recycle_batch.push(e);
                if recycle_batch.len() >= 256 {
                    flush_recycle(&mut recycle_batch, item, sh, &mut res, &tick);
                }
            }
            (_, Strategy::Quarantine { .. }) => {
                let batch = sh.batch_dir.as_deref().unwrap_or_default();
                let dst = quarantine::map_path(batch, &e.path);
                let mut r = rec_for(sh, item, op, e);
                r.dest = Some(dst.clone());
                match quarantine::move_file(&e.path, &dst) {
                    Ok(method) => {
                        r.method = method.into();
                        add_manifest(sh, item, e, Some(dst.clone()), method, "");
                        push(&mut res, EntryResult { path: e.path.clone(), kind: e.kind, bytes: e.bytes, result: "ok".into(), reason: String::new(), dest: Some(dst) });
                        tick(e.bytes, &e.path);
                    }
                    Err(why) => {
                        r.result = "failed".into();
                        r.reason = why.clone();
                        push(&mut res, EntryResult { path: e.path.clone(), kind: e.kind, bytes: e.bytes, result: "failed".into(), reason: why, dest: None });
                        tick(0, &e.path);
                    }
                }
                sh.opts.log.write(&r);
            }
        }
    }
    flush_recycle(&mut recycle_batch, item, sh, &mut res, &tick);

    // 只删变空的子文件夹（最深的先删）；清单项本身保留。
    for dir in &item.dirs {
        if sh.protector.entry_blocked(dir).is_some() {
            continue;
        }
        // 只删真正的空目录：目录如果在生成清单后被换成了链接，RemoveDirectoryW 也只会删链接本身。
        if let Ok(list) = winops::list_dir(dir) {
            if !list.is_empty() {
                continue;
            }
        } else {
            continue;
        }
        let mut r = LogRecord::new(&sh.batch_id, "rmdir", dir);
        r.item = item.path.clone();
        r.source = item.rule_id.clone().unwrap_or_else(|| item.source_label.clone());
        r.tier = tier_str(item.tier).into();
        r.method = "remove_directory".into();
        match winops::remove_dir(dir) {
            Ok(()) => res.dirs_removed += 1,
            Err(c) => {
                r.result = "failed".into();
                r.reason = format!("删不掉空文件夹（Win32 错误 {c}）");
            }
        }
        sh.opts.log.write(&r);
    }
    if let (Some(mp), Ok(g)) = (&sh.manifest_path, sh.manifest.lock()) {
        if let Some(m) = g.as_ref() {
            let _ = m.save(mp);
        }
    }
    // 明细里失败、跳过的排在前面（稳定排序，同类保持原顺序）
    res.details.sort_by_key(|d| match d.result.as_str() {
        "failed" => 0,
        "skipped" => 1,
        _ => 2,
    });
    res
}

fn flush_recycle<T: Fn(u64, &str)>(batch: &mut Vec<&Entry>, item: &PlanItem, sh: &Shared, res: &mut ItemResult, tick: &T) {
    if batch.is_empty() {
        return;
    }
    let paths: Vec<String> = batch.iter().map(|e| e.path.clone()).collect();
    let results = winops::recycle(&paths);
    for (e, r) in batch.iter().zip(results) {
        let mut rec = rec_for(sh, item, "recycle", e);
        rec.method = "ifileoperation".into();
        rec.dest = Some("回收站".into());
        match r {
            Ok(()) => {
                push(res, EntryResult { path: e.path.clone(), kind: e.kind, bytes: e.bytes, result: "ok".into(), reason: String::new(), dest: Some("回收站".into()) });
                tick(e.bytes, &e.path);
            }
            Err(why) => {
                rec.result = "failed".into();
                rec.reason = why.clone();
                rec.dest = None;
                push(res, EntryResult { path: e.path.clone(), kind: e.kind, bytes: e.bytes, result: "failed".into(), reason: why, dest: None });
                tick(0, &e.path);
            }
        }
        sh.opts.log.write(&rec);
    }
    batch.clear();
}

/// 链接：隔离且同卷 → 改名进隔离区（链接对象本身）；否则只删链接本身。
fn do_link(e: &Entry, item: &PlanItem, sh: &Shared) -> (String, String, Option<String>, &'static str) {
    if let (Strategy::Quarantine { .. }, Some(batch)) = (sh.strategy, sh.batch_dir.as_deref()) {
        let dst = quarantine::map_path(batch, &e.path);
        if let Some((parent, _)) = dst.rsplit_once('\\') {
            let _ = std::fs::create_dir_all(crate::win::long_path(parent));
        }
        if winops::rename(&e.path, &dst).is_ok() {
            add_manifest(sh, item, e, Some(dst.clone()), "rename", "链接自身被移进隔离区，指向的内容没动");
            return ("ok".into(), "链接自身移进隔离区，指向的内容没动".into(), Some(dst), "rename_link");
        }
    }
    let r = if e.link_is_dir { winops::remove_dir(&e.path) } else { winops::delete_file(&e.path) };
    match r {
        Ok(()) => {
            if matches!(sh.strategy, Strategy::Quarantine { .. }) {
                add_manifest(sh, item, e, None, "link_removed", "链接本身不占空间，跨盘无法移动，已直接删除链接；指向的内容没动");
            }
            (
                "ok".into(),
                "只删了链接本身（链接不占空间，直接删除，不进回收站或隔离区），它指向的内容没动".into(),
                None,
                if e.link_is_dir { "remove_directory_link" } else { "delete_file_link" },
            )
        }
        Err(c) => ("failed".into(), format!("删不掉链接（Win32 错误 {c}）"), None, "remove_link"),
    }
}

fn add_manifest(sh: &Shared, item: &PlanItem, e: &Entry, stored: Option<String>, method: &str, note: &str) {
    if let Ok(mut g) = sh.manifest.lock() {
        if let Some(m) = g.as_mut() {
            m.entries.push(ManifestEntry {
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
            });
        }
    }
}
