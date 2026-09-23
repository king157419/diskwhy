//! 文件系统后端抽象。
//!
//! 扫描逻辑只依赖 [`FsBackend`]，真实实现是 [`crate::win::Win32Fs`]（FindFirstFileExW +
//! GetCompressedFileSizeW），测试里可以换成内存 mock 来模拟“无权限”“云占位”等
//! 在普通临时目录里不好构造的情况。

/// Win32 文件属性常量（值取自 learn.microsoft.com “File Attribute Constants”）。
pub mod attrs {
    pub const READONLY: u32 = 0x0000_0001;
    pub const HIDDEN: u32 = 0x0000_0002;
    pub const SYSTEM: u32 = 0x0000_0004;
    pub const DIRECTORY: u32 = 0x0000_0010;
    pub const ARCHIVE: u32 = 0x0000_0020;
    pub const SPARSE_FILE: u32 = 0x0000_0200;
    pub const REPARSE_POINT: u32 = 0x0000_0400;
    pub const COMPRESSED: u32 = 0x0000_0800;
    pub const OFFLINE: u32 = 0x0000_1000;
    pub const ENCRYPTED: u32 = 0x0000_4000;
    /// 只在目录枚举结果中出现：条目没有本地实体，打开会触发下载。
    pub const RECALL_ON_OPEN: u32 = 0x0004_0000;
    pub const PINNED: u32 = 0x0008_0000;
    pub const UNPINNED: u32 = 0x0010_0000;
    /// 文件数据并非全部在本地，读取会触发下载。
    pub const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

    /// 云占位判定用的属性集合。
    pub const CLOUD_PLACEHOLDER_MASK: u32 = OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS;
}

/// reparse tag 常量（learn.microsoft.com “Reparse Point Tags” / WIN32_FIND_DATAW）。
pub mod reparse {
    pub const MOUNT_POINT: u32 = 0xA000_0003;
    pub const SYMLINK: u32 = 0xA000_000C;
    /// IO_REPARSE_TAG_CLOUD；CLOUD_1..CLOUD_F 在 bit 12..15 上变化（IO_REPARSE_TAG_CLOUD_MASK = 0xF000）。
    pub const CLOUD: u32 = 0x9000_001A;
    pub const CLOUD_MASK: u32 = 0x0000_F000;

    /// 是否是云文件（OneDrive 等 Cloud Files API）reparse tag。
    pub fn is_cloud(tag: u32) -> bool {
        (tag & !CLOUD_MASK) == CLOUD
    }
}

/// 常用 Win32 错误码。
pub mod errors {
    pub const ERROR_FILE_NOT_FOUND: u32 = 2;
    pub const ERROR_PATH_NOT_FOUND: u32 = 3;
    pub const ERROR_ACCESS_DENIED: u32 = 5;
}

/// 目录枚举得到的一条原始记录（来自 WIN32_FIND_DATAW）。
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: String,
    pub attrs: u32,
    /// nFileSizeHigh:nFileSizeLow
    pub size: u64,
    /// ftLastWriteTime 转成 Unix 秒。
    pub mtime: i64,
    /// dwReserved0（仅 REPARSE_POINT 时有意义）。
    pub reparse_tag: u32,
}

/// 卷上一个文件的唯一标识：(卷序列号, 128 位文件 ID)。
///
/// learn.microsoft.com（BY_HANDLE_FILE_INFORMATION / FILE_ID_INFO）：文件 ID 加卷序列号
/// 在一台机器上唯一标识一个文件；两个路径 key 相同 = 同一份数据（硬链接或目录映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileKey {
    pub volume: u64,
    pub id: [u8; 16],
}

impl FileKey {
    /// 测试用：从一个整数造 key。
    pub fn from_u64(volume: u64, id: u64) -> Self {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&id.to_le_bytes());
        FileKey { volume, id: b }
    }
}

/// 打开文件句柄一次拿到的信息（给硬链接去重用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileInfo {
    /// 与 GetCompressedFileSizeW 同口径的磁盘占用（未按簇取整）。
    pub on_disk: u64,
    /// nNumberOfLinks（硬链接个数，普通文件为 1）。
    pub links: u32,
    /// 只在 links > 1 时取（省一次查询）。
    pub key: Option<FileKey>,
}

/// 扫描用的文件系统接口。
pub trait FsBackend: Sync {
    /// 列出目录内容（不含 `.` 和 `..`）。失败时返回 Win32 错误码。
    fn list_dir(&self, path: &str) -> Result<Vec<RawEntry>, u32>;
    /// GetCompressedFileSizeW：返回磁盘实际占用字节（未按簇取整）。取不到返回 None。
    fn compressed_size(&self, path: &str) -> Option<u64>;
    /// 打开一次文件，同时取磁盘占用、硬链接数和（链接数 > 1 时）文件 ID。
    /// 默认实现退回 [`FsBackend::compressed_size`]、链接数按 1 算（不参与去重）。
    fn file_info(&self, path: &str) -> Option<FileInfo> {
        self.compressed_size(path).map(|on_disk| FileInfo { on_disk, links: 1, key: None })
    }
    /// 目录自身的文件 ID（用于识别商店应用的目录映射）。取不到返回 None。
    fn dir_key(&self, _path: &str) -> Option<FileKey> {
        None
    }
    /// 目录本身列不出来（无权限）时，从别的来源知道的子目录名。
    /// 目前只用于 `Program Files\WindowsApps`：普通用户不能列目录，但每个包的目录可读。
    fn known_children(&self, _dir: &str) -> Vec<String> {
        Vec::new()
    }
}

/// 把路径拼成 `parent\name`，正确处理卷根的结尾反斜杠。
pub fn join(parent: &str, name: &str) -> String {
    let mut s = String::with_capacity(parent.len() + name.len() + 1);
    s.push_str(parent);
    if !parent.ends_with('\\') {
        s.push('\\');
    }
    s.push_str(name);
    s
}

/// 按簇向上取整。cluster 为 0 时原样返回。
pub fn round_up(bytes: u64, cluster: u64) -> u64 {
    if cluster == 0 || bytes == 0 {
        return bytes;
    }
    bytes.div_ceil(cluster) * cluster
}

/// FILETIME（100ns，自 1601-01-01）转 Unix 秒。
pub fn filetime_to_unix(ft: u64) -> i64 {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    if ft < EPOCH_DIFF {
        return 0;
    }
    ((ft - EPOCH_DIFF) / 10_000_000) as i64
}
