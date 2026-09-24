//! 动作之后刷新受影响的目录：只重扫这些子树，替换进扫描树，再把祖先的大小重新加一遍，
//! 并用新的卷空间重算归属（不用全盘重扫）。

use crate::attribution::{compute_attribution, Measured};
use crate::dedup::find_dir_mut;
use crate::model::{Node, NodeKind, ScanOptions, ScanResult};
use crate::scan::{is_volume_root, normalize_root, scan_with, volume_root_of};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RefreshReport {
    /// 实际重扫的目录。
    pub rescanned: Vec<String>,
    /// 已经不存在、从树里拿掉的目录。
    pub removed: Vec<String>,
    /// 重扫前后这些目录合计的变化（字节，负数 = 变小）。
    pub delta_bytes: i64,
    pub duration_ms: u64,
}

fn parent_of(p: &str) -> Option<String> {
    let n = normalize_root(p);
    if is_volume_root(&n) {
        return None;
    }
    let (parent, _) = n.rsplit_once('\\')?;
    Some(if parent.len() == 2 { format!("{parent}\\") } else { parent.to_string() })
}

/// 刷新若干目录。路径不在扫描树里（在扫描根之外）会被忽略。
pub fn refresh_dirs(result: &mut ScanResult, dirs: &[String]) -> RefreshReport {
    let start = std::time::Instant::now();
    let mut rep = RefreshReport::default();
    // 不在树里的目录（例如还原时重新建出来的、之前被当成空文件夹删掉的子目录）换成树里最近的上层目录；
    // 在扫描根之外的丢掉。然后去重、去掉被包含的。
    let root_l = result.tree.path.to_lowercase();
    let mut list: Vec<String> = dirs
        .iter()
        .filter_map(|d| {
            let mut t = normalize_root(d);
            loop {
                let tl = t.to_lowercase();
                let inside = tl == root_l || tl.starts_with(&format!("{}\\", root_l.trim_end_matches('\\')));
                if !inside {
                    return None;
                }
                if crate::view::find_dir(&result.tree, &t).is_some() {
                    return Some(t);
                }
                t = parent_of(&t)?;
            }
        })
        .collect();
    list.sort_by_key(|s| s.to_lowercase());
    list.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let snapshot = list.clone();
    list.retain(|p| {
        !snapshot.iter().any(|q| {
            let (pl, ql) = (p.to_lowercase(), q.to_lowercase());
            pl != ql && pl.starts_with(&format!("{}\\", ql.trim_end_matches('\\')))
        })
    });
    let root_volume = is_volume_root(&result.tree.path);
    let mut sys_delta: i64 = 0;
    let mut scanned_delta: i64 = 0;
    for dir in list {
        let target = dir.clone();
        if crate::view::find_dir(&result.tree, &target).is_none() {
            continue;
        }
        let on_disk = std::fs::metadata(crate::win::long_path(&target)).map(|m| m.is_dir()).unwrap_or(false);
        if !on_disk {
            // 整个目录没了：从父节点里拿掉，祖先重新加总即可，不用重扫父目录
            if let Some(parent) = parent_of(&target) {
                if let Some(pn) = find_dir_mut(&mut result.tree, &parent) {
                    if let Some(pos) = pn.children.iter().position(|c| c.kind == NodeKind::Dir && c.path.eq_ignore_ascii_case(&target)) {
                        let old = pn.children.remove(pos);
                        if old.system_reserved {
                            sys_delta -= old.allocated as i64;
                        } else {
                            scanned_delta -= old.allocated as i64;
                        }
                        rep.delta_bytes -= old.allocated as i64;
                        rep.removed.push(target.clone());
                    }
                }
                recompute_chain(&mut result.tree, &parent);
            }
            continue;
        }
        let Some(vol) = volume_root_of(&target) else { continue };
        let Ok(space) = crate::win::volume_space(&vol) else { continue };
        let opts = ScanOptions { size_mode: result.size_mode, top_files_per_dir: result.top_files_per_dir, ..Default::default() };
        let Ok(sub) = scan_with(&crate::win::Win32Fs, &target, space, result.elevated, &opts, &AtomicBool::new(false), |_| {}) else {
            continue;
        };
        if let Some(node) = find_dir_mut(&mut result.tree, &target) {
            let old_alloc = node.allocated as i64;
            let sys = node.system_reserved;
            let name = node.name.clone();
            let path = node.path.clone();
            let mut fresh: Node = sub.tree;
            fresh.name = name;
            fresh.path = path;
            fresh.system_reserved = sys;
            let new_alloc = fresh.allocated as i64;
            *node = fresh;
            if sys {
                sys_delta += new_alloc - old_alloc;
            } else {
                scanned_delta += new_alloc - old_alloc;
            }
            rep.delta_bytes += new_alloc - old_alloc;
            rep.rescanned.push(target.clone());
        }
        if let Some(parent) = parent_of(&target) {
            recompute_chain(&mut result.tree, &parent);
        }
    }
    result.files = result.tree.files;
    result.dirs = result.tree.dirs;
    // 新的卷空间 + 调整后的量到部分 → 重新分配差额
    if let Some(vol) = volume_root_of(&result.tree.path) {
        if let Ok(space) = crate::win::volume_space(&vol) {
            result.total_bytes = space.total_bytes;
            result.free_bytes = space.free_bytes;
            result.used_bytes = space.used_bytes();
        }
    }
    let b = &result.buckets;
    let m = Measured {
        scanned: (b.scanned as i64 + scanned_delta).max(0) as u64,
        system_reserved: ((b.system_reserved - b.system_reserved_estimated) as i64 + sys_delta).max(0) as u64,
        cloud_alloc: b.cloud_placeholder,
        cloud_files: b.cloud_placeholder_files,
        cloud_logical: b.cloud_placeholder_logical,
        inaccessible_dirs_system: b.inaccessible_dirs_system,
        inaccessible_dirs_other: b.inaccessible_dirs_other,
    };
    let mut nb = compute_attribution(result.used_bytes, &m, root_volume);
    nb.dedup_enabled = b.dedup_enabled;
    nb.hardlink_dedup = b.hardlink_dedup;
    nb.hardlink_dedup_files = b.hardlink_dedup_files;
    nb.mapped_dedup = b.mapped_dedup;
    nb.mapped_dirs = b.mapped_dirs;
    result.buckets = nb;
    rep.duration_ms = start.elapsed().as_millis() as u64;
    rep
}

/// 从 `path` 一直到扫描根，逐层用直接子项重新加总（不递归进子树）并重排。
fn recompute_chain(tree: &mut Node, path: &str) {
    let mut cur = Some(normalize_root(path));
    while let Some(p) = cur {
        if let Some(n) = find_dir_mut(tree, &p) {
            let (mut a, mut l, mut f, mut d) = (0u64, 0u64, 0u64, 0u64);
            for c in &n.children {
                a += c.allocated;
                l += c.logical;
                f += c.files;
                d += c.dirs + u64::from(c.kind == NodeKind::Dir);
            }
            n.allocated = a;
            n.logical = l;
            n.files = f;
            n.dirs = d;
            n.children.sort_by(|x, y| y.allocated.cmp(&x.allocated).then_with(|| x.name.cmp(&y.name)));
        }
        if p.eq_ignore_ascii_case(&tree.path) {
            break;
        }
        cur = parent_of(&p);
    }
}
