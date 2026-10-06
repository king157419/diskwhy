//! 执行状态文件（阶段 J）：`%LOCALAPPDATA%\DiskWhy\last-run.json`。
//!
//! 执行开始时写 `status = "running"`，每 30 秒（heartbeat）更新已处理的个数 / 字节，结束时改成
//! `done` / `cancelled`。下次启动时如果还是 `running`（而且记录里的进程已经不在了），说明上次清理被打断：
//! 界面提示「上次清理中断，已处理 N 个」。用户看过之后改成 `dismissed`。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `last-run.json` 的内容。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RunState {
    pub batch: String,
    /// running / done / cancelled / dismissed
    pub status: String,
    pub pid: u32,
    /// Unix 秒。
    pub started: i64,
    pub updated: i64,
    /// 计划要处理的条目数 / 字节。
    pub total: usize,
    pub total_bytes: u64,
    /// 已处理（成功 + 跳过 + 失败）的条目数；`processed_bytes` = 已成功处理的字节。
    pub processed: usize,
    pub processed_bytes: u64,
    #[serde(default)]
    pub strategy: String,
    /// 执行这次清理的程序（完整路径）。下次启动时用它在 PCA 记录里找这个进程的退出码。
    #[serde(default)]
    pub exe: String,
}

/// 上次被打断的执行（给界面）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Interrupted {
    pub batch: String,
    pub started: i64,
    pub processed: usize,
    pub processed_bytes: u64,
    pub total: usize,
    /// 阶段 J 取证：Windows“程序兼容性助手”记下的这个程序的异常退出码（如 `0x8f`），找不到为 null。
    #[serde(default)]
    pub exit_code: Option<String>,
    /// 这次清理开始之后 crash.log 里有 panic / 原生异常记录（= 程序自己崩了，不是被关掉 / 被结束）。
    #[serde(default)]
    pub crash_logged: bool,
}

impl RunState {
    /// 默认位置 `%LOCALAPPDATA%\DiskWhy\last-run.json`。
    pub fn default_path() -> Option<PathBuf> {
        crate::explain::llm::app_data_dir().map(|d| d.join("last-run.json"))
    }

    pub fn load(path: &Path) -> Option<Self> {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// 写临时文件再改名（中途断电不会留下半个文件）。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("json.tmp");
        let s = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, s).map_err(|e| format!("写 {} 失败：{e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("写 {} 失败：{e}", path.display()))
    }
}

/// 上次的执行是不是被打断了：状态还是 `running`，而且记录里的进程不是现在这个、也已经不在运行了
/// （进程号被别的程序复用时按可执行文件名区分）。
pub fn interrupted(path: &Path) -> Option<Interrupted> {
    let s = RunState::load(path)?;
    if s.status != "running" {
        return None;
    }
    if s.pid == std::process::id() || pid_is_diskwhy(s.pid) {
        return None;
    }
    let exit_code = if s.exe.is_empty() { None } else { pca_exit_code(&s.exe, s.started, s.updated) };
    let crash_logged = crate::crash::crashes_since(s.started) > 0;
    Some(Interrupted {
        batch: s.batch,
        started: s.started,
        processed: s.processed,
        processed_bytes: s.processed_bytes,
        total: s.total,
        exit_code,
        crash_logged,
    })
}

/// `%SystemRoot%\appcompat\pca\PcaGeneralDb0.txt`（和 Db1）：程序兼容性助手的记录，UTF-16LE，每行
/// `UTC 时间|类型|不带盘符的路径|…|说明`；类型 2 的说明形如 `Abnormal process exit with code 0x8f`。
/// 找 `exe` 在 [started − 60 秒, last_update + 1 小时] 之间最后一次异常退出的退出码。读不到 / 没有返回 None。
pub fn pca_exit_code(exe: &str, started: i64, last_update: i64) -> Option<String> {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let mut best: Option<(i64, String)> = None;
    for f in ["PcaGeneralDb0.txt", "PcaGeneralDb1.txt"] {
        let Ok(b) = std::fs::read(Path::new(&root).join("appcompat").join("pca").join(f)) else { continue };
        let text = decode_utf16le(&b);
        if let Some(hit) = pca_find(&text, exe, started - 60, last_update.max(started) + 3600) {
            if best.as_ref().is_none_or(|(t, _)| hit.0 > *t) {
                best = Some(hit);
            }
        }
    }
    best.map(|(_, c)| c)
}

fn decode_utf16le(b: &[u8]) -> String {
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let u = if u.first() == Some(&0xFEFF) { &u[1..] } else { &u[..] };
    String::from_utf16_lossy(u)
}

/// 在 PCA 文本里找：(时间, 退出码)。路径比较不看盘符、不分大小写。
pub fn pca_find(text: &str, exe: &str, from: i64, to: i64) -> Option<(i64, String)> {
    // 路径列有两种写法：不带盘符（`\desk\…\DiskWhy.exe`）或以环境变量开头（`%programfiles(x86)%\…`）
    let no_drive = |p: &str| -> String { p.get(2..).filter(|_| p.as_bytes().get(1) == Some(&b':')).unwrap_or(p).to_lowercase() };
    let expand = |p: &str| -> String {
        if let Some(rest) = p.strip_prefix('%') {
            if let Some((var, tail)) = rest.split_once('%') {
                if let Ok(v) = std::env::var(var) {
                    return format!("{v}{tail}");
                }
            }
        }
        p.to_string()
    };
    let want = no_drive(exe);
    let mut best: Option<(i64, String)> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() < 4 || f[1].trim() != "2" || no_drive(&expand(f[2].trim())) != want {
            continue;
        }
        let Some(t) = parse_pca_time(f[0]) else { continue };
        if t < from || t > to {
            continue;
        }
        let last = f[f.len() - 1];
        let Some(i) = last.find("code ") else { continue };
        let code = last[i + 5..].trim().to_string();
        if best.as_ref().is_none_or(|(bt, _)| t >= *bt) {
            best = Some((t, code));
        }
    }
    best
}

/// `2026-10-06 12:19:52.787`（UTC）→ Unix 秒。
fn parse_pca_time(s: &str) -> Option<i64> {
    let (d, t) = s.trim().split_once(' ')?;
    let mut dp = d.split('-').map(|x| x.parse::<i64>());
    let (y, mo, da) = (dp.next()?.ok()?, dp.next()?.ok()?, dp.next()?.ok()?);
    let t = t.split('.').next()?;
    let mut tp = t.split(':').map(|x| x.parse::<u32>());
    let (h, mi, se) = (tp.next()?.ok()?, tp.next()?.ok()?, tp.next()?.ok()?);
    Some(crate::crash::unix_from_civil(y, mo as u32, da as u32, h, mi, se))
}

/// 用户看过提示了：改成 `dismissed`。
pub fn dismiss(path: &Path) -> Result<(), String> {
    match RunState::load(path) {
        Some(mut s) if s.status == "running" => {
            s.status = "dismissed".into();
            s.updated = crate::explain::now_unix();
            s.save(path)
        }
        _ => Ok(()),
    }
}

/// 这个进程号现在是不是一个还在运行的、和当前程序同名的进程。
#[cfg(windows)]
fn pid_is_diskwhy(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    if pid == 0 {
        return false;
    }
    let me = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut n) != 0;
        // 进程已经退出但句柄还有人拿着时 OpenProcess 也能成功：再看退出码
        let mut code = 0u32;
        let alive = windows_sys::Win32::System::Threading::GetExitCodeProcess(h, &mut code) != 0 && code == 259; // STILL_ACTIVE
        CloseHandle(h);
        if !ok || !alive {
            return false;
        }
        let name = String::from_utf16_lossy(&buf[..n as usize]).to_lowercase();
        match me {
            Some(m) => name.ends_with(&m),
            None => false,
        }
    }
}

#[cfg(not(windows))]
fn pid_is_diskwhy(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_state_from_dead_process_is_interrupted() {
        let dir = std::env::temp_dir().join(format!("dw-runstate-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("last-run.json");
        let _ = std::fs::remove_file(&p);
        assert!(interrupted(&p).is_none());
        // 当前进程自己写的 running：不算打断
        let mut s = RunState { batch: "b1".into(), status: "running".into(), pid: std::process::id(), started: 1, updated: 1, total: 10, processed: 4, processed_bytes: 400, ..Default::default() };
        s.save(&p).unwrap();
        assert!(interrupted(&p).is_none());
        // 一个不存在的进程号
        s.pid = 0xFFFF_FFF0;
        s.save(&p).unwrap();
        let i = interrupted(&p).expect("应该算被打断");
        assert_eq!((i.batch.as_str(), i.processed, i.processed_bytes, i.total), ("b1", 4, 400, 10));
        assert_eq!(i.exit_code, None, "没记 exe 时不查 PCA");
        dismiss(&p).unwrap();
        assert!(interrupted(&p).is_none());
        assert_eq!(RunState::load(&p).unwrap().status, "dismissed");
        s.status = "done".into();
        s.save(&p).unwrap();
        assert!(interrupted(&p).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pca_lines_give_exit_code() {
        let text = [
            r"2026-09-30 13:33:48.013|2|\desk\claudex\13\DiskWhy-app\DiskWhy.exe|diskwhy|diskwhy|0.1.0|00069|Abnormal process exit with code 0xffffffff",
            r"2026-10-06 12:19:42.676|2|\Program Files (x86)\Microsoft\EdgeWebView\msedgewebview2.exe|x|x|1|0|Abnormal process exit with code 0x8f",
            r"2026-10-06 12:19:52.787|2|\desk\claudex\13\DiskWhy-app\DiskWhy.exe|diskwhy|diskwhy|0.1.0|00069|Abnormal process exit with code 0x8f",
            r"2026-10-06 12:19:53.000|3|\desk\claudex\13\DiskWhy-app\DiskWhy.exe|diskwhy|diskwhy|0.1.0|00069|PCA resolve is called",
        ]
        .join("\r\n");
        let exe = r"D:\desk\claudex\13\DiskWhy-app\DiskWhy.exe";
        let start = crate::crash::unix_from_civil(2026, 10, 6, 11, 52, 8);
        let upd = crate::crash::unix_from_civil(2026, 10, 6, 12, 19, 47);
        let hit = pca_find(&text, exe, start - 60, upd + 3600).expect("应找到");
        assert_eq!(hit.1, "0x8f");
        assert_eq!(hit.0, crate::crash::unix_from_civil(2026, 10, 6, 12, 19, 52));
        // 时间窗之外（9 月 30 日那次）不算；别的程序不算
        assert!(pca_find(&text, exe, start + 7200, start + 9000).is_none());
        assert!(pca_find(&text, r"C:\x\other.exe", 0, i64::MAX).is_none());
        // 真机上读一次（只读，没有也不算错）
        let _ = pca_exit_code(exe, start, upd);
    }
}
