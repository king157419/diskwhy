//! Tauri 壳：只做 IPC 转发，所有逻辑在 diskwhy-core。
//!
//! 命令一览（前端用 `invoke` 调用，见 src/api.ts）：
//! - `get_env`            -> EnvInfo（是否管理员、规则库条数、规则警告、AI 是否可用）
//! - `list_drives`        -> Vec<DriveInfo>（本机固定盘）
//! - `start_scan`         (root, onProgress: Channel<ScanProgress>) -> ScanSummary（含各档合计 tiers）
//! - `cancel_scan`
//! - `list_dir`           (path, limit?) -> DirListing（子项按 allocated 降序，每项带 explanation）
//! - `relaunch_as_admin`  以管理员身份重启（ShellExecuteW runas），成功后退出当前进程
//! - `get_settings` / `save_settings(settings)` -> LLM 设置（%LOCALAPPDATA%\DiskWhy\settings.json）
//! - `test_llm(settings)` -> 连接测试结果文本
//! - `ask_ai(path)`       -> Explanation（source = Llm，tier = review），只在用户点“问 AI”时调用

use diskwhy_core::explain::llm::{self, LlmSettings};
use diskwhy_core::explain::{tier_summary, ExplainCtx, Explanation, Target, TierSummary};
use diskwhy_core::scan::scan;
use diskwhy_core::view::{find_node, listing, DirListing};
use diskwhy_core::win::{is_elevated, list_fixed_drives, relaunch_as_admin as core_relaunch, DriveInfo, RelaunchError};
use diskwhy_core::{Attribution, InaccessibleDir, NodeKind, ScanOptions, ScanProgress, ScanResult, SizeMode};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

#[derive(Default)]
struct AppState {
    result: Mutex<Option<Arc<ScanResult>>>,
    cancel: Mutex<Arc<AtomicBool>>,
    scanning: AtomicBool,
    /// 解释上下文（环境变量 + 规则库 + 注册表快照 + 设置），第一次用到时加载一次。
    ctx: RwLock<Option<(Arc<ExplainCtx>, Vec<String>)>>,
}

impl AppState {
    fn ctx(&self) -> Arc<ExplainCtx> {
        if let Some((c, _)) = self.ctx.read().unwrap().as_ref() {
            return c.clone();
        }
        let mut w = self.ctx.write().unwrap();
        if w.is_none() {
            let (c, warnings) = ExplainCtx::load_default();
            *w = Some((Arc::new(c), warnings));
        }
        w.as_ref().unwrap().0.clone()
    }

    fn warnings(&self) -> Vec<String> {
        self.ctx();
        self.ctx.read().unwrap().as_ref().map(|x| x.1.clone()).unwrap_or_default()
    }

    /// 换设置：复制一份上下文，只改 settings。
    fn set_settings(&self, s: LlmSettings) {
        let cur = self.ctx();
        let mut next = (*cur).clone();
        next.settings = s;
        let mut w = self.ctx.write().unwrap();
        let warnings = w.as_ref().map(|x| x.1.clone()).unwrap_or_default();
        *w = Some((Arc::new(next), warnings));
    }
}

#[derive(Serialize)]
struct EnvInfo {
    elevated: bool,
    version: &'static str,
    rules_total: usize,
    rule_warnings: Vec<String>,
    registry_apps: usize,
    llm_ready: bool,
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
    /// 本次扫描按规则库归到 safe / review / keep 的合计。
    tiers: TierSummary,
}

#[tauri::command]
fn get_env(state: State<'_, AppState>) -> EnvInfo {
    let ctx = state.ctx();
    EnvInfo {
        elevated: is_elevated(),
        version: env!("CARGO_PKG_VERSION"),
        rules_total: ctx.rules.rules.len(),
        rule_warnings: state.warnings(),
        registry_apps: ctx.registry.apps.len(),
        llm_ready: ctx.settings.usable(),
    }
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
    let ctx = state.ctx();

    let res = tauri::async_runtime::spawn_blocking(move || {
        let r = scan(&root, &ScanOptions::default(), &cancel, |p| {
            let _ = on_progress.send(p.clone());
        })?;
        let tiers = tier_summary(&r.tree, &ctx);
        Ok::<_, diskwhy_core::ScanError>((r, tiers))
    })
    .await;
    state.scanning.store(false, Ordering::SeqCst);

    let (r, tiers) = res.map_err(|e| format!("扫描线程异常：{e}"))?.map_err(|e| e.to_string())?;
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
        tiers,
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
    let r = state.result.lock().unwrap().clone().ok_or("还没有扫描结果")?;
    let ctx = state.ctx();
    listing(&r, &path, limit.unwrap_or(1000), Some(&ctx)).ok_or_else(|| format!("在扫描结果里找不到目录：{path}"))
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

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> LlmSettings {
    state.ctx().settings.clone()
}

#[tauri::command]
fn save_settings(settings: LlmSettings, state: State<'_, AppState>) -> Result<String, String> {
    let p = settings.save()?;
    state.set_settings(settings);
    Ok(p.display().to_string())
}

#[tauri::command]
async fn test_llm(settings: LlmSettings) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || llm::test_connection(&settings))
        .await
        .map_err(|e| format!("测试线程异常：{e}"))?
}

#[tauri::command]
async fn ask_ai(path: String, state: State<'_, AppState>) -> Result<Explanation, String> {
    let r = state.result.lock().unwrap().clone().ok_or("还没有扫描结果")?;
    let ctx = state.ctx();
    tauri::async_runtime::spawn_blocking(move || {
        let (chain, node) = find_node(&r.tree, &path).ok_or_else(|| format!("在扫描结果里找不到：{path}"))?;
        if node.kind == NodeKind::Rest {
            return Err("“其余小文件合计”不是一个真实的文件夹，没法问 AI".to_string());
        }
        let full = if node.kind == NodeKind::Dir { node.path.clone() } else { path.clone() };
        let t = Target { node, path: &full, parent: chain.last().copied(), ancestors: &chain };
        llm::ask(&t, &ctx)
    })
    .await
    .map_err(|e| format!("AI 线程异常：{e}"))?
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
            relaunch_as_admin,
            get_settings,
            save_settings,
            test_llm,
            ask_ai
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
