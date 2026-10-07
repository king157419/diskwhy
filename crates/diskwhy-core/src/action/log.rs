//! 动作日志：`%LOCALAPPDATA%\DiskWhy\actions.log`，JSON lines，每个动作一行，只追加。
//!
//! 每行字段固定（没有值的为 null / 空串 / 0，字段不会缺）：
//! `ts`（Unix 秒）、`time`（ISO 8601 UTC）、`batch`（这次执行 / 还原 / 清空的 id）、`op`（start / heartbeat /
//! delete / recycle / recycle_dir / quarantine / remove_link / rmdir / skip / cancel / restore / purge / summary）、`path`、`dest`、`bytes`、
//! `result`（ok / skipped / failed）、`reason`、`item`（所属清单项）、`source`（规则 id / 启发式名）、
//! `tier`（safe / review）、`method`（rename / copy_verify_delete / ifileoperation / ifileoperation_dir /
//! delete_disposition / remove_directory …）。
//!
//! 阶段 J：写入走 64 KB 缓冲（执行时每 200 ms 由计时线程 `flush` 一次，`start` / `heartbeat` / `summary`
//! 之后立刻 flush，析构时也 flush）；打开时文件超过 50 MB 就改名为 `actions.<UTC 时间>.log`，旧日志留最近
//! [`ROTATE_KEEP`] 份（阶段 J 复核：原来只留一份 `actions.1.log`，连着两次轮转就把出事那次的日志冲掉了）。
//! 更早的 `actions.1.log`（旧版本留下的）不动。

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 日志里的一行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogRecord {
    pub ts: i64,
    pub time: String,
    pub batch: String,
    pub op: String,
    pub path: String,
    pub dest: Option<String>,
    pub bytes: u64,
    pub result: String,
    pub reason: String,
    pub item: String,
    pub source: String,
    pub tier: String,
    pub method: String,
}

/// 所有字段名（测试用来核对“字段齐全”）。
pub const FIELDS: &[&str] = &["ts", "time", "batch", "op", "path", "dest", "bytes", "result", "reason", "item", "source", "tier", "method"];

impl LogRecord {
    pub fn new(batch: &str, op: &str, path: &str) -> Self {
        let ts = crate::explain::now_unix();
        LogRecord {
            ts,
            time: super::fmt_iso(ts),
            batch: batch.to_string(),
            op: op.to_string(),
            path: path.to_string(),
            dest: None,
            bytes: 0,
            result: "ok".into(),
            reason: String::new(),
            item: String::new(),
            source: String::new(),
            tier: String::new(),
            method: String::new(),
        }
    }
}

/// 打开时超过这个大小就轮转（改名为 `actions.<UTC 时间>.log`）。
pub const ROTATE_BYTES: u64 = 50 * 1024 * 1024;
/// 轮转出来的旧日志最多留几份（更早的删掉；只删本程序轮转出来、名字是 `actions.<时间>.log` 的）。
pub const ROTATE_KEEP: usize = 5;

/// 追加写日志（多线程共用，缓冲写，见模块说明）。
pub struct ActionLog {
    path: PathBuf,
    file: Mutex<Option<std::io::BufWriter<std::fs::File>>>,
    /// 写失败的次数（日志写不进去不影响动作本身，但会在报告里说明）。
    pub failures: std::sync::atomic::AtomicU64,
}

impl ActionLog {
    /// 默认位置 `%LOCALAPPDATA%\DiskWhy\actions.log`。
    pub fn default_path() -> Option<PathBuf> {
        crate::explain::llm::app_data_dir().map(|d| d.join("actions.log"))
    }

    pub fn open(path: &Path) -> Self {
        Self::open_rotating(path, ROTATE_BYTES)
    }

    /// 同 [`ActionLog::open`]，轮转阈值可指定（测试用）。
    pub fn open_rotating(path: &Path, max_bytes: u64) -> Self {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::metadata(path).map(|m| m.len() > max_bytes).unwrap_or(false) {
            // 不覆盖任何旧日志：名字里带时间，同一秒里再轮转就加序号
            let ts = crate::explain::now_unix();
            let old = (0..1000).map(|n| rotated_path_at(path, ts, n)).find(|p| !p.exists());
            if let Some(old) = old {
                if std::fs::rename(path, &old).is_ok() {
                    prune_rotated(path, ROTATE_KEEP);
                }
            }
        }
        let f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(|f| std::io::BufWriter::with_capacity(64 * 1024, f));
        ActionLog { path: path.to_path_buf(), file: Mutex::new(f), failures: Default::default() }
    }

    /// 把缓冲写进文件。
    pub fn flush(&self) {
        let mut g = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = g.as_mut() {
            if f.flush().is_err() {
                self.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, rec: &LogRecord) {
        let mut line = match serde_json::to_string(rec) {
            Ok(s) => s,
            Err(_) => {
                self.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };
        line.push('\n');
        let mut g = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let ok = match g.as_mut() {
            Some(f) => f.write_all(line.as_bytes()).is_ok(),
            None => false,
        };
        if !ok {
            self.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // 关键记录立刻落盘（中断后靠它们取证）
        if matches!(rec.op.as_str(), "start" | "heartbeat" | "summary") {
            if let Some(f) = g.as_mut() {
                let _ = f.flush();
            }
        }
    }
}

impl Drop for ActionLog {
    fn drop(&mut self) {
        self.flush();
    }
}

fn stem_ext(path: &Path) -> (String, String) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "actions".into());
    let ext = path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    (stem, ext)
}

/// `actions.log` → `actions.1.log`（旧版本的轮转名；现在只用来认它，不再写）。
pub fn rotated_path(path: &Path) -> PathBuf {
    let (stem, ext) = stem_ext(path);
    path.with_file_name(format!("{stem}.1{ext}"))
}

/// `actions.log` → `actions.20261006-121947.log`（UTC；`n > 0` 时 `actions.20261006-121947-n.log`）。
pub fn rotated_path_at(path: &Path, unix: i64, n: usize) -> PathBuf {
    let (stem, ext) = stem_ext(path);
    let iso = super::fmt_iso(unix); // 2026-10-06T12:19:47Z
    let t: String = iso.chars().filter(|c| c.is_ascii_digit() || *c == 'T').collect::<String>().replace('T', "-");
    if n == 0 {
        path.with_file_name(format!("{stem}.{t}{ext}"))
    } else {
        path.with_file_name(format!("{stem}.{t}-{n}{ext}"))
    }
}

/// 轮转出来的旧日志（`actions.<8 位日期>-<6 位时间>[-n].log`），按名字排序（= 按时间）。
pub fn rotated_logs(path: &Path) -> Vec<PathBuf> {
    let (stem, ext) = stem_ext(path);
    let Some(dir) = path.parent() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|d| d.path())
        .filter(|p| {
            let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let Some(mid) = name.strip_prefix(&format!("{stem}.")).and_then(|r| r.strip_suffix(ext.as_str())) else { return false };
            let b = mid.as_bytes();
            b.len() >= 15 && b[8] == b'-' && b[..8].iter().all(u8::is_ascii_digit) && b[9..15].iter().all(u8::is_ascii_digit)
        })
        .collect();
    // 按 (时间, 序号) 排：同一秒里的 `-n` 排在不带序号的那份后面
    let key = |p: &PathBuf| {
        let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let mid = name[stem.len() + 1..name.len() - ext.len()].to_string();
        let n: usize = mid.get(16..).and_then(|x| x.parse().ok()).unwrap_or(0);
        (mid[..15].to_string(), n)
    };
    v.sort_by_key(key);
    v
}

/// 只留最新的 `keep` 份轮转出来的旧日志。
fn prune_rotated(path: &Path, keep: usize) {
    let v = rotated_logs(path);
    if v.len() > keep {
        for p in &v[..v.len() - keep] {
            let _ = std::fs::remove_file(p);
        }
    }
}
