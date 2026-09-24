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
    /// 硬链接的“多余名字”：空间记在别的路径上，这里为 0。
    #[serde(default)]
    pub hardlink_dup: bool,
    /// 商店应用目录映射：和 `mapped_to` 是同一份数据，空间记在那边。
    #[serde(default)]
    pub mapped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapped_to: Option<String>,
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
            if n.mapped {
                "目录映射（同一份数据）".into()
            } else if n.reparse_skipped {
                "链接/挂载点（未跟随）".into()
            } else if n.inaccessible && !n.children.is_empty() {
                format!("文件夹（无权限，只量了已知的 {} 个子目录）", n.children.len())
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
            } else if n.hardlink_dup {
                "硬链接（空间记在别处）".into()
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
        hardlink_dup: n.hardlink_dup,
        mapped: n.mapped,
        mapped_to: None,
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
            if c.mapped {
                v.mapped_to = mapped_to(result, &c.path);
            }
            if let Some(ctx) = ctx {
                let t = Target { node: c, path: &v.path, parent: Some(node), ancestors: &chain };
                v.explanation = Some(explain::explain_with_inherited(&t, inherited.as_ref(), ctx));
            }
            v
        })
        .collect();
    let mut me = node_view(node, None, parent_alloc);
    if node.mapped {
        me.mapped_to = mapped_to(result, &node.path);
    }
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

/// 映射目录对应的“真正记账”路径。
pub fn mapped_to(result: &ScanResult, path: &str) -> Option<String> {
    result.mapped_dirs.iter().find(|m| m.path.eq_ignore_ascii_case(path)).map(|m| m.same_as.clone())
}

// ───────────────────────── 阶段 C：方块图（treemap）数据 ─────────────────────────

/// 方块图的一个节点（只带画图和悬停提示需要的字段；详情仍走 `listing`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreemapNode {
    pub name: String,
    /// File 由父路径拼出；Rest 为父目录路径。
    pub path: String,
    pub kind: NodeKind,
    pub allocated: u64,
    /// safe / review / keep / unknown（与列表色标一致）
    pub tier: String,
    pub what: String,
    pub owner: Option<String>,
    pub inaccessible: bool,
    pub reparse_skipped: bool,
    pub system_reserved: bool,
    pub cloud_placeholder: bool,
    pub hardlink_dup: bool,
    pub mapped: bool,
    /// 能否双击进入（有子项的普通目录）。
    pub can_open: bool,
    pub children: Vec<TreemapNode>,
    /// 因为太小 / 太多没有发过来的子项合计（前端画成“其他”）。
    pub hidden_bytes: u64,
    pub hidden_count: usize,
}

/// 每层最多发多少个子项、占根的最小比例。
const TM_LEVELS: [(usize, f64); 3] = [(300, 0.0), (80, 0.0004), (40, 0.001)];

/// 取以 `path` 为根、最多 `depth` 层（1~3）的方块图数据。每个节点都带档位和一句话说明。
pub fn treemap(result: &ScanResult, path: &str, depth: usize, ctx: Option<&ExplainCtx>) -> Option<TreemapNode> {
    let chain = find_dir(&result.tree, path)?;
    let node = *chain.last().unwrap();
    let anc = &chain[..chain.len() - 1];
    let me = tm_node(result, node, &node.path, anc, None, ctx);
    let root_alloc = node.allocated.max(1);
    let mut chain_v: Vec<&Node> = chain.clone();
    Some(tm_fill(result, me, node, &mut chain_v, 0, depth.clamp(1, 3), root_alloc, ctx))
}

fn tm_node(
    result: &ScanResult,
    n: &Node,
    path: &str,
    ancestors: &[&Node],
    inherited: Option<&crate::explain::rules::InheritedHit>,
    ctx: Option<&ExplainCtx>,
) -> TreemapNode {
    let _ = result;
    let (tier, what, owner) = match ctx {
        Some(ctx) => {
            let t = Target { node: n, path, parent: ancestors.last().copied(), ancestors };
            let e = match inherited {
                Some(h) => explain::explain_with_inherited(&t, Some(h), ctx),
                None if ancestors.is_empty() => explain::explain(&t, ctx),
                None => explain::explain_with_inherited(&t, None, ctx),
            };
            let tier = if e.is_unknown() {
                "unknown"
            } else {
                match e.tier {
                    explain::Tier::Safe => "safe",
                    explain::Tier::Review => "review",
                    explain::Tier::Keep => "keep",
                }
            };
            (tier.to_string(), e.what, e.owner)
        }
        None => ("unknown".to_string(), String::new(), None),
    };
    TreemapNode {
        name: n.name.clone(),
        path: path.to_string(),
        kind: n.kind,
        allocated: n.allocated,
        tier,
        what,
        owner,
        inaccessible: n.inaccessible,
        reparse_skipped: n.reparse_skipped,
        system_reserved: n.system_reserved,
        cloud_placeholder: n.cloud_placeholder,
        hardlink_dup: n.hardlink_dup,
        mapped: n.mapped,
        can_open: n.kind == NodeKind::Dir && !n.reparse_skipped && !n.children.is_empty(),
        children: Vec::new(),
        hidden_bytes: 0,
        hidden_count: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn tm_fill<'a>(
    result: &ScanResult,
    mut me: TreemapNode,
    node: &'a Node,
    chain: &mut Vec<&'a Node>,
    level: usize,
    depth: usize,
    root_alloc: u64,
    ctx: Option<&ExplainCtx>,
) -> TreemapNode {
    if level >= depth || node.kind != NodeKind::Dir || node.children.is_empty() {
        return me;
    }
    let (max_n, min_frac) = TM_LEVELS[level.min(TM_LEVELS.len() - 1)];
    let min_bytes = (root_alloc as f64 * min_frac) as u64;
    let inherited = ctx.and_then(|c| c.rules.best_inherited(chain));
    let mut sent = 0usize;
    for c in &node.children {
        if c.allocated == 0 {
            continue;
        }
        if sent >= max_n || c.allocated < min_bytes.max(1) {
            me.hidden_bytes += c.allocated;
            me.hidden_count += 1;
            continue;
        }
        sent += 1;
        let path = match c.kind {
            NodeKind::Dir => c.path.clone(),
            NodeKind::File => join(&node.path, &c.name),
            NodeKind::Rest => node.path.clone(),
        };
        let child = tm_node(result, c, &path, chain, inherited.as_ref(), ctx);
        let child = if c.kind == NodeKind::Dir {
            chain.push(c);
            let x = tm_fill(result, child, c, chain, level + 1, depth, root_alloc, ctx);
            chain.pop();
            x
        } else {
            child
        };
        me.children.push(child);
    }
    me
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
            hardlink_dup: self.hardlink_dup,
            mapped: self.mapped,
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

// ───────────────────────── 阶段 B-3：带解释的 JSON 导出 / 扫描根以上的祖先 / 子树是否完整 ─────────────────────────

/// `diskwhy-cli scan --json` 导出的目录树节点：原节点字段 + 目录的解释摘要。
///
/// 读回来时（[`crate::snapshot::from_json_bytes`]）按普通 [`Node`] 解析，`explanation` 字段被忽略。
#[derive(Debug, Clone, Serialize)]
pub struct ExplainedNode {
    #[serde(flatten)]
    pub node: Node,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<explain::ExplainSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ExplainedNode>,
}

/// 导出一棵截断过的树：
/// - 前 `depth` 层全部保留（depth=0 只保留根）；
/// - `keep` 里的每个目录（通常是前 N 大目录），连同它的所有上层文件夹，都把**直接子项**保留下来，
///   这样读回 JSON 后前 N 大目录还在、规则的 `when_child` 和“旁边有没有 package.json”这类判断还能做；
/// - 给了 `ctx` 时，每个目录节点附解释摘要（在完整的树上算，不受截断影响；不调 LLM）。
///
/// `outer` 是扫描根以上的祖先（见 [`outer_ancestors`]），用于上层规则的继承。
pub fn export_tree(tree: &Node, depth: usize, keep: &[&str], outer: &[Node], ctx: Option<&ExplainCtx>) -> ExplainedNode {
    let mut spine = std::collections::HashSet::new();
    for p in keep {
        let norm = crate::scan::normalize_root(p).to_lowercase();
        let mut cur = norm.as_str();
        loop {
            spine.insert(cur.to_string());
            match cur.rsplit_once('\\') {
                Some((parent, _)) if parent.len() >= 2 => {
                    cur = if parent.len() == 2 { &norm[..3] } else { parent };
                    if spine.contains(cur) {
                        break;
                    }
                }
                _ => break,
            }
        }
    }
    let mut chain: Vec<&Node> = outer.iter().collect();
    export_walk(tree, 0, depth, &spine, &mut chain, ctx)
}

fn export_walk<'a>(
    n: &'a Node,
    level: usize,
    depth: usize,
    spine: &std::collections::HashSet<String>,
    chain: &mut Vec<&'a Node>,
    ctx: Option<&ExplainCtx>,
) -> ExplainedNode {
    let explanation = match (ctx, n.kind) {
        (Some(ctx), NodeKind::Dir) => {
            let t = Target { node: n, path: &n.path, parent: chain.last().copied(), ancestors: chain };
            Some(explain::explain(&t, ctx).summary())
        }
        _ => None,
    };
    let expand = n.kind == NodeKind::Dir && (level < depth || spine.contains(&n.path.to_lowercase()));
    let mut children = Vec::new();
    if expand {
        chain.push(n);
        for c in &n.children {
            if c.kind == NodeKind::Dir {
                children.push(export_walk(c, level + 1, depth, spine, chain, ctx));
            } else {
                children.push(ExplainedNode { node: c.clone_shallow(), explanation: None, children: Vec::new() });
            }
        }
        chain.pop();
    }
    ExplainedNode { node: n.clone_shallow(), explanation, children }
}

/// 扫描根以上的各层文件夹（从盘符根开始，不含扫描根本身），每个只带**直接子项的名字和类型**
/// （读一次目录，不量大小）。只扫 `C:\Users\me\AppData\Local\pip\cache` 时，
/// 靠它才能知道上层 `pip` 文件夹命中了哪条规则（继承），以及 node_modules 旁边有没有 package.json。
pub fn outer_ancestors(root: &str) -> Vec<Node> {
    let norm = crate::scan::normalize_root(root);
    let mut prefixes = Vec::new();
    let mut cur = norm.as_str();
    while let Some((parent, _)) = cur.rsplit_once('\\') {
        if parent.len() < 2 {
            break;
        }
        let p = if parent.len() == 2 { &norm[..3] } else { parent };
        if p.eq_ignore_ascii_case(cur) {
            break;
        }
        prefixes.push(p.to_string());
        cur = p;
        if p.len() <= 3 {
            break;
        }
    }
    prefixes.reverse();
    prefixes
        .into_iter()
        .map(|p| {
            let name = if p.len() <= 3 { p.clone() } else { p.rsplit('\\').next().unwrap_or("").to_string() };
            let mut me = Node {
                name,
                path: p.clone(),
                kind: NodeKind::Dir,
                logical: 0,
                allocated: 0,
                mtime: 0,
                attrs: 0,
                reparse_tag: 0,
                inaccessible: false,
                error_code: 0,
                reparse_skipped: false,
                cloud_placeholder: false,
                system_reserved: false,
                hardlink_dup: false,
                mapped: false,
                files: 0,
                dirs: 0,
                children: Vec::new(),
            };
            if let Ok(rd) = std::fs::read_dir(&p) {
                for ent in rd.flatten().take(5_000) {
                    let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let cname = ent.file_name().to_string_lossy().to_string();
                    let mut c = me.clone_shallow();
                    c.path = if is_dir { join(&p, &cname) } else { String::new() };
                    c.name = cname;
                    c.kind = if is_dir { NodeKind::Dir } else { NodeKind::File };
                    me.children.push(c);
                }
            }
            me
        })
        .collect()
}

/// 这个目录的子树在内存里是不是完整的（没被 JSON 导出截断）。
///
/// 扫描出来的树满足：`dirs` = 子树里的目录数，`files` = 各子项 `files` 之和（Rest 节点记被合并的个数）。
/// 截断过的树（`--json-depth`）不满足。无权限、链接、目录映射本来就不展开，算完整。
pub fn subtree_complete(n: &Node) -> bool {
    if n.kind != NodeKind::Dir || n.inaccessible || n.reparse_skipped || n.mapped {
        return true;
    }
    let mut dirs = 0u64;
    let mut files = 0u64;
    for c in &n.children {
        files += c.files;
        if c.kind == NodeKind::Dir {
            dirs += 1 + c.dirs;
        }
    }
    dirs == n.dirs && files == n.files && n.children.iter().all(subtree_complete)
}

/// 解释一个节点：祖先链由调用方给（可以包含 [`outer_ancestors`] 造出来的扫描根以上的各层）。
/// `llm` 见 [`explain::explain_full`]：不传就不联网。
pub fn explain_at(node: &Node, path: &str, ancestors: &[&Node], ctx: &ExplainCtx, llm: Option<&dyn explain::llm::LlmOracle>) -> Explanation {
    let t = Target { node, path, parent: ancestors.last().copied(), ancestors };
    explain::explain_full(&t, ctx, llm)
}
