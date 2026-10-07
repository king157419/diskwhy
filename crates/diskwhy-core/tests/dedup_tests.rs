//! 阶段 C：硬链接 / 目录映射去重 + 快照存取。
//!
//! - 真实文件系统：`mklink /H` 在临时目录造硬链接（不需要管理员），用真实 Win32 API 扫描；
//!   file_info（CreateFileW + FileCompressionInfo）与 GetCompressedFileSizeW 逐文件对照；
//! - mock：WinSxS 优先级、商店应用目录映射（真实的 MSIX 映射造不出来）、被聚合进“其余小文件”的硬链接；
//! - 快照：真实扫描 → 存 gzip → 读回，逐字段相等，读回后解释照常计算。

use diskwhy_core::fs::{attrs, errors, round_up, FileInfo, FileKey, FsBackend, RawEntry};
use diskwhy_core::scan::scan_with;
use diskwhy_core::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

const CL: u64 = 4096;

// ───────────────────────── mock ─────────────────────────

#[derive(Default)]
struct LinkFs {
    dirs: HashMap<String, Vec<RawEntry>>,
    /// 文件路径（小写） → (链接数, key)
    links: HashMap<String, (u32, FileKey)>,
    /// 目录路径（小写） → key
    dir_keys: HashMap<String, FileKey>,
}

fn f(name: &str, size: u64) -> RawEntry {
    RawEntry { name: name.into(), attrs: attrs::ARCHIVE, size, mtime: 1_700_000_000, reparse_tag: 0 }
}
fn d(name: &str) -> RawEntry {
    RawEntry { name: name.into(), attrs: attrs::DIRECTORY, size: 0, mtime: 1_700_000_000, reparse_tag: 0 }
}

impl LinkFs {
    fn add(&mut self, p: &str, e: Vec<RawEntry>) {
        self.dirs.insert(p.to_lowercase(), e);
    }
    fn link(&mut self, p: &str, n: u32, id: u64) {
        self.links.insert(p.to_lowercase(), (n, FileKey::from_u64(7, id)));
    }
    fn size_of(&self, p: &str) -> Option<u64> {
        let (parent, name) = p.rsplit_once('\\')?;
        let parent = if parent.len() == 2 { format!("{parent}\\") } else { parent.to_string() };
        self.dirs.get(&parent.to_lowercase())?.iter().find(|e| e.name == name).map(|e| e.size)
    }
}

impl FsBackend for LinkFs {
    fn list_dir(&self, path: &str) -> Result<Vec<RawEntry>, u32> {
        self.dirs.get(&path.to_lowercase()).cloned().ok_or(errors::ERROR_PATH_NOT_FOUND)
    }
    fn compressed_size(&self, path: &str) -> Option<u64> {
        self.size_of(path)
    }
    fn file_info(&self, path: &str) -> Option<FileInfo> {
        let on_disk = self.size_of(path)?;
        match self.links.get(&path.to_lowercase()) {
            Some(&(n, k)) => Some(FileInfo { on_disk, links: n, key: Some(k), alloc: None }),
            None => Some(FileInfo { on_disk, links: 1, key: None, alloc: None }),
        }
    }
    fn dir_key(&self, path: &str) -> Option<FileKey> {
        self.dir_keys.get(&path.to_lowercase()).copied()
    }
}

fn vol(used: u64) -> VolumeSpace {
    VolumeSpace { total_bytes: 1 << 40, free_bytes: (1u64 << 40) - used, cluster_size: CL, ntfs: false }
}

fn kid<'a>(n: &'a Node, name: &str) -> &'a Node {
    n.children.iter().find(|c| c.name.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("缺少 {name}"))
}

fn at<'a>(n: &'a Node, path: &[&str]) -> &'a Node {
    path.iter().fold(n, |cur, p| kid(cur, p))
}

/// M:\Windows\WinSxS\x\a.dll 与 M:\Windows\System32\a.dll 是同一文件（2 个链接）；
/// M:\Users\u\AppData\Roaming\App 与 M:\Users\u\AppData\Local\Packages\App_1\LocalCache\Roaming\App
/// 是同一目录（目录文件 ID 相同）。
fn sample() -> LinkFs {
    let mut fs = LinkFs::default();
    fs.add(r"M:\", vec![d("Windows"), d("Users")]);
    fs.add(r"M:\Windows", vec![d("WinSxS"), d("System32")]);
    fs.add(r"M:\Windows\WinSxS", vec![d("x")]);
    fs.add(r"M:\Windows\WinSxS\x", vec![f("a.dll", 10 * CL), f("only-sxs.dll", 3 * CL)]);
    fs.add(r"M:\Windows\System32", vec![f("a.dll", 10 * CL), f("b.dll", CL)]);
    fs.link(r"M:\Windows\WinSxS\x\a.dll", 2, 1);
    fs.link(r"M:\Windows\System32\a.dll", 2, 1);
    fs.add(r"M:\Users", vec![d("u")]);
    fs.add(r"M:\Users\u", vec![d("AppData")]);
    fs.add(r"M:\Users\u\AppData", vec![d("Roaming"), d("Local")]);
    fs.add(r"M:\Users\u\AppData\Roaming", vec![d("App"), d("Other")]);
    fs.add(r"M:\Users\u\AppData\Roaming\App", vec![f("big.vhdx", 100 * CL), d("sub")]);
    fs.add(r"M:\Users\u\AppData\Roaming\App\sub", vec![f("x.bin", 5 * CL)]);
    fs.add(r"M:\Users\u\AppData\Roaming\Other", vec![f("o.bin", 2 * CL)]);
    fs.add(r"M:\Users\u\AppData\Local", vec![d("Packages")]);
    fs.add(r"M:\Users\u\AppData\Local\Packages", vec![d("App_1")]);
    fs.add(r"M:\Users\u\AppData\Local\Packages\App_1", vec![d("LocalCache")]);
    fs.add(r"M:\Users\u\AppData\Local\Packages\App_1\LocalCache", vec![d("Roaming")]);
    fs.add(r"M:\Users\u\AppData\Local\Packages\App_1\LocalCache\Roaming", vec![d("App"), d("Other")]);
    fs.add(r"M:\Users\u\AppData\Local\Packages\App_1\LocalCache\Roaming\App", vec![f("big.vhdx", 100 * CL), d("sub")]);
    fs.add(r"M:\Users\u\AppData\Local\Packages\App_1\LocalCache\Roaming\App\sub", vec![f("x.bin", 5 * CL)]);
    // 同名但不是同一个目录（文件 ID 不同）：不能被去重
    fs.add(r"M:\Users\u\AppData\Local\Packages\App_1\LocalCache\Roaming\Other", vec![f("o2.bin", 4 * CL)]);
    let k = FileKey::from_u64(7, 1000);
    fs.dir_keys.insert(r"m:\users\u\appdata\roaming\app".into(), k);
    fs.dir_keys.insert(r"m:\users\u\appdata\local\packages\app_1\localcache\roaming\app".into(), k);
    fs.dir_keys.insert(r"m:\users\u\appdata\roaming\other".into(), FileKey::from_u64(7, 2000));
    fs.dir_keys.insert(r"m:\users\u\appdata\local\packages\app_1\localcache\roaming\other".into(), FileKey::from_u64(7, 2001));
    fs
}

#[test]
fn mock_dedup_hardlink_prefers_non_winsxs_and_maps_store_app_dirs() {
    let fs = sample();
    // 真实占用：a.dll 10 + only-sxs 3 + b.dll 1 + App(100+5) + Other 2 + Other(pkg) 4 = 125 簇
    let used = 125 * CL;
    let r = scan_with(&fs, r"M:\", vol(used), false, &ScanOptions::default(), &AtomicBool::new(false), |_| {}).unwrap();

    // 硬链接：System32 那个名字记账，WinSxS 里的记 0
    let sxs = at(&r.tree, &["Windows", "WinSxS", "x", "a.dll"]);
    assert!(sxs.hardlink_dup);
    assert_eq!(sxs.allocated, 0);
    let s32 = at(&r.tree, &["Windows", "System32", "a.dll"]);
    assert!(!s32.hardlink_dup);
    assert_eq!(s32.allocated, 10 * CL);
    assert_eq!(at(&r.tree, &["Windows", "WinSxS"]).allocated, 3 * CL, "WinSxS 只剩独占的部分");
    assert_eq!(r.buckets.hardlink_dedup, 10 * CL);
    assert_eq!(r.buckets.hardlink_dedup_files, 1);

    // 目录映射：AppData\Roaming\App 记 0、不再展开；Packages 一侧记账
    let mirror = at(&r.tree, &["Users", "u", "AppData", "Roaming", "App"]);
    assert!(mirror.mapped);
    assert_eq!(mirror.allocated, 0);
    assert!(mirror.children.is_empty());
    let real = at(&r.tree, &["Users", "u", "AppData", "Local", "Packages", "App_1", "LocalCache", "Roaming", "App"]);
    assert_eq!(real.allocated, 105 * CL);
    assert_eq!(r.mapped_dirs.len(), 1);
    assert_eq!(r.mapped_dirs[0].path, r"M:\Users\u\AppData\Roaming\App");
    assert!(r.mapped_dirs[0].same_as.ends_with(r"LocalCache\Roaming\App"));
    assert_eq!(r.buckets.mapped_dedup, 105 * CL);
    // 文件 ID 不同的同名目录不去重
    assert!(!at(&r.tree, &["Users", "u", "AppData", "Roaming", "Other"]).mapped);

    // 汇总：树合计 == 真实占用，归属无重复计数
    assert_eq!(r.tree.allocated, used);
    assert_eq!(r.buckets.scanned, used);
    assert_eq!(r.buckets.overcount, 0);
    assert_eq!(r.buckets.gap, 0);
    assert_eq!(r.buckets.total(), used as i128);
    assert!(r.buckets.dedup_enabled);
    // 子项重新按大小排好序
    let w = at(&r.tree, &["Windows"]);
    assert_eq!(w.children[0].name, "System32");

    // 关掉去重 = 阶段 A/B 行为：重复计数 10 + 105 簇
    let opts = ScanOptions { dedup: false, ..Default::default() };
    let r0 = scan_with(&fs, r"M:\", vol(used), false, &opts, &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(r0.tree.allocated, used + 115 * CL);
    assert_eq!(r0.buckets.overcount, 115 * CL);
    assert_eq!(r0.buckets.hardlink_dedup, 0);
    assert!(!r0.buckets.dedup_enabled);
}

#[test]
fn mock_dedup_is_deterministic_and_handles_rest_aggregation() {
    // 三个名字指向同一文件，其中一个被合并进“其余小文件”
    let mut fs = LinkFs::default();
    fs.add(r"M:\", vec![d("b"), d("a"), d("c")]);
    fs.add(r"M:\a", vec![f("same.bin", 8 * CL)]);
    fs.add(r"M:\b", vec![f("same.bin", 8 * CL)]);
    fs.add(r"M:\c", vec![f("k1", 20 * CL), f("k2", 20 * CL), f("same.bin", 8 * CL), f("small", CL)]);
    for p in [r"M:\a\same.bin", r"M:\b\same.bin", r"M:\c\same.bin"] {
        fs.link(p, 3, 42);
    }
    let used = 8 * CL + 41 * CL;
    for threads in [1, 2, 8] {
        let opts = ScanOptions { top_files_per_dir: 2, threads, ..Default::default() };
        let r = scan_with(&fs, r"M:\", vol(used), false, &opts, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(!at(&r.tree, &["a", "same.bin"]).hardlink_dup, "字典序最小的 M:\\a 记账");
        assert!(at(&r.tree, &["b", "same.bin"]).hardlink_dup);
        let c = kid(&r.tree, "c");
        let rest = c.children.iter().find(|x| x.kind == NodeKind::Rest).unwrap();
        assert_eq!(rest.allocated, CL, "same.bin 从“其余小文件”里扣掉，只剩 small");
        assert_eq!(c.allocated, 41 * CL);
        assert_eq!(r.tree.allocated, used);
        assert_eq!(r.buckets.hardlink_dedup, 16 * CL);
        assert_eq!(r.buckets.hardlink_dedup_files, 2);
        assert_eq!(r.buckets.total(), used as i128);
        assert_eq!(r.buckets.overcount, 0);
    }
}

// ───────────────────────── 真实文件系统 ─────────────────────────

#[cfg(windows)]
mod real {
    use super::*;
    use diskwhy_core::scan::volume_root_of;
    use diskwhy_core::win::{volume_space, Win32Fs};
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    pub struct TempTree(pub PathBuf);
    impl TempTree {
        pub fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "diskwhy-c-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(&p).unwrap();
            TempTree(p)
        }
        pub fn s(&self) -> String {
            self.0.to_string_lossy().to_string()
        }
    }
    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(p: &Path, size: usize, byte: u8) {
        fs::File::create(p).unwrap().write_all(&vec![byte; size]).unwrap();
    }

    /// `mklink /H <新名字> <已有文件>`（cmd 内置命令，普通用户可用）。
    fn mklink_h(link: &Path, target: &Path) {
        let out = Command::new("cmd").arg("/C").arg("mklink").arg("/H").arg(link).arg(target).output().unwrap();
        assert!(out.status.success(), "mklink /H 失败：{}", String::from_utf8_lossy(&out.stderr));
    }

    fn scan(root: &str, opts: &ScanOptions) -> ScanResult {
        let v = volume_space(&volume_root_of(root).unwrap()).unwrap();
        scan_with(&Win32Fs, root, v, false, opts, &AtomicBool::new(false), |_| {}).unwrap()
    }

    #[test]
    fn real_mklink_hardlinks_counted_once() {
        let t = TempTree::new("hardlink");
        let r0 = &t.0;
        for sub in ["a", "b", "c", "z"] {
            fs::create_dir_all(r0.join(sub)).unwrap();
        }
        write(&r0.join("a").join("big.bin"), 300_000, 7);
        mklink_h(&r0.join("b").join("link.bin"), &r0.join("a").join("big.bin"));
        mklink_h(&r0.join("c").join("link2.bin"), &r0.join("a").join("big.bin"));
        write(&r0.join("z").join("plain.bin"), 50_000, 1);

        // 文件本身确实有 3 个链接
        let fi = Win32Fs.file_info(&r0.join("b").join("link.bin").to_string_lossy()).unwrap();
        assert_eq!(fi.links, 3);
        let fi_a = Win32Fs.file_info(&r0.join("a").join("big.bin").to_string_lossy()).unwrap();
        assert_eq!(fi.key, fi_a.key, "同一文件 key 相同");
        assert!(fi.key.is_some());
        let fi_p = Win32Fs.file_info(&r0.join("z").join("plain.bin").to_string_lossy()).unwrap();
        assert_eq!(fi_p.links, 1);
        assert_eq!(fi_p.key, None, "链接数为 1 时不取文件 ID");

        let cl = volume_space(&volume_root_of(&t.s()).unwrap()).unwrap().cluster_size;
        let big = round_up(300_000, cl);
        let plain = round_up(50_000, cl);

        let r = scan(&t.s(), &ScanOptions::default());
        assert_eq!(r.tree.allocated, big + plain, "三个名字只算一次");
        assert_eq!(r.buckets.hardlink_dedup, 2 * big);
        assert_eq!(r.buckets.hardlink_dedup_files, 2);
        let a = at(&r.tree, &["a", "big.bin"]);
        assert!(!a.hardlink_dup && a.allocated == big, "路径字典序最小的 a\\big.bin 记账");
        assert!(at(&r.tree, &["b", "link.bin"]).hardlink_dup);
        assert!(at(&r.tree, &["c", "link2.bin"]).hardlink_dup);
        assert_eq!(kid(&r.tree, "b").allocated, 0);
        assert_eq!(r.files, 4, "文件名个数不变，只是空间不重复算");
        assert_eq!(r.buckets.total(), r.used_bytes as i128);

        // Fast 模式：临时目录不在硬链接密集区，不打开句柄 → 不去重（有意为之，文档里写明）
        let rf = scan(&t.s(), &ScanOptions { size_mode: SizeMode::Fast, ..Default::default() });
        assert_eq!(rf.tree.allocated, 3 * big + plain);
        // 关掉去重：阶段 A/B 行为
        let rn = scan(&t.s(), &ScanOptions { dedup: false, ..Default::default() });
        assert_eq!(rn.tree.allocated, 3 * big + plain);
        assert_eq!(rn.buckets.hardlink_dedup, 0);
    }

    /// file_info（CreateFileW + GetFileInformationByHandleEx(FileCompressionInfo)）必须与
    /// GetCompressedFileSizeW 同口径：普通文件、NTFS 压缩文件、稀疏文件、硬链接。
    #[test]
    fn real_file_info_matches_get_compressed_file_size() {
        let t = TempTree::new("fileinfo");
        let r0 = &t.0;
        write(&r0.join("plain.bin"), 12_345, 1);
        write(&r0.join("zeros.bin"), 1 << 20, 0);
        let out = Command::new("compact").args(["/c", "/q"]).arg(r0.join("zeros.bin")).output().unwrap();
        assert!(out.status.success());
        let sp = r0.join("sparse.bin");
        fs::File::create(&sp).unwrap();
        let out = Command::new("fsutil").args(["sparse", "setflag"]).arg(&sp).output().unwrap();
        assert!(out.status.success(), "fsutil sparse setflag: {}", String::from_utf8_lossy(&out.stdout));
        fs::OpenOptions::new().write(true).open(&sp).unwrap().set_len(64 << 20).unwrap();
        mklink_h(&r0.join("plain-link.bin"), &r0.join("plain.bin"));
        for name in ["plain.bin", "zeros.bin", "sparse.bin", "plain-link.bin"] {
            let p = r0.join(name).to_string_lossy().to_string();
            let a = Win32Fs.compressed_size(&p).unwrap();
            let b = Win32Fs.file_info(&p).unwrap().on_disk;
            assert_eq!(a, b, "{name}: GetCompressedFileSizeW={a} file_info={b}");
        }
        let z = Win32Fs.file_info(&r0.join("zeros.bin").to_string_lossy()).unwrap().on_disk;
        assert!(z < (1 << 18), "压缩后应远小于 1 MB：{z}");
        let s = Win32Fs.file_info(&sp.to_string_lossy()).unwrap().on_disk;
        assert!(s < (1 << 20), "稀疏文件 64 MB 但几乎没分配：{s}");
    }

    /// 手动跑：对 DISKWHY_CMP_DIR 下所有文件对照两种取法（用于 WOF 压缩的 Edge 目录、System32 等）。
    /// `set DISKWHY_CMP_DIR=C:\Program Files (x86)\Microsoft\Edge && cargo test -p diskwhy-core --test dedup_tests cmp_dir -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cmp_dir_file_info_vs_compressed_size() {
        let dir = std::env::var("DISKWHY_CMP_DIR").expect("设置 DISKWHY_CMP_DIR");
        let mut stack = vec![PathBuf::from(&dir)];
        let (mut n, mut same, mut diff, mut failed, mut linked) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let (mut sum_a, mut sum_b) = (0u64, 0u64);
        while let Some(p) = stack.pop() {
            let Ok(rd) = fs::read_dir(&p) else { continue };
            for e in rd.flatten() {
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    stack.push(e.path());
                    continue;
                }
                let s = e.path().to_string_lossy().to_string();
                n += 1;
                match (Win32Fs.compressed_size(&s), Win32Fs.file_info(&s)) {
                    (Some(a), Some(fi)) => {
                        sum_a += a;
                        sum_b += fi.on_disk;
                        if fi.links > 1 {
                            linked += 1;
                        }
                        if a == fi.on_disk {
                            same += 1;
                        } else {
                            diff += 1;
                            if diff <= 10 {
                                println!("不一致：{s} GetCompressedFileSizeW={a} file_info={}", fi.on_disk);
                            }
                        }
                    }
                    _ => failed += 1,
                }
            }
        }
        println!(
            "{dir}: 文件 {n}，一致 {same}，不一致 {diff}，取不到 {failed}，链接数>1 的 {linked}；合计 {sum_a} vs {sum_b}"
        );
        assert_eq!(diff, 0);
    }

    #[test]
    fn real_snapshot_roundtrip_and_explain_after_load() {
        let t = TempTree::new("snap");
        let r0 = &t.0;
        fs::create_dir_all(r0.join("node_modules").join("x")).unwrap();
        fs::create_dir_all(r0.join("深层").join("b").join("c")).unwrap();
        fs::File::create(r0.join("package.json")).unwrap().write_all(b"{}").unwrap();
        write(&r0.join("node_modules").join("x").join("index.js"), 20_000, 1);
        write(&r0.join("深层").join("b").join("c").join("数据.bin"), 70_000, 2);
        for i in 0..60 {
            write(&r0.join(format!("f{i}.txt")), 100 + i, 3);
        }
        mklink_h(&r0.join("深层").join("link.bin"), &r0.join("深层").join("b").join("c").join("数据.bin"));
        let r = scan(&t.s(), &ScanOptions::default());
        assert!(r.buckets.hardlink_dedup > 0);

        let snap = r0.join("..").join(format!("diskwhy-snap-{}.dwsnap", std::process::id()));
        let bytes = snapshot::save(&r, &snap, "test").unwrap();
        assert!(bytes > 0);
        let raw = fs::read(&snap).unwrap();
        assert_eq!(&raw[..2], &[0x1f, 0x8b], "gzip");
        let (back, meta) = snapshot::with_big_stack({
            let snap = snap.clone();
            move || snapshot::load(&snap).unwrap()
        });
        let _ = fs::remove_file(&snap);
        assert_eq!(meta.kind, "snapshot");
        assert_eq!(meta.version, snapshot::VERSION);
        assert_eq!(back, r, "存 → 读 后逐字段相等（含目录 path、硬链接标记、归属）");

        // 读回后解释照常计算
        let (ctx, _) = explain::ExplainCtx::load_default();
        let l = view::listing(&back, &t.s(), 100, Some(&ctx)).unwrap();
        let nm = l.children.iter().find(|c| c.name == "node_modules").unwrap();
        let e = nm.explanation.as_ref().unwrap();
        assert!(matches!(&e.source, explain::Source::Heuristic { name } if name == "node-modules"), "{:?}", e.source);

        // CLI --json 风格（扁平 ScanResult、带 path）也能读
        let json = serde_json::to_vec(&r).unwrap();
        let (back2, meta2) = snapshot::from_json_bytes(&json).unwrap();
        assert_eq!(meta2.kind, "cli-json");
        assert_eq!(back2, r);
    }
}
