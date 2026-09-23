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
  files: number;
  dirs: number;
  child_count: number;
  percent_of_parent: number;
  type_label: string;
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

export function startScan(root: string, onProgress: (p: ScanProgress) => void): Promise<ScanSummary> {
  const ch = new Channel<ScanProgress>();
  ch.onmessage = onProgress;
  return invoke<ScanSummary>("start_scan", { root, onProgress: ch });
}
