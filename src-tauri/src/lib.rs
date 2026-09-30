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
//! - `save_snapshot`      -> 保存路径（弹系统“另存为”对话框，写 gzip JSON）
//! - `open_snapshot`      (path?) -> ScanSummary（弹“打开”对话框读快照，替换当前结果；解释照常现算）
//! - `startup_snapshot`   -> 启动参数里给的快照路径（`diskwhy.exe 某次扫描.dwsnap`，或双击关联的 .dwsnap）
//! - `relaunch_as_admin`  以管理员身份重启（ShellExecuteW runas），成功后退出当前进程
//! - `get_settings` / `save_settings(settings)` -> LLM 设置（%LOCALAPPDATA%\DiskWhy\settings.json）
//! - `test_llm(settings)` -> 连接测试结果文本
//! - `ask_ai(path)`       -> Explanation（source = Llm，tier = review），只在用户点“问 AI”时调用
//! - `get_ui_prefs` / `set_ui_prefs(prefs)` -> 界面偏好（主题 A / B / C），存在 %LOCALAPPDATA%\DiskWhy\ui.json
//!
//! 阶段 F：`ScanSummary.candidates` = 清理页的候选（按规则 / 启发式分组，见 `diskwhy_core::explain::candidates`），
//! 扫描、打开快照、清理 / 还原后的刷新都会重新算。
//!
//! 阶段 D（带护栏的动作，逻辑全在 `diskwhy_core::action`）：
//! - `action_env`         -> 盘符 + 隔离区批次（超过 7 天的个数）+ 日志位置
//! - `preview_plan(items, strategy, onEvent: Channel<PreviewEvent>)` -> PreflightReport（后端按当前扫描结果重新判断档位 / 规则 / 保护路径，
//!   先发 quote 让弹层立刻出现，再在后台枚举实际内容并预检，期间发 progress；阶段 G）
//! - `cancel_preview()`：取消正在进行的预检
//! - `execute_plan(plan_id, skip_locked, onProgress)` -> ExecOutcome（执行 + 重扫受影响的子树 + 新摘要）
//! - `list_quarantine` / `restore_batch(manifest_path)` / `purge_batch(manifest_path)`
//! - `get_guidance(path, kind)` -> keep / 只能用工具 / 迁移 的指引（只展示命令，不运行）
//! - `open_system_settings(uri)` -> 打开白名单里的 ms-settings 页
//! - `pick_scan_folder` / `startup_scan_dir`（`--scan <文件夹>`）：只扫描某个文件夹
//!
//! 插件：`tauri-plugin-opener`（前端打开参考链接）、`tauri-plugin-dialog`（快照的保存/打开对话框，
//! 只在 Rust 侧调用，前端不需要 dialog 权限）。

use diskwhy_core::action::guidance::{self, Guidance};
use diskwhy_core::action::log::ActionLog;
use diskwhy_core::action::protect::Protector;
use diskwhy_core::action::quarantine::{self, BatchInfo, PurgeReport, RestoreReport};
use diskwhy_core::action::refresh::RefreshReport;
use diskwhy_core::action::{self, ExecOptions, ExecProgress, ExecReport, Plan, PlanRequest, PreflightOptions, PreflightReport, Progress, ProgressView, Strategy};
use diskwhy_core::explain::llm::{self, LlmSettings};
use diskwhy_core::explain::candidates::{cleanup_candidates, CandidateGroup};
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
use std::sync::{Arc, Mutex, OnceLock, RwLock};
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
    /// 最近一次预览的清理计划（执行时按 id 取出，执行一次就没了）。
    plan: Mutex<Option<Plan>>,
    /// 正在进行的预检（阶段 G：后台进行，可取消）。
    preview: Mutex<Option<Arc<Progress>>>,
    /// 正在执行 / 还原 / 清空。
    acting: AtomicBool,
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
    /// 清理页的候选（阶段 F）。
    candidates: Vec<CandidateGroup>,
}

/// 各档合计 + 清理候选（扫描 / 打开快照 / 动作后的刷新共用）。
fn analyze(tree: &diskwhy_core::Node, ctx: &ExplainCtx) -> (TierSummary, Vec<CandidateGroup>) {
    let tiers = tier_summary(tree, ctx);
    let p = protector();
    let candidates = cleanup_candidates(tree, ctx, &|path: &str| p.check(path).is_some());
    (tiers, candidates)
}

fn summarize(r: &ScanResult, (tiers, candidates): (TierSummary, Vec<CandidateGroup>), snapshot: Option<SnapshotInfo>) -> ScanSummary {
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
        candidates,
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
        let tiers = analyze(&r.tree, &ctx);
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

/// 任意节点（目录 / 文件 / “其余小文件”）的详情视图，带解释。
#[tauri::command]
fn node_info(path: String, kind: NodeKind, state: State<'_, AppState>) -> Result<NodeView, String> {
    let r = state.result()?;
    let ctx = state.ctx();
    let not_found = || format!("在扫描结果里找不到：{path}");
    let (chain, node, full) = if kind == NodeKind::Rest {
        let chain = find_dir(&r.tree, &path).ok_or_else(not_found)?;
        let dir = *chain.last().unwrap();
        let rest = dir.children.iter().find(|c| c.kind == NodeKind::Rest).ok_or_else(not_found)?;
        (chain, rest, dir.path.clone())
    } else {
        let (chain, node) = find_node(&r.tree, &path).ok_or_else(not_found)?;
        let full = if node.kind == NodeKind::Dir { node.path.clone() } else { path.clone() };
        (chain, node, full)
    };
    let parent = chain.last().copied();
    let parent_path = parent.map(|p| p.path.as_str());
    let parent_alloc = parent.map(|p| p.allocated).unwrap_or(r.used_bytes);
    let mut v = node_view(node, parent_path, parent_alloc);
    if node.kind == NodeKind::Rest {
        v.path = full.clone();
    }
    if node.mapped {
        v.mapped_to = view::mapped_to(&r, &node.path);
    }
    let t = Target { node, path: &full, parent, ancestors: &chain };
    v.explanation = Some(diskwhy_core::explain::explain(&t, &ctx));
    Ok(v)
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
        let tiers = analyze(&r.tree, &ctx);
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

// ───────────────────────── 阶段 F：界面偏好 ─────────────────────────

/// 界面偏好。`theme`：A 浅色 / B 深色 / C 趣味；None = 还没选过（前端按系统深浅色给默认值再存）。
#[derive(Serialize, serde::Deserialize, Default, Clone)]
struct UiPrefs {
    #[serde(default)]
    theme: Option<String>,
}

fn ui_prefs_path() -> Option<PathBuf> {
    llm::app_data_dir().map(|d| d.join("ui.json"))
}

#[tauri::command]
fn get_ui_prefs() -> UiPrefs {
    let Some(p) = ui_prefs_path() else { return UiPrefs::default() };
    let mut prefs: UiPrefs = std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if !matches!(prefs.theme.as_deref(), Some("A" | "B" | "C")) {
        prefs.theme = None;
    }
    prefs
}

#[tauri::command]
fn set_ui_prefs(prefs: UiPrefs) -> Result<(), String> {
    if let Some(t) = prefs.theme.as_deref() {
        if !matches!(t, "A" | "B" | "C") {
            return Err(format!("不认识的主题：{t}"));
        }
    }
    let p = ui_prefs_path().ok_or("找不到 %LOCALAPPDATA%")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&prefs).map_err(|e| e.to_string())?;
    std::fs::write(&p, text).map_err(|e| format!("写 {} 失败：{e}", p.display()))
}

// ───────────────────────── 阶段 D：带护栏的动作 ─────────────────────────

fn protector() -> &'static Protector {
    static P: OnceLock<Protector> = OnceLock::new();
    P.get_or_init(Protector::from_system)
}

fn quarantine_roots() -> Vec<String> {
    let mut roots: Vec<String> = list_fixed_drives().into_iter().map(|d| d.root).collect();
    for r in quarantine::known_roots() {
        if !roots.iter().any(|x| x.eq_ignore_ascii_case(&r)) {
            roots.push(r);
        }
    }
    roots
}

#[derive(Serialize)]
struct ActionEnv {
    drives: Vec<DriveInfo>,
    batches: Vec<BatchInfo>,
    expired_batches: usize,
    log_path: String,
    keep_days: i64,
}

#[tauri::command]
async fn action_env() -> Result<ActionEnv, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let batches = quarantine::list_batches(&quarantine_roots());
        ActionEnv {
            drives: list_fixed_drives(),
            expired_batches: batches.iter().filter(|b| b.expired).count(),
            batches,
            log_path: ActionLog::default_path().map(|p| p.display().to_string()).unwrap_or_default(),
            keep_days: quarantine::KEEP_DAYS,
        }
    })
    .await
    .map_err(|e| format!("线程异常：{e}"))
}

/// 预检过程中发给前端的消息（阶段 G）。
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PreviewEvent {
    /// 快速计划做完（约 0.1–0.5 秒）：弹层用它立刻出现，`report.pending = true`，不能执行。
    Quote { report: PreflightReport },
    /// 后台枚举 / 逐个检查的进度（约每 150 毫秒一次）。
    Progress { progress: ProgressView, elapsed_ms: u64 },
}

/// 清理清单 → 计划 + 预检。档位 / 规则 / 保护路径全部由后端按当前扫描结果重新判断。
///
/// 阶段 G：先做快速计划并立刻通过 `on_event` 发 `quote`（弹层马上出现），再在后台枚举、逐个检查，
/// 期间发 `progress`；全部做完才返回完整的预检报告并保存计划（只有这时才能执行）。
/// `cancel_preview` 或新的一次预览会让这次尽快停下并返回“已取消”。
#[tauri::command]
async fn preview_plan(
    items: Vec<PlanRequest>,
    strategy: Strategy,
    on_event: Channel<PreviewEvent>,
    state: State<'_, AppState>,
) -> Result<PreflightReport, String> {
    if state.scanning.load(Ordering::SeqCst) {
        return Err("正在扫描，先等扫描结束".into());
    }
    if items.is_empty() {
        return Err("清理清单是空的".into());
    }
    let r = state.result()?;
    let ctx = state.ctx();
    let prog = Arc::new(Progress::default());
    {
        // 新的预览替换旧的：旧的尽快停下
        let mut cur = state.preview.lock().unwrap();
        if let Some(old) = cur.replace(prog.clone()) {
            old.cancel.store(true, Ordering::SeqCst);
        }
    }
    // 旧计划作废（执行只认最新一次做完的预检）
    *state.plan.lock().unwrap() = None;
    let p2 = prog.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let t0 = std::time::Instant::now();
        let mut plan = action::build_plan_quick(&r, &ctx, protector(), &items, strategy);
        let _ = on_event.send(PreviewEvent::Quote { report: action::quote(&plan) });
        let quote_ms = t0.elapsed().as_millis() as u64;
        let done = Arc::new(AtomicBool::new(false));
        let ticker = {
            let (prog, done, ch) = (p2.clone(), done.clone(), on_event.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
                    let _ = ch.send(PreviewEvent::Progress { progress: prog.view(), elapsed_ms: t0.elapsed().as_millis() as u64 });
                }
            })
        };
        let te = std::time::Instant::now();
        action::enumerate_plan(&mut plan, &p2);
        let enumerate_ms = te.elapsed().as_millis() as u64;
        let rep = if p2.cancelled() { None } else { Some(action::preflight_with(&mut plan, protector(), &PreflightOptions::default(), &p2)) };
        done.store(true, Ordering::Relaxed);
        let _ = ticker.join();
        match rep {
            Some(mut rep) if !p2.cancelled() => {
                rep.timing_ms.insert("click_to_done".into(), t0.elapsed().as_millis() as u64);
                rep.timing_ms.insert("quote".into(), quote_ms);
                rep.timing_ms.insert("enumerate".into(), enumerate_ms);
                // 排查用：设了环境变量 DISKWHY_PREFLIGHT_LOG=<文件> 时，把各阶段用时追加一行进去
                if let Ok(p) = std::env::var("DISKWHY_PREFLIGHT_LOG") {
                    use std::io::Write as _;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                        let v = p2.view();
                        let _ = writeln!(
                            f,
                            "{}",
                            serde_json::json!({ "timing_ms": rep.timing_ms, "entries": v.to_check, "dirs": v.dirs, "locked": v.locked, "rm_sessions": v.rm_total })
                        );
                    }
                }
                Some((plan, rep))
            }
            _ => None,
        }
    })
    .await
    .map_err(|e| format!("预检线程异常：{e}"));
    // 只有仍是“当前这次”预览时才保存计划
    let mut cur = state.preview.lock().unwrap();
    let still_current = cur.as_ref().is_some_and(|c| Arc::ptr_eq(c, &prog));
    if still_current {
        *cur = None;
    }
    drop(cur);
    match res? {
        Some((plan, rep)) if still_current && !prog.cancelled() => {
            *state.plan.lock().unwrap() = Some(plan);
            Ok(rep)
        }
        _ => Err(PREVIEW_CANCELLED.into()),
    }
}

const PREVIEW_CANCELLED: &str = "已取消";

/// 取消正在进行的预检（关掉确认弹层时）。
#[tauri::command]
fn cancel_preview(state: State<'_, AppState>) {
    if let Some(p) = state.preview.lock().unwrap().take() {
        p.cancel.store(true, Ordering::SeqCst);
    }
}

#[derive(Serialize)]
struct ExecOutcome {
    report: ExecReport,
    refresh: RefreshReport,
    summary: Option<ScanSummary>,
}

/// 刷新受影响的目录并返回新的摘要（扫描结果不在时返回 None）。
fn refresh_after(state: &AppState, ctx: &ExplainCtx, dirs: &[String]) -> (RefreshReport, Option<ScanSummary>) {
    let taken = state.result.lock().unwrap().take();
    let Some(mut arc) = taken else { return (RefreshReport::default(), None) };
    let rep = {
        let r = Arc::make_mut(&mut arc);
        action::refresh::refresh_dirs(r, dirs)
    };
    let tiers = analyze(&arc.tree, ctx);
    let snap = state.snapshot.lock().unwrap().clone();
    let summary = summarize(&arc, tiers, snap);
    *state.result.lock().unwrap() = Some(arc);
    (rep, Some(summary))
}

#[tauri::command]
async fn execute_plan(
    plan_id: String,
    skip_locked: bool,
    on_progress: Channel<ExecProgress>,
    state: State<'_, AppState>,
) -> Result<ExecOutcome, String> {
    if state.scanning.load(Ordering::SeqCst) {
        return Err("正在扫描，先等扫描结束".into());
    }
    if state.acting.swap(true, Ordering::SeqCst) {
        return Err("已经有一个清理动作在进行".into());
    }
    let plan = {
        let mut g = state.plan.lock().unwrap();
        match g.as_ref() {
            Some(p) if p.id == plan_id => g.take(),
            _ => None,
        }
    };
    let Some(plan) = plan else {
        state.acting.store(false, Ordering::SeqCst);
        return Err("找不到这次预览的清单（可能已经执行过或被新的预览替换），请重新预览".into());
    };
    let res = tauri::async_runtime::spawn_blocking(move || {
        let r = (|| {
            let log_path = ActionLog::default_path().ok_or("找不到 %LOCALAPPDATA%")?;
            let log = ActionLog::open(&log_path);
            let mut opts = ExecOptions::new(&log);
            opts.skip_locked = skip_locked;
            let r = action::execute(&plan, protector(), &opts, |p| {
                let _ = on_progress.send(p.clone());
            });
            if r.is_ok() {
                if let Strategy::Quarantine { dest_root } = &plan.strategy {
                    quarantine::remember_root(dest_root);
                }
            }
            r
        })();
        (plan, r)
    })
    .await
    .map_err(|e| format!("执行线程异常：{e}"));
    let report = match res {
        Ok((_, Ok(r))) => r,
        Ok((plan, Err(e))) => {
            // execute 只在动任何文件之前返回 Err（没预检、隔离区建不了、选了不跳过被占用的……），
            // 所以把计划放回去，用户可以改一下再试，不用重新预览。
            *state.plan.lock().unwrap() = Some(plan);
            state.acting.store(false, Ordering::SeqCst);
            return Err(e);
        }
        Err(e) => {
            state.acting.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    let ctx = state.ctx();
    let dirs = report.affected_dirs.clone();
    let st: &AppState = &state;
    let (refresh, summary) = refresh_after(st, &ctx, &dirs);
    state.acting.store(false, Ordering::SeqCst);
    Ok(ExecOutcome { report, refresh, summary })
}

#[tauri::command]
async fn list_quarantine() -> Result<Vec<BatchInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| quarantine::list_batches(&quarantine_roots()))
        .await
        .map_err(|e| format!("线程异常：{e}"))
}

#[derive(Serialize)]
struct RestoreOutcome {
    report: RestoreReport,
    summary: Option<ScanSummary>,
}

#[tauri::command]
async fn restore_batch(manifest_path: String, state: State<'_, AppState>) -> Result<RestoreOutcome, String> {
    if state.acting.swap(true, Ordering::SeqCst) {
        return Err("已经有一个清理动作在进行".into());
    }
    let res = tauri::async_runtime::spawn_blocking(move || {
        let log = ActionLog::open(&ActionLog::default_path().ok_or("找不到 %LOCALAPPDATA%")?);
        quarantine::restore(std::path::Path::new(&manifest_path), &log)
    })
    .await
    .map_err(|e| format!("还原线程异常：{e}"));
    let report = match res {
        Ok(Ok(r)) => r,
        Ok(Err(e)) | Err(e) => {
            state.acting.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    let ctx = state.ctx();
    let st: &AppState = &state;
    let summary = if st.result.lock().unwrap().is_some() { refresh_after(st, &ctx, &report.touched_dirs).1 } else { None };
    state.acting.store(false, Ordering::SeqCst);
    Ok(RestoreOutcome { report, summary })
}

#[tauri::command]
async fn purge_batch(manifest_path: String, state: State<'_, AppState>) -> Result<PurgeReport, String> {
    if state.acting.swap(true, Ordering::SeqCst) {
        return Err("已经有一个清理动作在进行".into());
    }
    let res = tauri::async_runtime::spawn_blocking(move || {
        let log = ActionLog::open(&ActionLog::default_path().ok_or("找不到 %LOCALAPPDATA%")?);
        quarantine::purge(std::path::Path::new(&manifest_path), &log)
    })
    .await
    .map_err(|e| format!("清空线程异常：{e}"));
    state.acting.store(false, Ordering::SeqCst);
    res?
}

/// keep / 只能用工具 / 迁移 的指引（只展示命令，不运行）。
#[tauri::command]
fn get_guidance(path: String, kind: NodeKind, state: State<'_, AppState>) -> Result<Vec<Guidance>, String> {
    let r = state.result()?;
    let ctx = state.ctx();
    if kind == NodeKind::Rest {
        return Ok(Vec::new());
    }
    let (_, e) = view::explain_path(&r.tree, &path, &ctx).ok_or_else(|| format!("在扫描结果里找不到：{path}"))?;
    Ok(guidance::guidance_for(&e))
}

/// 打开白名单里的系统设置页（ms-settings:storagesense 等）。
#[tauri::command]
fn open_system_settings(uri: String) -> Result<(), String> {
    guidance::open_settings(&uri)
}

/// 选一个文件夹来扫描（Rust 侧弹系统对话框）。
#[tauri::command]
async fn pick_scan_folder(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(fp) = app.dialog().file().set_title("选择要扫描的文件夹").blocking_pick_folder() else {
            return Ok(None);
        };
        let p: PathBuf = fp.into_path().map_err(|e| e.to_string())?;
        Ok(Some(p.display().to_string()))
    })
    .await
    .map_err(|e| format!("对话框线程异常：{e}"))?
}

/// 选一个文件夹（隔离区位置等）。
#[tauri::command]
async fn pick_folder(title: String, app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(fp) = app.dialog().file().set_title(title).blocking_pick_folder() else {
            return Ok(None);
        };
        let p: PathBuf = fp.into_path().map_err(|e| e.to_string())?;
        Ok(Some(p.display().to_string()))
    })
    .await
    .map_err(|e| format!("对话框线程异常：{e}"))?
}

/// 启动参数 `--scan <文件夹>`：启动后直接扫描这个文件夹（自动化 / 演示用）。
#[tauri::command]
fn startup_scan_dir() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == "--scan").and_then(|i| args.get(i + 1)).filter(|p| std::path::Path::new(p).is_dir()).cloned()
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
            save_snapshot,
            open_snapshot,
            startup_snapshot,
            relaunch_as_admin,
            get_settings,
            save_settings,
            test_llm,
            ask_ai,
            action_env,
            preview_plan,
            cancel_preview,
            execute_plan,
            list_quarantine,
            restore_batch,
            purge_batch,
            get_guidance,
            open_system_settings,
            pick_scan_folder,
            pick_folder,
            startup_scan_dir,
            get_ui_prefs,
            set_ui_prefs
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
