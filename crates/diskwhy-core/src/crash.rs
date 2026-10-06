//! 闪退取证（阶段 J）：`%LOCALAPPDATA%\DiskWhy\crash.log` + `%LOCALAPPDATA%\DiskWhy\crash\diskwhy-<时间>.dmp`。
//!
//! - [`install`]：
//!   1. `std::panic::set_hook`：把 panic 的消息、位置、线程名、完整 backtrace 追加到 `crash.log`，
//!      然后照常调用原来的 hook（默认行为不变：stderr 打印、该线程展开 / 主线程退出码 101）；
//!   2. `SetUnhandledExceptionFilter`：未处理的原生异常（访问违例、栈溢出……）写一个 minidump 到
//!      `crash\diskwhy-<UTC 时间>-<pid>.dmp`，并在 `crash.log` 记一行。按 MiniDumpWriteDump 文档的建议，
//!      **不在出事的线程里写**（它的栈可能已经用完），而是唤醒一个事先建好的专用线程去写，出事的线程只等它
//!      写完（最多 60 秒）；之后交给原来的过滤器，没有就返回 `EXCEPTION_CONTINUE_SEARCH`（照常结束 / WER）。
//!      过滤器和写 dump 的线程里不分配内存：路径事先算好放在静态区，文件名和日志行写在栈上的定长缓冲区里。
//! - [`note`]：其他值得记的事（执行中拦下关窗、WebView2 进程挂了、退出请求、前端报错……）也追加到 `crash.log`。
//!
//! 每行开头是 `YYYY-MM-DD HH:MM:SSZ [unix 秒]`（UTC），下次启动时据此判断“上次中断之后有没有崩溃记录”。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// crash.log 超过这个大小时（安装时检查）改名成 crash.log.1。
const ROTATE_BYTES: u64 = 2 << 20;
/// 最多保留几个 minidump（安装时删掉更早的，只删我们自己写的 `diskwhy-*.dmp`）。
const KEEP_DUMPS: usize = 5;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// `%LOCALAPPDATA%\DiskWhy\crash.log`。
pub fn default_log_path() -> Option<PathBuf> {
    crate::explain::llm::app_data_dir().map(|d| d.join("crash.log"))
}

/// 现在用的 crash.log（装过 [`install`] 就是装的那个，否则默认位置）。
pub fn log_path() -> Option<PathBuf> {
    LOG_PATH.get().cloned().or_else(default_log_path)
}

/// 安装到默认位置（`%LOCALAPPDATA%\DiskWhy`）。找不到 %LOCALAPPDATA% 时什么都不做。
pub fn install() {
    if let Some(d) = crate::explain::llm::app_data_dir() {
        install_at(&d);
    }
}

/// 安装到指定文件夹：`<dir>\crash.log`、`<dir>\crash\*.dmp`。只第一次调用有效。
pub fn install_at(dir: &Path) {
    let log = dir.join("crash.log");
    if LOG_PATH.set(log.clone()).is_err() {
        return;
    }
    let _ = std::fs::create_dir_all(dir);
    if std::fs::metadata(&log).map(|m| m.len() > ROTATE_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&log, dir.join("crash.log.1"));
    }
    let dump_dir = dir.join("crash");
    let _ = std::fs::create_dir_all(&dump_dir);
    prune_dumps(&dump_dir);
    install_panic_hook();
    #[cfg(windows)]
    native::install(&dump_dir, &log);
}

/// 追加一行（`kind` 是短标签，如 `close-blocked`、`webview2`、`exit`、`frontend`）。失败就算了。
pub fn note(kind: &str, msg: &str) {
    let Some(p) = log_path() else { return };
    append(&p, &line(kind, msg));
}

fn line(kind: &str, msg: &str) -> String {
    let now = crate::explain::now_unix();
    format!("{} [unix {now}] {kind} pid={} {msg}\r\n", utc_text(now), std::process::id())
}

fn append(p: &Path, s: &str) {
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = f.write_all(s.as_bytes());
    }
}

/// Unix 秒 → `YYYY-MM-DD HH:MM:SSZ`。
pub fn utc_text(t: i64) -> String {
    let (y, mo, d, h, mi, s) = civil(t);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}Z")
}

/// Unix 秒 → (年, 月, 日, 时, 分, 秒)，UTC（Howard Hinnant 的 civil_from_days）。
pub fn civil(t: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (secs / 3600) as u32, (secs % 3600 / 60) as u32, (secs % 60) as u32)
}

/// (年, 月, 日, 时, 分, 秒) UTC → Unix 秒（civil 的反函数）。
pub fn unix_from_civil(y: i64, m: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + h as i64 * 3600 + mi as i64 * 60 + s as i64
}

/// crash.log 里 `unix` 时间 ≥ `since` 的 panic / 原生异常记录条数（下次启动判断“中断是不是崩溃”用）。
pub fn crashes_since(since: i64) -> usize {
    let Some(p) = log_path() else { return 0 };
    let Ok(s) = std::fs::read_to_string(p) else { return 0 };
    s.lines()
        .filter(|l| l.contains(" panic pid=") || l.contains(" native pid="))
        .filter_map(|l| {
            let a = l.find("[unix ")? + 6;
            let b = a + l[a..].find(']')?;
            l[a..b].trim().parse::<i64>().ok()
        })
        .filter(|&t| t >= since)
        .count()
}

fn prune_dumps(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut dumps: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.starts_with("diskwhy-") && n.ends_with(".dmp")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if dumps.len() <= KEEP_DUMPS {
        return;
    }
    dumps.sort();
    let n = dumps.len() - KEEP_DUMPS;
    for (_, p) in dumps.into_iter().take(n) {
        let _ = std::fs::remove_file(p);
    }
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let th = std::thread::current();
        let name = th.name().unwrap_or("<unnamed>").to_string();
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<非文本的 panic 内容>".to_string()
        };
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "<未知位置>".into());
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!("thread '{name}' ({:?}) panicked at {loc}: {msg}\r\nbacktrace:\r\n{bt}", th.id());
        if let Some(p) = log_path() {
            append(&p, &line("panic", &text.replace('\n', "\r\n").replace("\r\r\n", "\r\n")));
        }
        prev(info);
    }));
}

#[cfg(windows)]
mod native {
    //! 未处理原生异常 → minidump。见模块文档。
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, WriteFile, CREATE_ALWAYS, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_ALWAYS,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        MiniDumpWithHandleData, MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules,
        MiniDumpWriteDump, SetUnhandledExceptionFilter, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER,
        MINIDUMP_EXCEPTION_INFORMATION,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemTime;
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, SetEvent, WaitForSingleObject, INFINITE,
    };

    /// 事先算好的宽字符路径（不含结尾 NUL）。
    static DUMP_DIR: OnceLock<Vec<u16>> = OnceLock::new();
    static LOG_FILE: OnceLock<Vec<u16>> = OnceLock::new();
    static REQ: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());
    static DONE: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());
    static EXC: AtomicPtr<EXCEPTION_POINTERS> = AtomicPtr::new(null_mut());
    static EXC_TID: AtomicU32 = AtomicU32::new(0);
    static BUSY: AtomicBool = AtomicBool::new(false);
    static PREV: AtomicUsize = AtomicUsize::new(0);

    fn wide(p: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        p.as_os_str().encode_wide().collect()
    }

    pub fn install(dump_dir: &Path, log: &Path) {
        let mut d = wide(dump_dir);
        if d.last() != Some(&(b'\\' as u16)) {
            d.push(b'\\' as u16);
        }
        let _ = DUMP_DIR.set(d);
        let mut l = wide(log);
        l.push(0);
        let _ = LOG_FILE.set(l);
        unsafe {
            // 自动复位事件：一次请求、一次完成
            let req = CreateEventW(null(), 0, 0, null());
            let done = CreateEventW(null(), 0, 0, null());
            if req.is_null() || done.is_null() {
                return;
            }
            REQ.store(req, Ordering::SeqCst);
            DONE.store(done, Ordering::SeqCst);
        }
        // 专用线程：平时一直等；被唤醒就写 dump + 日志行。栈给足，不靠出事线程的栈。
        let spawned = std::thread::Builder::new().name("crash-dumper".into()).stack_size(1 << 20).spawn(|| loop {
            let req = REQ.load(Ordering::SeqCst) as HANDLE;
            unsafe {
                WaitForSingleObject(req, INFINITE);
                write_dump();
                SetEvent(DONE.load(Ordering::SeqCst) as HANDLE);
            }
        });
        if spawned.is_err() {
            return;
        }
        let prev: LPTOP_LEVEL_EXCEPTION_FILTER = unsafe { SetUnhandledExceptionFilter(Some(filter)) };
        if let Some(f) = prev {
            PREV.store(f as usize, Ordering::SeqCst);
        }
    }

    /// 顶层异常过滤器：在出事的线程里运行。只记下异常指针、唤醒写 dump 的线程、等它写完，然后交给原来的过滤器。
    unsafe extern "system" fn filter(ep: *const EXCEPTION_POINTERS) -> i32 {
        unsafe {
            if !BUSY.swap(true, Ordering::SeqCst) {
                let req = REQ.load(Ordering::SeqCst) as HANDLE;
                let done = DONE.load(Ordering::SeqCst) as HANDLE;
                if !req.is_null() && !done.is_null() {
                    EXC.store(ep as *mut _, Ordering::SeqCst);
                    EXC_TID.store(GetCurrentThreadId(), Ordering::SeqCst);
                    SetEvent(req);
                    WaitForSingleObject(done, 60_000);
                }
            }
            let prev = PREV.load(Ordering::SeqCst);
            if prev != 0 {
                let f: unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32 = core::mem::transmute(prev);
                return f(ep);
            }
            EXCEPTION_CONTINUE_SEARCH
        }
    }

    /// 栈上的定长缓冲区，实现 fmt::Write（不分配内存）。
    struct Buf<const N: usize> {
        b: [u8; N],
        n: usize,
    }

    impl<const N: usize> core::fmt::Write for Buf<N> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let k = s.len().min(N - self.n);
            self.b[self.n..self.n + k].copy_from_slice(&s.as_bytes()[..k]);
            self.n += k;
            Ok(())
        }
    }

    unsafe fn write_dump() {
        use core::fmt::Write as _;
        unsafe {
            let ep = EXC.load(Ordering::SeqCst);
            let tid = EXC_TID.load(Ordering::SeqCst);
            let pid = GetCurrentProcessId();
            let st = {
                let mut st = core::mem::zeroed();
                GetSystemTime(&mut st);
                st
            };
            // 文件名：diskwhy-YYYYMMDD-HHMMSS-<pid>.dmp
            let mut name: Buf<64> = Buf { b: [0; 64], n: 0 };
            let _ = write!(name, "diskwhy-{:04}{:02}{:02}-{:02}{:02}{:02}-{pid}.dmp", st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond);
            let mut path = [0u16; 1024];
            let mut k = 0usize;
            if let Some(d) = DUMP_DIR.get() {
                for &c in d.iter().take(path.len() - 80) {
                    path[k] = c;
                    k += 1;
                }
            }
            for &c in &name.b[..name.n] {
                path[k] = c as u16;
                k += 1;
            }
            path[k] = 0;
            let h = CreateFileW(path.as_ptr(), GENERIC_WRITE, 0, null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, null_mut());
            let (ok, err) = if h == INVALID_HANDLE_VALUE || h.is_null() {
                (false, GetLastError())
            } else {
                let mei = MINIDUMP_EXCEPTION_INFORMATION { ThreadId: tid, ExceptionPointers: ep, ClientPointers: 0 };
                let ty = MiniDumpWithThreadInfo | MiniDumpWithIndirectlyReferencedMemory | MiniDumpWithUnloadedModules | MiniDumpWithHandleData;
                let r = MiniDumpWriteDump(GetCurrentProcess(), pid, h, ty, if ep.is_null() { null() } else { &mei }, null(), null());
                let e = if r == 0 { GetLastError() } else { 0 };
                CloseHandle(h);
                (r != 0, e)
            };
            // 日志行
            let (code, addr) = if !ep.is_null() && !(*ep).ExceptionRecord.is_null() {
                let rec = &*(*ep).ExceptionRecord;
                (rec.ExceptionCode as u32, rec.ExceptionAddress as usize)
            } else {
                (0, 0)
            };
            let unix = super::unix_from_civil(st.wYear as i64, st.wMonth as u32, st.wDay as u32, st.wHour as u32, st.wMinute as u32, st.wSecond as u32);
            let mut lb: Buf<512> = Buf { b: [0; 512], n: 0 };
            let _ = write!(
                lb,
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}Z [unix {unix}] native pid={pid} 未处理的原生异常 0x{code:08X}，地址 0x{addr:X}，线程 {tid}；minidump ",
                st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
            );
            let _ = lb.write_str(core::str::from_utf8(&name.b[..name.n]).unwrap_or("?"));
            if ok {
                let _ = lb.write_str(" 已写好\r\n");
            } else {
                let _ = write!(lb, " 写失败（错误 0x{err:08X}）\r\n");
            }
            if let Some(l) = LOG_FILE.get() {
                let lh = CreateFileW(l.as_ptr(), FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, null_mut());
                if lh != INVALID_HANDLE_VALUE && !lh.is_null() {
                    let mut w = 0u32;
                    WriteFile(lh, lb.b.as_ptr(), lb.n as u32, &mut w, null_mut());
                    CloseHandle(lh);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        // 2026-10-06 12:19:52Z（PCA 里那次退出的时间）
        let t = unix_from_civil(2026, 10, 6, 12, 19, 52);
        assert_eq!(civil(t), (2026, 10, 6, 12, 19, 52));
        assert_eq!(utc_text(0), "1970-01-01 00:00:00Z");
        assert_eq!(unix_from_civil(2000, 2, 29, 0, 0, 0), 951_782_400);
        for t in [0i64, 951_782_400, 1_759_753_192, 4_102_444_800] {
            let (y, mo, d, h, mi, s) = civil(t);
            assert_eq!(unix_from_civil(y, mo, d, h, mi, s), t);
        }
    }
}
