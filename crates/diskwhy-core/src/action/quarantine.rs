//! 隔离区：`<dest_root>\DiskWhy-Quarantine\<时间戳>\<原路径映射>` + `manifest.json`。
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

pub const DIR_NAME: &str = "DiskWhy-Quarantine";
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

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        let s = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, s).map_err(|e| format!("写 {} 失败：{e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("写 {} 失败：{e}", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let s = std::fs::read(path).map_err(|e| format!("读 {} 失败：{e}", path.display()))?;
        let m: Manifest = serde_json::from_slice(&s).map_err(|e| format!("{} 不是有效的隔离区清单：{e}", path.display()))?;
        if m.format != FORMAT {
            return Err(format!("{} 不是 DiskWhy 的隔离区清单", path.display()));
        }
        Ok(m)
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
    ensure_parent(dst)?;
    if std::fs::symlink_metadata(winops_long(dst)).is_ok() {
        return Err(format!("目标位置已经有东西了：{dst}（不覆盖）"));
    }
    match winops::rename(src, dst) {
        Ok(()) => Ok("rename"),
        Err(code) if code == winops::ERROR_NOT_SAME_DEVICE => {
            // 跨卷：CopyFileExW（std::fs::copy 在 Windows 上就是它，保留时间和属性）
            std::fs::copy(winops_long(src), winops_long(dst)).map_err(|e| {
                let _ = std::fs::remove_file(winops_long(dst));
                format!("复制到另一个盘失败：{e}")
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

/// 彻底删除一个批次（不可恢复）。只接受 `...\DiskWhy-Quarantine\<批次>\manifest.json`。
pub fn purge(manifest_path: &Path, log: &ActionLog) -> Result<PurgeReport, String> {
    let m = Manifest::load(manifest_path)?;
    let dir = manifest_path.parent().ok_or("清单路径不对")?;
    let parent_name = dir.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if !parent_name.eq_ignore_ascii_case(DIR_NAME) {
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

/// 列出若干根目录下（`<root>\DiskWhy-Quarantine\*\manifest.json`）的批次，新的在前。
pub fn list_batches(roots: &[String]) -> Vec<BatchInfo> {
    let now = crate::explain::now_unix();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for r in roots {
        let q = crate::fs::join(r, DIR_NAME);
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
