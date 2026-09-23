//! 去重（阶段 C）：同一份数据被多个路径看到时只算一次。
//!
//! 两种情况：
//!
//! 1. **硬链接**：NTFS 上一个文件可以有多个名字（`nNumberOfLinks > 1`），最典型的是
//!    `Windows\WinSxS` 与 `System32` 等目录之间。扫描时对链接数 > 1 的文件记下
//!    (卷序列号, 文件 ID)，扫描结束后按 key 分组：每组挑一个名字记账，其余名字记 0 并标
//!    `hardlink_dup`。挑选规则（与线程调度无关，结果可复现）：
//!    - 不在 `\Windows\WinSxS\` 下的优先——这样 WinSxS 显示的是它“自己独占”的大小，
//!      与 Microsoft 文档“WinSxS 看起来比实际大，因为大量文件是硬链接”的说法一致；
//!    - 其余按完整路径（小写）字典序最小的优先。
//! 2. **商店应用目录映射**：MSIX 打包的桌面应用写 `%APPDATA%\X` 时，数据实际落在
//!    `%LOCALAPPDATA%\Packages\<包名>\LocalCache\Roaming\X`，而普通程序从两个路径都能看到
//!    同一个目录（不是硬链接、也没有 reparse 属性，但两边目录的文件 ID 相同）。
//!    对树里每个 `...\AppData\Local\Packages\<包>\LocalCache\{Roaming,Local}\X`，如果
//!    `...\AppData\{Roaming,Local}\X` 也在树里且两边目录文件 ID 相同，就把 AppData 那一侧
//!    记 0（`mapped = true`，子项不再展开），空间记在 Packages 那一侧（数据真正所在的位置）。
//!
//! 去重后重新自底向上汇总目录大小并重排子项。

use crate::fs::{join, FileKey, FsBackend};
use crate::model::{MappedDir, Node, NodeKind};
use std::collections::HashMap;
use std::sync::Arc;

/// 扫描时记下的一次“链接数 > 1 的文件”出现。
#[derive(Debug, Clone)]
pub struct Occurrence {
    pub key: FileKey,
    /// 所在目录完整路径。
    pub dir: Arc<str>,
    pub name: String,
    /// 这次出现时计入的 allocated。
    pub alloc: u64,
    /// 是否位于系统保留项下（决定从哪个归属桶里扣）。
    pub sys: bool,
}

/// 去重结果。
#[derive(Debug, Clone, Default)]
pub struct DedupStats {
    pub hardlink_bytes: u64,
    pub hardlink_files: u64,
    pub mapped_bytes: u64,
    pub mapped: Vec<MappedDir>,
    /// 需要从 Measured.scanned / system_reserved 里扣掉的字节。
    pub removed_scanned: u64,
    pub removed_sys: u64,
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `p` 是否等于 `prefix` 或位于其下（都已小写）。
fn under(p: &str, prefix: &str) -> bool {
    p == prefix || (p.len() > prefix.len() && p.starts_with(prefix) && p.as_bytes()[prefix.len()] == b'\\')
}

fn in_winsxs(dir_lower: &str) -> bool {
    dir_lower.contains("\\windows\\winsxs\\") || dir_lower.ends_with("\\windows\\winsxs")
}

/// 执行全部去重。`occ` 为扫描期间收集的硬链接出现记录。
pub fn apply<B: FsBackend>(tree: &mut Node, occ: Vec<Occurrence>, fs: &B) -> DedupStats {
    let mut st = DedupStats::default();

    // ── 1. 目录映射 ──
    let pairs = find_mappings(tree, fs);
    let mut dropped: Vec<String> = Vec::new();
    for (loser, winner) in pairs {
        if let Some(n) = find_dir_mut(tree, &loser) {
            if n.mapped {
                continue;
            }
            let (sc, sy) = measure(n);
            st.removed_scanned += sc;
            st.removed_sys += sy;
            st.mapped_bytes += n.allocated;
            st.mapped.push(MappedDir { path: n.path.clone(), same_as: winner, bytes: n.allocated, files: n.files });
            n.children = Vec::new();
            n.allocated = 0;
            n.logical = 0;
            n.files = 0;
            n.dirs = 0;
            n.mapped = true;
            dropped.push(lower(&loser));
        }
    }

    // ── 2. 硬链接 ──
    let mut occ: Vec<(Occurrence, String)> = occ
        .into_iter()
        .map(|o| {
            let dl = lower(&o.dir);
            (o, dl)
        })
        .filter(|(_, dl)| !dropped.iter().any(|d| under(dl, d)))
        .collect();
    occ.sort_by(|a, b| a.0.key.cmp(&b.0.key));
    // 目录（小写） → [(名字, 占用)]
    let mut losers: HashMap<String, Vec<(String, u64)>> = HashMap::new();
    let mut i = 0;
    while i < occ.len() {
        let mut j = i + 1;
        while j < occ.len() && occ[j].0.key == occ[i].0.key {
            j += 1;
        }
        if j - i >= 2 {
            let group = &occ[i..j];
            let rank = |x: &(Occurrence, String)| (in_winsxs(&x.1), join(&x.1, &lower(&x.0.name)));
            let win = group.iter().enumerate().min_by(|a, b| rank(a.1).cmp(&rank(b.1))).map(|(k, _)| k).unwrap_or(0);
            for (k, (o, dl)) in group.iter().enumerate() {
                if k == win {
                    continue;
                }
                losers.entry(dl.clone()).or_default().push((o.name.clone(), o.alloc));
                st.hardlink_files += 1;
                if o.sys {
                    st.removed_sys += o.alloc;
                } else {
                    st.removed_scanned += o.alloc;
                }
                st.hardlink_bytes += o.alloc;
            }
        }
        i = j;
    }
    if !losers.is_empty() {
        mark_losers(tree, &mut losers);
    }

    // ── 3. 重新汇总 ──
    if !st.mapped.is_empty() || st.hardlink_files > 0 {
        recompute(tree);
    }
    st
}

/// 量一棵子树里普通文件 / 系统保留文件各多少（云占位本地按 0，不用管）。
fn measure(n: &Node) -> (u64, u64) {
    match n.kind {
        NodeKind::File | NodeKind::Rest => {
            if n.cloud_placeholder {
                (0, 0)
            } else if n.system_reserved {
                (0, n.allocated)
            } else {
                (n.allocated, 0)
            }
        }
        NodeKind::Dir => n.children.iter().fold((0, 0), |acc, c| {
            let (a, b) = measure(c);
            (acc.0 + a, acc.1 + b)
        }),
    }
}

fn mark_losers(n: &mut Node, losers: &mut HashMap<String, Vec<(String, u64)>>) {
    if n.kind != NodeKind::Dir {
        return;
    }
    if let Some(list) = losers.remove(&lower(&n.path)) {
        for (name, alloc) in list {
            if let Some(f) = n
                .children
                .iter_mut()
                .find(|c| c.kind == NodeKind::File && !c.hardlink_dup && c.name == name)
            {
                f.allocated = 0;
                f.hardlink_dup = true;
            } else if let Some(r) = n.children.iter_mut().find(|c| c.kind == NodeKind::Rest) {
                // 被合并进“其余小文件”的：从合计里扣掉。
                r.allocated = r.allocated.saturating_sub(alloc);
            }
        }
    }
    if losers.is_empty() {
        return;
    }
    for c in n.children.iter_mut() {
        if c.kind == NodeKind::Dir && !c.children.is_empty() {
            mark_losers(c, losers);
            if losers.is_empty() {
                return;
            }
        }
    }
}

/// 自底向上重算目录的 allocated / logical / files / dirs，并按 allocated 降序重排。
pub fn recompute(n: &mut Node) {
    if n.kind != NodeKind::Dir || n.children.is_empty() {
        return;
    }
    let (mut a, mut l, mut f, mut d) = (0u64, 0u64, 0u64, 0u64);
    for c in n.children.iter_mut() {
        recompute(c);
        a += c.allocated;
        l += c.logical;
        f += c.files;
        d += c.dirs + if c.kind == NodeKind::Dir { 1 } else { 0 };
    }
    n.allocated = a;
    n.logical = l;
    n.files = f;
    n.dirs = d;
    n.children.sort_by(|x, y| y.allocated.cmp(&x.allocated).then_with(|| x.name.cmp(&y.name)));
}

/// 找商店应用目录映射：返回 (记 0 的 AppData 路径, 记账的 Packages 路径)。
pub fn find_mappings<B: FsBackend>(tree: &Node, fs: &B) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<&Node> = vec![tree];
    let mut cands: Vec<(&Node, &Node)> = Vec::new(); // (AppData 节点, LocalCache 节点)
    // 先找 AppData 节点：名字是 AppData 的目录。
    while let Some(n) = stack.pop() {
        for c in &n.children {
            if c.kind != NodeKind::Dir {
                continue;
            }
            if c.name.eq_ignore_ascii_case("AppData") {
                // AppData\Local\Packages\<pkg>\LocalCache
                if let Some(pk) = child(c, "Local").and_then(|l| child(l, "Packages")) {
                    for pkg in pk.children.iter().filter(|p| p.kind == NodeKind::Dir) {
                        if let Some(lc) = child(pkg, "LocalCache") {
                            cands.push((c, lc));
                        }
                    }
                }
            } else {
                stack.push(c);
            }
        }
    }
    for (appdata, lc) in cands {
        for kind in ["Roaming", "Local"] {
            let (Some(src), Some(dst_parent)) = (child(lc, kind), child(appdata, kind)) else { continue };
            for x in src.children.iter().filter(|x| x.kind == NodeKind::Dir && !x.reparse_skipped && !x.inaccessible) {
                let Some(twin) = child(dst_parent, &x.name) else { continue };
                if twin.reparse_skipped || twin.inaccessible || twin.mapped {
                    continue;
                }
                // 防御：twin 不能是 x 的祖先（例如 LocalCache\Local\Packages 之类的怪名字）。
                if under(&lower(&x.path), &lower(&twin.path)) {
                    continue;
                }
                match (fs.dir_key(&x.path), fs.dir_key(&twin.path)) {
                    (Some(a), Some(b)) if a == b => out.push((twin.path.clone(), x.path.clone())),
                    _ => {}
                }
            }
        }
    }
    out
}

fn child<'a>(n: &'a Node, name: &str) -> Option<&'a Node> {
    n.children.iter().find(|c| c.kind == NodeKind::Dir && c.name.eq_ignore_ascii_case(name))
}

/// 按完整路径找目录（可变）。
pub fn find_dir_mut<'a>(tree: &'a mut Node, path: &str) -> Option<&'a mut Node> {
    let target = crate::scan::normalize_root(path);
    let root = tree.path.clone();
    if target.eq_ignore_ascii_case(&root) {
        return Some(tree);
    }
    if !(target.len() > root.len() && target[..root.len()].eq_ignore_ascii_case(&root)) {
        return None;
    }
    let rest = target[root.len()..].trim_start_matches('\\').to_string();
    let mut cur = tree;
    for comp in rest.split('\\').filter(|c| !c.is_empty()) {
        cur = cur.children.iter_mut().find(|c| c.kind == NodeKind::Dir && c.name.eq_ignore_ascii_case(comp))?;
    }
    Some(cur)
}
