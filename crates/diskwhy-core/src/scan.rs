//! 多线程扫描引擎。
//!
//! - rayon 线程池，每个目录一个任务（子目录用 `par_iter` 递归，work-stealing 调度）。
//! - 独立的进度线程按 `progress_interval_ms`（默认 100ms，≤200ms）回调一次。
//! - `cancel` 置位后所有任务尽快返回，最终结果为 `Err(ScanError::Cancelled)`。
//! - reparse point 目录（junction / 符号链接 / 挂载点）只记自身不递归；
//!   云文件（OneDrive 等 Cloud Files）目录的 reparse tag 例外，会进去枚举（枚举目录不会下载文件内容）。
//! - 云占位文件只读目录枚举给出的属性，绝不打开（也不调 GetCompressedFileSizeW）。

use crate::attribution::{compute_attribution, Measured};
use crate::dedup::{self, Occurrence};
use crate::fs::{attrs, errors, join, reparse, round_up, FileKey, FsBackend, RawEntry};
use crate::model::*;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 卷根下按名字归类为“系统保留”的条目（大小写不敏感）。
pub const SYSTEM_RESERVED_NAMES: &[&str] = &[
    "$Recycle.Bin",
    "System Volume Information",
    "pagefile.sys",
    "hiberfil.sys",
    "swapfile.sys",
    "Recovery",
    "$WinREAgent",
];

pub fn is_system_reserved_name(name: &str) -> bool {
    SYSTEM_RESERVED_NAMES.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// 最多记录多少条无权限目录明细。
const MAX_INACCESSIBLE_RECORDS: usize = 1000;

/// 规范化扫描根：`c:` / `c:/` / `\\?\C:\` → `C:\`；普通目录去掉结尾反斜杠。
pub fn normalize_root(input: &str) -> String {
    let mut s = input.trim().replace('/', "\\");
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let mut out = String::new();
        out.push((b[0] as char).to_ascii_uppercase());
        out.push_str(&s[1..]);
        s = out;
        if s.len() == 2 {
            s.push('\\');
        }
    }
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

/// 是否是卷根（`X:\`）。
pub fn is_volume_root(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

/// 取路径所在卷根（仅支持盘符路径）。
pub fn volume_root_of(path: &str) -> Option<String> {
    let b = path.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        Some(format!("{}:\\", (b[0] as char).to_ascii_uppercase()))
    } else {
        None
    }
}

struct Ctx<'a, B: FsBackend> {
    fs: &'a B,
    opts: &'a ScanOptions,
    cluster: u64,
    cancel: &'a AtomicBool,
    root_is_volume: bool,
    // 进度
    files: AtomicU64,
    dirs: AtomicU64,
    bytes: AtomicU64,
    inaccessible: AtomicU64,
    current: Mutex<String>,
    // 归属（按文件逐个分类累加，避免聚合节点丢失分类信息）
    scanned: AtomicU64,
    sys: AtomicU64,
    cloud_alloc: AtomicU64,
    cloud_files: AtomicU64,
    cloud_logical: AtomicU64,
    inacc_sys: AtomicU64,
    inacc_other: AtomicU64,
    inacc_list: Mutex<Vec<InaccessibleDir>>,
    /// 链接数 > 1 的文件出现记录（去重用）。
    occ: Mutex<Vec<Occurrence>>,
}

/// 硬链接密集区（Fast 模式下只在这些地方打开句柄查链接数）：卷根下的 `Windows`、
/// `Program Files`、`Program Files (x86)`。WinSxS 的硬链接另一端几乎都在这些目录里。
pub fn is_hardlink_dense(dir: &str) -> bool {
    let b = dir.as_bytes();
    if b.len() < 3 || b[1] != b':' {
        return false;
    }
    let rest = dir[3..].to_ascii_lowercase();
    let first = rest.split('\\').next().unwrap_or("");
    matches!(first, "windows" | "program files" | "program files (x86)")
}

impl<'a, B: FsBackend> Ctx<'a, B> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }

    /// 返回 (allocated, 硬链接 key)。key 只在去重开启且 nNumberOfLinks > 1 时有值。
    ///
    /// - Exact + 去重：每个文件用 `file_info` 打开一次句柄，同时拿到占用和链接数
    ///   （取代原来的 GetCompressedFileSizeW，同样是一次打开，所以耗时增加很少）；
    /// - Fast + 去重：只对压缩/稀疏/reparse 文件，以及硬链接密集区（`Windows`、`Program Files*`）
    ///   里的文件打开句柄；其余按逻辑大小按簇取整，不参与去重；
    /// - 去重关闭：与阶段 A/B 完全相同。
    fn allocated_for(&self, path: &str, e: &RawEntry, dense: bool) -> (u64, Option<FileKey>) {
        let special = e.attrs & (attrs::COMPRESSED | attrs::SPARSE_FILE | attrs::REPARSE_POINT) != 0;
        let need_query = match self.opts.size_mode {
            SizeMode::Exact => true,
            SizeMode::Fast => special || (self.opts.dedup && dense),
        };
        if need_query {
            if self.opts.dedup {
                if let Some(fi) = self.fs.file_info(path) {
                    let key = if fi.links > 1 { fi.key } else { None };
                    return (round_up(fi.on_disk, self.cluster), key);
                }
            } else if let Some(v) = self.fs.compressed_size(path) {
                return (round_up(v, self.cluster), None);
            }
        }
        // 取不到时退回逻辑大小（同样按簇取整，与卷上的实际分配粒度一致）。
        (round_up(e.size, self.cluster), None)
    }

    fn record_inaccessible(&self, path: &str, code: u32, sys: bool) {
        self.inaccessible.fetch_add(1, Relaxed);
        if sys {
            self.inacc_sys.fetch_add(1, Relaxed);
        } else {
            self.inacc_other.fetch_add(1, Relaxed);
        }
        if let Ok(mut l) = self.inacc_list.lock() {
            if l.len() < MAX_INACCESSIBLE_RECORDS {
                l.push(InaccessibleDir { path: path.to_string(), error_code: code, system_reserved: sys });
            }
        }
    }

    /// 硬链接：扫描时先照常计入，并把 (key, 目录, 名字, 占用) 记下来；扫描结束后
    /// [`crate::dedup::apply`] 统一挑一个路径记账，其余名字记 0（结果与线程调度无关）。
    fn file_node(&self, parent: &str, dir_arc: &mut Option<Arc<str>>, e: RawEntry, sys: bool, dense: bool) -> Node {
        let placeholder = e.attrs & attrs::CLOUD_PLACEHOLDER_MASK != 0;
        let allocated = if placeholder {
            // 绝不打开：内容不在本地，本地占用按 0 计。
            self.cloud_files.fetch_add(1, Relaxed);
            self.cloud_logical.fetch_add(e.size, Relaxed);
            0
        } else {
            let (a, key) = self.allocated_for(&join(parent, &e.name), &e, dense);
            if let Some(key) = key {
                let dir = dir_arc.get_or_insert_with(|| Arc::from(parent)).clone();
                if let Ok(mut v) = self.occ.lock() {
                    v.push(Occurrence { key, dir, name: e.name.clone(), alloc: a, sys });
                }
            }
            a
        };
        if placeholder {
            self.cloud_alloc.fetch_add(allocated, Relaxed);
        } else if sys {
            self.sys.fetch_add(allocated, Relaxed);
        } else {
            self.scanned.fetch_add(allocated, Relaxed);
        }
        self.files.fetch_add(1, Relaxed);
        self.bytes.fetch_add(allocated, Relaxed);
        Node {
            name: e.name,
            path: String::new(),
            kind: NodeKind::File,
            logical: e.size,
            allocated,
            mtime: e.mtime,
            attrs: e.attrs,
            reparse_tag: if e.attrs & attrs::REPARSE_POINT != 0 { e.reparse_tag } else { 0 },
            inaccessible: false,
            error_code: 0,
            reparse_skipped: false,
            cloud_placeholder: placeholder,
            system_reserved: sys,
            hardlink_dup: false,
            mapped: false,
            files: 1,
            dirs: 0,
            children: Vec::new(),
        }
    }

    fn empty_dir(&self, path: String, name: String, e_attrs: u32, mtime: i64, tag: u32, sys: bool) -> Node {
        Node {
            name,
            path,
            kind: NodeKind::Dir,
            logical: 0,
            allocated: 0,
            mtime,
            attrs: e_attrs,
            reparse_tag: tag,
            inaccessible: false,
            error_code: 0,
            reparse_skipped: false,
            cloud_placeholder: false,
            system_reserved: sys,
            hardlink_dup: false,
            mapped: false,
            files: 0,
            dirs: 0,
            children: Vec::new(),
        }
    }

    /// 扫描一个目录（已知可以进入的普通目录或云目录）。
    fn scan_dir(&self, path: String, name: String, e_attrs: u32, mtime: i64, tag: u32, sys: bool, depth: u32) -> Node {
        let mut node = self.empty_dir(path, name, e_attrs, mtime, tag, sys);
        if self.cancelled() {
            return node;
        }
        if let Ok(mut cur) = self.current.try_lock() {
            cur.clear();
            cur.push_str(&node.path);
        }
        let entries = match self.fs.list_dir(&node.path) {
            Ok(v) => v,
            Err(code) if code == errors::ERROR_FILE_NOT_FOUND => Vec::new(),
            Err(code) => {
                node.inaccessible = true;
                node.error_code = code;
                self.record_inaccessible(&node.path, code, sys);
                // 列不出目录，但从别处知道一些子目录（WindowsApps 的包目录）：能量的先量上。
                // 节点仍记“无权限”（清单不完整），差额推算照常进行，只是推算值会小一些。
                let known = self.fs.known_children(&node.path);
                if !known.is_empty() {
                    let kids: Vec<Node> = known
                        .into_par_iter()
                        // 注册表里还记着、但目录已经不在的包：跳过，不算无权限。
                        .filter(|n| {
                            !matches!(
                                self.fs.list_dir(&join(&node.path, n)),
                                Err(errors::ERROR_FILE_NOT_FOUND) | Err(errors::ERROR_PATH_NOT_FOUND)
                            )
                        })
                        .map(|n| {
                            let p = join(&node.path, &n);
                            self.scan_dir(p, n, attrs::DIRECTORY, 0, 0, sys, depth + 1)
                        })
                        .filter(|c| c.files > 0 || c.dirs > 0 || c.inaccessible)
                        .collect();
                    self.dirs.fetch_add(kids.len() as u64, Relaxed);
                    for c in &kids {
                        node.logical += c.logical;
                        node.allocated += c.allocated;
                        node.files += c.files;
                        node.dirs += c.dirs + 1;
                    }
                    node.children = kids;
                    node.children.sort_unstable_by(|a, b| b.allocated.cmp(&a.allocated).then_with(|| a.name.cmp(&b.name)));
                }
                return node;
            }
        };

        let mut files: Vec<Node> = Vec::new();
        let mut subdirs: Vec<(String, String, u32, i64, u32, bool)> = Vec::new();
        let mut skipped: Vec<Node> = Vec::new();
        let dense = self.opts.size_mode == SizeMode::Fast && self.opts.dedup && is_hardlink_dense(&node.path);
        let mut dir_arc: Option<Arc<str>> = None;

        for e in entries {
            if self.cancelled() {
                return node;
            }
            let child_sys = sys || (depth == 0 && self.root_is_volume && is_system_reserved_name(&e.name));
            if e.attrs & attrs::DIRECTORY != 0 {
                let is_reparse = e.attrs & attrs::REPARSE_POINT != 0;
                let child_path = join(&node.path, &e.name);
                if is_reparse && !reparse::is_cloud(e.reparse_tag) {
                    // junction / 符号链接 / 挂载点等：只记自身，不递归。
                    self.dirs.fetch_add(1, Relaxed);
                    let mut d = self.empty_dir(child_path, e.name, e.attrs, e.mtime, e.reparse_tag, child_sys);
                    d.reparse_skipped = true;
                    skipped.push(d);
                } else {
                    let tag = if is_reparse { e.reparse_tag } else { 0 };
                    subdirs.push((child_path, e.name, e.attrs, e.mtime, tag, child_sys));
                }
            } else {
                files.push(self.file_node(&node.path, &mut dir_arc, e, child_sys, dense));
            }
        }

        self.dirs.fetch_add(subdirs.len() as u64, Relaxed);
        let mut dir_nodes: Vec<Node> = subdirs
            .into_par_iter()
            .map(|(p, n, a, m, t, s)| self.scan_dir(p, n, a, m, t, s, depth + 1))
            .collect();

        // 文件只保留前 N 大，其余合并成一个 Rest 节点。
        let n_keep = self.opts.top_files_per_dir;
        if files.len() > n_keep {
            files.sort_unstable_by(|a, b| b.allocated.cmp(&a.allocated));
            let rest = files.split_off(n_keep);
            let mut r = Node {
                name: format!("（其余 {} 个文件）", rest.len()),
                path: String::new(),
                kind: NodeKind::Rest,
                logical: 0,
                allocated: 0,
                mtime: 0,
                attrs: 0,
                reparse_tag: 0,
                inaccessible: false,
                error_code: 0,
                reparse_skipped: false,
                cloud_placeholder: false,
                system_reserved: sys,
                hardlink_dup: false,
                mapped: false,
                files: rest.len() as u64,
                dirs: 0,
                children: Vec::new(),
            };
            for f in &rest {
                r.logical += f.logical;
                r.allocated += f.allocated;
                r.mtime = r.mtime.max(f.mtime);
            }
            files.push(r);
        }

        let mut children = Vec::with_capacity(dir_nodes.len() + skipped.len() + files.len());
        children.append(&mut dir_nodes);
        children.append(&mut skipped);
        children.append(&mut files);
        for c in &children {
            node.logical += c.logical;
            node.allocated += c.allocated;
            node.files += c.files;
            node.dirs += c.dirs + if c.kind == NodeKind::Dir { 1 } else { 0 };
        }
        children.sort_unstable_by(|a, b| b.allocated.cmp(&a.allocated).then_with(|| a.name.cmp(&b.name)));
        children.shrink_to_fit();
        node.children = children;
        node
    }

    fn snapshot(&self, start: Instant, used: u64) -> ScanProgress {
        ScanProgress {
            files: self.files.load(Relaxed),
            dirs: self.dirs.load(Relaxed),
            bytes: self.bytes.load(Relaxed),
            inaccessible: self.inaccessible.load(Relaxed),
            current_path: self.current.lock().map(|s| s.clone()).unwrap_or_default(),
            elapsed_ms: start.elapsed().as_millis() as u64,
            used_bytes: used,
        }
    }
}

/// 用任意后端扫描（测试用 mock，生产用 [`crate::win::Win32Fs`]）。
///
/// `volume`：卷空间信息（used 用于归属计算，cluster 用于取整）。
pub fn scan_with<B, F>(
    fs: &B,
    root: &str,
    volume: VolumeSpace,
    elevated: bool,
    opts: &ScanOptions,
    cancel: &AtomicBool,
    mut progress: F,
) -> Result<ScanResult, ScanError>
where
    B: FsBackend,
    F: FnMut(&ScanProgress) + Send,
{
    let root = normalize_root(root);
    let start = Instant::now();
    let ctx = Ctx {
        fs,
        opts,
        cluster: volume.cluster_size,
        cancel,
        root_is_volume: is_volume_root(&root),
        files: AtomicU64::new(0),
        dirs: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
        inaccessible: AtomicU64::new(0),
        current: Mutex::new(root.clone()),
        scanned: AtomicU64::new(0),
        sys: AtomicU64::new(0),
        cloud_alloc: AtomicU64::new(0),
        cloud_files: AtomicU64::new(0),
        cloud_logical: AtomicU64::new(0),
        inacc_sys: AtomicU64::new(0),
        inacc_other: AtomicU64::new(0),
        inacc_list: Mutex::new(Vec::new()),
        occ: Mutex::new(Vec::new()),
    };

    // 先确认根能打开，给出明确错误。
    if let Err(code) = fs.list_dir(&root) {
        if code != errors::ERROR_FILE_NOT_FOUND {
            return Err(ScanError::RootUnavailable { path: root, code });
        }
    }

    let threads = if opts.threads > 0 {
        opts.threads
    } else {
        // 扫描以 IO/系统调用为主，线程数取逻辑核数 ×2（上限 32）。
        (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) * 2).min(32)
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .stack_size(8 * 1024 * 1024)
        .thread_name(|i| format!("diskwhy-scan-{i}"))
        .build()
        .map_err(|e| ScanError::Volume(format!("创建线程池失败：{e}")))?;

    let root_mtime = std::fs::metadata(&root)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let interval = Duration::from_millis(opts.progress_interval_ms.clamp(10, 200));
    let done = AtomicBool::new(false);
    let used = volume.used_bytes();

    let tree = std::thread::scope(|s| {
        let ctx_ref = &ctx;
        let done_ref = &done;
        let reporter = s.spawn(move || {
            while !done_ref.load(Relaxed) {
                std::thread::sleep(interval);
                progress(&ctx_ref.snapshot(start, used));
            }
            progress(&ctx_ref.snapshot(start, used));
        });
        let tree = pool.install(|| {
            ctx.scan_dir(root.clone(), root.clone(), attrs::DIRECTORY, root_mtime, 0, false, 0)
        });
        done.store(true, Relaxed);
        let _ = reporter.join();
        tree
    });

    if cancel.load(Relaxed) {
        return Err(ScanError::Cancelled);
    }

    let mut tree = tree;
    let mut measured = Measured {
        scanned: ctx.scanned.load(Relaxed),
        system_reserved: ctx.sys.load(Relaxed),
        cloud_alloc: ctx.cloud_alloc.load(Relaxed),
        cloud_files: ctx.cloud_files.load(Relaxed),
        cloud_logical: ctx.cloud_logical.load(Relaxed),
        inaccessible_dirs_system: ctx.inacc_sys.load(Relaxed),
        inaccessible_dirs_other: ctx.inacc_other.load(Relaxed),
    };

    // 去重（硬链接 + 商店应用目录映射）：扫描完成后统一处理，结果与线程调度无关。
    let occ = std::mem::take(&mut *ctx.occ.lock().unwrap_or_else(|e| e.into_inner()));
    let stats = if opts.dedup {
        let st = pool.install(|| dedup::apply(&mut tree, occ, fs));
        measured.scanned = measured.scanned.saturating_sub(st.removed_scanned);
        measured.system_reserved = measured.system_reserved.saturating_sub(st.removed_sys);
        Some(st)
    } else {
        None
    };

    let mut buckets = compute_attribution(used, &measured, ctx.root_is_volume);
    let mut mapped_dirs = Vec::new();
    if let Some(st) = stats {
        buckets.dedup_enabled = true;
        buckets.hardlink_dedup = st.hardlink_bytes;
        buckets.hardlink_dedup_files = st.hardlink_files;
        buckets.mapped_dedup = st.mapped_bytes;
        buckets.mapped_dirs = st.mapped.len() as u64;
        mapped_dirs = st.mapped;
    }
    let inaccessible_dirs = ctx.inacc_list.into_inner().unwrap_or_default();

    Ok(ScanResult {
        mapped_dirs,
        root: root.clone(),
        total_bytes: volume.total_bytes,
        used_bytes: used,
        free_bytes: volume.free_bytes,
        cluster_size: volume.cluster_size,
        files: tree.files,
        dirs: tree.dirs,
        tree,
        buckets,
        duration_ms: start.elapsed().as_millis() as u64,
        elevated,
        inaccessible_total: ctx.inaccessible.load(Relaxed),
        inaccessible_dirs,
        size_mode: opts.size_mode,
        top_files_per_dir: opts.top_files_per_dir,
    })
}

/// 扫描真实目录 / 盘符（Windows）。
#[cfg(windows)]
pub fn scan<F>(root: &str, opts: &ScanOptions, cancel: &AtomicBool, progress: F) -> Result<ScanResult, ScanError>
where
    F: FnMut(&ScanProgress) + Send,
{
    let root = normalize_root(root);
    let vol_root = volume_root_of(&root).ok_or_else(|| ScanError::Volume(format!("不支持的路径：{root}（目前只支持盘符路径）")))?;
    let volume = crate::win::volume_space(&vol_root).map_err(ScanError::Volume)?;
    scan_with(&crate::win::Win32Fs, &root, volume, crate::win::is_elevated(), opts, cancel, progress)
}
