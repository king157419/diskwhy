//! Tauri 壳：只做 IPC 转发，所有逻辑在 diskwhy-core。
//!
//! 命令一览（前端用 `invoke` 调用，见 src/api.ts）：
//! - `get_env`            -> EnvInfo（是否管理员）
//! - `list_drives`        -> Vec<DriveInfo>（本机固定盘）
//! - `start_scan`         (root, onProgress: Channel<ScanProgress>) -> ScanSummary
//! - `cancel_scan`
//! - `list_dir`           (path, limit?) -> DirListing（某目录的子项，按 allocated 降序）
//! - `relaunch_as_admin`  以管理员身份重启（ShellExecuteW runas），成功后退出当前进程

use diskwhy_core::scan::scan;
use diskwhy_core::view::{listing, DirListing};
use diskwhy_core::win::{is_elevated, list_fixed_drives, relaunch_as_admin as core_relaunch, DriveInfo, RelaunchError};
use diskwhy_core::{Attribution, InaccessibleDir, ScanOptions, ScanProgress, ScanResult, SizeMode};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

#[derive(Default)]
struct AppState {
    result: Mutex<Option<Arc<ScanResult>>>,
    cancel: Mutex<Arc<AtomicBool>>,
    scanning: AtomicBool,
}

#[derive(Serialize)]
struct EnvInfo {
    elevated: bool,
    version: &'static str,
}

/// 扫描完成后返回给前端的摘要（不含整棵树；树通过 list_dir 按需取）。
#[derive(Serialize)]
struct ScanSummary {
    root: String,
    total_bytes: u64,
    used_bytes: u64,
    free_bytes: u64,
    cluster_size: u64,
    buckets: Attribution,
    duration_ms: u64,
    files: u64,
    dirs: u64,
    elevated: bool,
    inaccessible_total: u64,
    inaccessible_sample: Vec<InaccessibleDir>,
    size_mode: SizeMode,
    top_files_per_dir: usize,
}

#[tauri::command]
fn get_env() -> EnvInfo {
    EnvInfo { elevated: is_elevated(), version: env!("CARGO_PKG_VERSION") }
}

#[tauri::command]
fn list_drives() -> Vec<DriveInfo> {
    list_fixed_drives()
}

#[tauri::command]
async fn start_scan(
    root: String,
    on_progress: Channel<ScanProgress>,
    state: State<'_, AppState>,
) -> Result<ScanSummary, String> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("已有扫描正在进行".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = cancel.clone();
    // 扫描期间释放旧结果，避免同时占两份内存
    *state.result.lock().unwrap() = None;

    let res = tauri::async_runtime::spawn_blocking(move || {
        scan(&root, &ScanOptions::default(), &cancel, |p| {
            let _ = on_progress.send(p.clone());
        })
    })
    .await;
    state.scanning.store(false, Ordering::SeqCst);

    let r = res.map_err(|e| format!("扫描线程异常：{e}"))?.map_err(|e| e.to_string())?;
    let summary = ScanSummary {
        root: r.root.clone(),
        total_bytes: r.total_bytes,
        used_bytes: r.used_bytes,
        free_bytes: r.free_bytes,
        cluster_size: r.cluster_size,
        buckets: r.buckets.clone(),
        duration_ms: r.duration_ms,
        files: r.files,
        dirs: r.dirs,
        elevated: r.elevated,
        inaccessible_total: r.inaccessible_total,
        inaccessible_sample: r.inaccessible_dirs.iter().take(50).cloned().collect(),
        size_mode: r.size_mode,
        top_files_per_dir: r.top_files_per_dir,
    };
    *state.result.lock().unwrap() = Some(Arc::new(r));
    Ok(summary)
}

#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) {
    state.cancel.lock().unwrap().store(true, Ordering::SeqCst);
}

#[tauri::command]
fn list_dir(path: String, limit: Option<usize>, state: State<'_, AppState>) -> Result<DirListing, String> {
    let guard = state.result.lock().unwrap();
    let r = guard.as_ref().ok_or("还没有扫描结果")?;
    listing(r, &path, limit.unwrap_or(1000)).ok_or_else(|| format!("在扫描结果里找不到目录：{path}"))
}

#[tauri::command]
fn relaunch_as_admin(app: AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    match core_relaunch(&exe.to_string_lossy(), "") {
        Ok(()) => {
            app.exit(0);
            Ok(())
        }
        Err(RelaunchError::Cancelled) => Err("你取消了管理员授权".into()),
        Err(RelaunchError::Failed(s)) => Err(s),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_env,
            list_drives,
            start_scan,
            cancel_scan,
            list_dir,
            relaunch_as_admin
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
