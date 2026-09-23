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
}

// ───── 阶段 B：解释 ─────
export type Tier = "safe" | "review" | "keep";
/** 界面上的四种颜色：未知单独算灰色（后端 tier 仍是 review） */
export type TierKey = Tier | "unknown";
export type How = "delete" | "builtin_tool" | "migrate" | "manual" | "none";
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
  category: string;
  what: string;
  why: string;
  lose: string;
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
