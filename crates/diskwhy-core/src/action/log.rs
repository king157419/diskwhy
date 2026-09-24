//! 动作日志：`%LOCALAPPDATA%\DiskWhy\actions.log`，JSON lines，每个动作一行，只追加。
//!
//! 每行字段固定（没有值的为 null / 空串 / 0，字段不会缺）：
//! `ts`（Unix 秒）、`time`（ISO 8601 UTC）、`batch`（这次执行 / 还原 / 清空的 id）、`op`（recycle /
//! quarantine / remove_link / rmdir / restore / purge / summary）、`path`、`dest`、`bytes`、
//! `result`（ok / skipped / failed）、`reason`、`item`（所属清单项）、`source`（规则 id / 启发式名）、
//! `tier`（safe / review）、`method`（rename / copy_verify_delete / ifileoperation / remove_directory …）。

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

/// 追加写日志（多线程共用，一行一次 write）。
pub struct ActionLog {
    path: PathBuf,
    file: Mutex<Option<std::fs::File>>,
    /// 写失败的次数（日志写不进去不影响动作本身，但会在报告里说明）。
    pub failures: std::sync::atomic::AtomicU64,
}

impl ActionLog {
    /// 默认位置 `%LOCALAPPDATA%\DiskWhy\actions.log`。
    pub fn default_path() -> Option<PathBuf> {
        crate::explain::llm::app_data_dir().map(|d| d.join("actions.log"))
    }

    pub fn open(path: &Path) -> Self {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
        ActionLog { path: path.to_path_buf(), file: Mutex::new(f), failures: Default::default() }
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
    }
}
