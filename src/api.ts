// 前端与 Tauri 后端的全部交互都在这里。类型与 crates/diskwhy-core 的 serde 输出一一对应。
import { invoke, Channel } from "@tauri-apps/api/core";

export type NodeKind = "File" | "Dir" | "Rest";

export interface DriveInfo {
  root: string;
  label: string;
  file_system: string;
  total_bytes: number;
  free_bytes: number;
}

export interface EnvInfo {
  elevated: boolean;
  version: string;
  rules_total: number;
  rule_warnings: string[];
  registry_apps: number;
  llm_ready: boolean;
  /** Windows 所在的盘（"C:\\"），默认暂存区不放这里 */
  system_drive?: string;
}

// ───── 阶段 B：解释 ─────
export type Tier = "safe" | "review" | "keep";
/** 界面上的四种颜色：未知单独算灰色（后端 tier 仍是 review） */
export type TierKey = Tier | "unknown";
export type How = "delete" | "builtin_tool" | "migrate" | "manual" | "none";
/** 删了最坏会怎样（阶段 F：后果标签，由规则 / 启发式明确给出） */
export type LoseKind = "irreversible" | "repair" | "unsure" | "reinstall" | "relogin" | "feature" | "redownload" | "slower";
export type Category = "cache" | "log" | "installer" | "build" | "model" | "userdata" | "system" | "vm" | "trash" | "app" | "unknown";
export type Source =
  | { kind: "rule"; id: string; group: string; inherited_from: string | null }
  | { kind: "heuristic"; name: string }
  | { kind: "llm"; model: string }
  /** 前端合成：归属统计色块 */
  | { kind: "attribution" }
  | { kind: "unknown" };

export interface Explanation {
  tier: Tier;
  owner: string | null;
  category: Category;
  what: string;
  why: string;
  lose: string;
  lose_kind: LoseKind | null;
  /** 删了具体会怎样（≤ 8 个字的标签，阶段 H）；没有时退回 lose_kind 的通用说法 */
  lose_short?: string | null;
  how: How[];
  how_detail: string;
  gain_bytes: number;
  gain_note: string;
  source: Source;
  confidence: number;
  refs: string[];
  verified_on: string | null;
  notes: string[];
  clues: string[];
}

export interface RuleBytes {
  id: string;
  owner: string;
  what: string;
  tier: Tier;
  bytes: number;
}

export interface TierSummary {
  safe_bytes: number;
  review_bytes: number;
  keep_bytes: number;
  uncovered_bytes: number;
  rules_hit: number;
  rules_total: number;
  top_safe: RuleBytes[];
  top_review: RuleBytes[];
  top_keep: RuleBytes[];
  duration_ms: number;
}

export interface LlmSettings {
  enabled: boolean;
  base_url: string;
  model: string;
  api_key: string;
}

export function tierKey(e: Explanation | null | undefined): TierKey {
  if (!e) return "unknown";
  return e.source.kind === "unknown" ? "unknown" : e.tier;
}

export interface ScanProgress {
  files: number;
  dirs: number;
  bytes: number;
  inaccessible: number;
  current_path: string;
  elapsed_ms: number;
  used_bytes: number;
}

export interface Attribution {
  scanned: number;
  inaccessible: number;
  system_reserved: number;
  system_reserved_estimated: number;
  cloud_placeholder: number;
  cloud_placeholder_files: number;
  cloud_placeholder_logical: number;
  unattributed: number;
  overcount: number;
  /** 阶段 C 去重（旧快照里没有这些字段时为 false / 0） */
  dedup_enabled: boolean;
  hardlink_dedup: number;
  hardlink_dedup_files: number;
  mapped_dedup: number;
  mapped_dirs: number;
  gap: number;
  scope_is_volume_root: boolean;
  inaccessible_dirs_system: number;
  inaccessible_dirs_other: number;
}

export interface InaccessibleDir {
  path: string;
  error_code: number;
  system_reserved: boolean;
}

export interface ScanSummary {
  root: string;
  total_bytes: number;
  used_bytes: number;
  free_bytes: number;
  cluster_size: number;
  buckets: Attribution;
  duration_ms: number;
  files: number;
  dirs: number;
  elevated: boolean;
  inaccessible_total: number;
  inaccessible_sample: InaccessibleDir[];
  size_mode: "Exact" | "Fast";
  top_files_per_dir: number;
  tiers: TierSummary;
  mapped_dirs: MappedDir[];
  snapshot: SnapshotInfo | null;
  /** 清理页的候选（阶段 F） */
  candidates: CandidateGroup[];
}

export interface CandidatePath {
  path: string;
  bytes: number;
  deletable: boolean;
}

/** 一组清理候选：同一条规则（或同一种启发式）命中的所有位置 */
export interface CandidateGroup {
  key: string;
  rule_id: string | null;
  group: string;
  heuristic: string | null;
  name: string;
  owner: string;
  category: Category;
  tier: "safe" | "review";
  lose_kind: LoseKind | null;
  lose_short?: string | null;
  how: How[];
  what: string;
  lose: string;
  how_detail: string;
  bytes: number;
  deletable_bytes: number;
  paths: CandidatePath[];
}

export interface MappedDir {
  path: string;
  same_as: string;
  bytes: number;
  files: number;
}

export interface SnapshotInfo {
  path: string;
  meta: { format: string; version: number; saved_at: number; app_version: string; kind: string };
  load_ms: number;
}

/** 方块图根层的“归属色块” */
export type PseudoKey = "inacc" | "sysest" | "unattr" | "cloud";

/** 方块图单击选中的东西 */
export interface TmSelect {
  key: string;
  node: TreemapNode | null;
  pseudo: PseudoKey | null;
  /** 所在父目录路径 */
  parentPath: string | null;
}

/** 列表行 / 方块共用的 key：“其余小文件”的 path 是父目录，所以加后缀区分 */
export function tmKey(n: { kind: NodeKind; path: string }): string {
  return n.kind === "Rest" ? n.path + "|rest" : n.path;
}

/** 方块图数据（阶段 C） */
export interface TreemapNode {
  name: string;
  path: string;
  kind: NodeKind;
  allocated: number;
  tier: TierKey;
  what: string;
  owner: string | null;
  lose_kind: LoseKind | null;
  lose_short?: string | null;
  category: Category;
  inaccessible: boolean;
  reparse_skipped: boolean;
  system_reserved: boolean;
  cloud_placeholder: boolean;
  hardlink_dup: boolean;
  mapped: boolean;
  can_open: boolean;
  children: TreemapNode[];
  hidden_bytes: number;
  hidden_count: number;
}

export interface NodeView {
  name: string;
  path: string;
  kind: NodeKind;
  logical: number;
  allocated: number;
  mtime: number;
  attrs: number;
  reparse_tag: number;
  inaccessible: boolean;
  error_code: number;
  reparse_skipped: boolean;
  cloud_placeholder: boolean;
  system_reserved: boolean;
  hardlink_dup?: boolean;
  mapped?: boolean;
  mapped_to?: string;
  files: number;
  dirs: number;
  child_count: number;
  percent_of_parent: number;
  type_label: string;
  explanation?: Explanation;
  /** 前端合成的“归属色块”（无权限 / 系统保留推算 / 未归属），不是真实文件夹 */
  pseudo?: string;
}

export interface Crumb {
  name: string;
  path: string;
}

export interface DirListing {
  node: NodeView;
  children: NodeView[];
  total_children: number;
  breadcrumb: Crumb[];
}

export const getEnv = () => invoke<EnvInfo>("get_env");
export const listDrives = () => invoke<DriveInfo[]>("list_drives");
export const cancelScan = () => invoke<void>("cancel_scan");
export const listDir = (path: string, limit = 1000) => invoke<DirListing>("list_dir", { path, limit });
export const relaunchAsAdmin = () => invoke<void>("relaunch_as_admin");
export const getSettings = () => invoke<LlmSettings>("get_settings");
export const saveSettings = (settings: LlmSettings) => invoke<string>("save_settings", { settings });
export const testLlm = (settings: LlmSettings) => invoke<string>("test_llm", { settings });
export const askAi = (path: string) => invoke<Explanation>("ask_ai", { path });
export const getTreemap = (path: string, depth = 3) => invoke<TreemapNode>("get_treemap", { path, depth });
export const nodeInfo = (path: string, kind: NodeKind) => invoke<NodeView>("node_info", { path, kind });
/** 弹“另存为”对话框；用户取消返回 null */
export const saveSnapshot = () => invoke<string | null>("save_snapshot");
/** 启动参数里的快照路径（没有为 null） */
export const startupSnapshot = () => invoke<string | null>("startup_snapshot");
/** 弹“打开”对话框读快照；用户取消返回 null。给了 path 就不弹框 */
export const openSnapshot = (path?: string) => invoke<ScanSummary | null>("open_snapshot", { path: path ?? null });

export function startScan(root: string, onProgress: (p: ScanProgress) => void): Promise<ScanSummary> {
  const ch = new Channel<ScanProgress>();
  ch.onmessage = onProgress;
  return invoke<ScanSummary>("start_scan", { root, onProgress: ch });
}

// ───── 阶段 D：带护栏的动作 ─────
/** 怎么删（阶段 J 加了 permanent = 直接删除）。safe 档默认直接删，strategy 只管 review 档（见 previewPlan 的 safeDirect） */
export type Strategy = { kind: "recycle_bin" } | { kind: "quarantine"; dest_root: string } | { kind: "permanent" };
/** 一条实际去了哪里 */
export type DestKind = "permanent" | "recycle" | "quarantine";
export interface CountBytes {
  count: number;
  bytes: number;
}
/** 按去向分别多少个 / 多少字节 */
export interface ByDest {
  permanent: CountBytes;
  recycle: CountBytes;
  quarantine: CountBytes;
  /** 本该进回收站、却被 Windows 直接删掉的（旧报告没有） */
  nuked?: CountBytes;
}
/**
 * 跳过原因代码（预检 / 执行报告共用）：hot / cloud / locked / no_permission / protected / changed / missing / unreadable /
 * hardlink（跨盘暂存的硬链接，删了不腾空间）/ recycle_unavailable（回收站收不下，送进去会被直接删掉）
 */
export type SkipCode =
  | "hot"
  | "cloud"
  | "locked"
  | "no_permission"
  | "protected"
  | "changed"
  | "missing"
  | "unreadable"
  | "hardlink"
  | "recycle_unavailable";
export type EntryKind = "file" | "link" | "cloud_dir";

export interface PlanRequest {
  path: string;
  expect_rule: string | null;
}

/** 前端的清理清单项（只用于显示；真正的档位 / 规则由后端在预览时重新判断） */
export interface CartItem {
  path: string;
  name: string;
  kind: NodeKind;
  allocated: number;
  tier: Tier;
  rule_id: string | null;
  what: string;
  lose: string;
}

export interface LockingProcess {
  pid: number;
  name: string;
  service: string;
}

export interface EntryView {
  path: string;
  kind: EntryKind;
  bytes: number;
  mtime: number;
  note: string | null;
}

export interface SkipView {
  path: string;
  kind: EntryKind;
  bytes: number;
  code: string;
  reason: string;
  processes: LockingProcess[];
}

export interface ItemReport {
  path: string;
  is_dir: boolean;
  tier: Tier;
  unknown: boolean;
  source_label: string;
  rule_id: string | null;
  what: string;
  owner: string | null;
  lose: string;
  scan_allocated: number;
  blocked: string | null;
  will_delete: EntryView[];
  will_delete_count: number;
  will_delete_bytes: number;
  link_count: number;
  skipped: SkipView[];
  skipped_count: number;
  skipped_bytes: number;
  skipped_by_reason: Record<string, number>;
  dirs_to_remove: number;
  enum_errors: string[];
}

export interface ActionWarning {
  code: string;
  message: string;
  blocking: boolean;
}

export interface DriveChoice {
  root: string;
  label: string;
  free_bytes: number;
  total_bytes: number;
  /** 阶段 J：这个盘上已经能用的暂存区位置；null = 要先 prepareQuarantine / elevateQuarantine 或让用户选文件夹 */
  dest_root?: string | null;
}

export interface PreflightReport {
  plan_id: string;
  strategy: Strategy;
  items: ItemReport[];
  total_entries: number;
  total_bytes: number;
  total_skipped: number;
  warnings: ActionWarning[];
  locked_processes: LockingProcess[];
  locked_count: number;
  hot_count: number;
  cloud_count: number;
  link_count: number;
  review_items: { path: string; what: string; lose: string; bytes: number }[];
  can_execute: boolean;
  quarantine_choices: DriveChoice[];
  /** 阶段 G：true = 弹层先用的估算（扫描时的大小），逐个文件的检查还在后台进行，不能执行 */
  pending: boolean;
  timing_ms: Record<string, number>;
  /** 阶段 J：safe 档是否直接删除 */
  safe_direct?: boolean;
  /** 阶段 J：预计用时（秒）；pending 时为 0 = 还不知道 */
  est_secs?: number;
  /** 阶段 J：按去向分（pending 时个数为 0、字节按扫描时的大小） */
  by_dest?: ByDest;
  skipped_bytes?: number;
  skipped_by_reason?: Record<string, CountBytes>;
  /** 阶段 J：扫描时的估算总量（= quote 的 total_bytes），用来解释“数字为什么变小了” */
  scan_total_bytes?: number;
}

/** 预检进度（阶段 G） */
export interface PreviewProgress {
  /** plan / list / check / who / done */
  phase: "plan" | "list" | "check" | "who" | "done";
  dirs: number;
  entries: number;
  checked: number;
  to_check: number;
  locked: number;
  rm_done: number;
  rm_total: number;
}

export type PreviewEvent =
  | { kind: "quote"; report: PreflightReport }
  | { kind: "progress"; progress: PreviewProgress; elapsed_ms: number };

/** 后端取消预检时 invoke 失败的原文 */
export const PREVIEW_CANCELLED = "已取消";

/** 执行进度（约每 200 ms 一次，最后再发一次）。进度条 = cost_done / cost_total（按估计耗时加权，单调） */
export interface ExecProgress {
  done: number;
  total: number;
  bytes_done: number;
  total_bytes: number;
  cost_done: number;
  cost_total: number;
  elapsed_ms: number;
  current: string;
  /** 已经收到停止，正在收尾 */
  cancelling: boolean;
  /** 执行已结束、报告可以取了（后端随后还要重扫几秒） */
  finished?: boolean;
}

export interface EntryResult {
  path: string;
  kind: EntryKind;
  bytes: number;
  result: "ok" | "skipped" | "failed";
  reason: string;
  dest: string | null;
  dest_kind?: DestKind | null;
}

export interface ItemResult {
  path: string;
  blocked: string | null;
  ok: number;
  ok_bytes: number;
  skipped: number;
  failed: number;
  links_removed: number;
  dirs_removed: number;
  details: EntryResult[];
  skipped_bytes?: number;
  failed_bytes?: number;
  by_dest?: ByDest;
  not_processed?: number;
  not_processed_bytes?: number;
}

export interface ExecReport {
  batch_id: string;
  strategy_label: string;
  started_at: number;
  finished_at: number;
  items: ItemResult[];
  ok: number;
  ok_bytes: number;
  skipped: number;
  failed: number;
  manifest_path: string | null;
  log_path: string;
  log_failures: number;
  cancelled: boolean;
  affected_dirs: string[];
  notes: string[];
  /** 阶段 J：源盘执行前后剩余空间的实测差值（≥ 0） */
  freed_bytes?: number;
  by_dest?: ByDest;
  skipped_bytes?: number;
  skipped_by_reason?: Record<string, CountBytes>;
  failed_bytes?: number;
  /** 停止时还没处理、原样没动的 */
  not_processed?: CountBytes;
  duration_ms?: number;
  safe_direct?: boolean;
  workers?: number;
}

/** 上次被打断的清理（last_interrupted） */
export interface Interrupted {
  batch: string;
  /** unix 秒 */
  started: number;
  processed: number;
  processed_bytes: number;
  total: number;
  /** Windows 记下的异常退出码（如 0x8f），没有为 null */
  exit_code?: string | null;
  /** 这次清理之后 crash.log 里有崩溃记录 */
  crash_logged?: boolean;
}

/** prepare_quarantine / elevate_quarantine 的结果 */
export type DestStatus = { status: "ok"; dest_root: string } | { status: "needs_elevation" } | { status: "failed"; message: string };

export interface RefreshReport {
  rescanned: string[];
  removed: string[];
  delta_bytes: number;
  duration_ms: number;
}

export interface ExecOutcome {
  report: ExecReport;
  refresh: RefreshReport;
  summary: ScanSummary | null;
}

export interface BatchInfo {
  batch_id: string;
  manifest_path: string;
  batch_dir: string;
  dest_root: string;
  created_at: number;
  age_days: number;
  expired: boolean;
  entries: number;
  stored_entries: number;
  bytes: number;
  items: string[];
}

export interface RestoreReport {
  batch_id: string;
  restored: number;
  restored_bytes: number;
  skipped: number;
  failed: number;
  batch_removed: boolean;
  problems: [string, string][];
  touched_dirs: string[];
}

export interface PurgeReport {
  batch_id: string;
  deleted_entries: number;
  freed_bytes: number;
  removed: boolean;
  error: string | null;
}

export interface ActionEnv {
  drives: DriveInfo[];
  batches: BatchInfo[];
  expired_batches: number;
  log_path: string;
  keep_days: number;
}

export interface Guidance {
  title: string;
  command: string | null;
  shell: string | null;
  needs_admin: boolean;
  explain: string;
  settings_uri: string | null;
}

export const actionEnv = () => invoke<ActionEnv>("action_env");
/** 预检：先收到 quote（弹层立刻出现），之后是 progress，全部检查完 promise 才返回完整报告 */
/** safeDirect（默认 true）：safe 档直接删；用户明确选了「回收站」时传 false，safe 也进回收站 */
export function previewPlan(
  items: PlanRequest[],
  strategy: Strategy,
  onEvent: (e: PreviewEvent) => void,
  safeDirect = true,
): Promise<PreflightReport> {
  const ch = new Channel<PreviewEvent>();
  ch.onmessage = onEvent;
  return invoke<PreflightReport>("preview_plan", { items, strategy, safeDirect, onEvent: ch });
}
export const cancelPreview = () => invoke<void>("cancel_preview");
export function executePlan(planId: string, skipLocked: boolean, onProgress: (p: ExecProgress) => void): Promise<ExecOutcome> {
  const ch = new Channel<ExecProgress>();
  ch.onmessage = onProgress;
  return invoke<ExecOutcome>("execute_plan", { planId, skipLocked, onProgress: ch });
}
/** 停止正在进行的执行；true = 确实有一个在执行（它会尽快返回 cancelled = true 的报告） */
export const cancelExecute = () => invoke<boolean>("cancel_execute");
/** 现在是否有执行 / 还原 / 清空在进行（界面重载后恢复状态用） */
export const actionStatus = () => invoke<boolean>("action_status");
/** 执行中最近一次进度（重新附着用；没在执行为 null） */
export const execProgress = () => invoke<ExecProgress | null>("exec_progress");
/** 本次运行里最近一次执行的报告 */
export const lastExecReport = () => invoke<ExecReport | null>("last_exec_report");
/** 上次清理被打断了吗（正在执行时也返回 null） */
export const lastInterrupted = () => invoke<Interrupted | null>("last_interrupted");
export const dismissInterrupted = () => invoke<null>("dismiss_interrupted");
/** 前端错误记进 crash.log（每次运行最多 200 条） */
export const logFrontendError = (message: string, stack?: string, source?: string) =>
  invoke<null>("log_frontend_error", { message, stack: stack ?? null, source: source ?? null });
/** 某个盘（"D:\\"）上能用的暂存区位置（不提权；可能会建 X:\DiskWhy暂存区） */
export const prepareQuarantine = (driveRoot: string) => invoke<DestStatus>("prepare_quarantine", { driveRoot });
/** 提权建暂存区（UAC 一次）；用户取消授权也是 failed */
export const elevateQuarantine = (driveRoot: string) => invoke<DestStatus>("elevate_quarantine", { driveRoot });
/** 执行中拦下了关窗（✕ / Alt+F4 / 任务栏），payload 为 null */
export const CLOSE_BLOCKED_EVENT = "close-blocked";
export const listQuarantine = () => invoke<BatchInfo[]>("list_quarantine");
export const restoreBatch = (manifestPath: string) =>
  invoke<{ report: RestoreReport; summary: ScanSummary | null }>("restore_batch", { manifestPath });
export const purgeBatch = (manifestPath: string) => invoke<PurgeReport>("purge_batch", { manifestPath });
export const getGuidance = (path: string, kind: NodeKind) => invoke<Guidance[]>("get_guidance", { path, kind });
export const openSystemSettings = (uri: string) => invoke<void>("open_system_settings", { uri });
export const pickScanFolder = () => invoke<string | null>("pick_scan_folder");
export const startupScanDir = () => invoke<string | null>("startup_scan_dir");

export const pickFolder = (title: string) => invoke<string | null>("pick_folder", { title });
export const getUiPrefs = () => invoke<{ theme: string | null }>("get_ui_prefs");
export const setUiPrefs = (prefs: { theme: string | null }) => invoke<void>("set_ui_prefs", { prefs });
