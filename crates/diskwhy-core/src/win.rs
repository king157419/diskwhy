//! Windows 实现：目录枚举、实际占用、卷空间、盘符列表、管理员检测与提权重启。
//!
//! 所有 API 签名按 learn.microsoft.com 与 docs.rs windows-sys 0.61 核对过。
//! 路径一律加 `\\?\` 前缀以绕过 MAX_PATH。

use crate::fs::{filetime_to_unix, FileInfo, FileKey, FsBackend, RawEntry};
use crate::model::VolumeSpace;
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetLastError, ERROR_CANCELLED, ERROR_NO_MORE_FILES, HANDLE,
    INVALID_HANDLE_VALUE, NO_ERROR,
};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileCompressionInfo, FileIdInfo, FindClose, FindExInfoBasic, FindExSearchNameMatch,
    FindFirstFileExW, FindNextFileW, GetCompressedFileSizeW, GetDiskFreeSpaceExW, GetDiskFreeSpaceW,
    GetDriveTypeW, GetFileInformationByHandle, GetFileInformationByHandleEx, GetLogicalDrives,
    GetVolumeInformationW, BY_HANDLE_FILE_INFORMATION, FILE_COMPRESSION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FIND_FIRST_EX_LARGE_FETCH, OPEN_EXISTING, WIN32_FIND_DATAW,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// GetDriveTypeW 返回值 DRIVE_FIXED（learn.microsoft.com：3）。
const DRIVE_FIXED: u32 = 3;
/// GetCompressedFileSizeW 失败时的返回值 INVALID_FILE_SIZE (0xFFFFFFFF)。
const INVALID_FILE_SIZE: u32 = 0xFFFF_FFFF;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 加 `\\?\` 长路径前缀（已带前缀的原样返回）。
fn long_path(path: &str) -> String {
    if path.starts_with(r"\\?\") {
        path.to_string()
    } else {
        format!(r"\\?\{path}")
    }
}

fn from_wide_nul(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// 真实文件系统后端。
#[derive(Debug, Default, Clone, Copy)]
pub struct Win32Fs;

impl FsBackend for Win32Fs {
    fn list_dir(&self, path: &str) -> Result<Vec<RawEntry>, u32> {
        let mut pattern = long_path(path);
        if !pattern.ends_with('\\') {
            pattern.push('\\');
        }
        pattern.push('*');
        let wpat = wide(&pattern);
        let mut out = Vec::new();
        unsafe {
            let mut data: WIN32_FIND_DATAW = zeroed();
            let h: HANDLE = FindFirstFileExW(
                wpat.as_ptr(),
                FindExInfoBasic,
                &mut data as *mut WIN32_FIND_DATAW as *mut c_void,
                FindExSearchNameMatch,
                null(),
                FIND_FIRST_EX_LARGE_FETCH,
            );
            if h == INVALID_HANDLE_VALUE {
                return Err(GetLastError());
            }
            loop {
                let name = from_wide_nul(&data.cFileName);
                if name != "." && name != ".." {
                    let ft = ((data.ftLastWriteTime.dwHighDateTime as u64) << 32)
                        | data.ftLastWriteTime.dwLowDateTime as u64;
                    out.push(RawEntry {
                        name,
                        attrs: data.dwFileAttributes,
                        size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
                        mtime: filetime_to_unix(ft),
                        reparse_tag: data.dwReserved0,
                    });
                }
                if FindNextFileW(h, &mut data) == 0 {
                    let e = GetLastError();
                    FindClose(h);
                    if e != ERROR_NO_MORE_FILES && out.is_empty() {
                        return Err(e);
                    }
                    break;
                }
            }
        }
        Ok(out)
    }

    fn compressed_size(&self, path: &str) -> Option<u64> {
        let w = wide(&long_path(path));
        unsafe {
            let mut high: u32 = 0;
            // 文档：返回 INVALID_FILE_SIZE 且 lpFileSizeHigh 非空时，需用 GetLastError 区分成功/失败。
            SetLastError(NO_ERROR);
            let low = GetCompressedFileSizeW(w.as_ptr(), &mut high);
            if low == INVALID_FILE_SIZE && GetLastError() != NO_ERROR {
                return None;
            }
            Some(((high as u64) << 32) | low as u64)
        }
    }

    /// 打开一次句柄（只要 FILE_READ_ATTRIBUTES），取：
    /// - `GetFileInformationByHandleEx(FileCompressionInfo)` 的 CompressedFileSize —— 与
    ///   GetCompressedFileSizeW 同口径（压缩 / 稀疏 / WOF 文件给实际占用，其余给文件大小），
    ///   由 `tests/real_fs_tests.rs` 的对照测试逐文件核对；
    /// - `GetFileInformationByHandle` 的 nNumberOfLinks；
    /// - 链接数 > 1 时再取 `FileIdInfo`（128 位文件 ID + 64 位卷序列号，ReFS 也唯一），
    ///   失败时退回 BY_HANDLE_FILE_INFORMATION 的 (dwVolumeSerialNumber, nFileIndexHigh/Low)。
    ///
    /// 打不开时退回 GetCompressedFileSizeW（链接数按 1 算，不参与去重）。
    fn file_info(&self, path: &str) -> Option<FileInfo> {
        let w = wide(&long_path(path));
        unsafe {
            let h = CreateFileW(
                w.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if h == INVALID_HANDLE_VALUE {
                return self.compressed_size(path).map(|on_disk| FileInfo { on_disk, links: 1, key: None });
            }
            let mut bh: BY_HANDLE_FILE_INFORMATION = zeroed();
            let have_bh = GetFileInformationByHandle(h, &mut bh) != 0;
            let mut ci: FILE_COMPRESSION_INFO = zeroed();
            let on_disk = if GetFileInformationByHandleEx(
                h,
                FileCompressionInfo,
                &mut ci as *mut FILE_COMPRESSION_INFO as *mut c_void,
                size_of::<FILE_COMPRESSION_INFO>() as u32,
            ) != 0
            {
                Some(ci.CompressedFileSize.max(0) as u64)
            } else if have_bh {
                // 文件系统不支持压缩信息（如 FAT）：GetCompressedFileSizeW 此时也返回文件大小。
                Some(((bh.nFileSizeHigh as u64) << 32) | bh.nFileSizeLow as u64)
            } else {
                None
            };
            let links = if have_bh { bh.nNumberOfLinks.max(1) } else { 1 };
            let key = if links > 1 { file_key(h, have_bh.then_some(&bh)) } else { None };
            CloseHandle(h);
            match on_disk {
                Some(on_disk) => Some(FileInfo { on_disk, links, key }),
                None => self.compressed_size(path).map(|on_disk| FileInfo { on_disk, links: 1, key: None }),
            }
        }
    }

    /// `X:\Program Files\WindowsApps` 列不出来时，用注册表里当前用户的包安装目录
    /// （`PackageRootFolder`）补上能量的那部分。清单只读一次。
    fn known_children(&self, dir: &str) -> Vec<String> {
        static PKG: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        let b = dir.as_bytes();
        if b.len() < 3 || b[1] != b':' || !dir[2..].eq_ignore_ascii_case(r"\Program Files\WindowsApps") {
            return Vec::new();
        }
        let prefix = format!("{}\\", dir.to_lowercase());
        PKG.get_or_init(crate::explain::registry::package_root_folders)
            .iter()
            .filter_map(|p| {
                let pl = p.to_lowercase();
                let rest = pl.strip_prefix(&prefix)?;
                if rest.is_empty() || rest.contains('\\') {
                    return None;
                }
                Some(p[prefix.len()..].to_string())
            })
            .collect()
    }

    /// 目录自身的文件 ID（CreateFileW 打开目录必须带 FILE_FLAG_BACKUP_SEMANTICS）。
    fn dir_key(&self, path: &str) -> Option<FileKey> {
        let w = wide(&long_path(path));
        unsafe {
            let h = CreateFileW(
                w.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            );
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut bh: BY_HANDLE_FILE_INFORMATION = zeroed();
            let have_bh = GetFileInformationByHandle(h, &mut bh) != 0;
            let k = file_key(h, have_bh.then_some(&bh));
            CloseHandle(h);
            k
        }
    }
}

/// FileIdInfo 优先；失败退回 BY_HANDLE_FILE_INFORMATION 的 64 位文件索引。
unsafe fn file_key(h: HANDLE, bh: Option<&BY_HANDLE_FILE_INFORMATION>) -> Option<FileKey> {
    let mut fi: FILE_ID_INFO = zeroed();
    if GetFileInformationByHandleEx(
        h,
        FileIdInfo,
        &mut fi as *mut FILE_ID_INFO as *mut c_void,
        size_of::<FILE_ID_INFO>() as u32,
    ) != 0
    {
        return Some(FileKey { volume: fi.VolumeSerialNumber, id: fi.FileId.Identifier });
    }
    let bh = bh?;
    let idx = ((bh.nFileIndexHigh as u64) << 32) | bh.nFileIndexLow as u64;
    Some(FileKey::from_u64(bh.dwVolumeSerialNumber as u64, idx))
}

/// 卷根（`C:\`）的容量 / 空闲 / 簇大小。
pub fn volume_space(root: &str) -> Result<VolumeSpace, String> {
    let w = wide(root);
    unsafe {
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        if GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) == 0 {
            return Err(format!("GetDiskFreeSpaceExW({root}) 失败，错误 {}", GetLastError()));
        }
        let (mut spc, mut bps, mut nfc, mut tnc) = (0u32, 0u32, 0u32, 0u32);
        if GetDiskFreeSpaceW(w.as_ptr(), &mut spc, &mut bps, &mut nfc, &mut tnc) == 0 {
            return Err(format!("GetDiskFreeSpaceW({root}) 失败，错误 {}", GetLastError()));
        }
        Ok(VolumeSpace {
            total_bytes: total,
            free_bytes: free,
            cluster_size: spc as u64 * bps as u64,
        })
    }
}

/// 本机一个固定盘。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    /// 形如 `C:\`
    pub root: String,
    pub label: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// 列出本机所有固定盘（GetLogicalDrives + GetDriveTypeW == DRIVE_FIXED）。
pub fn list_fixed_drives() -> Vec<DriveInfo> {
    let mut out = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let w = wide(&root);
        if unsafe { GetDriveTypeW(w.as_ptr()) } != DRIVE_FIXED {
            continue;
        }
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        let ok = unsafe {
            GetVolumeInformationW(
                w.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                null_mut(),
                null_mut(),
                null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        let space = volume_space(&root).unwrap_or_default();
        out.push(DriveInfo {
            root,
            label: if ok != 0 { from_wide_nul(&label) } else { String::new() },
            file_system: if ok != 0 { from_wide_nul(&fs) } else { String::new() },
            total_bytes: space.total_bytes,
            free_bytes: space.free_bytes,
        });
    }
    out
}

/// 当前进程令牌是否已提权（GetTokenInformation(TokenElevation)）。
pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev: TOKEN_ELEVATION = zeroed();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut TOKEN_ELEVATION as *mut c_void,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/// 提权重启结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelaunchError {
    /// 用户在 UAC 对话框点了“否”。
    Cancelled,
    Failed(String),
}

/// 用 ShellExecuteW("runas") 以管理员身份启动 `exe`。成功时调用方应自行退出当前进程。
pub fn relaunch_as_admin(exe: &str, params: &str) -> Result<(), RelaunchError> {
    let verb = wide("runas");
    let file = wide(exe);
    let par = wide(params);
    unsafe {
        let h = ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            if params.is_empty() { null() } else { par.as_ptr() },
            null(),
            SW_SHOWNORMAL,
        );
        // 文档：返回值 > 32 表示成功。
        let code = h as isize;
        if code > 32 {
            return Ok(());
        }
        let e = GetLastError();
        if e == ERROR_CANCELLED {
            Err(RelaunchError::Cancelled)
        } else {
            Err(RelaunchError::Failed(format!("ShellExecuteW 返回 {code}，GetLastError={e}")))
        }
    }
}
