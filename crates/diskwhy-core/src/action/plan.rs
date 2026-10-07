//! 清理清单 → 计划：重新解释每一项、过闸（档位 / 保护路径 / 规则一致性）、枚举实际内容（阶段 G：快速部分和枚举分开，枚举并行、不打开文件）。

use super::preflight::{node_gate, rule_consistent, tier_gate};
use super::protect::Protector;
use super::winops;
use super::{new_id, Decision, Entry, EntryKind, Plan, PlanItem, PlanRequest, Progress, Strategy, MAX_ENTRIES_PER_ITEM};
use crate::explain::{self, ExplainCtx, Source, Target, Tier};
use crate::fs::{attrs, cluster_bytes, join, reparse};
use crate::model::{NodeKind, ScanResult};
use crate::view;
use std::collections::HashMap;

/// 解释来源的中文说明。
pub fn source_label(s: &Source) -> String {
    match s {
        Source::Rule { id, inherited_from: Some(f), .. } => format!("规则库：{id}（按上层文件夹「{f}」的规则）"),
        Source::Rule { id, group, .. } => format!("规则库：{id}（分组 {group}）"),
        Source::Heuristic { name } => format!("启发式推测：{name}"),
        Source::Llm { model } => format!("AI 推测（模型 {model}）"),
        Source::Unknown => "未知".into(),
    }
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// 生成计划（快速部分 + 枚举）。被拦下的项保留在计划里（带原因，界面上列出来），但不枚举内容、不会执行。
pub fn build_plan(result: &ScanResult, ctx: &ExplainCtx, protector: &Protector, reqs: &[PlanRequest], strategy: Strategy) -> Plan {
    let mut plan = build_plan_quick(result, ctx, protector, reqs, strategy);
    enumerate_plan(&mut plan, &Progress::default());
    plan
}

/// 计划的快速部分（阶段 G，本机 911 项约 0.1 秒）：去重合并、每项重新解释、过闸（保护路径 / 节点 / 档位 /
/// 规则一致性）。**不枚举磁盘内容**，`entries` 为空；之后由 [`enumerate_plan`] 填。确认弹层先用它的结果
/// （扫描时的大小）立刻出现，枚举和预检在后台做完再更新。
pub fn build_plan_quick(result: &ScanResult, ctx: &ExplainCtx, protector: &Protector, reqs: &[PlanRequest], strategy: Strategy) -> Plan {
    let mut warnings = Vec::new();
    // 去重 + 去掉被别的项包含的项（删父目录内容时子项自然包含在内）。
    let mut paths: Vec<&PlanRequest> = Vec::new();
    for r in reqs {
        let p = crate::scan::normalize_root(&r.path);
        if paths.iter().any(|q| lower(&crate::scan::normalize_root(&q.path)) == lower(&p)) {
            continue;
        }
        paths.push(r);
    }
    let norm: Vec<String> = paths.iter().map(|r| crate::scan::normalize_root(&r.path)).collect();
    let mut keep = vec![true; paths.len()];
    for i in 0..paths.len() {
        for j in 0..paths.len() {
            if i != j && keep[j] {
                let (a, b) = (lower(&norm[i]), lower(&norm[j]));
                let parent = if b.ends_with('\\') { b.clone() } else { format!("{b}\\") };
                if a.starts_with(&parent) {
                    keep[i] = false;
                    warnings.push(format!("「{}」已经包含在「{}」里，合并为一项", norm[i], norm[j]));
                    break;
                }
            }
        }
    }
    let mut cluster_cache: HashMap<String, (u64, bool)> = HashMap::new();
    let mut items = Vec::new();
    for (i, r) in paths.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        items.push(build_item(result, ctx, protector, r, &norm[i], &mut cluster_cache));
    }
    Plan {
        id: new_id("plan-"),
        created_at: explain::now_unix(),
        scan_root: result.root.clone(),
        strategy,
        items,
        warnings,
        preflight_done: false,
        safe_direct: true,
    }
}

fn blocked_item(path: &str, reason: String) -> PlanItem {
    PlanItem {
        path: path.to_string(),
        is_dir: false,
        scan_allocated: 0,
        tier: Tier::Review,
        unknown: true,
        source: Source::Unknown,
        source_label: "未知".into(),
        rule_id: None,
        what: String::new(),
        owner: None,
        lose: String::new(),
        how: Vec::new(),
        blocked: Some(reason),
        entries: Vec::new(),
        decisions: Vec::new(),
        dirs: Vec::new(),
        enum_errors: Vec::new(),
        cluster: 4096,
        ntfs: false,
    }
}

fn build_item(
    result: &ScanResult,
    ctx: &ExplainCtx,
    protector: &Protector,
    req: &PlanRequest,
    path: &str,
    cluster_cache: &mut HashMap<String, (u64, bool)>,
) -> PlanItem {
    // 1. 保护路径（最先查，连解释都不用做）
    if let Some(why) = protector.check(path) {
        return blocked_item(path, why);
    }
    // 2. 必须在当前扫描结果里（解释要用扫描树）
    let Some((chain, node)) = view::find_node(&result.tree, path) else {
        return blocked_item(path, "在当前扫描结果里找不到这一项（可能已经被删或者没扫到），请重新扫描后再加入".into());
    };
    let full = if node.kind == NodeKind::Dir { node.path.clone() } else { join(&chain.last().map(|p| p.path.clone()).unwrap_or_default(), &node.name) };
    let t = Target { node, path: &full, parent: chain.last().copied(), ancestors: &chain };
    let e = explain::explain(&t, ctx);
    let rule_id = match &e.source {
        Source::Rule { id, .. } => Some(id.clone()),
        _ => None,
    };
    let mut item = PlanItem {
        path: full.clone(),
        is_dir: node.kind == NodeKind::Dir,
        scan_allocated: node.allocated,
        tier: e.tier,
        unknown: e.is_unknown(),
        source: e.source.clone(),
        source_label: source_label(&e.source),
        rule_id: rule_id.clone(),
        what: e.what.clone(),
        owner: e.owner.clone(),
        lose: e.lose.clone(),
        how: e.how.clone(),
        blocked: None,
        entries: Vec::new(),
        decisions: Vec::new(),
        dirs: Vec::new(),
        enum_errors: Vec::new(),
        cluster: 4096,
        ntfs: false,
    };
    // 3. 节点本身（系统保留 / 映射 / 无权限 / 云占位 / 其余小文件）
    if let Some(why) = node_gate(node) {
        item.blocked = Some(why);
        return item;
    }
    // 4. 档位门槛
    if let Some(why) = tier_gate(&e) {
        item.blocked = Some(why);
        return item;
    }
    // 5. 规则一致性
    if let Err(why) = rule_consistent(ctx, &e.source, &full, node, req.expect_rule.as_deref()) {
        item.blocked = Some(why);
        return item;
    }
    let vol = crate::scan::volume_root_of(&full).unwrap_or_default();
    (item.cluster, item.ntfs) =
        *cluster_cache.entry(vol.clone()).or_insert_with(|| crate::win::volume_space(&vol).map(|v| (v.cluster_size, v.ntfs)).unwrap_or((4096, false)));
    item
}

fn is_cloud(a: u32, tag: u32) -> bool {
    a & attrs::CLOUD_PLACEHOLDER_MASK != 0 || (a & attrs::REPARSE_POINT != 0 && reparse::is_cloud(tag))
}

/// 由目录项（FindFirstFileExW 的结果）生成一条。**这里不打开任何文件**：指纹（文件 ID / 大小 / 修改时间）
/// 在预检里和占用探测用同一次打开取（阶段 G），云占位 / 云文件永远不打开。
fn make_entry(path: String, a: u32, tag: u32, size: u64, mtime: i64, cluster: u64, ntfs: bool) -> Entry {
    if is_cloud(a, tag) {
        return Entry { path, kind: EntryKind::File, attrs: a, reparse_tag: tag, size, bytes: 0, mtime, fp: None, link_is_dir: false };
    }
    // 只有“名字代理”类 reparse（符号链接 / 联接 / 挂载点，IsReparseTagNameSurrogate：bit 29）
    // 或目录型 reparse 才当链接；WOF 压缩、重复数据删除等其他 reparse 文件就是普通文件。
    if a & attrs::REPARSE_POINT != 0 && (tag & 0x2000_0000 != 0 || a & attrs::DIRECTORY != 0) {
        return Entry { path, kind: EntryKind::Link, attrs: a, reparse_tag: tag, size, bytes: 0, mtime, fp: None, link_is_dir: a & attrs::DIRECTORY != 0 };
    }
    // 预检取到指纹后按真实磁盘占用 / 硬链接数重算；这里先按逻辑大小估（和扫描同一条规则：常驻 MFT 的小文件按 0）。
    let plain = a & (attrs::COMPRESSED | attrs::SPARSE_FILE | attrs::REPARSE_POINT) == 0;
    let bytes = cluster_bytes(size, None, plain, crate::fs::name_units(&path), cluster, ntfs);
    Entry { path, kind: EntryKind::File, attrs: a, reparse_tag: tag, size, bytes, mtime, fp: None, link_is_dir: false }
}

/// 枚举每个没被拦下的清单项在磁盘上的实际内容：文件、链接（不跟随）、云目录（不进去）、子目录。
///
/// 阶段 G：所有清单项的目录一起按层并行列（rayon），每层列完再合并；不打开文件（指纹在预检里取）。
/// 本机默认勾选 6.5 万个文件夹，单线程逐个列要 7.6 秒。取消时尽快返回（计划不完整，之后不会被执行）。
pub fn enumerate_plan(plan: &mut Plan, prog: &Progress) {
    use rayon::prelude::*;
    use std::sync::atomic::Ordering::Relaxed;
    prog.set_phase(1);
    // (清单项序号, 目录, 深度)
    let mut frontier: Vec<(usize, String, usize)> = Vec::new();
    let mut raw_dirs: Vec<Vec<(String, usize)>> = vec![Vec::new(); plan.items.len()];
    let mut overflow = vec![false; plan.items.len()];
    for (ii, item) in plan.items.iter_mut().enumerate() {
        if item.blocked.is_some() || !item.entries.is_empty() {
            continue;
        }
        let (cluster, ntfs) = (item.cluster, item.ntfs);
        if !item.is_dir {
            match winops::stat_entry(&item.path) {
                Ok(r) => item.entries.push(make_entry(item.path.clone(), r.attrs, r.reparse_tag, r.size, r.mtime, cluster, ntfs)),
                Err(code) => item.enum_errors.push(format!("读不到「{}」（Win32 错误 {code}）", item.path)),
            }
            continue;
        }
        // 清单项本身如果是链接（联接 / 符号链接），就只处理链接自身。
        if let Ok(me) = winops::stat_entry(&item.path) {
            if me.attrs & attrs::REPARSE_POINT != 0 && !reparse::is_cloud(me.reparse_tag) {
                item.entries.push(Entry {
                    path: item.path.clone(),
                    kind: EntryKind::Link,
                    attrs: me.attrs,
                    reparse_tag: me.reparse_tag,
                    size: 0,
                    bytes: 0,
                    mtime: me.mtime,
                    fp: None,
                    link_is_dir: true,
                });
                continue;
            }
        }
        frontier.push((ii, item.path.clone(), 0));
    }
    while !frontier.is_empty() {
        if prog.cancelled() {
            return;
        }
        let listed: Vec<_> = frontier
            .par_iter()
            .map(|(ii, dir, depth)| {
                let r = winops::list_dir(dir);
                prog.dirs.fetch_add(1, Relaxed);
                if let Ok(l) = &r {
                    prog.entries.fetch_add(l.len(), Relaxed);
                }
                (*ii, dir, *depth, r)
            })
            .collect();
        let mut next = Vec::new();
        for (ii, dir, depth, r) in listed {
            if overflow[ii] {
                continue;
            }
            let item = &mut plan.items[ii];
            let list = match r {
                Ok(l) => l,
                Err(code) => {
                    item.enum_errors.push(format!("列不出「{dir}」（Win32 错误 {code}），里面的东西不会动"));
                    continue;
                }
            };
            let (cluster, ntfs) = (item.cluster, item.ntfs);
            for r in list {
                let p = join(dir, &r.name);
                let a = r.attrs;
                if a & attrs::DIRECTORY != 0 {
                    if is_cloud(a, r.reparse_tag) {
                        item.entries.push(Entry {
                            path: p,
                            kind: EntryKind::CloudDir,
                            attrs: a,
                            reparse_tag: r.reparse_tag,
                            size: 0,
                            bytes: 0,
                            mtime: r.mtime,
                            fp: None,
                            link_is_dir: true,
                        });
                    } else if a & attrs::REPARSE_POINT != 0 {
                        item.entries.push(make_entry(p, a, r.reparse_tag, 0, r.mtime, cluster, ntfs));
                    } else {
                        raw_dirs[ii].push((p.clone(), depth + 1));
                        next.push((ii, p, depth + 1));
                    }
                } else {
                    item.entries.push(make_entry(p, a, r.reparse_tag, r.size, r.mtime, cluster, ntfs));
                }
            }
            if item.entries.len() > MAX_ENTRIES_PER_ITEM {
                overflow[ii] = true;
            }
        }
        next.retain(|(ii, _, _)| !overflow[*ii]);
        frontier = next;
    }
    for (ii, item) in plan.items.iter_mut().enumerate() {
        if overflow[ii] || item.entries.len() > MAX_ENTRIES_PER_ITEM {
            item.blocked = Some(format!("这一项里有 {} 个以上的文件，太多了，请拆成更小的文件夹分批清理", MAX_ENTRIES_PER_ITEM));
            item.entries.clear();
            item.dirs.clear();
            item.decisions.clear();
            continue;
        }
        if item.blocked.is_some() {
            continue;
        }
        let mut dirs = std::mem::take(&mut raw_dirs[ii]);
        dirs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        item.dirs = dirs.into_iter().map(|d| d.0).collect();
        item.entries.sort_by_cached_key(|e| e.path.to_lowercase());
        item.decisions = vec![Decision::Pending; item.entries.len()];
    }
}
