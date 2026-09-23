//! 用内存 mock 后端测试扫描逻辑中在真实临时目录里难以构造的情形：
//! - 卷根下的系统保留名字归类；
//! - RECALL_ON_DATA_ACCESS / RECALL_ON_OPEN 云占位（普通用户态无法设置这两个属性）；
//! - 云目录（cloud reparse tag）要进去、符号链接目录不进去；
//! - 无权限目录与差额归属；
//! - 每目录前 N 大文件聚合；
//! - 取消、进度回调间隔。

use diskwhy_core::fs::{attrs, errors, reparse, FsBackend, RawEntry};
use diskwhy_core::scan::scan_with;
use diskwhy_core::view;
use diskwhy_core::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone)]
enum MockDir {
    Ok(Vec<RawEntry>),
    Denied,
}

#[derive(Default)]
struct MockFs {
    dirs: HashMap<String, MockDir>,
    /// 路径 -> GetCompressedFileSizeW 结果
    compressed: HashMap<String, u64>,
    /// 记录所有 compressed_size 调用（用来证明占位文件从未被“打开”）
    queried: Mutex<Vec<String>>,
    delay: Duration,
}

fn file(name: &str, size: u64, a: u32) -> RawEntry {
    RawEntry { name: name.into(), attrs: a | attrs::ARCHIVE, size, mtime: 1_700_000_000, reparse_tag: 0 }
}
fn dir(name: &str) -> RawEntry {
    RawEntry { name: name.into(), attrs: attrs::DIRECTORY, size: 0, mtime: 1_700_000_000, reparse_tag: 0 }
}
fn reparse_dir(name: &str, tag: u32) -> RawEntry {
    RawEntry {
        name: name.into(),
        attrs: attrs::DIRECTORY | attrs::REPARSE_POINT,
        size: 0,
        mtime: 1_700_000_000,
        reparse_tag: tag,
    }
}

impl MockFs {
    fn add(&mut self, path: &str, entries: Vec<RawEntry>) {
        self.dirs.insert(path.to_lowercase(), MockDir::Ok(entries));
    }
    fn deny(&mut self, path: &str) {
        self.dirs.insert(path.to_lowercase(), MockDir::Denied);
    }
}

impl FsBackend for MockFs {
    fn list_dir(&self, path: &str) -> Result<Vec<RawEntry>, u32> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        match self.dirs.get(&path.to_lowercase()) {
            Some(MockDir::Ok(v)) => Ok(v.clone()),
            Some(MockDir::Denied) => Err(errors::ERROR_ACCESS_DENIED),
            None => Err(errors::ERROR_PATH_NOT_FOUND),
        }
    }
    fn compressed_size(&self, path: &str) -> Option<u64> {
        self.queried.lock().unwrap().push(path.to_string());
        self.compressed.get(&path.to_lowercase()).copied()
    }
}

const CLUSTER: u64 = 4096;

/// 构造一个模拟的 M:\ 卷。
fn sample_volume() -> MockFs {
    let mut fs = MockFs::default();
    fs.add(
        r"M:\",
        vec![
            dir("Users"),
            dir("$Recycle.Bin"),
            dir("System Volume Information"),
            file("pagefile.sys", 8 * CLUSTER, attrs::HIDDEN | attrs::SYSTEM),
            dir("Denied"),
            reparse_dir("Documents and Settings", reparse::MOUNT_POINT),
        ],
    );
    fs.add(
        r"M:\Users",
        vec![
            file("a.bin", 5000, 0),                                         // -> 8192
            file("cloud1.docx", 1_000_000, attrs::RECALL_ON_DATA_ACCESS),   // 占位
            file("cloud2.docx", 2_000_000, attrs::RECALL_ON_OPEN),          // 占位
            file("cloud3.docx", 3_000_000, attrs::OFFLINE),                 // 占位
            reparse_dir("OneDrive", reparse::CLOUD | 0x3000),               // 云目录：进去
            reparse_dir("link", reparse::SYMLINK),                          // 符号链接：不进去
            file("comp.dat", 100_000, attrs::COMPRESSED),                  // 压缩文件
        ],
    );
    fs.add(r"M:\Users\OneDrive", vec![file("hydrated.txt", 10, 0)]);
    // 如果扫描器错误地跟随了符号链接，会把这个巨大文件算进去
    fs.add(r"M:\Users\link", vec![file("huge.iso", 1 << 40, 0)]);
    fs.add(r"M:\$Recycle.Bin", vec![file("deleted.bin", 3 * CLUSTER, 0)]);
    fs.deny(r"M:\System Volume Information");
    fs.deny(r"M:\Denied");
    fs.compressed.insert(r"m:\users\comp.dat".into(), 30_000); // -> 32768
    fs
}

fn volume(used: u64) -> VolumeSpace {
    VolumeSpace { total_bytes: 1 << 40, free_bytes: (1u64 << 40) - used, cluster_size: CLUSTER }
}

fn find<'a>(n: &'a Node, name: &str) -> &'a Node {
    n.children.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("找不到子项 {name}"))
}

#[test]
fn mock_volume_sizes_placeholders_reparse_and_system_reserved() {
    let fs = sample_volume();
    let used = 1_000_000;
    let cancel = AtomicBool::new(false);
    let r = scan_with(&fs, "m:", volume(used), false, &ScanOptions::default(), &cancel, |_| {}).unwrap();

    assert_eq!(r.root, r"M:\");
    let users = find(&r.tree, "Users");
    // a.bin 5000 -> 8192；comp.dat 压缩后 30000 -> 32768；OneDrive\hydrated.txt 10 -> 4096；占位 0
    assert_eq!(find(users, "a.bin").allocated, 8192);
    assert_eq!(find(users, "comp.dat").allocated, 32768);
    assert_eq!(find(users, "comp.dat").logical, 100_000);
    let onedrive = find(users, "OneDrive");
    assert!(!onedrive.reparse_skipped, "云目录应当进入枚举");
    assert_eq!(onedrive.allocated, 4096);

    // 符号链接目录：只记自身，不递归
    let link = find(users, "link");
    assert!(link.reparse_skipped);
    assert_eq!(link.allocated, 0);
    assert!(link.children.is_empty());
    assert!(!fs.queried.lock().unwrap().iter().any(|p| p.contains("huge.iso")));
    // junction/挂载点同理
    assert!(find(&r.tree, "Documents and Settings").reparse_skipped);

    // 云占位：标记、本地占用 0、从未调用 GetCompressedFileSizeW
    for n in ["cloud1.docx", "cloud2.docx", "cloud3.docx"] {
        let c = find(users, n);
        assert!(c.cloud_placeholder, "{n} 应标记为云占位");
        assert_eq!(c.allocated, 0);
        assert!(!fs.queried.lock().unwrap().iter().any(|p| p.ends_with(n)), "{n} 不应被打开");
    }
    assert_eq!(r.buckets.cloud_placeholder_files, 3);
    assert_eq!(r.buckets.cloud_placeholder_logical, 6_000_000);
    assert_eq!(r.buckets.cloud_placeholder, 0);
    assert_eq!(users.allocated, 8192 + 32768 + 4096);
    assert_eq!(users.logical, 5000 + 6_000_000 + 10 + 100_000);

    // 系统保留：pagefile 8 簇 + 回收站 3 簇
    assert!(find(&r.tree, "pagefile.sys").system_reserved);
    assert!(find(&r.tree, "$Recycle.Bin").system_reserved);
    assert!(find(find(&r.tree, "$Recycle.Bin"), "deleted.bin").system_reserved);
    let sys_measured = 11 * CLUSTER;
    let scanned = 8192 + 32768 + 4096;
    assert_eq!(r.buckets.scanned, scanned);

    // 无权限：SVI（系统保留）+ Denied（其他）
    let svi = find(&r.tree, "System Volume Information");
    assert!(svi.inaccessible && svi.system_reserved);
    assert_eq!(svi.error_code, 5);
    assert!(find(&r.tree, "Denied").inaccessible);
    assert_eq!(r.inaccessible_total, 2);
    assert_eq!(r.buckets.inaccessible_dirs_system, 1);
    assert_eq!(r.buckets.inaccessible_dirs_other, 1);

    // 差额进 inaccessible，四项加总 == used
    let gap = used - scanned - sys_measured;
    assert_eq!(r.buckets.inaccessible, gap);
    assert_eq!(r.buckets.system_reserved, sys_measured);
    assert_eq!(r.buckets.unattributed, 0);
    assert_eq!(r.buckets.total(), used as i128);

    // 目录树汇总
    assert_eq!(r.tree.allocated, scanned + sys_measured);
    // pagefile + Users 下 5 个（a、cloud×3、comp）+ hydrated + deleted
    assert_eq!(r.files, 1 + 5 + 1 + 1);
    // 目录：Users, $Recycle.Bin, SVI, Denied, Documents and Settings, OneDrive, link
    assert_eq!(r.dirs, 7);
}

#[test]
fn mock_system_reserved_fallback_when_only_system_dirs_denied() {
    let mut fs = sample_volume();
    fs.add(r"M:\Denied", vec![]);
    let used = 2_000_000;
    let r = scan_with(&fs, r"M:\", volume(used), false, &ScanOptions::default(), &AtomicBool::new(false), |_| {})
        .unwrap();
    assert_eq!(r.buckets.inaccessible_dirs_other, 0);
    assert_eq!(r.buckets.inaccessible, 0);
    assert!(r.buckets.system_reserved_estimated > 0);
    assert_eq!(r.buckets.total(), used as i128);
}

#[test]
fn mock_top_n_aggregation() {
    let mut fs = MockFs::default();
    let entries: Vec<RawEntry> = (1..=120).map(|i| file(&format!("f{i:03}.log"), i * 1000, 0)).collect();
    fs.add(r"M:\", vec![dir("big")]);
    fs.add(r"M:\big", entries);
    let opts = ScanOptions { top_files_per_dir: 50, ..Default::default() };
    let r = scan_with(&fs, r"M:\", volume(10_000_000), false, &opts, &AtomicBool::new(false), |_| {}).unwrap();
    let big = find(&r.tree, "big");
    assert_eq!(big.children.len(), 51, "50 个文件 + 1 个聚合节点");
    let rest = big.children.iter().find(|c| c.kind == NodeKind::Rest).unwrap();
    assert_eq!(rest.files, 70);
    assert_eq!(big.files, 120);
    // 保留的是最大的 50 个
    assert!(big.children.iter().any(|c| c.name == "f120.log"));
    assert!(!big.children.iter().any(|c| c.name == "f001.log"));
    let expect: u64 = (1..=120u64).map(|i| (i * 1000).div_ceil(CLUSTER) * CLUSTER).sum();
    assert_eq!(big.allocated, expect, "聚合后总量不变");
    // 子项按 allocated 降序
    assert!(big.children.windows(2).all(|w| w[0].allocated >= w[1].allocated));
}

fn wide_tree(n_dirs: usize, delay_ms: u64) -> MockFs {
    let mut fs = MockFs { delay: Duration::from_millis(delay_ms), ..Default::default() };
    fs.add(r"M:\", (0..n_dirs).map(|i| dir(&format!("d{i}"))).collect());
    for i in 0..n_dirs {
        fs.add(&format!(r"M:\d{i}"), vec![file("x.bin", 4096, 0)]);
    }
    fs
}

#[test]
fn mock_progress_interval_at_most_200ms() {
    // 400 个目录，每个 list_dir 20ms，4 线程 -> 约 2 秒
    let fs = wide_tree(400, 20);
    let opts = ScanOptions { threads: 4, ..Default::default() };
    let stamps = Mutex::new(Vec::<(Instant, ScanProgress)>::new());
    let t0 = Instant::now();
    let r = scan_with(&fs, r"M:\", volume(1 << 30), false, &opts, &AtomicBool::new(false), |p| {
        stamps.lock().unwrap().push((Instant::now(), p.clone()));
    })
    .unwrap();
    let total = t0.elapsed();
    let s = stamps.into_inner().unwrap();
    assert!(total > Duration::from_millis(1000), "测试树应当足够慢：{total:?}");
    assert!(s.len() >= 5, "回调次数 {}", s.len());
    let mut prev = t0;
    for (t, _) in &s {
        let gap = t.duration_since(prev);
        assert!(gap <= Duration::from_millis(250), "进度间隔过长：{gap:?}");
        prev = *t;
    }
    // 最后一次回调的数字与结果一致
    let last = &s.last().unwrap().1;
    assert_eq!(last.files, 400);
    assert_eq!(last.files, r.files);
    assert!(s.iter().any(|(_, p)| p.current_path.starts_with(r"M:\d")));
}

#[test]
fn mock_cancel_mid_scan() {
    let fs = wide_tree(400, 20);
    let opts = ScanOptions { threads: 4, ..Default::default() };
    let cancel = AtomicBool::new(false);
    let t0 = Instant::now();
    let res = scan_with(&fs, r"M:\", volume(1 << 30), false, &opts, &cancel, |p| {
        if p.files >= 20 {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert_eq!(res.unwrap_err(), ScanError::Cancelled);
    assert!(t0.elapsed() < Duration::from_millis(1500), "取消后应尽快返回：{:?}", t0.elapsed());
}

#[test]
fn mock_root_denied_is_error() {
    let mut fs = MockFs::default();
    fs.deny(r"M:\");
    let e = scan_with(&fs, r"M:\", volume(0), false, &ScanOptions::default(), &AtomicBool::new(false), |_| {})
        .unwrap_err();
    assert!(matches!(e, ScanError::RootUnavailable { code: 5, .. }));
}

#[test]
fn view_listing_and_top_dirs() {
    let fs = sample_volume();
    let r = scan_with(&fs, r"M:\", volume(1_000_000), false, &ScanOptions::default(), &AtomicBool::new(false), |_| {})
        .unwrap();
    let l = view::listing(&r, r"m:\users", 100, None).unwrap();
    assert_eq!(l.node.path, r"M:\Users");
    assert_eq!(l.breadcrumb.len(), 2);
    let a = l.children.iter().find(|c| c.name == "a.bin").unwrap();
    assert_eq!(a.path, r"M:\Users\a.bin");
    let pct: f64 = l.children.iter().map(|c| c.percent_of_parent).sum();
    assert!((pct - 100.0).abs() < 1e-6);
    let top = view::top_dirs(&r.tree, 3);
    assert_eq!(top[0].name, "Users");
    assert!(view::listing(&r, r"M:\nope", 10, None).is_none());
    // JSON 可序列化
    let _ = serde_json::to_string(&view::truncate_tree(&r.tree, 2)).unwrap();
}
