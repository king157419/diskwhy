//! 面向前端 / CLI 的只读视图：按路径查节点、列子项、全局前 N 大目录。

use crate::explain::{self, ExplainCtx, Explanation, Target};
use crate::fs::join;
use crate::model::{Node, NodeKind, ScanResult};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// 单个节点的展示信息（不含子树）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeView {
    pub name: String,
    /// 完整路径（File 节点由父路径拼出；Rest 节点为父目录路径）。
    pub path: String,
    pub kind: NodeKind,
    pub logical: u64,
    pub allocated: u64,
    pub mtime: i64,
    pub attrs: u32,
    pub reparse_tag: u32,
    pub inaccessible: bool,
    pub error_code: u32,
    pub reparse_skipped: bool,
    pub cloud_placeholder: bool,
    pub system_reserved: bool,
    pub files: u64,
    pub dirs: u64,
    /// 直接子项数（内存里的，Rest 算一个）。
    pub child_count: usize,
    /// 占父目录 allocated 的百分比（0~100）。根节点为占卷已用空间的百分比。
    pub percent_of_parent: f64,
    /// 中文类型说明。
    pub type_label: String,
    /// 解释（阶段 B）。没有解释上下文时为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<Explanation>,
}

/// 一个目录的列表视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirListing {
    pub node: NodeView,
    pub children: Vec<NodeView>,
    /// 子项总数（children 可能因 limit 被截断）。
    pub total_children: usize,
    /// 面包屑：从扫描根到当前目录的 (名字, 路径)。
    pub breadcrumb: Vec<Crumb>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crumb {
    pub name: String,
    pub path: String,
}

/// 中文类型说明。
pub fn type_label(n: &Node) -> String {
    match n.kind {
        NodeKind::Rest => "其余文件合计".into(),
        NodeKind::Dir => {
            if n.reparse_skipped {
                "链接/挂载点（未跟随）".into()
            } else if n.inaccessible {
                "文件夹（无权限）".into()
            } else if n.system_reserved {
                "系统保留文件夹".into()
            } else {
                "文件夹".into()
            }
        }
        NodeKind::File => {
            if n.cloud_placeholder {
                "云端占位文件".into()
            } else if n.system_reserved {
                "系统保留文件".into()
            } else {
                match n.name.rsplit_once('.') {
                    Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 10 => {
                        format!("{} 文件", ext.to_uppercase())
                    }
                    _ => "文件".into(),
                }
            }
        }
    }
}

pub fn node_view(n: &Node, parent_path: Option<&str>, parent_alloc: u64) -> NodeView {
    let path = match n.kind {
        NodeKind::Dir => n.path.clone(),
        NodeKind::File => parent_path.map(|p| join(p, &n.name)).unwrap_or_else(|| n.name.clone()),
        NodeKind::Rest => parent_path.unwrap_or("").to_string(),
    };
    NodeView {
        name: n.name.clone(),
        path,
        kind: n.kind,
        logical: n.logical,
        allocated: n.allocated,
        mtime: n.mtime,
        attrs: n.attrs,
        reparse_tag: n.reparse_tag,
        inaccessible: n.inaccessible,
        error_code: n.error_code,
        reparse_skipped: n.reparse_skipped,
        cloud_placeholder: n.cloud_placeholder,
        system_reserved: n.system_reserved,
        files: n.files,
        dirs: n.dirs,
        child_count: n.children.len(),
        percent_of_parent: if parent_alloc > 0 { n.allocated as f64 * 100.0 / parent_alloc as f64 } else { 0.0 },
        type_label: type_label(n),
        explanation: None,
    }
}

/// 按路径查找目录节点（大小写不敏感）。返回节点及从根开始的路径链。
pub fn find_dir<'a>(tree: &'a Node, path: &str) -> Option<Vec<&'a Node>> {
    let target = crate::scan::normalize_root(path);
    let root = &tree.path;
    if target.eq_ignore_ascii_case(root) {
        return Some(vec![tree]);
    }
    let rest = if target.len() > root.len() && target[..root.len()].eq_ignore_ascii_case(root) {
        target[root.len()..].trim_start_matches('\\')
    } else {
        return None;
    };
    let mut chain = vec![tree];
    let mut cur = tree;
    for comp in rest.split('\\').filter(|c| !c.is_empty()) {
        let lc = comp.to_lowercase();
        cur = cur
            .children
            .iter()
            .find(|c| c.kind == NodeKind::Dir && (c.name == comp || c.name.to_lowercase() == lc))?;
        chain.push(cur);
    }
    Some(chain)
}

/// 列出某个目录的子项（按 allocated 降序，最多 `limit` 条）。
///
/// 给了 `ctx` 时，对当前目录和每个子项都算解释（规则匹配 + 启发式，不调 LLM）。
pub fn listing(result: &ScanResult, path: &str, limit: usize, ctx: Option<&ExplainCtx>) -> Option<DirListing> {
    let chain = find_dir(&result.tree, path)?;
    let node = *chain.last().unwrap();
    let parent_alloc = if chain.len() >= 2 { chain[chain.len() - 2].allocated } else { result.used_bytes };
    // 子项的“上层规则继承”只跟当前目录的祖先链有关，算一次。
    let inherited = ctx.and_then(|c| c.rules.best_inherited(&chain));
    let children = node
        .children
        .iter()
        .take(limit)
        .map(|c| {
            let mut v = node_view(c, Some(&node.path), node.allocated);
            if let Some(ctx) = ctx {
                let t = Target { node: c, path: &v.path, parent: Some(node), ancestors: &chain };
                v.explanation = Some(explain::explain_with_inherited(&t, inherited.as_ref(), ctx));
            }
            v
        })
        .collect();
    let mut me = node_view(node, None, parent_alloc);
    if let Some(ctx) = ctx {
        let anc = &chain[..chain.len() - 1];
        let t = Target { node, path: &node.path, parent: anc.last().copied(), ancestors: anc };
        me.explanation = Some(explain::explain(&t, ctx));
    }
    Some(DirListing {
        node: me,
        children,
        total_children: node.children.len(),
        breadcrumb: chain.iter().map(|n| Crumb { name: n.name.clone(), path: n.path.clone() }).collect(),
    })
}

/// 按路径找任意节点（目录或文件）。返回 (从根到父目录的链, 节点)。
/// 找目录时链不含自身。文件只能在内存里保留的前 N 大文件里找到。
pub fn find_node<'a>(tree: &'a Node, path: &str) -> Option<(Vec<&'a Node>, &'a Node)> {
    if let Some(mut chain) = find_dir(tree, path) {
        let me = chain.pop().unwrap();
        return Some((chain, me));
    }
    let norm = crate::scan::normalize_root(path);
    let (parent, name) = norm.rsplit_once('\\')?;
    let parent = if parent.len() == 2 { format!("{parent}\\") } else { parent.to_string() };
    let chain = find_dir(tree, &parent)?;
    let dir = *chain.last().unwrap();
    let f = dir.children.iter().find(|c| c.kind == NodeKind::File && c.name.eq_ignore_ascii_case(name))?;
    Some((chain, f))
}

/// 解释树里的任意节点（CLI / “问 AI”用）。返回 (完整路径, 解释)。
pub fn explain_path(tree: &Node, path: &str, ctx: &ExplainCtx) -> Option<(String, Explanation)> {
    let (chain, node) = find_node(tree, path)?;
    let full = if node.kind == NodeKind::Dir { node.path.clone() } else { join(&chain.last()?.path, &node.name) };
    let t = Target { node, path: &full, parent: chain.last().copied(), ancestors: &chain };
    let e = explain::explain(&t, ctx);
    Some((full, e))
}

/// 全树按 allocated 排序的前 `n` 大目录（不含扫描根本身）。
pub fn top_dirs(tree: &Node, n: usize) -> Vec<&Node> {
    struct Item<'a>(u64, &'a Node);
    impl PartialEq for Item<'_> {
        fn eq(&self, o: &Self) -> bool {
            self.0 == o.0
        }
    }
    impl Eq for Item<'_> {}
    impl PartialOrd for Item<'_> {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Item<'_> {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.cmp(&o.0)
        }
    }
    let mut heap: BinaryHeap<Reverse<Item>> = BinaryHeap::new();
    let mut stack: Vec<&Node> = tree.children.iter().filter(|c| c.is_dir()).collect();
    while let Some(d) = stack.pop() {
        if heap.len() < n {
            heap.push(Reverse(Item(d.allocated, d)));
        } else if let Some(Reverse(min)) = heap.peek() {
            if d.allocated > min.0 {
                heap.pop();
                heap.push(Reverse(Item(d.allocated, d)));
            }
        }
        stack.extend(d.children.iter().filter(|c| c.is_dir()));
    }
    let mut v: Vec<&Node> = heap.into_iter().map(|Reverse(i)| i.1).collect();
    v.sort_by(|a, b| b.allocated.cmp(&a.allocated));
    v
}

/// 复制一棵只到 `depth` 层的树（用于导出 JSON，控制体积）。depth=0 只保留根。
pub fn truncate_tree(n: &Node, depth: usize) -> Node {
    let mut out = n.clone_shallow();
    if depth > 0 {
        out.children = n.children.iter().map(|c| truncate_tree(c, depth - 1)).collect();
    }
    out
}

impl Node {
    /// 复制节点本身，不带子项。
    pub fn clone_shallow(&self) -> Node {
        Node {
            name: self.name.clone(),
            path: self.path.clone(),
            kind: self.kind,
            logical: self.logical,
            allocated: self.allocated,
            mtime: self.mtime,
            attrs: self.attrs,
            reparse_tag: self.reparse_tag,
            inaccessible: self.inaccessible,
            error_code: self.error_code,
            reparse_skipped: self.reparse_skipped,
            cloud_placeholder: self.cloud_placeholder,
            system_reserved: self.system_reserved,
            files: self.files,
            dirs: self.dirs,
            children: Vec::new(),
        }
    }
}

/// 人类可读的字节数（1024 进制，中文单位习惯用 KB/MB/GB）。
pub fn human_bytes(b: u64) -> String {
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.2} {}", U[i])
    }
}
