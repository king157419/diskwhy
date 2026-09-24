//! Tauri 壳：只做 IPC 转发，所有逻辑在 diskwhy-core。
//!
//! 命令一览（前端用 `invoke` 调用，见 src/api.ts）：
//! - `get_env`            -> EnvInfo（是否管理员、规则库条数、规则警告、AI 是否可用）
//! - `list_drives`        -> Vec<DriveInfo>（本机固定盘）
//! - `start_scan`         (root, onProgress: Channel<ScanProgress>) -> ScanSummary（含各档合计 tiers）
//! - `cancel_scan`
//! - `list_dir`           (path, limit?) -> DirListing（子项按 allocated 降序，每项带 explanation）
//! - `get_treemap`        (path, depth?) -> TreemapNode（阶段 C：方块图数据，最多 3 层，每块带档位）
//! - `node_info`          (path, kind) -> NodeView（方块图里点了更深一层的块时取详情）
//! - `explain`            (path, kind?) -> Explanation（阶段 B-4：单独取一项的完整解释，直接转发 core 的
//!                        `explain`；规则 → 启发式 → 未知，不联网。列表里的每一行已经随 `list_dir` 带回解释，
//!                        这个命令给详情面板补取 / 刷新用）
//! - `save_snapshot`      -> 保存路径（弹系统“另存为”对话框，写 gzip JSON）
//! - `open_snapshot`      (path?) -> ScanSummary（弹“打开”对话框读快照，替换当前结果；解释照常现算）
//! - `startup_snapshot`   -> 启动参数里给的快照路径（`diskwhy.exe 某次扫描.dwsnap`，或双击关联的 .dwsnap）
//! - `relaunch_as_admin`  以管理员身份重启（ShellExecuteW runas），成功后退出当前进程
//! - `get_settings` / `save_settings(settings)` -> LLM 设置（%LOCALAPPDATA%\DiskWhy\settings.json）
//! - `test_llm(settings)` -> 连接测试结果文本
//! - `ask_ai(path)`       -> Explanation（source = Llm，tier = review），只在用户点“问 AI”时调用
//!
//! 插件：`tauri-plugin-opener`（前端打开参考链接）、`tauri-plugin-dialog`（快照的保存/打开对话框，
//! 只在 Rust 侧调用，前端不需要 dialog 权限）。

use diskwhy_core::explain::llm::{self, LlmSettings};
use diskwhy_core::explain::{tier_summary, ExplainCtx, Explanation, Target, TierSummary};
use diskwhy_core::scan::scan;
use diskwhy_core::snapshot::{self, SnapshotMeta};
use diskwhy_core::view::{self, find_dir, find_node, listing, node_view, DirListing, NodeView, TreemapNode};
use diskwhy_core::win::{is_elevated, list_fixed_drives, relaunch_as_admin as core_relaunch, DriveInfo, RelaunchError};
use diskwhy_core::{
    Attribution, InaccessibleDir, MappedDir, NodeKind, ScanOptions, ScanProgress, ScanResult, SizeMode,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
struct AppState {
    result: Mutex<Option<Arc<ScanResult>>>,
    /// 当前结果如果来自快照，记下来源。
    snapshot: Mutex<Option<SnapshotInfo>>,
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

    fn result(&self) -> Result<Arc<ScanResult>, String> {
        self.result.lock().unwrap().clone().ok_or_else(|| "还没有扫描结果".to_string())
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

/// 快照来源信息（给界面显示）。
#[derive(Serialize, Clone)]
struct SnapshotInfo {
    path: String,
    meta: SnapshotMeta,
    /// 读取用时（毫秒）。
    load_ms: u64,
}

/// 扫描完成后返回给前端的摘要（不含整棵树；树通过 list_dir / get_treemap 按需取）。
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
    /// 商店应用目录映射（阶段 C 去重）。
    mapped_dirs: Vec<MappedDir>,
    /// 结果来自快照时不为空。
    snapshot: Option<SnapshotInfo>,
}

fn summarize(r: &ScanResult, tiers: TierSummary, snapshot: Option<SnapshotInfo>) -> ScanSummary {
    ScanSummary {
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
        mapped_dirs: r.mapped_dirs.clone(),
        snapshot,
    }
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
    *state.snapshot.lock().unwrap() = None;
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
    let summary = summarize(&r, tiers, None);
    *state.result.lock().unwrap() = Some(Arc::new(r));
    Ok(summary)
}

#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) {
    state.cancel.lock().unwrap().store(true, Ordering::SeqCst);
}

#[tauri::command]
fn list_dir(path: String, limit: Option<usize>, state: State<'_, AppState>) -> Result<DirListing, String> {
    let r = state.result()?;
    let ctx = state.ctx();
    listing(&r, &path, limit.unwrap_or(1000), Some(&ctx)).ok_or_else(|| format!("在扫描结果里找不到目录：{path}"))
}

#[tauri::command]
async fn get_treemap(path: String, depth: Option<usize>, state: State<'_, AppState>) -> Result<TreemapNode, String> {
    let r = state.result()?;
    let ctx = state.ctx();
    tauri::async_runtime::spawn_blocking(move || {
        view::treemap(&r, &path, depth.unwrap_or(3), Some(&ctx)).ok_or_else(|| format!("在扫描结果里找不到目录：{path}"))
    })
    .await
    .map_err(|e| format!("方块图线程异常：{e}"))?
}

/// 在扫描结果里找到一项并建好解释用的 [`Target`]，交给 `f`。
/// `kind = Rest` 时 `path` 是所在目录，找的是它的“其余小文件合计”。
fn with_target<T>(
    r: &ScanResult,
    path: &str,
    kind: NodeKind,
    f: impl FnOnce(&Target, Option<&diskwhy_core::Node>) -> T,
) -> Result<T, String> {
    let not_found = || format!("在扫描结果里找不到：{path}");
    let (chain, node, full) = if kind == NodeKind::Rest {
        let chain = find_dir(&r.tree, path).ok_or_else(not_found)?;
        let dir = *chain.last().unwrap();
        let rest = dir.children.iter().find(|c| c.kind == NodeKind::Rest).ok_or_else(not_found)?;
        (chain, rest, dir.path.clone())
    } else {
        let (chain, node) = find_node(&r.tree, path).ok_or_else(not_found)?;
        let full = if node.kind == NodeKind::Dir { node.path.clone() } else { path.to_string() };
        (chain, node, full)
    };
    let parent = chain.last().copied();
    let t = Target { node, path: &full, parent, ancestors: &chain };
    Ok(f(&t, parent))
}

/// 任意节点（目录 / 文件 / “其余小文件”）的详情视图，带解释。
#[tauri::command]
fn node_info(path: String, kind: NodeKind, state: State<'_, AppState>) -> Result<NodeView, String> {
    let r = state.result()?;
    let ctx = state.ctx();
    with_target(&r, &path, kind, |t, parent| {
        let node = t.node;
        let parent_path = parent.map(|p| p.path.as_str());
        let parent_alloc = parent.map(|p| p.allocated).unwrap_or(r.used_bytes);
        let mut v = node_view(node, parent_path, parent_alloc);
        if node.kind == NodeKind::Rest {
            v.path = t.path.to_string();
        }
        if node.mapped {
            v.mapped_to = view::mapped_to(&r, &node.path);
        }
        v.explanation = Some(diskwhy_core::explain::explain(t, &ctx));
        v
    })
}

/// 单独解释一项（阶段 B-4）：直接转发 core 的 `explain`（规则 → 启发式 → 未知，不联网，不问 AI）。
/// `kind` 省略时按路径找（目录或文件）。
#[tauri::command]
fn explain(path: String, kind: Option<NodeKind>, state: State<'_, AppState>) -> Result<Explanation, String> {
    let r = state.result()?;
    let ctx = state.ctx();
    with_target(&r, &path, kind.unwrap_or(NodeKind::Dir), |t, _| diskwhy_core::explain::explain(t, &ctx))
}

#[tauri::command]
async fn save_snapshot(app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let r = state.result()?;
    let default_name = format!(
        "DiskWhy-{}-{}.dwsnap",
        r.root.trim_end_matches(['\\', ':']).replace(['\\', ':'], "_"),
        diskwhy_core::explain::now_unix()
    );
    tauri::async_runtime::spawn_blocking(move || {
        let Some(fp) = app
            .dialog()
            .file()
            .set_title("保存扫描快照")
            .set_file_name(default_name)
            .add_filter("DiskWhy 快照", &["dwsnap"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let p: PathBuf = fp.into_path().map_err(|e| e.to_string())?;
        let p2 = p.clone();
        let n = snapshot::with_big_stack(move || snapshot::save(&r, &p2, env!("CARGO_PKG_VERSION")))?;
        Ok(Some(format!("{}（{}）", p.display(), view::human_bytes(n))))
    })
    .await
    .map_err(|e| format!("保存线程异常：{e}"))?
}

#[tauri::command]
async fn open_snapshot(path: Option<String>, app: AppHandle, state: State<'_, AppState>) -> Result<Option<ScanSummary>, String> {
    if state.scanning.load(Ordering::SeqCst) {
        return Err("正在扫描，先等扫描结束或取消".into());
    }
    let ctx = state.ctx();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let p: PathBuf = match path {
            Some(p) => PathBuf::from(p),
            None => {
                let Some(fp) = app
                    .dialog()
                    .file()
                    .set_title("打开扫描快照")
                    .add_filter("DiskWhy 快照 / CLI JSON", &["dwsnap", "json"])
                    .blocking_pick_file()
                else {
                    return Ok(None);
                };
                fp.into_path().map_err(|e| e.to_string())?
            }
        };
        let t = std::time::Instant::now();
        let p2 = p.clone();
        let (r, meta) = snapshot::with_big_stack(move || snapshot::load(&p2))?;
        let load_ms = t.elapsed().as_millis() as u64;
        let tiers = tier_summary(&r.tree, &ctx);
        let info = SnapshotInfo { path: p.display().to_string(), meta, load_ms };
        Ok::<_, String>(Some((r, tiers, info)))
    })
    .await
    .map_err(|e| format!("读取线程异常：{e}"))??;
    let Some((r, tiers, info)) = res else { return Ok(None) };
    let summary = summarize(&r, tiers, Some(info.clone()));
    *state.result.lock().unwrap() = Some(Arc::new(r));
    *state.snapshot.lock().unwrap() = Some(info);
    Ok(Some(summary))
}

/// 启动参数里的快照文件（第一个以 .dwsnap / .json 结尾的参数）。前端启动时问一次，有就直接打开。
#[tauri::command]
fn startup_snapshot() -> Option<String> {
    std::env::args().skip(1).find(|a| {
        let l = a.to_lowercase();
        (l.ends_with(".dwsnap") || l.ends_with(".json")) && std::path::Path::new(a).is_file()
    })
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
    let r = state.result()?;
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_env,
            list_drives,
            start_scan,
            cancel_scan,
            list_dir,
            get_treemap,
            node_info,
            explain,
            save_snapshot,
            open_snapshot,
            startup_snapshot,
            relaunch_as_admin,
            get_settings,
            save_settings,
            test_llm,
            ask_ai
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
