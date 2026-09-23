//! 用真实临时目录 + 真实 Win32 API（FindFirstFileExW / GetCompressedFileSizeW）测试。
//!
//! - 大小汇总、按簇取整；
//! - NTFS 压缩文件（compact.exe）allocated < logical，证明确实用了 GetCompressedFileSizeW；
//! - junction（mklink /J，无需管理员）不递归；
//! - 无权限目录（icacls 对 Everyone 拒绝“列目录”，测试结束恢复）记 inaccessible、不中断扫描；
//! - FILE_ATTRIBUTE_OFFLINE（attrib +O，用户态可设）按云占位统计，且从未调用 GetCompressedFileSizeW。
//!   RECALL_ON_OPEN / RECALL_ON_DATA_ACCESS 用户态无法设置，由 mock_tests 覆盖。
#![cfg(windows)]

use diskwhy_core::fs::{round_up, FsBackend, RawEntry};
use diskwhy_core::scan::{scan_with, volume_root_of};
use diskwhy_core::win::{volume_space, Win32Fs};
use diskwhy_core::*;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

struct TempTree(PathBuf);

impl TempTree {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "diskwhy-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        TempTree(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn s(&self) -> String {
        self.0.to_string_lossy().to_string()
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        // 先恢复可能被拒绝的 ACL，再删除
        let _ = Command::new("icacls").arg(&self.0).args(["/reset", "/T", "/C", "/Q"]).output();
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_file(p: &Path, size: usize, byte: u8) {
    let mut f = fs::File::create(p).unwrap();
    f.write_all(&vec![byte; size]).unwrap();
}

/// 记录 compressed_size 调用的包装后端。
struct Spy {
    inner: Win32Fs,
    queried: Mutex<Vec<String>>,
}
impl FsBackend for Spy {
    fn list_dir(&self, path: &str) -> Result<Vec<RawEntry>, u32> {
        self.inner.list_dir(path)
    }
    fn compressed_size(&self, path: &str) -> Option<u64> {
        self.queried.lock().unwrap().push(path.to_string());
        self.inner.compressed_size(path)
    }
}

fn do_scan<B: FsBackend>(b: &B, root: &str, mode: SizeMode) -> ScanResult {
    let vol = volume_space(&volume_root_of(root).unwrap()).unwrap();
    let opts = ScanOptions { size_mode: mode, ..Default::default() };
    scan_with(b, root, vol, false, &opts, &AtomicBool::new(false), |_| {}).unwrap()
}

fn child<'a>(n: &'a Node, name: &str) -> &'a Node {
    n.children.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("缺少 {name}"))
}

#[test]
fn real_sizes_and_cluster_rounding() {
    let t = TempTree::new("sizes");
    let root = t.path();
    fs::create_dir_all(root.join("a").join("b")).unwrap();
    fs::create_dir_all(root.join("empty")).unwrap();
    write_file(&root.join("a").join("f1.bin"), 5000, 1);
    write_file(&root.join("a").join("tiny.txt"), 1, 1);
    write_file(&root.join("a").join("b").join("f3.bin"), 100_000, 2);
    write_file(&root.join("top.bin"), 4096 * 3, 3);

    let vol = volume_space(&volume_root_of(&t.s()).unwrap()).unwrap();
    let cl = vol.cluster_size;
    assert!(cl >= 512, "簇大小 {cl}");

    for mode in [SizeMode::Exact, SizeMode::Fast] {
        let r = do_scan(&Win32Fs, &t.s(), mode);
        assert_eq!(r.files, 4);
        assert_eq!(r.dirs, 3);
        assert_eq!(r.tree.logical, 5000 + 1 + 100_000 + 4096 * 3);
        let a = child(&r.tree, "a");
        assert_eq!(child(a, "f1.bin").allocated, round_up(5000, cl));
        assert_eq!(child(a, "tiny.txt").allocated, cl, "1 字节文件按簇取整为 1 簇");
        assert_eq!(child(child(a, "b"), "f3.bin").allocated, round_up(100_000, cl));
        assert_eq!(a.allocated, round_up(5000, cl) + cl + round_up(100_000, cl));
        assert_eq!(r.tree.allocated, a.allocated + round_up(4096 * 3, cl));
        assert_eq!(child(&r.tree, "empty").allocated, 0);
        // 非卷根：差额全部算“扫描范围外”
        assert!(!r.buckets.scope_is_volume_root);
        assert_eq!(r.buckets.total(), r.used_bytes as i128);
        // 所有文件 mtime 有效
        assert!(child(a, "f1.bin").mtime > 1_600_000_000);
    }
}

#[test]
fn real_ntfs_compressed_file_uses_compressed_size() {
    let t = TempTree::new("compress");
    let f = t.path().join("zeros.bin");
    write_file(&f, 1 << 20, 0);
    let out = Command::new("compact").args(["/c", "/q"]).arg(&f).output().unwrap();
    assert!(out.status.success(), "compact 失败: {}", String::from_utf8_lossy(&out.stdout));
    for mode in [SizeMode::Exact, SizeMode::Fast] {
        let r = do_scan(&Win32Fs, &t.s(), mode);
        let n = child(&r.tree, "zeros.bin");
        assert_eq!(n.logical, 1 << 20);
        assert!(n.attrs & diskwhy_core::fs::attrs::COMPRESSED != 0);
        assert!(n.allocated < n.logical / 4, "压缩后 allocated={} 应远小于 logical", n.allocated);
        assert_eq!(n.allocated % r.cluster_size, 0);
    }
}

#[test]
fn real_junction_not_followed() {
    let t = TempTree::new("junction");
    let target = TempTree::new("junction-target");
    write_file(&target.path().join("big.bin"), 2 << 20, 7);
    write_file(&t.path().join("own.bin"), 10_000, 1);
    let link = t.path().join("link");
    let out = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(target.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "mklink /J 失败: {}", String::from_utf8_lossy(&out.stderr));

    let spy = Spy { inner: Win32Fs, queried: Mutex::new(vec![]) };
    let r = do_scan(&spy, &t.s(), SizeMode::Exact);
    let l = child(&r.tree, "link");
    assert!(l.reparse_skipped);
    assert_eq!(l.reparse_tag, diskwhy_core::fs::reparse::MOUNT_POINT);
    assert!(l.children.is_empty());
    assert_eq!(l.allocated, 0);
    assert_eq!(r.files, 1);
    assert_eq!(r.tree.allocated, round_up(10_000, r.cluster_size));
    assert!(!spy.queried.lock().unwrap().iter().any(|p| p.contains("big.bin")));
    // 先删 junction 本身，避免 remove_dir_all 误入目标
    fs::remove_dir(&link).unwrap();
}

#[test]
fn real_access_denied_dir_is_marked_and_scan_continues() {
    let t = TempTree::new("denied");
    let locked = t.path().join("locked");
    fs::create_dir_all(&locked).unwrap();
    write_file(&locked.join("secret.bin"), 50_000, 1);
    write_file(&t.path().join("visible.bin"), 20_000, 1);
    fs::create_dir_all(t.path().join("after")).unwrap();
    write_file(&t.path().join("after").join("x.bin"), 30_000, 1);
    // 对 Everyone 拒绝“列出文件夹/读取数据”(RD)
    let out = Command::new("icacls").arg(&locked).args(["/deny", "*S-1-1-0:(RD)"]).output().unwrap();
    assert!(out.status.success(), "icacls 失败: {}", String::from_utf8_lossy(&out.stdout));

    let r = do_scan(&Win32Fs, &t.s(), SizeMode::Exact);
    let l = child(&r.tree, "locked");
    assert!(l.inaccessible);
    assert_eq!(l.error_code, 5, "ERROR_ACCESS_DENIED");
    assert_eq!(l.allocated, 0);
    assert_eq!(r.inaccessible_total, 1);
    assert_eq!(r.buckets.inaccessible_dirs_other, 1);
    assert_eq!(r.inaccessible_dirs[0].path, locked.to_string_lossy());
    // 其他内容照常扫描
    assert_eq!(r.files, 2);
    assert_eq!(child(child(&r.tree, "after"), "x.bin").logical, 30_000);

    let _ = Command::new("icacls").arg(&locked).args(["/remove:d", "*S-1-1-0"]).output();
}

#[test]
fn real_offline_attribute_counted_as_placeholder_never_opened() {
    let t = TempTree::new("offline");
    let f = t.path().join("cloud.docx");
    write_file(&f, 300_000, 9);
    write_file(&t.path().join("normal.bin"), 8000, 1);
    let out = Command::new("attrib").arg("+O").arg(&f).output().unwrap();
    assert!(out.status.success());

    let spy = Spy { inner: Win32Fs, queried: Mutex::new(vec![]) };
    let r = do_scan(&spy, &t.s(), SizeMode::Exact);
    let c = child(&r.tree, "cloud.docx");
    assert!(c.cloud_placeholder);
    assert_eq!(c.allocated, 0);
    assert_eq!(c.logical, 300_000);
    assert_eq!(r.buckets.cloud_placeholder_files, 1);
    assert_eq!(r.buckets.cloud_placeholder_logical, 300_000);
    let q = spy.queried.lock().unwrap();
    assert!(q.iter().any(|p| p.ends_with("normal.bin")));
    assert!(!q.iter().any(|p| p.ends_with("cloud.docx")), "占位文件不应被打开：{q:?}");
    drop(q);
    let _ = Command::new("attrib").arg("-O").arg(&f).output();
}

#[test]
fn real_long_path_beyond_max_path() {
    let t = TempTree::new("long");
    let mut p = t.path().to_path_buf();
    let seg = "x".repeat(100);
    for _ in 0..4 {
        p.push(&seg);
    }
    // 超过 260 字符，用 \\?\ 前缀创建
    let long = format!(r"\\?\{}", p.to_string_lossy());
    fs::create_dir_all(&long).unwrap();
    write_file(Path::new(&format!(r"{long}\deep.bin")), 12_345, 1);
    assert!(p.to_string_lossy().len() > 260);
    let r = do_scan(&Win32Fs, &t.s(), SizeMode::Exact);
    assert_eq!(r.files, 1);
    assert_eq!(r.tree.logical, 12_345);
    assert_eq!(r.inaccessible_total, 0);
    let _ = fs::remove_dir_all(format!(r"\\?\{}", t.s()));
}
