//! 清理清单 → 计划：重新解释每一项、过闸（档位 / 保护路径 / 规则一致性）、枚举实际内容。

use super::preflight::{node_gate, rule_consistent, tier_gate};
use super::protect::Protector;
use super::winops;
use super::{new_id, Decision, Entry, EntryKind, Plan, PlanItem, PlanRequest, Strategy, MAX_ENTRIES_PER_ITEM};
use crate::explain::{self, ExplainCtx, Source, Target, Tier};
use crate::fs::{attrs, join, reparse, round_up};
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

/// 生成计划。被拦下的项保留在计划里（带原因，界面上列出来），但不枚举内容、不会执行。
pub fn build_plan(result: &ScanResult, ctx: &ExplainCtx, protector: &Protector, reqs: &[PlanRequest], strategy: Strategy) -> Plan {
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
    let mut cluster_cache: HashMap<String, u64> = HashMap::new();
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
    }
}

fn build_item(
    result: &ScanResult,
    ctx: &ExplainCtx,
    protector: &Protector,
    req: &PlanRequest,
    path: &str,
    cluster_cache: &mut HashMap<String, u64>,
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
    // 6. 枚举实际内容
    let vol = crate::scan::volume_root_of(&full).unwrap_or_default();
    let cluster = *cluster_cache.entry(vol.clone()).or_insert_with(|| crate::win::volume_space(&vol).map(|v| v.cluster_size).unwrap_or(4096));
    enumerate(&mut item, cluster);
    if item.entries.len() > MAX_ENTRIES_PER_ITEM {
        item.blocked = Some(format!("这一项里有 {} 个以上的文件，太多了，请拆成更小的文件夹分批清理", MAX_ENTRIES_PER_ITEM));
        item.entries.clear();
        item.dirs.clear();
    }
    item.decisions = vec![Decision::Pending; item.entries.len()];
    item
}

fn is_cloud(a: u32, tag: u32) -> bool {
    a & attrs::CLOUD_PLACEHOLDER_MASK != 0 || (a & attrs::REPARSE_POINT != 0 && reparse::is_cloud(tag))
}

/// 枚举一个清单项在磁盘上的实际内容：文件（带指纹）、链接（不跟随）、云目录（不进去）、子目录。
fn enumerate(item: &mut PlanItem, cluster: u64) {
    let make_file = |path: String, a: u32, tag: u32, size: u64, mtime: i64| -> Entry {
        if is_cloud(a, tag) {
            // 云占位 / 云文件：只看目录项属性，绝不打开
            return Entry { path, kind: EntryKind::File, attrs: a, reparse_tag: tag, size, bytes: 0, mtime, fp: None, link_is_dir: false };
        }
        // 只有“名字代理”类 reparse（符号链接 / 联接 / 挂载点，IsReparseTagNameSurrogate：bit 29）
        // 或目录型 reparse 才当链接；WOF 压缩、重复数据删除等其他 reparse 文件就是普通文件。
        if a & attrs::REPARSE_POINT != 0 && (tag & 0x2000_0000 != 0 || a & attrs::DIRECTORY != 0) {
            let fp = winops::fingerprint(&path, true).ok();
            return Entry { path, kind: EntryKind::Link, attrs: a, reparse_tag: tag, size, bytes: 0, mtime, fp, link_is_dir: a & attrs::DIRECTORY != 0 };
        }
        let fp = winops::fingerprint(&path, true).ok();
        let bytes = match fp {
            Some(f) if f.links > 1 => 0,
            Some(f) => round_up(f.on_disk, cluster),
            None => round_up(size, cluster),
        };
        Entry { path, kind: EntryKind::File, attrs: a, reparse_tag: tag, size, bytes, mtime, fp, link_is_dir: false }
    };

    if !item.is_dir {
        match winops::stat_entry(&item.path) {
            Ok(r) => item.entries.push(make_file(item.path.clone(), r.attrs, r.reparse_tag, r.size, r.mtime)),
            Err(code) => item.enum_errors.push(format!("读不到「{}」（Win32 错误 {code}）", item.path)),
        }
        return;
    }
    // 清单项本身如果是链接（联接 / 符号链接），就只处理链接自身。
    if let Ok(me) = winops::stat_entry(&item.path) {
        if me.attrs & attrs::REPARSE_POINT != 0 && !reparse::is_cloud(me.reparse_tag) {
            let fp = winops::fingerprint(&item.path, true).ok();
            item.entries.push(Entry {
                path: item.path.clone(),
                kind: EntryKind::Link,
                attrs: me.attrs,
                reparse_tag: me.reparse_tag,
                size: 0,
                bytes: 0,
                mtime: me.mtime,
                fp,
                link_is_dir: true,
            });
            return;
        }
    }
    let mut stack: Vec<(String, usize)> = vec![(item.path.clone(), 0)];
    let mut dirs: Vec<(String, usize)> = Vec::new();
    while let Some((dir, depth)) = stack.pop() {
        if item.entries.len() > MAX_ENTRIES_PER_ITEM {
            return;
        }
        let list = match winops::list_dir(&dir) {
            Ok(l) => l,
            Err(code) => {
                item.enum_errors.push(format!("列不出「{dir}」（Win32 错误 {code}），里面的东西不会动"));
                continue;
            }
        };
        for r in list {
            let p = join(&dir, &r.name);
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
                    item.entries.push(make_file(p, a, r.reparse_tag, 0, r.mtime));
                } else {
                    dirs.push((p.clone(), depth + 1));
                    stack.push((p, depth + 1));
                }
            } else {
                item.entries.push(make_file(p, a, r.reparse_tag, r.size, r.mtime));
            }
        }
    }
    dirs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    item.dirs = dirs.into_iter().map(|d| d.0).collect();
    item.entries.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
}
