//! 隔离区（界面上叫“暂存区”）：`<dest_root>\DiskWhy暂存区\<时间戳>\<原路径映射>` + `manifest.json`
//! （阶段 J 之前的批次在 `<dest_root>\DiskWhy-Quarantine\…`，[`list_batches`] / [`restore`] / [`purge`] 照样认）。
//!
//! - 原路径映射：`C:\Users\me\AppData\Local\Temp\x.bin` → `<批次>\C\Users\me\AppData\Local\Temp\x.bin`；
//! - 移入：先 `MoveFileExW` 同卷改名；跨卷（ERROR_NOT_SAME_DEVICE）则复制（`CopyFileExW`，保留时间和属性）
//!   → 逐字节比对 → 删除原文件；删除失败就把复制品删掉、原文件不动，记为失败；
//! - `manifest.json`：原路径、隔离后的位置、大小、修改时间、隔离时间、来源规则、档位、方法；
//! - [`restore`]：整批还原到原位置（原位置已有同名文件的不覆盖，跳过并说明）；
//! - [`purge`]：彻底删除整批（界面上二次确认）；只接受带本程序 manifest 的批次目录；
//! - 保留 7 天：[`list_batches`] 标出超过 7 天的批次，启动时提示可以清空，**不自动清**。

use super::log::{ActionLog, LogRecord};
use super::winops;
use super::{EntryKind, Tier};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// 新批次用的文件夹名（阶段 J 起）。
pub const DIR_NAME: &str = "DiskWhy暂存区";
/// 以前用过的文件夹名（列批次 / 清空时同样认）。
pub const OLD_DIR_NAMES: &[&str] = &["DiskWhy-Quarantine"];

/// 新旧所有文件夹名。
pub fn all_dir_names() -> impl Iterator<Item = &'static str> {
    std::iter::once(DIR_NAME).chain(OLD_DIR_NAMES.iter().copied())
}
pub const MANIFEST: &str = "manifest.json";
pub const FORMAT: &str = "diskwhy-quarantine";
pub const KEEP_DAYS: i64 = 7;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestEntry {
    /// 原路径。
    pub original: String,
    /// 隔离后的位置（链接被直接删除时为 None）。
    pub stored: Option<String>,
    pub kind: EntryKind,
    /// 逻辑大小。
    pub size: u64,
    /// 腾出的字节（按簇取整）。
    pub bytes: u64,
    /// 原文件的修改时间（Unix 秒）。
    pub mtime: i64,
    /// 隔离时间。
    pub quarantined_at: i64,
    /// 所属清单项。
    pub item: String,
    /// 来源规则 / 启发式。
    pub source: String,
    pub tier: Tier,
    /// rename / copy_verify_delete / link_removed
    pub method: String,
    /// stored / restored / restore_failed / restore_skipped
    pub status: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub batch_id: String,
    pub created_at: i64,
    pub dest_root: String,
    pub batch_dir: String,
    pub entries: Vec<ManifestEntry>,
    #[serde(default)]
    pub restored_at: Option<i64>,
}

impl Manifest {
    pub fn new(batch_id: &str, dest_root: &str, batch_dir: &str) -> Self {
        Manifest {
            format: FORMAT.into(),
            version: 1,
            batch_id: batch_id.into(),
            created_at: crate::explain::now_unix(),
            dest_root: dest_root.into(),
            batch_dir: batch_dir.into(),
            entries: Vec::new(),
            restored_at: None,
        }
    }

    /// 整份写入（临时文件再改名）。写好之后旁边的 [`JOURNAL`] 就多余了（它的条目都已经在这份里），删掉。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        // 阶段 J：紧凑格式（20 万条时带缩进的要 80 MB 左右）
        let s = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, s).map_err(|e| format!("写 {} 失败：{e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("写 {} 失败：{e}", path.display()))?;
        let _ = std::fs::remove_file(journal_path(path));
        Ok(())
    }

    /// 读 manifest.json，再把旁边 [`JOURNAL`] 里还没进 manifest 的条目补上（执行中途程序被结束时，
    /// 移进来的文件只记在日志式的 journal 里）。
    pub fn load(path: &Path) -> Result<Self, String> {
        let s = std::fs::read(path).map_err(|e| format!("读 {} 失败：{e}", path.display()))?;
        let mut m: Manifest = serde_json::from_slice(&s).map_err(|e| format!("{} 不是有效的暂存区清单：{e}", path.display()))?;
        if m.format != FORMAT {
            return Err(format!("{} 不是 DiskWhy 的暂存区清单", path.display()));
        }
        if let Ok(j) = std::fs::read(journal_path(path)) {
            let have: std::collections::HashSet<(String, Option<String>)> = m.entries.iter().map(|e| (e.original.to_lowercase(), e.stored.clone())).collect();
            // 最后一行可能只写了一半（被结束的那一刻）：解析不了的行跳过
            for line in j.split(|&b| b == b'\n') {
                if let Ok(e) = serde_json::from_slice::<ManifestEntry>(line) {
                    if !have.contains(&(e.original.to_lowercase(), e.stored.clone())) {
                        m.entries.push(e);
                    }
                }
            }
        }
        Ok(m)
    }
}

/// manifest.json 旁边的追加式记录：执行中每移进来一个就写一行（ManifestEntry 的 JSON），不缓冲——程序在执行中途
/// 被结束时，已经移进来的文件都还能从这里找回来（[`Manifest::load`] 会合并）。manifest.json 整份写好后删掉。
pub const JOURNAL: &str = "manifest.journal";

pub fn journal_path(manifest: &Path) -> PathBuf {
    manifest.with_file_name(JOURNAL)
}

/// 往 [`JOURNAL`] 里追加（每条一次 WriteFile，不缓冲）。
pub struct Journal(std::sync::Mutex<Option<std::fs::File>>);

impl Journal {
    pub fn open(manifest: &Path) -> Self {
        let f = std::fs::OpenOptions::new().create(true).append(true).open(journal_path(manifest)).ok();
        Journal(std::sync::Mutex::new(f))
    }

    /// 写一条；写不进去返回 false。
    pub fn append(&self, e: &ManifestEntry) -> bool {
        use std::io::Write;
        let Ok(mut line) = serde_json::to_vec(e) else { return false };
        line.push(b'\n');
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        g.as_mut().is_some_and(|f| f.write_all(&line).is_ok())
    }

    /// 关掉文件（之后才能删它 / 删批次文件夹）。
    pub fn close(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

/// 批次目录：`<dest_root>\DiskWhy-Quarantine\<batch_id>`。
pub fn batch_dir(dest_root: &str, batch_id: &str) -> String {
    crate::fs::join(&crate::fs::join(dest_root, DIR_NAME), batch_id)
}

/// 原路径 → 批次里的位置：`C:\a\b` → `<batch>\C\a\b`。
pub fn map_path(batch: &str, original: &str) -> String {
    let n = crate::scan::normalize_root(original);
    let (drive, rest) = n.split_at(2.min(n.len()));
    let letter = drive.trim_end_matches(':');
    let rest = rest.trim_start_matches('\\');
    if rest.is_empty() {
        crate::fs::join(batch, letter)
    } else {
        crate::fs::join(&crate::fs::join(batch, letter), rest)
    }
}

fn ensure_parent(p: &str) -> Result<(), String> {
    if let Some((parent, _)) = p.rsplit_once('\\') {
        std::fs::create_dir_all(winops_long(parent)).map_err(|e| format!("建不了文件夹 {parent}：{e}"))?;
    }
    Ok(())
}

fn winops_long(p: &str) -> PathBuf {
    PathBuf::from(crate::win::long_path(p))
}

/// 逐字节比较两个文件。
pub fn same_content(a: &str, b: &str) -> std::io::Result<bool> {
    let mut fa = std::fs::File::open(winops_long(a))?;
    let mut fb = std::fs::File::open(winops_long(b))?;
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let mut ba = vec![0u8; 1 << 20];
    let mut bb = vec![0u8; 1 << 20];
    loop {
        let na = read_full(&mut fa, &mut ba)?;
        let nb = read_full(&mut fb, &mut bb)?;
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

/// 移动一个文件：同卷改名；跨卷复制 → 校验 → 删除原文件。返回用的方法。
/// `dst` 已存在时失败（不覆盖任何东西）。
pub fn move_file(src: &str, dst: &str) -> Result<&'static str, String> {
    move_file_cancellable(src, dst, None)
}

/// 同 [`move_file`]；跨卷复制用 `CopyFileExW`，`cancel` 变成非 0 时大文件也能中途停下（半截的复制品删掉，原文件不动）。
pub fn move_file_cancellable(src: &str, dst: &str, cancel: Option<&std::sync::atomic::AtomicI32>) -> Result<&'static str, String> {
    move_file_with(src, dst, cancel, false)
}

/// 同 [`move_file_cancellable`]。`parent_ready = true`（调用方刚在同一个文件夹里放过东西）时省掉“建上层文件夹”和
/// “目标已存在”这两次额外的文件系统调用：MoveFileExW 不带 REPLACE_EXISTING、CopyFileExW 带 FAIL_IF_EXISTS，
/// 目标已存在时本来就会失败（183 / 80），照样不覆盖；上层文件夹不在（3）时补建再试一次。
pub fn move_file_with(src: &str, dst: &str, cancel: Option<&std::sync::atomic::AtomicI32>, parent_ready: bool) -> Result<&'static str, String> {
    if !parent_ready {
        ensure_parent(dst)?;
        if std::fs::symlink_metadata(winops_long(dst)).is_ok() {
            return Err(format!("目标位置已经有东西了：{dst}（不覆盖）"));
        }
    }
    let mut first = winops::rename(src, dst);
    if parent_ready && first == Err(3) {
        ensure_parent(dst)?;
        first = winops::rename(src, dst);
    }
    match first {
        Ok(()) => Ok("rename"),
        Err(183) | Err(80) => Err(format!("目标位置已经有东西了：{dst}（不覆盖）")),
        Err(code) if code == winops::ERROR_NOT_SAME_DEVICE => {
            winops::copy_file_cancellable(src, dst, cancel).map_err(|c| {
                if c == winops::ERROR_REQUEST_ABORTED {
                    "已停止：复制到一半停下了，复制品已删掉，原文件没动".to_string()
                } else {
                    format!("复制到另一个盘失败（Win32 错误 {c}）")
                }
            })?;
            match same_content(src, dst) {
                Ok(true) => {}
                Ok(false) => {
                    let _ = std::fs::remove_file(winops_long(dst));
                    return Err("复制后比对内容不一致，已删掉复制品，原文件没动".into());
                }
                Err(e) => {
                    let _ = std::fs::remove_file(winops_long(dst));
                    return Err(format!("复制后无法比对内容（{e}），已删掉复制品，原文件没动"));
                }
            }
            if let Err(code) = winops::delete_file(src) {
                let _ = std::fs::remove_file(winops_long(dst));
                return Err(format!("复制成功但删不掉原文件（Win32 错误 {code}），已删掉复制品，原文件没动"));
            }
            Ok("copy_verify_delete")
        }
        Err(code) => Err(format!("移动失败（Win32 错误 {code}）")),
    }
}

/// 还原结果。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RestoreReport {
    pub batch_id: String,
    pub restored: usize,
    pub restored_bytes: u64,
    pub skipped: usize,
    pub failed: usize,
    /// 还原后被删掉的空批次目录。
    pub batch_removed: bool,
    /// 前 200 条非成功的明细。
    pub problems: Vec<(String, String)>,
    /// 还原到的原路径（给界面刷新用，去重后的上层目录）。
    pub touched_dirs: Vec<String>,
}

/// 整批还原。
pub fn restore(manifest_path: &Path, log: &ActionLog) -> Result<RestoreReport, String> {
    let mut m = Manifest::load(manifest_path)?;
    let mut rep = RestoreReport { batch_id: m.batch_id.clone(), ..Default::default() };
    let batch_log = format!("restore-{}", m.batch_id);
    for e in m.entries.iter_mut() {
        if e.status != "stored" && e.status != "restore_failed" {
            continue;
        }
        let mut rec = LogRecord::new(&batch_log, "restore", &e.original);
        rec.item = e.item.clone();
        rec.source = e.source.clone();
        rec.tier = tier_str(e.tier).into();
        rec.bytes = e.bytes;
        let Some(stored) = e.stored.clone() else {
            e.status = "restore_skipped".into();
            e.note = "链接当时被直接删除（链接本身不占空间），无法还原；需要的话请自己重建链接".into();
            rep.skipped += 1;
            push_problem(&mut rep, &e.original, &e.note);
            rec.result = "skipped".into();
            rec.reason = e.note.clone();
            log.write(&rec);
            continue;
        };
        rec.dest = Some(e.original.clone());
        rec.path = stored.clone();
        match move_file(&stored, &e.original) {
            Ok(method) => {
                e.status = "restored".into();
                e.note.clear();
                rep.restored += 1;
                rep.restored_bytes += e.bytes;
                rec.method = method.into();
                if let Some((parent, _)) = e.original.rsplit_once('\\') {
                    rep.touched_dirs.push(parent.to_string());
                }
            }
            Err(why) => {
                e.status = "restore_failed".into();
                e.note = why.clone();
                rep.failed += 1;
                push_problem(&mut rep, &e.original, &why);
                rec.result = "failed".into();
                rec.reason = why;
            }
        }
        log.write(&rec);
    }
    let all_back = m.entries.iter().all(|e| e.status == "restored" || e.status == "restore_skipped");
    if all_back {
        m.restored_at = Some(crate::explain::now_unix());
    }
    m.save(manifest_path)?;
    if all_back {
        // 批次目录里只剩空文件夹和 manifest：清掉（只删空目录，任何剩余文件都会让它停下）
        let dir = manifest_path.parent().map(|p| p.to_path_buf());
        if let Some(dir) = dir {
            let _ = std::fs::remove_file(manifest_path);
            rep.batch_removed = remove_empty_tree(&dir.to_string_lossy());
            if rep.batch_removed {
                // 空的 DiskWhy-Quarantine 也顺手删掉（里面还有别的批次就删不掉，正好）
                if let Some(q) = dir.parent() {
                    let _ = std::fs::remove_dir(q);
                }
            }
            if !rep.batch_removed {
                // 没删干净就把 manifest 写回去，免得丢记录
                let _ = m.save(manifest_path);
            }
        }
    }
    rep.touched_dirs.sort();
    rep.touched_dirs.dedup();
    let mut sum = LogRecord::new(&batch_log, "summary", &manifest_path.to_string_lossy());
    sum.bytes = rep.restored_bytes;
    sum.reason = format!("还原 {} 个，跳过 {}，失败 {}", rep.restored, rep.skipped, rep.failed);
    sum.result = if rep.failed == 0 { "ok".into() } else { "failed".into() };
    log.write(&sum);
    Ok(rep)
}

fn push_problem(rep: &mut RestoreReport, p: &str, why: &str) {
    if rep.problems.len() < 200 {
        rep.problems.push((p.to_string(), why.to_string()));
    }
}

/// 自底向上只删空目录；全部删掉返回 true。不跟随链接（链接目录用 RemoveDirectoryW 只删链接本身——但隔离区里
/// 本来就不会有：链接在同卷时是整体改名进来的，restore 时会被移回去）。
pub fn remove_empty_tree(dir: &str) -> bool {
    let Ok(list) = winops::list_dir(dir) else { return false };
    let mut ok = true;
    for r in list {
        let p = crate::fs::join(dir, &r.name);
        if r.attrs & crate::fs::attrs::DIRECTORY != 0 && r.attrs & crate::fs::attrs::REPARSE_POINT == 0 {
            ok &= remove_empty_tree(&p);
        } else {
            ok = false;
        }
    }
    ok && winops::remove_dir(dir).is_ok()
}

/// 清空结果。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PurgeReport {
    pub batch_id: String,
    pub deleted_entries: usize,
    pub freed_bytes: u64,
    pub removed: bool,
    pub error: Option<String>,
}

/// 彻底删除一个批次（不可恢复）。只接受 `...\DiskWhy暂存区\<批次>\manifest.json`（或旧名 `DiskWhy-Quarantine`）。
pub fn purge(manifest_path: &Path, log: &ActionLog) -> Result<PurgeReport, String> {
    let m = Manifest::load(manifest_path)?;
    let dir = manifest_path.parent().ok_or("清单路径不对")?;
    let parent_name = dir.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if !all_dir_names().any(|n| parent_name.eq_ignore_ascii_case(n)) {
        return Err(format!("{} 不在 {DIR_NAME} 文件夹里，拒绝清空", dir.display()));
    }
    let dir_s = dir.to_string_lossy().to_string();
    if !dir_s.eq_ignore_ascii_case(&m.batch_dir) {
        return Err(format!("清单里记录的批次目录（{}）和实际位置（{dir_s}）不一致，拒绝清空", m.batch_dir));
    }
    let mut rep = PurgeReport { batch_id: m.batch_id.clone(), ..Default::default() };
    let batch_log = format!("purge-{}", m.batch_id);
    for e in m.entries.iter().filter(|e| e.status == "stored" || e.status == "restore_failed") {
        rep.deleted_entries += 1;
        rep.freed_bytes += e.bytes;
        let mut rec = LogRecord::new(&batch_log, "purge", e.stored.as_deref().unwrap_or(&e.original));
        rec.bytes = e.bytes;
        rec.item = e.item.clone();
        rec.source = e.source.clone();
        rec.tier = tier_str(e.tier).into();
        rec.method = "remove_dir_all".into();
        rec.reason = format!("原路径 {}", e.original);
        log.write(&rec);
    }
    // std::fs::remove_dir_all 在 Windows 上不跟随符号链接 / 联接（只删链接本身）。
    match std::fs::remove_dir_all(winops_long(&dir_s)) {
        Ok(()) => rep.removed = true,
        Err(e) => rep.error = Some(format!("没能全部删掉：{e}")),
    }
    // 空的 DiskWhy-Quarantine 顺手删掉
    if let Some(q) = dir.parent() {
        let _ = std::fs::remove_dir(q);
    }
    let mut sum = LogRecord::new(&batch_log, "summary", &manifest_path.to_string_lossy());
    sum.bytes = rep.freed_bytes;
    sum.result = if rep.removed { "ok".into() } else { "failed".into() };
    sum.reason = rep.error.clone().unwrap_or_else(|| format!("彻底删除 {} 个条目", rep.deleted_entries));
    log.write(&sum);
    Ok(rep)
}

/// 隔离区里的一个批次。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchInfo {
    pub batch_id: String,
    pub manifest_path: String,
    pub batch_dir: String,
    pub dest_root: String,
    pub created_at: i64,
    pub age_days: i64,
    /// 超过 7 天（启动时提示可以清空，不自动清）。
    pub expired: bool,
    pub entries: usize,
    pub stored_entries: usize,
    pub bytes: u64,
    /// 原来所在的清单项（去重，最多 20 个）。
    pub items: Vec<String>,
}

/// 列出若干根目录下（`<root>\DiskWhy暂存区\*\manifest.json`，以及旧名 `DiskWhy-Quarantine`）的批次，新的在前。
pub fn list_batches(roots: &[String]) -> Vec<BatchInfo> {
    let now = crate::explain::now_unix();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for q in roots.iter().flat_map(|r| all_dir_names().map(move |n| crate::fs::join(r, n))) {
        let Ok(rd) = std::fs::read_dir(&q) else { continue };
        for d in rd.flatten() {
            let mp = d.path().join(MANIFEST);
            let Ok(m) = Manifest::load(&mp) else { continue };
            if !seen.insert(mp.to_string_lossy().to_lowercase()) {
                continue;
            }
            let stored: Vec<&ManifestEntry> = m.entries.iter().filter(|e| e.status == "stored" || e.status == "restore_failed").collect();
            if stored.is_empty() {
                continue;
            }
            let mut items: Vec<String> = stored.iter().map(|e| e.item.clone()).collect();
            items.sort();
            items.dedup();
            items.truncate(20);
            let age_days = (now - m.created_at).max(0) / 86_400;
            out.push(BatchInfo {
                batch_id: m.batch_id.clone(),
                manifest_path: mp.to_string_lossy().to_string(),
                batch_dir: m.batch_dir.clone(),
                dest_root: m.dest_root.clone(),
                created_at: m.created_at,
                age_days,
                expired: age_days >= KEEP_DAYS,
                entries: m.entries.len(),
                stored_entries: stored.len(),
                bytes: stored.iter().map(|e| e.bytes).sum(),
                items,
            });
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// 用过的隔离区根目录记在 `%LOCALAPPDATA%\DiskWhy\quarantine-roots.json`，列批次时连同所有盘符根一起查。
pub fn roots_index_path() -> Option<PathBuf> {
    crate::explain::llm::app_data_dir().map(|d| d.join("quarantine-roots.json"))
}

pub fn remember_root(root: &str) {
    let Some(p) = roots_index_path() else { return };
    let mut v = known_roots();
    if !v.iter().any(|x| x.eq_ignore_ascii_case(root)) {
        v.push(root.to_string());
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap_or_default());
    }
}

pub fn known_roots() -> Vec<String> {
    roots_index_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn tier_str(t: Tier) -> &'static str {
    match t {
        Tier::Safe => "safe",
        Tier::Review => "review",
        Tier::Keep => "keep",
    }
}

// ───────────────────────── 阶段 J：暂存区落点（盘根不可写时） ─────────────────────────

/// 某个盘上暂存区的落点（[`prepare_dest`] 的结果）。序列化成 `{status:"ok", dest_root}` /
/// `{status:"needs_elevation"}` / `{status:"failed", message}`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DestStatus {
    /// `dest_root` 可以直接当 `Strategy::Quarantine { dest_root }` 用（批次放在 `<dest_root>\DiskWhy暂存区\…`）。
    Ok { dest_root: String },
    /// 这个盘的根目录普通权限建不了文件夹，当前账户是管理员（受限令牌）：可以点“授权”提权建一次。
    NeedsElevation,
    /// 建不了，也提不了权：请用户选文件夹。
    Failed { message: String },
}

/// `X:\` 形式的盘根（大小写不限）。
pub fn is_drive_root(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

/// 在 `dir` 里建一个临时子文件夹再删掉，确认能写（不留下任何东西）。
pub fn dir_writable(dir: &str) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let probe = crate::fs::join(dir, &format!(".dw-probe-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    match std::fs::create_dir(winops_long(&probe)) {
        Ok(()) => {
            let _ = std::fs::remove_dir(winops_long(&probe));
            true
        }
        Err(_) => false,
    }
}

/// 是普通文件夹（不是链接 / 联接）。
fn plain_dir(p: &str) -> bool {
    std::fs::symlink_metadata(winops_long(p)).map(|m| m.is_dir() && !m.file_type().is_symlink()).unwrap_or(false)
        && winops::stat_entry(p).map(|r| r.attrs & crate::fs::attrs::REPARSE_POINT == 0).unwrap_or(false)
}

/// 找某个盘上能用的暂存区位置（`docs/PHASE_J_PLAN.md` §3.2 的 ①②③，不提权）：
/// ① `X:\DiskWhy暂存区` 已存在且可写 → 用；② 试建（`create = false` 时只试一下就删掉，不留东西）→ 能建就用；
/// ③ `quarantine-roots.json` 里这个盘以前用过的位置仍可写 → 用；都不行 → 管理员账户（受限令牌）给
/// `NeedsElevation`，否则 `Failed`。
pub fn prepare_dest(drive_root: &str, create: bool) -> DestStatus {
    prepare_dest_with(drive_root, create, &known_roots(), winops::elevation_type())
}

/// 同 [`prepare_dest`]，以前用过的位置和提权类型由调用方给（测试用）。
pub fn prepare_dest_with(drive_root: &str, create: bool, known: &[String], elevation_type: i32) -> DestStatus {
    if !is_drive_root(drive_root) {
        return DestStatus::Failed { message: format!("「{drive_root}」不是盘的根目录（应形如 D:\\）") };
    }
    if !std::path::Path::new(drive_root).is_dir() {
        return DestStatus::Failed { message: format!("{drive_root} 不存在") };
    }
    let q = crate::fs::join(drive_root, DIR_NAME);
    // ①
    if plain_dir(&q) && dir_writable(&q) {
        return DestStatus::Ok { dest_root: drive_root.to_string() };
    }
    // ②
    if !std::path::Path::new(&winops_long(&q)).exists() {
        if create {
            if std::fs::create_dir(winops_long(&q)).is_ok() {
                return DestStatus::Ok { dest_root: drive_root.to_string() };
            }
        } else if std::fs::create_dir(winops_long(&q)).is_ok() {
            let _ = std::fs::remove_dir(winops_long(&q));
            return DestStatus::Ok { dest_root: drive_root.to_string() };
        }
    }
    // ③
    for r in known {
        if r.eq_ignore_ascii_case(drive_root) || !crate::scan::volume_root_of(r).is_some_and(|v| v.eq_ignore_ascii_case(drive_root)) {
            continue;
        }
        if !plain_dir(r) {
            continue;
        }
        let rq = crate::fs::join(r, DIR_NAME);
        if (plain_dir(&rq) && dir_writable(&rq)) || (!std::path::Path::new(&winops_long(&rq)).exists() && dir_writable(r)) {
            return DestStatus::Ok { dest_root: r.clone() };
        }
    }
    if elevation_type == 3 {
        DestStatus::NeedsElevation
    } else {
        DestStatus::Failed { message: format!("{drive_root} 不让建文件夹，请选一个能写的文件夹") }
    }
}

/// 提权子进程的命令行开关：`diskwhy.exe --mkdir-quarantine X:\ --grant-sid S-1-5-21-…`。
pub const MKDIR_FLAG: &str = "--mkdir-quarantine";
pub const SID_FLAG: &str = "--grant-sid";

/// 提权子进程的退出码。
pub mod mkdir_exit {
    pub const OK: i32 = 0;
    pub const BAD_ARGS: i32 = 2;
    pub const MKDIR_FAILED: i32 = 3;
    pub const NOT_PLAIN_DIR: i32 = 4;
    pub const ICACLS_FAILED: i32 = 5;
}

/// 给提权子进程的参数（盘根带结尾 `\`，命令行里要写成 `"X:\\"` 防止 `\"` 被当成转义的引号）。
pub fn mkdir_params(drive_root: &str, sid: &str) -> String {
    format!("{MKDIR_FLAG} \"{drive_root}\\\" {SID_FLAG} {sid}")
}

/// 只接受**个人账户**的 SID：本机 / 域账户 `S-1-5-21-…`，Microsoft Entra（Azure AD）账户 `S-1-12-1-…`。
/// Everyone（S-1-1-0）、Users（S-1-5-32-545）、Authenticated Users（S-1-5-11）这类组一律拒绝。
pub fn valid_sid(s: &str) -> bool {
    (s.starts_with("S-1-5-21-") || s.starts_with("S-1-12-1-"))
        && s.len() < 200
        && s[2..].chars().all(|c| c.is_ascii_digit() || c == '-')
        && !s.ends_with('-')
        && !s.contains("--")
}

/// 程序一启动就调用（在建任何窗口之前）：命令行是 `--mkdir-quarantine X:\ --grant-sid <SID>` 时，
/// 建 `X:\DiskWhy暂存区`、在它的句柄上给这个 SID 加“修改”权限（`(OI)(CI)M`，子文件夹 / 文件继承），返回退出码；
/// 不是这个模式返回 None。只接受固定盘的盘根和个人账户的 SID（见 [`valid_sid`]）；文件夹已存在但是链接 / 联接时拒绝。
pub fn elevated_mkdir_from_args(args: &[String]) -> Option<i32> {
    let i = args.iter().position(|a| a == MKDIR_FLAG)?;
    let root = args.get(i + 1).map(|s| {
        let t = s.trim_matches('"');
        if t.len() == 2 { format!("{t}\\") } else { t.trim_end_matches('\\').to_string() + "\\" }
    });
    let sid = args.iter().position(|a| a == SID_FLAG).and_then(|j| args.get(j + 1)).cloned();
    let (Some(root), Some(sid)) = (root, sid) else { return Some(mkdir_exit::BAD_ARGS) };
    if !is_drive_root(&root) || !valid_sid(&sid) {
        return Some(mkdir_exit::BAD_ARGS);
    }
    if !crate::win::list_fixed_drives().iter().any(|d| d.root.eq_ignore_ascii_case(&root)) {
        return Some(mkdir_exit::BAD_ARGS);
    }
    let q = crate::fs::join(&root, DIR_NAME);
    if std::fs::symlink_metadata(winops_long(&q)).is_ok() {
        if !plain_dir(&q) {
            return Some(mkdir_exit::NOT_PLAIN_DIR);
        }
    } else if std::fs::create_dir(winops_long(&q)).is_err() {
        return Some(mkdir_exit::MKDIR_FAILED);
    }
    // 在同一个句柄上确认是普通文件夹再改权限（不用 icacls：它跟随联接，检查和使用之间可以被换掉）
    match winops::grant_modify_plain_dir(&q, &sid) {
        Ok(()) => Some(mkdir_exit::OK),
        Err(336) => Some(mkdir_exit::NOT_PLAIN_DIR),
        Err(_) => Some(mkdir_exit::ICACLS_FAILED),
    }
}

/// 非提权一侧：用当前用户的 SID 以管理员身份运行本程序的 `--mkdir-quarantine`，等它结束，再走一遍
/// [`prepare_dest`]（`create = true`）。用户在 UAC 里点“否”→ `Failed { message: "你取消了授权" }`。
pub fn elevate_and_prepare(exe: &str, drive_root: &str) -> DestStatus {
    if !is_drive_root(drive_root) {
        return DestStatus::Failed { message: format!("「{drive_root}」不是盘的根目录") };
    }
    let Some(sid) = winops::current_user_sid() else {
        return DestStatus::Failed { message: "读不到当前用户".into() };
    };
    match winops::run_elevated_wait(exe, &mkdir_params(drive_root, &sid)) {
        Ok(0) => match prepare_dest(drive_root, true) {
            DestStatus::NeedsElevation => DestStatus::Failed { message: "授权后还是写不进去，请选其他文件夹".into() },
            s => s,
        },
        Ok(code) => DestStatus::Failed { message: format!("授权建文件夹失败（代码 {code}），请选其他文件夹") },
        Err(None) => DestStatus::Failed { message: "你取消了授权".into() },
        Err(Some(m)) => DestStatus::Failed { message: m },
    }
}
