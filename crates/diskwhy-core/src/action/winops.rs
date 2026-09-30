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
    CreateFileW, DeleteFileW, FileCompressionInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, GetLongPathNameW, MoveFileExW, RemoveDirectoryW, BY_HANDLE_FILE_INFORMATION,
    DELETE, FILE_COMPRESSION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
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
        Ok(Fingerprint {
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
/// - FOF_NOERRORUI (0x400)：出错不弹框，跳过该项继续（结果由我们逐个检查）；
/// - FOF_WANTNUKEWARNING (0x4000)：如果某项会被**永久删除而不是进回收站**，仍然弹警告
///   （文档：部分覆盖 FOF_NOCONFIRMATION）——防止“回收站放不下 → 静默永久删除”；
/// - FOFX_RECYCLEONDELETE (0x80000，Win8+)：删除时送进回收站；
/// - FOFX_ADDUNDORECORD (0x20000000，Win8+)：用户发起的操作，放进撤销栈。
pub const RECYCLE_FLAGS: u32 = 0x40 | 0x10 | 0x4 | 0x400 | 0x4000 | 0x0008_0000 | 0x2000_0000;

/// 把一组文件送进回收站。必须在已经 `CoInitializeEx(COINIT_APARTMENTTHREADED)` 的线程上调用
/// （文档：IFileOperation 只能在 STA 里用）。返回每个路径的结果：之后路径不存在 = 成功。
pub fn recycle(paths: &[String]) -> Vec<Result<(), String>> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IFileOperationProgressSink, IShellItem, SHCreateItemFromParsingName,
        FILEOPERATION_FLAGS,
    };
    let mut results: Vec<Result<(), String>> = vec![Err("没有执行".into()); paths.len()];
    if paths.is_empty() {
        return results;
    }
    let op: IFileOperation = match unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) } {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("创建 IFileOperation 失败：{e}");
            return results.into_iter().map(|_| Err(msg.clone())).collect();
        }
    };
    if let Err(e) = unsafe { op.SetOperationFlags(FILEOPERATION_FLAGS(RECYCLE_FLAGS)) } {
        let msg = format!("SetOperationFlags 失败：{e}");
        return results.into_iter().map(|_| Err(msg.clone())).collect();
    }
    let mut queued = vec![false; paths.len()];
    for (i, p) in paths.iter().enumerate() {
        let h = HSTRING::from(p.as_str());
        let item: windows::core::Result<IShellItem> = unsafe { SHCreateItemFromParsingName(PCWSTR(h.as_ptr()), None) };
        match item {
            Ok(it) => match unsafe { op.DeleteItem(&it, None::<&IFileOperationProgressSink>) } {
                Ok(()) => queued[i] = true,
                Err(e) => results[i] = Err(format!("DeleteItem 失败：{e}")),
            },
            Err(e) => results[i] = Err(format!("Shell 打不开这个路径：{e}")),
        }
    }
    let perform = unsafe { op.PerformOperations() };
    let aborted = unsafe { op.GetAnyOperationsAborted() }.map(|b| b.as_bool()).unwrap_or(false);
    for (i, p) in paths.iter().enumerate() {
        if !queued[i] {
            continue;
        }
        let still_there = std::fs::symlink_metadata(long_path(p)).is_ok();
        results[i] = if !still_there {
            Ok(())
        } else if let Err(e) = &perform {
            Err(format!("回收站操作失败：{e}"))
        } else if aborted {
            Err("回收站操作被中止（可能被占用、没有权限，或你在“永久删除”警告里点了否）".into())
        } else {
            Err("操作结束后文件仍在原处".into())
        };
    }
    results
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
