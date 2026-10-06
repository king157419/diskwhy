//! 阶段 J 闪退取证：panic hook 写 crash.log；未处理原生异常写 minidump + crash.log 一行。
//!
//! 用子进程测：本测试程序带环境变量重新运行自己里面的 `child_*` 测试（不带环境变量时它们什么都不做）。
//! 测试文件放在 `DISKWHY_TEST_DIR`（默认 `D:\desk\claudex\13`，没有就用临时目录）下的 `dw-test-crash-<pid>`，结束删掉。
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn test_root(tag: &str) -> PathBuf {
    let base = std::env::var_os("DISKWHY_TEST_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| Some(PathBuf::from(r"D:\desk\claudex\13")).filter(|p| p.is_dir()))
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("dw-test-crash-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run_child(test: &str, var: &str, dir: &Path) -> std::process::Output {
    Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(var, dir)
        .output()
        .unwrap()
}

#[test]
fn child_panic() {
    let Some(dir) = std::env::var_os("DW_CRASH_CHILD_PANIC") else { return };
    diskwhy_core::crash::install_at(Path::new(&dir));
    let h = std::thread::Builder::new().name("exec-7".into()).spawn(|| panic!("测试 panic 42")).unwrap();
    assert!(h.join().is_err());
    diskwhy_core::crash::note("test", "panic 之后还能写");
}

#[test]
fn child_native_crash() {
    let Some(dir) = std::env::var_os("DW_CRASH_CHILD_NATIVE") else { return };
    // 不弹 WER 的“已停止工作”对话框
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(
            windows_sys::Win32::System::Diagnostics::Debug::SEM_NOGPFAULTERRORBOX | windows_sys::Win32::System::Diagnostics::Debug::SEM_FAILCRITICALERRORS,
        );
    }
    diskwhy_core::crash::install_at(Path::new(&dir));
    let h = std::thread::Builder::new()
        .name("boom".into())
        .spawn(|| unsafe {
            // 非空、对齐的坏地址：真正的访问违例（0xC0000005），不是 Rust 的 panic
            std::ptr::write_volatile(0x10 as *mut u32, 1);
        })
        .unwrap();
    let _ = h.join();
    unreachable!("应该已经崩溃了");
}

#[test]
fn panic_hook_writes_crash_log_and_keeps_default_behaviour() {
    let dir = test_root("panic");
    let out = run_child("child_panic", "DW_CRASH_CHILD_PANIC", &dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "子进程应正常结束（panic 在子线程里）：{stdout}\n{stderr}");
    // 默认 hook 照常打印
    assert!(stderr.contains("panicked at") && stderr.contains("测试 panic 42"), "stderr: {stderr}");
    let log = std::fs::read_to_string(dir.join("crash.log")).expect("应该有 crash.log");
    assert!(log.contains(" panic pid="), "{log}");
    assert!(log.contains("测试 panic 42"), "{log}");
    assert!(log.contains("thread 'exec-7'"), "线程名：{log}");
    assert!(log.contains("crash_tests.rs:"), "位置：{log}");
    assert!(log.contains("backtrace:") && log.contains("child_panic"), "backtrace 里应有函数名：{log}");
    assert!(log.contains(" test pid=") && log.contains("panic 之后还能写"), "{log}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn native_exception_writes_minidump() {
    let dir = test_root("native");
    let out = run_child("child_native_crash", "DW_CRASH_CHILD_NATIVE", &dir);
    let code = out.status.code().map(|c| c as u32);
    assert_eq!(code, Some(0xC000_0005), "应是访问违例退出：{:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    let dumps: Vec<_> = std::fs::read_dir(dir.join("crash"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".dmp"))
        .collect();
    assert_eq!(dumps.len(), 1, "应有一个 minidump");
    let d = &dumps[0];
    let bytes = std::fs::read(d.path()).unwrap();
    assert!(bytes.len() > 4096, "dump 太小：{}", bytes.len());
    assert_eq!(&bytes[..4], b"MDMP", "minidump 文件头");
    let name = d.file_name().to_string_lossy().to_string();
    assert!(name.starts_with("diskwhy-"), "{name}");
    let log = std::fs::read_to_string(dir.join("crash.log")).expect("应该有 crash.log");
    assert!(log.contains(" native pid="), "{log}");
    assert!(log.contains("0xC0000005"), "{log}");
    assert!(log.contains(&name) && log.contains("已写好"), "{log}");
    let _ = std::fs::remove_dir_all(&dir);
}
