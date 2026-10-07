//! 动作用到的底层 Windows 调用。每个函数的签名与语义都按 learn.microsoft.com 核对过：
//!
//! - `CreateFileW` + `GetFileInformationByHandle` / `GetFileInformationByHandleEx(FileIdInfo)`：
//!   文件 ID、大小、修改时间、链接数（重新校验用）；
//! - `CreateFileW(DELETE, 共享读写删)`：只探测“现在能不能删”，不改文件；
//! - `GetFinalPathNameByHandleW`（VOLUME_NAME_DOS 返回 `\\?\` 形式）：解析路径里的链接 / 短文件名；
//! - `GetLongPathNameW`：8.3 短文件名 → 长文件名；
//! - `MoveFileExW(MOVEFILE_WRITE_THROUGH)`：同卷改名（不带 COPY_ALLOWED，跨卷时返回
//!   ERROR_NOT_SAME_DEVICE，由上层改走“复制 → 校验 → 删除”）；
//! - `RemoveDirectoryW`：只删空目录；对目录联接只删链接本身，不碰目标（官方文档原话）；
//! - `DeleteFileW`：对符号链接只删链接本身；
//! - Restart Manager：`RmStartSession` / `RmRegisterResources` / `RmGetList` / `RmEndSession`；
//! - `IFileOperation`（STA）：`FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI |
//!   FOF_WANTNUKEWARNING | FOFX_RECYCLEONDELETE | FOFX_ADDUNDORECORD`；
//! - `SHGetKnownFolderPath`：桌面 / 文档等的真实位置（考虑文件夹重定向）。

use crate::fs::{FileKey, RawEntry};
use crate::win::{file_key, from_wide_nul, long_path, wide};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_MORE_DATA, ERROR_SHARING_VIOLATION, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DeleteFileW, FileCompressionInfo, FileStandardInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, GetLongPathNameW, MoveFileExW, RemoveDirectoryW, BY_HANDLE_FILE_INFORMATION,
    DELETE, FILE_COMPRESSION_INFO, FILE_STANDARD_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_WRITE_THROUGH, OPEN_EXISTING,
    VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY, RM_PROCESS_INFO,
};
use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;

pub use windows_sys::Win32::Foundation::ERROR_NOT_SAME_DEVICE;

/// 打开句柄后取到的“指纹”：执行前再取一次，任何一项不同就跳过（防止扫描后被替换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fingerprint {
    /// 卷序列号 + 128 位文件 ID（FileIdInfo；失败退回 nFileIndexHigh/Low）。
    pub volume: u64,
    pub id: [u8; 16],
    pub size: u64,
    /// ftLastWriteTime 原始值（100ns）。
    pub mtime_ft: u64,
    pub attrs: u32,
    pub links: u32,
    /// 磁盘实际占用（FileCompressionInfo.CompressedFileSize，未按簇取整）。
    pub on_disk: u64,
    /// FileStandardInfo.AllocationSize：只对 4 KB 以下的普通文件取（其余为 0 = 没取）。NTFS 上数据放在文件记录（MFT）
    /// 里的小文件这里是按 8 字节取整的大小、不是整簇——删掉它不腾出任何簇（阶段 J 复核：60 字节的缓存文件按 4 KB 算，
    /// 预计腾出的空间比实测多 10 倍以上）。
    #[serde(default)]
    pub alloc: u64,
}

impl Fingerprint {
    pub fn key(&self) -> FileKey {
        FileKey { volume: self.volume, id: self.id }
    }
}

fn open(path: &str, access: u32, flags: u32) -> Result<HANDLE, u32> {
    let w = wide(&long_path(path));
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(unsafe { GetLastError() })
    } else {
        Ok(h)
    }
}

/// 取指纹。`no_follow = true` 时带 FILE_FLAG_OPEN_REPARSE_POINT（取链接自身的信息，不跟随）。
pub fn fingerprint(path: &str, no_follow: bool) -> Result<Fingerprint, u32> {
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if no_follow {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    let h = open(path, FILE_READ_ATTRIBUTES, flags)?;
    let r = unsafe { fingerprint_of(h, true) };
    unsafe { CloseHandle(h) };
    r
}

/// 从已打开的句柄取指纹（不关句柄）。`always_compression = false` 时，只有带压缩 / 稀疏 / reparse 属性的文件
/// 才多问一次 FileCompressionInfo（其余文件磁盘占用就是逻辑大小，按簇取整在上层做），省一次 IRP。
unsafe fn fingerprint_of(h: HANDLE, always_compression: bool) -> Result<Fingerprint, u32> {
    unsafe {
        let mut bh: BY_HANDLE_FILE_INFORMATION = zeroed();
        if GetFileInformationByHandle(h, &mut bh) == 0 {
            return Err(GetLastError());
        }
        let key = file_key(h, Some(&bh)).unwrap_or(FileKey::from_u64(
            bh.dwVolumeSerialNumber as u64,
            ((bh.nFileIndexHigh as u64) << 32) | bh.nFileIndexLow as u64,
        ));
        let size = ((bh.nFileSizeHigh as u64) << 32) | bh.nFileSizeLow as u64;
        // FILE_ATTRIBUTE_SPARSE_FILE 0x200 | REPARSE_POINT 0x400（WOF 压缩等）| COMPRESSED 0x800
        let special = bh.dwFileAttributes & (0x200 | 0x400 | 0x800) != 0;
        let mut ci: FILE_COMPRESSION_INFO = zeroed();
        let on_disk = if (always_compression || special)
            && GetFileInformationByHandleEx(
                h,
                FileCompressionInfo,
                &mut ci as *mut FILE_COMPRESSION_INFO as *mut c_void,
                size_of::<FILE_COMPRESSION_INFO>() as u32,
            ) != 0
        {
            ci.CompressedFileSize.max(0) as u64
        } else {
            size
        };
        // 小文件：问一次真实分配大小（常驻 MFT 的文件不占簇）
        let mut alloc = 0u64;
        if !special && size > 0 && size < 4096 {
            let mut si: FILE_STANDARD_INFO = zeroed();
            if GetFileInformationByHandleEx(h, FileStandardInfo, &mut si as *mut FILE_STANDARD_INFO as *mut c_void, size_of::<FILE_STANDARD_INFO>() as u32) != 0 {
                alloc = si.AllocationSize.max(0) as u64;
            }
        }
        Ok(Fingerprint {
            alloc,
            volume: key.volume,
            id: key.id,
            size,
            mtime_ft: ((bh.ftLastWriteTime.dwHighDateTime as u64) << 32) | bh.ftLastWriteTime.dwLowDateTime as u64,
            attrs: bh.dwFileAttributes,
            links: bh.nNumberOfLinks.max(1),
            on_disk,
        })
    }
}

/// 探测“现在能不能删”：用 DELETE 权限 + 全共享打开（不跟随链接）。
/// 别的进程打开它时没给 FILE_SHARE_DELETE → ERROR_SHARING_VIOLATION(32)；没权限 → 5。
/// 成功就立刻关掉句柄，不做任何修改。
pub fn delete_probe(path: &str, is_dir: bool) -> Result<(), u32> {
    let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
    if is_dir {
        flags |= FILE_FLAG_BACKUP_SEMANTICS;
    }
    let h = open(path, DELETE, flags)?;
    unsafe { CloseHandle(h) };
    Ok(())
}

/// 预检用：**一次打开**同时做占用探测和取指纹（阶段 G）。
///
/// 用 `DELETE | FILE_READ_ATTRIBUTES` + 全共享（读 / 写 / 删）+ `FILE_FLAG_OPEN_REPARSE_POINT` 打开：
/// - 成功 → 没有别的句柄拒绝共享删除，现在能删；顺手从同一个句柄取指纹，立刻关闭，不做任何修改；
/// - `ERROR_SHARING_VIOLATION` → 有别的句柄打开时没给 FILE_SHARE_DELETE（文档：“If this flag is not
///   specified, but the file or device has been opened for delete access, the function fails”，反过来
///   同理），交给上层问 Restart Manager 是谁；指纹改用只要 FILE_READ_ATTRIBUTES 的打开取
///   （文档：共享模式不影响对属性的访问请求）；
/// - 其他错误（5 没权限等）→ 同样退回只读属性的打开取指纹，探测结果原样返回。
///
/// 返回 `(探测结果, 指纹)`，语义与分别调用 [`delete_probe`] 和 [`fingerprint`] 相同，只是少开一次文件。
pub fn probe_and_fingerprint(path: &str, is_dir: bool) -> (Result<(), u32>, Result<Fingerprint, u32>) {
    let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
    if is_dir {
        flags |= FILE_FLAG_BACKUP_SEMANTICS;
    }
    match open(path, DELETE | FILE_READ_ATTRIBUTES, flags) {
        Ok(h) => {
            let fp = unsafe { fingerprint_of(h, false) };
            unsafe { CloseHandle(h) };
            (Ok(()), fp)
        }
        Err(c) if c == 2 || c == 3 => (Err(c), Err(c)),
        Err(c) => {
            // 没权限时再单独用 DELETE 探一次，确认是删除权限不够而不是读属性不够（和旧的 delete_probe 结果一致）。
            let probe = if c == 5 { delete_probe(path, is_dir) } else { Err(c) };
            (probe, fingerprint(path, true))
        }
    }
}

pub fn is_sharing_violation(code: u32) -> bool {
    code == ERROR_SHARING_VIOLATION
}

/// 路径完全解析后的真实位置（跟随链接、展开短文件名），去掉 `\\?\` 前缀。
pub fn final_path(path: &str) -> Option<String> {
    let h = open(path, FILE_READ_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS).ok()?;
    let mut buf = vec![0u16; 1024];
    let s = unsafe {
        let mut n = GetFinalPathNameByHandleW(h, buf.as_mut_ptr(), buf.len() as u32, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
        if n as usize >= buf.len() {
            buf = vec![0u16; n as usize + 1];
            n = GetFinalPathNameByHandleW(h, buf.as_mut_ptr(), buf.len() as u32, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
        }
        CloseHandle(h);
        if n == 0 || n as usize >= buf.len() {
            return None;
        }
        String::from_utf16_lossy(&buf[..n as usize])
    };
    Some(strip_verbatim(&s))
}

/// `\\?\C:\x` → `C:\x`；`\\?\UNC\srv\share` → `\\srv\share`。
pub fn strip_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// 8.3 短文件名 → 长文件名（路径必须存在）。
pub fn long_name(path: &str) -> Option<String> {
    let w = wide(&long_path(path));
    let mut buf = vec![0u16; 1024];
    unsafe {
        let mut n = GetLongPathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32);
        if n as usize >= buf.len() {
            buf = vec![0u16; n as usize + 1];
            n = GetLongPathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32);
        }
        if n == 0 || n as usize >= buf.len() {
            return None;
        }
        Some(strip_verbatim(&String::from_utf16_lossy(&buf[..n as usize])))
    }
}

/// 同卷改名（MoveFileExW，不带 MOVEFILE_COPY_ALLOWED）。跨卷返回 Err(ERROR_NOT_SAME_DEVICE)。
/// 对链接（目录联接 / 符号链接）移动的是链接对象本身。
pub fn rename(src: &str, dst: &str) -> Result<(), u32> {
    let s = wide(&long_path(src));
    let d = wide(&long_path(dst));
    if unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

/// 删除一个文件（对符号链接只删链接本身）。
pub fn delete_file(path: &str) -> Result<(), u32> {
    let w = wide(&long_path(path));
    if unsafe { DeleteFileW(w.as_ptr()) } == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

/// 删除空目录；对目录联接只删链接本身（不管目标是否为空、目标不受影响）。
pub fn remove_dir(path: &str) -> Result<(), u32> {
    let w = wide(&long_path(path));
    if unsafe { RemoveDirectoryW(w.as_ptr()) } == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

/// 列目录（与扫描用的是同一个实现：FindFirstFileExW，不跟随链接）。
pub fn list_dir(path: &str) -> Result<Vec<RawEntry>, u32> {
    use crate::fs::FsBackend;
    crate::win::Win32Fs.list_dir(path)
}

/// 不打开文件、只读目录项：FindFirstFileExW 直接用完整路径（不含通配符）作模式，返回这一项自己的
/// 属性 / 大小 / 时间 / reparse tag。云占位文件用这个判断，绝不打开（打开 RECALL_ON_OPEN 会触发下载）。
pub fn stat_entry(path: &str) -> Result<RawEntry, u32> {
    use windows_sys::Win32::Storage::FileSystem::{FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, WIN32_FIND_DATAW};
    let w = wide(&long_path(path));
    unsafe {
        let mut data: WIN32_FIND_DATAW = zeroed();
        let h = FindFirstFileExW(
            w.as_ptr(),
            FindExInfoBasic,
            &mut data as *mut WIN32_FIND_DATAW as *mut c_void,
            FindExSearchNameMatch,
            null(),
            0,
        );
        if h == INVALID_HANDLE_VALUE {
            return Err(GetLastError());
        }
        FindClose(h);
        let ft = ((data.ftLastWriteTime.dwHighDateTime as u64) << 32) | data.ftLastWriteTime.dwLowDateTime as u64;
        Ok(RawEntry {
            name: from_wide_nul(&data.cFileName),
            attrs: data.dwFileAttributes,
            size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
            mtime: crate::fs::filetime_to_unix(ft),
            reparse_tag: data.dwReserved0,
        })
    }
}

// ───────────────────────── Restart Manager（只报告，不关进程） ─────────────────────────

/// 占用文件的进程。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LockingProcess {
    pub pid: u32,
    pub name: String,
    /// 是服务时的短名。
    pub service: String,
}

/// 问 Restart Manager：哪些进程正在用这些文件。只调用 RmStartSession / RmRegisterResources /
/// RmGetList / RmEndSession，**从不调用 RmShutdown**，不会关掉任何进程。
///
/// 文档：RmRegisterResources 每次调用都有较重的写操作，不要一文件一调用，要成组注册——
/// 所以这里一次注册整组文件。
pub fn rm_locking_processes(files: &[String]) -> Result<Vec<LockingProcess>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    unsafe {
        let mut session: u32 = 0;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        let mut e = RmStartSession(&mut session, 0, key.as_mut_ptr());
        // 文档：每个用户会话最多同时 64 个 Restart Manager 会话，满了返回 ERROR_MAX_SESSIONS_REACHED（353）。
        // 预检最多并发几个会话，别的程序也可能占着一些：满了就稍等再试（最多约 2 秒）。
        let mut tries = 0;
        while e == 353 && tries < 20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            tries += 1;
            e = RmStartSession(&mut session, 0, key.as_mut_ptr());
        }
        if e != 0 {
            return Err(format!("RmStartSession 失败，错误 {e}"));
        }
        let wides: Vec<Vec<u16>> = files.iter().map(|f| wide(f)).collect();
        let ptrs: Vec<*const u16> = wides.iter().map(|w| w.as_ptr()).collect();
        let e = RmRegisterResources(session, ptrs.len() as u32, ptrs.as_ptr(), 0, null(), 0, null());
        if e != 0 {
            RmEndSession(session);
            return Err(format!("RmRegisterResources 失败，错误 {e}"));
        }
        let mut out = Vec::new();
        let mut needed: u32 = 0;
        let mut count: u32 = 0;
        let mut reasons: u32 = 0;
        // 先问需要多大，再按需要的大小取（文档：缓冲区不够返回 ERROR_MORE_DATA）。
        let mut e = RmGetList(session, &mut needed, &mut count, null_mut(), &mut reasons);
        let mut tries = 0;
        while e == ERROR_MORE_DATA && tries < 5 {
            tries += 1;
            let mut buf: Vec<RM_PROCESS_INFO> = vec![zeroed(); needed as usize + 4];
            count = buf.len() as u32;
            e = RmGetList(session, &mut needed, &mut count, buf.as_mut_ptr(), &mut reasons);
            if e == 0 {
                for p in buf.iter().take(count as usize) {
                    out.push(LockingProcess {
                        pid: p.Process.dwProcessId,
                        name: from_wide_nul(&p.strAppName),
                        service: from_wide_nul(&p.strServiceShortName),
                    });
                }
            }
        }
        RmEndSession(session);
        if e != 0 {
            return Err(format!("RmGetList 失败，错误 {e}"));
        }
        out.sort_by_key(|p| p.pid);
        out.dedup_by_key(|p| p.pid);
        Ok(out)
    }
}

// ───────────────────────── 已知文件夹 ─────────────────────────

/// 当前用户的桌面 / 文档 / 图片 / 视频 / 音乐 / 下载的真实位置（考虑文件夹重定向到别的盘或 OneDrive）。
pub fn personal_known_folders() -> Vec<(String, &'static str)> {
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos,
    };
    let ids = [
        (FOLDERID_Desktop, "桌面"),
        (FOLDERID_Documents, "文档"),
        (FOLDERID_Pictures, "图片"),
        (FOLDERID_Videos, "视频"),
        (FOLDERID_Music, "音乐"),
        (FOLDERID_Downloads, "下载"),
    ];
    let mut out = Vec::new();
    for (id, label) in ids {
        unsafe {
            let mut p: *mut u16 = null_mut();
            // dwFlags = 0（KF_FLAG_DEFAULT），hToken = NULL（当前用户）；返回的字符串要用 CoTaskMemFree 释放。
            let hr = SHGetKnownFolderPath(&id, 0, null_mut(), &mut p);
            if hr >= 0 && !p.is_null() {
                let mut len = 0;
                while *p.add(len) != 0 {
                    len += 1;
                }
                out.push((String::from_utf16_lossy(std::slice::from_raw_parts(p, len)), label));
            }
            if !p.is_null() {
                CoTaskMemFree(p as *const c_void);
            }
        }
    }
    out
}

// ───────────────────────── 回收站：IFileOperation ─────────────────────────

/// 回收站操作用的标志（值见 learn.microsoft.com IFileOperation::SetOperationFlags）：
/// - FOF_ALLOWUNDO (0x40)：保留撤销信息；
/// - FOF_NOCONFIRMATION (0x10)：不弹“确定要删除吗”（用户已在 DiskWhy 的预览里确认过）；
/// - FOF_SILENT (0x4)：不显示进度框；
/// - FOF_NOERRORUI (0x400)：出错不弹框，跳过该项继续（结果由进度回调 + 事后检查逐个确定）；
/// - FOFX_RECYCLEONDELETE (0x80000，Win8+)：删除时送进回收站；
/// - FOFX_ADDUNDORECORD (0x20000000，Win8+)：用户发起的操作，放进撤销栈。
///
/// 阶段 J **去掉了 FOF_WANTNUKEWARNING (0x4000)**：它让“这个会被永久删除”的确认框绕过 FOF_NOCONFIRMATION
/// 弹出来，而工作线程是没有窗口、没有消息循环的 STA 线程——框看不见，线程永远卡住（PHASE_J_CRASH.md 隐患 2）。
/// 现在“回收站放不下”的情况在送进去之前就分流掉（[`RecycleBinInfo::refuse`]：比回收站上限还大的文件、
/// 这个盘的回收站设成了“直接删除”），万一还有漏网的，进度回调 `PostDeleteItem` 的 `psiNewlyCreated == NULL`
/// 会告诉我们它是被直接删掉的，如实记账（[`RecycleOutcome::Deleted`]），不会当成“进了回收站”。
pub const RECYCLE_FLAGS: u32 = 0x40 | 0x10 | 0x4 | 0x400 | 0x0008_0000 | 0x2000_0000;

/// 一项回收的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecycleOutcome {
    /// 进了回收站（原路径已经不在，回调给出了回收站里的新项）。
    Recycled,
    /// 原路径已经不在，但不是进了回收站，而是被直接删掉了（回调里 `psiNewlyCreated == NULL`）。
    Deleted,
    /// 没成功：原路径还在（或打不开）。`hr` 是回调给出的这一项的 HRESULT（没收到回调为 0），`msg` 是说明。
    Failed { hr: i32, msg: String },
}

#[windows::core::implement(windows::Win32::UI::Shell::IFileOperationProgressSink)]
struct RecycleSink {
    /// (小写路径, hrDelete, 是否在回收站里生成了新项)
    seen: std::cell::RefCell<Vec<(String, i32, bool)>>,
}

#[allow(non_snake_case)]
impl windows::Win32::UI::Shell::IFileOperationProgressSink_Impl for RecycleSink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hr: windows::core::HRESULT) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _f: u32, _i: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>, _n: &windows::core::PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _f: u32,
        _i: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _n: &windows::core::PCWSTR,
        _hr: windows::core::HRESULT,
        _c: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _f: u32,
        _i: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _d: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _n: &windows::core::PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _f: u32,
        _i: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _d: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _n: &windows::core::PCWSTR,
        _hr: windows::core::HRESULT,
        _c: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _f: u32,
        _i: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _d: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _n: &windows::core::PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _f: u32,
        _i: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _d: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _n: &windows::core::PCWSTR,
        _hr: windows::core::HRESULT,
        _c: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreDeleteItem(&self, _f: u32, _i: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>) -> windows::core::Result<()> {
        Ok(())
    }
    /// 文档：`hrDelete` 是这一项真正删除的结果（不是 DeleteItem 的排队结果）；`psiNewlyCreated` 是进了回收站的新项，
    /// “If the item was fully deleted, this value is NULL”。
    fn PostDeleteItem(
        &self,
        _f: u32,
        item: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        hr: windows::core::HRESULT,
        created: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
    ) -> windows::core::Result<()> {
        use windows::Win32::UI::Shell::SIGDN_FILESYSPATH;
        if let Some(it) = item.as_ref() {
            if let Ok(p) = unsafe { it.GetDisplayName(SIGDN_FILESYSPATH) } {
                let s = unsafe { p.to_string() }.unwrap_or_default();
                unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const c_void)) };
                if let Ok(mut v) = self.seen.try_borrow_mut() {
                    v.push((strip_verbatim(&s).to_lowercase(), hr.0, created.as_ref().is_some()));
                }
            }
        }
        Ok(())
    }
    fn PreNewItem(&self, _f: u32, _d: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>, _n: &windows::core::PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _f: u32,
        _d: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
        _n: &windows::core::PCWSTR,
        _t: &windows::core::PCWSTR,
        _a: u32,
        _hr: windows::core::HRESULT,
        _c: windows::core::Ref<'_, windows::Win32::UI::Shell::IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _t: u32, _s: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

/// HRESULT 的说明（系统消息 + 十六进制码）。
pub fn hresult_text(hr: i32) -> String {
    let e = windows::core::Error::from_hresult(windows::core::HRESULT(hr));
    let m = e.message();
    let m = m.trim();
    if m.is_empty() {
        format!("0x{:08X}", hr as u32)
    } else {
        format!("{m}（0x{:08X}）", hr as u32)
    }
}

/// 把一组文件 / 文件夹送进回收站。必须在已经 `CoInitializeEx(COINIT_APARTMENTTHREADED)` 的线程上调用
/// （文档：IFileOperation 只能在 STA 里用）。每一项的结果由进度回调（`IFileOperationProgressSink::PostDeleteItem`，
/// 用 `Advise` 挂在整个操作上）加事后“原路径还在不在”共同决定：
/// - 原路径不在了 + 回调说回收站里有新项（或没收到回调）→ [`RecycleOutcome::Recycled`]；
/// - 原路径不在了 + 回调说没有新项 → [`RecycleOutcome::Deleted`]（被直接删了，如实报告）；
/// - 原路径还在 → [`RecycleOutcome::Failed`]，带回调里的 HRESULT（例如被占用 0x80270027 / 0x80070020）。
///
/// 不会弹任何界面（见 [`RECYCLE_FLAGS`]），所以工作线程不需要属主窗口 / 消息循环。
pub fn recycle_items(paths: &[String]) -> Vec<RecycleOutcome> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IFileOperationProgressSink, IShellItem, SHCreateItemFromParsingName, FILEOPERATION_FLAGS,
    };
    let fail_all = |msg: String| paths.iter().map(|_| RecycleOutcome::Failed { hr: 0, msg: msg.clone() }).collect::<Vec<_>>();
    if paths.is_empty() {
        return Vec::new();
    }
    let op: IFileOperation = match unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) } {
        Ok(o) => o,
        Err(e) => return fail_all(format!("创建 IFileOperation 失败：{e}")),
    };
    if let Err(e) = unsafe { op.SetOperationFlags(FILEOPERATION_FLAGS(RECYCLE_FLAGS)) } {
        return fail_all(format!("SetOperationFlags 失败：{e}"));
    }
    let sink_obj = windows::core::ComObject::new(RecycleSink { seen: std::cell::RefCell::new(Vec::with_capacity(paths.len())) });
    let sink: IFileOperationProgressSink = sink_obj.to_interface();
    let cookie = unsafe { op.Advise(&sink) }.ok();
    let mut results: Vec<RecycleOutcome> = vec![RecycleOutcome::Failed { hr: 0, msg: "没有执行".into() }; paths.len()];
    let mut queued = vec![false; paths.len()];
    for (i, p) in paths.iter().enumerate() {
        let h = HSTRING::from(p.as_str());
        let item: windows::core::Result<IShellItem> = unsafe { SHCreateItemFromParsingName(PCWSTR(h.as_ptr()), None) };
        match item {
            Ok(it) => match unsafe { op.DeleteItem(&it, None::<&IFileOperationProgressSink>) } {
                Ok(()) => queued[i] = true,
                Err(e) => results[i] = RecycleOutcome::Failed { hr: e.code().0, msg: format!("DeleteItem 失败：{e}") },
            },
            Err(e) => results[i] = RecycleOutcome::Failed { hr: e.code().0, msg: format!("Shell 打不开这个路径：{e}") },
        }
    }
    let perform = if queued.iter().any(|&q| q) { unsafe { op.PerformOperations() } } else { Ok(()) };
    if let Some(c) = cookie {
        let _ = unsafe { op.Unadvise(c) };
    }
    // 回调记录：按小写路径找（同一路径多条时取最后一条）
    let seen: std::collections::HashMap<String, (i32, bool)> = {
        // PerformOperations 已经返回（同一线程、同步），回调不会再来
        let v = sink_obj.seen.borrow();
        v.iter().map(|(p, hr, c)| (p.clone(), (*hr, *c))).collect()
    };
    for (i, p) in paths.iter().enumerate() {
        if !queued[i] {
            continue;
        }
        let rec = seen.get(&strip_verbatim(p).to_lowercase()).copied();
        let still_there = std::fs::symlink_metadata(long_path(p)).is_ok();
        results[i] = match (still_there, rec) {
            (false, Some((_, false))) => RecycleOutcome::Deleted,
            (false, _) => RecycleOutcome::Recycled,
            (true, Some((hr, _))) if hr < 0 => RecycleOutcome::Failed { hr, msg: format!("回收站操作失败：{}", hresult_text(hr)) },
            (true, Some((hr, _))) => RecycleOutcome::Failed { hr, msg: format!("回收站跳过了这个文件（0x{:08X}）", hr as u32) },
            (true, None) => match &perform {
                Err(e) => RecycleOutcome::Failed { hr: e.code().0, msg: format!("回收站操作失败：{e}") },
                Ok(()) => RecycleOutcome::Failed { hr: 0, msg: "操作结束后文件仍在原处".into() },
            },
        };
    }
    results
}

/// 旧接口：成功（进了回收站 / 被直接删）= Ok。
pub fn recycle(paths: &[String]) -> Vec<Result<(), String>> {
    recycle_items(paths)
        .into_iter()
        .map(|r| match r {
            RecycleOutcome::Recycled | RecycleOutcome::Deleted => Ok(()),
            RecycleOutcome::Failed { msg, .. } => Err(msg),
        })
        .collect()
}

/// 某个盘的回收站设置（执行前读一次），用来在送进回收站**之前**分流“回收站放不下 / 关掉了”的条目——
/// 否则 Windows 会直接永久删除（我们不再要 FOF_WANTNUKEWARNING 的确认框，见 [`RECYCLE_FLAGS`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecycleBinInfo {
    /// 回收站上限（字节）。注册表 `BitBucket\Volume\<GUID>\MaxCapacity`（MB）；读不到时按卷大小的 5% 估
    /// （Windows 默认值的量级），再读不到就是 None（不按大小分流）。
    pub max_bytes: Option<u64>,
    /// 这个盘的回收站设成了“不移到回收站，直接删除”（`NukeOnDelete = 1`），或策略 `NoRecycleFiles = 1`。
    pub nuke: bool,
}

impl RecycleBinInfo {
    pub fn read(root: &str) -> Self {
        let max_bytes = recycle_bin_max_capacity(root).or_else(|| crate::win::volume_space(root).ok().map(|s| s.total_bytes / 20));
        let nuke = volume_guid(root)
            .and_then(|g| reg_dword(false, &format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{g}"), "NukeOnDelete"))
            .is_some_and(|v| v != 0)
            || [false, true]
                .iter()
                .any(|&m| reg_dword(m, r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer", "NoRecycleFiles").is_some_and(|v| v != 0));
        RecycleBinInfo { max_bytes, nuke }
    }

    /// 这个路径、这么大的东西送进这个盘的回收站会不会被 Windows 直接删掉：会 → Some(原因)。
    /// 先看路径长度（见 [`too_long_for_bin`]），再看 [`RecycleBinInfo::refuse`]。
    pub fn refuse_path(&self, path: &str, size: u64) -> Option<String> {
        if too_long_for_bin(path) {
            return Some(format!("路径太长（{} 个字符），进不了回收站，没动它", path.encode_utf16().count()));
        }
        self.refuse(size)
    }

    /// 这么大的东西送进这个盘的回收站会不会被 Windows 直接删掉：会 → Some(原因)。
    pub fn refuse(&self, size: u64) -> Option<String> {
        if self.nuke {
            return Some("这个盘的回收站设成了“直接删除”，进不了回收站，没动它".into());
        }
        match self.max_bytes {
            Some(0) => Some("这个盘的回收站上限是 0，进不了回收站，没动它".into()),
            Some(m) if size > m => Some(format!("比回收站上限（{} MB）还大，进不了回收站，没动它", m >> 20)),
            _ => None,
        }
    }
}

/// 回收站收得下的最长路径（UTF-16 字符数，不含结尾 0）。实测（阶段 J 复核）：`IFileOperation` + FOFX_RECYCLEONDELETE
/// 对 259 个字符的路径照常回收，260 个及以上的**直接永久删除**、不报错（回收站的 `$I` 记录只放得下 MAX_PATH）。
/// 留 1 个字符余量：≥ 259 一律不送。
pub const BIN_MAX_PATH: usize = 258;

/// 这个路径送进回收站会不会因为太长被 Windows 直接删掉。
pub fn too_long_for_bin(path: &str) -> bool {
    path.encode_utf16().count() > BIN_MAX_PATH
}

/// 整夹回收时，文件夹在回收站里变成 `X:\$Recycle.Bin\<SID>\$Rxxxxxx[.扩展名]`，里面的东西路径前缀换成这个；
/// 按最长的 SID 和较长的扩展名留出来的余量（字符数）。
pub const BIN_DIR_PREFIX: usize = 100;

/// 读一个 REG_DWORD（`machine` = HKLM，否则 HKCU）。
fn reg_dword(machine: bool, key: &str, val: &str) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
    let root = if machine { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER };
    let k = wide(key);
    let v = wide(val);
    let mut data: u32 = 0;
    let mut cb = size_of::<u32>() as u32;
    let e = unsafe { RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut data as *mut u32 as *mut c_void, &mut cb) };
    (e == 0).then_some(data)
}

/// 当前线程进入 STA（IFileOperation 需要）；返回的守卫在析构时 CoUninitialize。
pub struct ComSta(bool);

impl ComSta {
    pub fn init() -> Self {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        ComSta(hr.is_ok())
    }
}

impl Drop for ComSta {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// FILETIME（100ns，自 1601）→ Unix 秒。
pub fn ft_to_unix(ft: u64) -> i64 {
    crate::fs::filetime_to_unix(ft)
}

// ───────────────────────── 阶段 J：直接删除（同一个句柄上重新校验后删） ─────────────────────────

/// 为删除打开的句柄：`DELETE | FILE_READ_ATTRIBUTES`、全共享、`FILE_FLAG_OPEN_REPARSE_POINT`（不跟随链接）。
/// 先用 [`DeleteHandle::fingerprint`] 在**同一个句柄**上重新校验，再 [`DeleteHandle::delete`]：
/// 校验和删除之间文件不可能被换成别的（换了名字的话这个句柄指的还是原来那个文件）。析构时关闭句柄。
pub struct DeleteHandle(HANDLE);

impl DeleteHandle {
    pub fn open(path: &str, is_dir: bool) -> Result<Self, u32> {
        let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
        if is_dir {
            flags |= FILE_FLAG_BACKUP_SEMANTICS;
        }
        open(path, DELETE | FILE_READ_ATTRIBUTES, flags).map(DeleteHandle)
    }

    pub fn fingerprint(&self) -> Result<Fingerprint, u32> {
        unsafe { fingerprint_of(self.0, false) }
    }

    /// 设删除标记（learn.microsoft.com：SetFileInformationByHandle + FILE_DISPOSITION_INFO_EX；
    /// `FILE_DISPOSITION_FLAG_DELETE | POSIX_SEMANTICS | IGNORE_READONLY_ATTRIBUTE`：名字在关句柄时立刻消失，
    /// 只读文件也能删）。不支持 Ex 的文件系统（FAT 等，返回 ERROR_INVALID_PARAMETER / NOT_SUPPORTED）
    /// 退回 FILE_DISPOSITION_INFO（DeleteFile = TRUE，和 DeleteFileW 等价）。删除在句柄关闭（析构）时生效。
    pub fn delete(self) -> Result<(), u32> {
        use windows_sys::Win32::Storage::FileSystem::{
            FileDispositionInfo, FileDispositionInfoEx, SetFileInformationByHandle, FILE_DISPOSITION_FLAG_DELETE,
            FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO,
            FILE_DISPOSITION_INFO_EX,
        };
        unsafe {
            let ex = FILE_DISPOSITION_INFO_EX {
                Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
            };
            if SetFileInformationByHandle(self.0, FileDispositionInfoEx, &ex as *const _ as *const c_void, size_of::<FILE_DISPOSITION_INFO_EX>() as u32) != 0 {
                return Ok(());
            }
            let e = GetLastError();
            // 87 ERROR_INVALID_PARAMETER / 50 ERROR_NOT_SUPPORTED / 1 ERROR_INVALID_FUNCTION：文件系统不支持 Ex
            if !matches!(e, 87 | 50 | 1) {
                return Err(e);
            }
            let fdi = FILE_DISPOSITION_INFO { DeleteFile: true.into() };
            if SetFileInformationByHandle(self.0, FileDispositionInfo, &fdi as *const _ as *const c_void, size_of::<FILE_DISPOSITION_INFO>() as u32) != 0 {
                Ok(())
            } else {
                Err(GetLastError())
            }
        }
    }
}

impl Drop for DeleteHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// ───────────────────────── 阶段 J：可中断的跨盘复制 ─────────────────────────

/// `CopyFileExW(src, dst, NULL, NULL, pbCancel, COPY_FILE_FAIL_IF_EXISTS)`。`cancel` 指向的值变成非 0
/// 时复制中途停下（文档：“If this flag is set to TRUE during the copy operation, the operation is
/// canceled”），返回 Err，**半截的目标文件由这里删掉**（文档没保证 pbCancel 时会删，所以自己删）。
/// 复制保留属性、修改时间、备用数据流（CopyFileEx 文档 Remarks）。
pub fn copy_file_cancellable(src: &str, dst: &str, cancel: Option<&std::sync::atomic::AtomicI32>) -> Result<(), u32> {
    use windows_sys::Win32::Storage::FileSystem::{CopyFileExW, COPY_FILE_FAIL_IF_EXISTS};
    let s = wide(&long_path(src));
    let d = wide(&long_path(dst));
    let pc: *mut i32 = match cancel {
        Some(c) => c.as_ptr(),
        None => null_mut(),
    };
    let ok = unsafe { CopyFileExW(s.as_ptr(), d.as_ptr(), None, null(), pc, COPY_FILE_FAIL_IF_EXISTS) };
    if ok != 0 {
        return Ok(());
    }
    let e = unsafe { GetLastError() };
    // 80 = ERROR_FILE_EXISTS：目标本来就有，不是我们建的，不能删
    if e != 80 {
        let _ = std::fs::remove_file(long_path(dst));
    }
    Err(e)
}

/// ERROR_REQUEST_ABORTED（1235）：复制被取消。
pub const ERROR_REQUEST_ABORTED: u32 = 1235;

// ───────────────────────── 阶段 J：回收站用量 / 上限 ─────────────────────────

/// 当前用户在某个盘上的回收站现在占多少：(字节, 项数)。
///
/// 不用 SHQueryRecycleBinW：本机回收站 26 万项时它要 23 秒（实测）。这里只列一遍
/// `X:\$Recycle.Bin\<当前用户 SID>`（一次 FindFirstFileExW 遍历）：`$R*` 文件按目录项里的大小算；
/// `$R*` 是文件夹时读对应 `$I*` 记录里的原始大小（第 8–16 字节，little-endian i64）。项数 = `$I*` 的个数。
/// 列不出（没有这个文件夹 / 没权限）返回 None。
pub fn recycle_bin_usage(root: &str) -> Option<(u64, u64)> {
    let sid = current_user_sid()?;
    let dir = crate::fs::join(&crate::fs::join(root, "$Recycle.Bin"), &sid);
    let list = list_dir(&dir).ok()?;
    let mut bytes = 0u64;
    let mut items = 0u64;
    for r in &list {
        if r.name.starts_with("$I") {
            items += 1;
        } else if r.name.starts_with("$R") {
            if r.attrs & crate::fs::attrs::DIRECTORY != 0 {
                let ipath = crate::fs::join(&dir, &format!("$I{}", &r.name[2..]));
                if let Ok(b) = std::fs::read(long_path(&ipath)) {
                    if b.len() >= 16 {
                        bytes += i64::from_le_bytes(b[8..16].try_into().unwrap_or([0; 8])).max(0) as u64;
                    }
                }
            } else {
                bytes += r.size;
            }
        }
    }
    Some((bytes, items))
}

/// 同 [`recycle_bin_usage`]，但用 SHQueryRecycleBinW（所有用户、权威但可能很慢）。
pub fn recycle_bin_usage_shell(root: &str) -> Option<(u64, u64)> {
    use windows_sys::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
    let w = wide(root);
    let mut info = SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, i64Size: 0, i64NumItems: 0 };
    let hr = unsafe { SHQueryRecycleBinW(w.as_ptr(), &mut info) };
    if hr < 0 {
        return None;
    }
    Some((info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64))
}

/// 卷 GUID（`{...}`），由 GetVolumeNameForVolumeMountPointW（`\\?\Volume{GUID}\`）取出。
pub fn volume_guid(root: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
    let mut r = root.to_string();
    if !r.ends_with('\\') {
        r.push('\\');
    }
    let w = wide(&r);
    let mut buf = [0u16; 64];
    if unsafe { GetVolumeNameForVolumeMountPointW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } == 0 {
        return None;
    }
    let s = from_wide_nul(&buf);
    let a = s.find('{')?;
    let b = s.find('}')?;
    (b > a).then(|| s[a..=b].to_string())
}

/// 回收站上限（字节）：`HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\<GUID>\MaxCapacity`
/// （REG_DWORD，单位 MB）。读不到返回 None。
pub fn recycle_bin_max_capacity(root: &str) -> Option<u64> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let guid = volume_guid(root)?;
    let key = wide(&format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{guid}"));
    let val = wide("MaxCapacity");
    let mut data: u32 = 0;
    let mut cb = size_of::<u32>() as u32;
    let e = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut data as *mut u32 as *mut c_void, &mut cb)
    };
    (e == 0).then_some(data as u64 * 1024 * 1024)
}

// ───────────────────────── 阶段 J：提权建暂存区 ─────────────────────────

/// 当前进程令牌的用户 SID（`S-1-5-21-…`）。
pub fn current_user_sid() -> Option<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut need = 0u32;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut need);
        if need == 0 {
            CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u64; (need as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as *mut c_void, need, &mut need);
        CloseHandle(token);
        if ok == 0 {
            return None;
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = null_mut();
        if ConvertSidToStringSidW(tu.User.Sid, &mut s) == 0 || s.is_null() {
            return None;
        }
        let mut len = 0;
        while *s.add(len) != 0 {
            len += 1;
        }
        let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, len));
        LocalFree(s as _);
        Some(out)
    }
}

/// 令牌的提权类型（GetTokenInformation(TokenElevationType)）：1 = Default（普通用户，或关了 UAC），
/// 2 = Full（已提权），3 = Limited（管理员账户的受限令牌，能通过 UAC 提权）。读不到返回 0。
pub fn elevation_type() -> i32 {
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevationType, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return 0;
        }
        let mut t: i32 = 0;
        let mut n = 0u32;
        let ok = GetTokenInformation(token, TokenElevationType, &mut t as *mut i32 as *mut c_void, size_of::<i32>() as u32, &mut n);
        CloseHandle(token);
        if ok == 0 {
            0
        } else {
            t
        }
    }
}

/// 以管理员身份运行 `exe params` 并等它退出（ShellExecuteExW "runas" + SEE_MASK_NOCLOSEPROCESS +
/// WaitForSingleObject + GetExitCodeProcess）。返回子进程退出码；用户在 UAC 里点“否”（ERROR_CANCELLED 1223）
/// → Err(None)，其他失败 → Err(Some(说明))。
pub fn run_elevated_wait(exe: &str, params: &str) -> Result<u32, Option<String>> {
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    let verb = wide("runas");
    let file = wide(exe);
    let par = wide(params);
    unsafe {
        let mut info: SHELLEXECUTEINFOW = zeroed();
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = par.as_ptr();
        info.nShow = 0; // SW_HIDE
        if ShellExecuteExW(&mut info) == 0 {
            let e = GetLastError();
            return if e == 1223 { Err(None) } else { Err(Some(format!("ShellExecuteExW 失败，错误 {e}"))) };
        }
        if info.hProcess.is_null() {
            return Err(Some("没拿到提权进程的句柄".into()));
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0u32;
        let ok = GetExitCodeProcess(info.hProcess, &mut code) != 0;
        CloseHandle(info.hProcess);
        if ok {
            Ok(code)
        } else {
            Err(Some("读不到提权进程的退出码".into()))
        }
    }
}

/// 给 `dir` 加一条可继承的“修改”权限（等同 `icacls dir /grant *SID:(OI)(CI)M`），但**在同一个句柄上**做：
/// 不跟随链接打开（FILE_FLAG_OPEN_REPARSE_POINT）→ 确认它是普通文件夹（不是联接 / 符号链接）→ 在这个句柄上
/// 改 DACL。检查和修改之间路径被换成联接也没用：改的是已经打开的那个对象（阶段 J 复核：icacls 会跟随联接）。
/// 错误：`Err(Win32 错误码)`；不是普通文件夹 → `Err(ERROR_DIRECTORY_NOT_SUPPORTED 336)`；SID 写错 → `Err(1337)`。
pub fn grant_modify_plain_dir(dir: &str, sid: &str) -> Result<(), u32> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSidToSidW, GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE,
        SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT};
    use windows_sys::Win32::Storage::FileSystem::{READ_CONTROL, WRITE_DAC};
    /// icacls 的 “M”（修改）= FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE。
    const MODIFY: u32 = 0x0013_01BF;
    let h = open(dir, READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)?;
    let r = unsafe {
        let mut bh: BY_HANDLE_FILE_INFORMATION = zeroed();
        // FILE_ATTRIBUTE_DIRECTORY 0x10，REPARSE_POINT 0x400
        if GetFileInformationByHandle(h, &mut bh) == 0 {
            Err(GetLastError())
        } else if bh.dwFileAttributes & 0x10 == 0 || bh.dwFileAttributes & 0x400 != 0 {
            Err(336)
        } else {
            let ws = wide(sid);
            let mut psid: PSID = null_mut();
            if ConvertStringSidToSidW(ws.as_ptr(), &mut psid) == 0 {
                Err(1337)
            } else {
                let mut old: *mut ACL = null_mut();
                let mut sd: PSECURITY_DESCRIPTOR = null_mut();
                let e = GetSecurityInfo(h, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut old, null_mut(), &mut sd);
                let res = if e != 0 {
                    Err(e)
                } else {
                    let ea = EXPLICIT_ACCESS_W {
                        grfAccessPermissions: MODIFY,
                        grfAccessMode: GRANT_ACCESS,
                        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
                        Trustee: TRUSTEE_W {
                            pMultipleTrustee: null_mut(),
                            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                            TrusteeForm: TRUSTEE_IS_SID,
                            TrusteeType: TRUSTEE_IS_USER,
                            ptstrName: psid as *mut u16,
                        },
                    };
                    let mut new: *mut ACL = null_mut();
                    let e = SetEntriesInAclW(1, &ea, old, &mut new);
                    let res = if e != 0 {
                        Err(e)
                    } else {
                        let e = SetSecurityInfo(h, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), new, null());
                        if e != 0 {
                            Err(e)
                        } else {
                            Ok(())
                        }
                    };
                    if !new.is_null() {
                        LocalFree(new as _);
                    }
                    res
                };
                if !sd.is_null() {
                    LocalFree(sd as _);
                }
                LocalFree(psid as _);
                res
            }
        }
    };
    unsafe { CloseHandle(h) };
    r
}
