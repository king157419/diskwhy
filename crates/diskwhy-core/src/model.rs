//! 核心数据模型（对应 PLAN.md 第 2 节）。
//!
//! 内存布局说明：
//! - `Dir` 节点完整保留，`path` 字段总是填好的完整路径（不带 `\\?\` 前缀）。
//! - `File` 节点为了省内存 **不在内存里存 `path`**（为空字符串），需要时由
//!   [`crate::view`] 用 “父目录 path + 名字” 拼出来。
//! - 每个目录只保留前 N 大的文件（默认 50），其余文件合并成一个 `Rest` 节点。

use serde::{Deserialize, Serialize};

/// 节点类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    File,
    Dir,
    /// 聚合节点：一个目录里除前 N 大文件以外的其余文件合计。
    Rest,
}

/// 文件树节点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    /// 完整路径。Dir 节点总有值；File/Rest 节点在内存里为空，见模块说明。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    pub kind: NodeKind,
    /// 逻辑大小（nFileSizeHigh/Low 之和；目录为子树合计）。
    pub logical: u64,
    /// 磁盘实际占用（GetCompressedFileSizeW 按簇向上取整；目录为子树合计）。
    pub allocated: u64,
    /// 最后修改时间，Unix 秒；0 表示未知。
    pub mtime: i64,
    /// 原始 Win32 文件属性（dwFileAttributes）。
    pub attrs: u32,
    /// reparse tag（仅当 attrs 含 REPARSE_POINT 时有意义）。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub reparse_tag: u32,
    /// 目录无法列出（无权限等）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub inaccessible: bool,
    /// 列目录失败时的 Win32 错误码。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub error_code: u32,
    /// 这是一个 reparse point（junction / 符号链接 / 挂载点等），没有跟进去。
    #[serde(default, skip_serializing_if = "is_false")]
    pub reparse_skipped: bool,
    /// 云占位文件（OFFLINE / RECALL_ON_OPEN / RECALL_ON_DATA_ACCESS）。只读了属性，没打开过。
    #[serde(default, skip_serializing_if = "is_false")]
    pub cloud_placeholder: bool,
    /// 位于系统保留项之下（回收站 / 卷影 / pagefile 等，按名字归类）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub system_reserved: bool,
    /// 硬链接去重：这个文件还有别的名字（nNumberOfLinks > 1），它的空间已记在另一个路径上，
    /// 这里 allocated 记 0（阶段 C）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub hardlink_dup: bool,
    /// 目录映射去重：这个目录和另一个路径是同一份数据（文件 ID 相同，商店应用的 AppData 映射），
    /// 空间记在另一边，这里 allocated 记 0、子项不再展开；对应关系见 `ScanResult.mapped_dirs`。
    #[serde(default, skip_serializing_if = "is_false")]
    pub mapped: bool,
    /// 子树中文件数（Rest 节点为被合并的文件数；File 节点为 1）。
    pub files: u64,
    /// 子树中目录数（不含自身）。
    pub dirs: u64,
    /// 子项，按 allocated 降序。仅 Dir 有。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.kind == NodeKind::Dir
    }
}

/// 已用空间的归属。
///
/// 不变式：`scanned + inaccessible + system_reserved + cloud_placeholder + unattributed
///          - overcount == used_bytes`（见 [`crate::attribution`]）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attribution {
    /// 扫到并量出来的普通文件占用。
    pub scanned: u64,
    /// 无权限目录里的内容（大小无法直接量，由卷已用空间差额推算）。
    pub inaccessible: u64,
    /// 系统保留：按名字归类量到的部分 + （必要时）差额兜底部分。
    pub system_reserved: u64,
    /// 其中由差额兜底估算的系统保留字节数（已包含在 system_reserved 里）。
    pub system_reserved_estimated: u64,
    /// 云占位文件在本地的占用（不打开文件，按 0 计；逻辑大小见下）。
    pub cloud_placeholder: u64,
    /// 云占位文件个数（仅统计，不计入字节）。
    pub cloud_placeholder_files: u64,
    /// 云占位文件的逻辑大小合计（不占本地空间，仅供参考）。
    pub cloud_placeholder_logical: u64,
    /// 其余说不清的部分 = used - 其余各项。
    pub unattributed: u64,
    /// 扫描量出的字节（去重之后）仍超过卷已用空间的部分。阶段 C 去重后应接近 0；
    /// 仍不为 0 说明还有没识别出的重复计数（或扫描期间卷上数据在变）。
    pub overcount: u64,
    /// 是否做了去重（硬链接 + 目录映射）。旧快照里没有这个字段，读出来为 false。
    #[serde(default)]
    pub dedup_enabled: bool,
    /// 硬链接去重掉的字节：同一个文件（卷序列号 + 文件 ID 相同）在多个路径出现，只算一次。
    #[serde(default)]
    pub hardlink_dedup: u64,
    /// 被去重的硬链接“多余名字”个数。
    #[serde(default)]
    pub hardlink_dedup_files: u64,
    /// 目录映射去重掉的字节（商店应用 `Packages\…\LocalCache\Roaming\X` 与 `AppData\Roaming\X` 是同一份）。
    #[serde(default)]
    pub mapped_dedup: u64,
    /// 识别出的映射目录个数。
    #[serde(default)]
    pub mapped_dirs: u64,
    /// 差额的原始值（used - 量到的各项；可能为负）。
    pub gap: i64,
    /// 本次扫描根是不是卷根（不是卷根时，差额里包含“扫描范围之外”的内容）。
    pub scope_is_volume_root: bool,
    /// 无权限目录计数：位于系统保留项之下的 / 其他。
    pub inaccessible_dirs_system: u64,
    pub inaccessible_dirs_other: u64,
}

/// 一个无法访问的目录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InaccessibleDir {
    pub path: String,
    pub error_code: u32,
    pub system_reserved: bool,
}

/// 扫描结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanResult {
    pub root: String,
    /// 卷总容量（GetDiskFreeSpaceExW lpTotalNumberOfBytes）。
    pub total_bytes: u64,
    /// 卷已用 = total - lpTotalNumberOfFreeBytes。
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub cluster_size: u64,
    pub tree: Node,
    pub buckets: Attribution,
    pub duration_ms: u64,
    pub files: u64,
    pub dirs: u64,
    pub elevated: bool,
    /// 最多保留前 1000 条无权限目录明细。
    pub inaccessible_dirs: Vec<InaccessibleDir>,
    pub inaccessible_total: u64,
    pub size_mode: SizeMode,
    pub top_files_per_dir: usize,
    /// 识别出的目录映射（去重时被记 0 的一侧 → 真正记账的一侧）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mapped_dirs: Vec<MappedDir>,
}

/// 一对“同一份数据、两个路径”的目录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappedDir {
    /// 记 0 的一侧（例如 `C:\Users\me\AppData\Roaming\Claude`）。
    pub path: String,
    /// 空间记在这一侧（例如 `C:\Users\me\AppData\Local\Packages\Claude_…\LocalCache\Roaming\Claude`）。
    pub same_as: String,
    /// 被去掉的字节（= same_as 那一侧量到的占用）。
    pub bytes: u64,
    pub files: u64,
}

/// 扫描进度。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanProgress {
    pub files: u64,
    pub dirs: u64,
    /// 已扫文件的 allocated 合计。
    pub bytes: u64,
    pub inaccessible: u64,
    pub current_path: String,
    pub elapsed_ms: u64,
    /// 卷已用空间（用于前端画百分比；未知为 0）。
    pub used_bytes: u64,
}

/// allocated 的取法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SizeMode {
    /// 每个文件都调 GetCompressedFileSizeW（最准，最慢）。
    #[default]
    Exact,
    /// 只对压缩 / 稀疏 / reparse 文件调 GetCompressedFileSizeW，其余按逻辑大小按簇取整。
    Fast,
}

/// 扫描选项。
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// 每目录保留前 N 大文件，其余聚合。
    pub top_files_per_dir: usize,
    pub size_mode: SizeMode,
    /// 线程数；0 = 自动。
    pub threads: usize,
    /// 进度回调间隔（毫秒），要求 ≤ 200。
    pub progress_interval_ms: u64,
    /// 硬链接 + 目录映射去重（阶段 C，默认开）。关掉时行为与阶段 A/B 相同（用于对比耗时）。
    pub dedup: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            top_files_per_dir: 50,
            size_mode: SizeMode::Exact,
            threads: 0,
            progress_interval_ms: 100,
            dedup: true,
        }
    }
}

/// 卷空间信息。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct VolumeSpace {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub cluster_size: u64,
    /// 文件系统是 NTFS（小文件的数据可能常驻在 MFT 文件记录里、不占簇，见 [`crate::fs::cluster_bytes`]）。
    #[serde(default)]
    pub ntfs: bool,
}

impl VolumeSpace {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.free_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    Cancelled,
    /// 根目录本身打不开。
    RootUnavailable { path: String, code: u32 },
    Volume(String),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::Cancelled => write!(f, "扫描已取消"),
            ScanError::RootUnavailable { path, code } => {
                write!(f, "无法打开扫描根目录 {path}（Win32 错误 {code}）")
            }
            ScanError::Volume(s) => write!(f, "读取卷信息失败：{s}"),
        }
    }
}

impl std::error::Error for ScanError {}
