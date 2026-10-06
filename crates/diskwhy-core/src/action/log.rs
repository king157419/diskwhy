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
//! 之后立刻 flush，析构时也 flush）；打开时文件超过 50 MB 就改名为 `actions.1.log`（只留一份旧的）。

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

/// 打开时超过这个大小就轮转（改名为 `actions.1.log`，只留一份旧的）。
pub const ROTATE_BYTES: u64 = 50 * 1024 * 1024;

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
            let old = rotated_path(path);
            // 只保留一份旧日志：先把更早的那份去掉（这是本程序自己的日志文件）
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(path, &old);
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

/// `actions.log` → `actions.1.log`。
pub fn rotated_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "actions".into());
    let ext = path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    path.with_file_name(format!("{stem}.1{ext}"))
}
