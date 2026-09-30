//! 预检：9 条护栏。
//!
//! | # | 护栏 | 在哪 | 结果 |
//! |---|---|---|---|
//! | 1 | 档位门槛：只有 safe / review 可进清单；keep、未知（Unknown）、AI 推测（Llm）一律拒绝；`how` 里没有“删除”的也拒绝（给出该用的工具） | [`tier_gate`] | 整项拒绝 |
//! | 2 | 保护路径硬编码（本身 / 里面 / 祖先） | [`super::protect`] | 整项拒绝；条目级命中则跳过该条 |
//! | 3 | 规则一致性：路径必须被该规则的 match 命中（或在 tree 规则命中的文件夹里面），不许删规则目录的父目录；加入清单时的规则和现在算出来的必须一样 | [`rule_consistent`] | 整项拒绝 |
//! | 4 | 热文件：30 分钟内修改过的跳过并计数 | [`preflight`] | 跳过该条 |
//! | 5 | 云占位 / 云文件：跳过并说明（不打开） | [`preflight`] | 跳过该条 |
//! | 6 | reparse point：不跟随，只删链接自身，预览里标出 | [`preflight`] | `RemoveLink` |
//! | 7 | 占用检测：DELETE 权限探测 + Restart Manager 报告进程名和 PID，只报告不关进程 | [`preflight`] | 跳过该条（用户可选“跳过被占用的”继续，或取消） |
//! | 8 | 空间提示：回收站策略且所在卷可用空间 < 10% | [`preflight`] | 警告 + 可切换到隔离区 |
//! | 9 | 重新校验：执行前每条再取一次文件 ID / 大小 / 修改时间 | [`super::execute`] | 对不上的跳过 |

use super::protect::Protector;
use super::winops::{self, LockingProcess};
use super::{Decision, EntryKind, EntryView, Plan, SkipReason, Strategy, HOT_WINDOW_SECS, LOW_SPACE_RATIO, PREVIEW_LIMIT};
use crate::explain::{ExplainCtx, Explanation, How, Source, Tier};
use crate::explain::pattern::split_lower;
use crate::explain::rules::Scope;
use crate::model::{Node, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 节点本身不能删的情况。
pub fn node_gate(node: &Node) -> Option<String> {
    if node.kind == NodeKind::Rest {
        return Some("“其余小文件合计”不是一个真实的文件夹，请选具体的文件夹".into());
    }
    if node.system_reserved {
        return Some("系统保留项（回收站 / 卷影 / 页面文件等），请用系统工具处理".into());
    }
    if node.mapped {
        return Some("这是商店应用的目录映射（和另一个路径是同一份数据），请到“真正记账的位置”去操作".into());
    }
    if node.inaccessible {
        return Some("这个文件夹没有权限读取，DiskWhy 不知道里面有什么，不会删".into());
    }
    if node.cloud_placeholder {
        return Some("云端占位文件：删了可能连云端的一起删，DiskWhy 不碰".into());
    }
    None
}

/// 护栏 1：档位门槛。
pub fn tier_gate(e: &Explanation) -> Option<String> {
    match &e.source {
        Source::Unknown => return Some("我还不知道这是什么：先搞清楚它是什么再说，未知的东西一律不能加入清理清单".into()),
        Source::Llm { .. } => return Some("这个说明来自 AI 推测，不能作为删除依据".into()),
        _ => {}
    }
    match e.tier {
        Tier::Keep => Some(format!("“建议保留”的东西不能删（{}）。想腾空间请看“该怎么腾”：迁移或用系统工具", e.what)),
        Tier::Safe | Tier::Review if !e.how.contains(&How::Delete) => {
            let how: Vec<&str> = e
                .how
                .iter()
                .map(|h| match h {
                    How::BuiltinTool => "用系统或软件自带的工具清理",
                    How::Migrate => "迁到别的盘",
                    How::Manual => "手动处理",
                    How::None => "不需要处理",
                    How::Delete => "直接删除",
                })
                .collect();
            Some(format!("这一项不建议直接删，应该{}。见详情里的“该怎么腾”", how.join("或")))
        }
        _ => None,
    }
}

/// 护栏 3：规则一致性。
///
/// - 解释来自规则时：路径（精确命中）或 `inherited_from`（继承命中）必须真的被这条规则的某个 match 命中；
///   继承时这条规则必须是 `scope = tree`，而且路径确实在那个文件夹里面。这样规则目录的父目录永远过不了
///   （父目录要么命中别的规则，要么没有规则）。
/// - `expect_rule`：加入清单时看到的规则 id，和现在算出来的不一样就拒绝（说明变了，让用户重新看一眼）。
pub fn rule_consistent(ctx: &ExplainCtx, source: &Source, path: &str, node: &Node, expect_rule: Option<&str>) -> Result<(), String> {
    let (id, inherited_from) = match source {
        Source::Rule { id, inherited_from, .. } => (id.as_str(), inherited_from.as_deref()),
        Source::Heuristic { .. } => {
            return match expect_rule {
                Some(x) => Err(format!("加入清单时这项由规则「{x}」说明，现在变成了启发式推测，请重新查看后再加入")),
                None => Ok(()),
            };
        }
        _ => return Err("没有规则或启发式依据".into()),
    };
    if let Some(x) = expect_rule {
        if x != id {
            return Err(format!("加入清单时这项由规则「{x}」说明，现在变成了「{id}」，请重新查看后再加入"));
        }
    }
    let Some(ri) = ctx.rules.rules.iter().position(|r| r.id == id) else {
        return Err(format!("规则「{id}」不在规则库里"));
    };
    let rule = &ctx.rules.rules[ri];
    let pats: Vec<_> = ctx.rules.patterns.iter().filter(|(i, _)| *i == ri).map(|(_, p)| p).collect();
    let segs = split_lower(path);
    match inherited_from {
        None => {
            if pats.iter().any(|p| p.matches(&segs)) {
                Ok(())
            } else {
                Err(format!("路径「{path}」和规则「{id}」的匹配范围对不上，拒绝"))
            }
        }
        Some(from) => {
            let from_segs = split_lower(from);
            let inside = segs.len() > from_segs.len() && segs[..from_segs.len()] == from_segs[..];
            if rule.scope != Scope::Tree {
                return Err(format!("规则「{id}」只说明文件夹本身，不管里面的东西"));
            }
            if !inside {
                return Err(format!("「{path}」不在规则「{id}」命中的文件夹「{from}」里面，拒绝（不允许删规则目录的父目录）"));
            }
            if !pats.iter().any(|p| p.matches(&from_segs)) {
                return Err(format!("「{from}」和规则「{id}」的匹配范围对不上，拒绝"));
            }
            let _ = node;
            Ok(())
        }
    }
}

/// 预检选项。
#[derive(Debug, Clone)]
pub struct PreflightOptions {
    /// 当前时间（Unix 秒），测试可固定。
    pub now: i64,
    pub hot_window_secs: i64,
    /// 被占用的文件不超过这么多个时，逐个问 Restart Manager 以便精确到文件；否则成组问一次。
    pub rm_per_file_limit: usize,
    /// 成组问 Restart Manager 最多带多少个文件。
    pub rm_group_limit: usize,
}

impl Default for PreflightOptions {
    fn default() -> Self {
        PreflightOptions { now: crate::explain::now_unix(), hot_window_secs: HOT_WINDOW_SECS, rm_per_file_limit: 20, rm_group_limit: 1000 }
    }
}

/// 预览里的一条跳过项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkipView {
    pub path: String,
    pub kind: EntryKind,
    pub bytes: u64,
    pub code: String,
    pub reason: String,
    pub processes: Vec<LockingProcess>,
}

/// 预检结果里的一项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemReport {
    pub path: String,
    pub is_dir: bool,
    pub tier: Tier,
    pub unknown: bool,
    pub source_label: String,
    pub rule_id: Option<String>,
    pub what: String,
    pub owner: Option<String>,
    pub lose: String,
    pub scan_allocated: u64,
    pub blocked: Option<String>,
    /// 前 200 条实际会删的（含链接）。
    pub will_delete: Vec<EntryView>,
    pub will_delete_count: usize,
    pub will_delete_bytes: u64,
    /// 其中链接的个数（只删链接本身）。
    pub link_count: usize,
    /// 前 200 条跳过的。
    pub skipped: Vec<SkipView>,
    pub skipped_count: usize,
    pub skipped_bytes: u64,
    /// 各跳过原因的个数（hot / cloud / locked / …）。
    pub skipped_by_reason: BTreeMap<String, usize>,
    /// 会删掉的空子文件夹个数（清单项本身的文件夹保留）。
    pub dirs_to_remove: usize,
    pub enum_errors: Vec<String>,
}

/// 警告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    /// `recycle_low_space` / `quarantine_same_volume` / `quarantine_no_space` / `locked` / `hot` …
    pub code: String,
    pub message: String,
    /// 这个警告是否阻止执行（例如隔离区空间不够）。
    pub blocking: bool,
}

/// 可以作为隔离区的盘。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveChoice {
    pub root: String,
    pub label: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

/// 预检报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightReport {
    pub plan_id: String,
    pub strategy: Strategy,
    pub items: Vec<ItemReport>,
    pub total_entries: usize,
    pub total_bytes: u64,
    pub total_skipped: usize,
    pub warnings: Vec<Warning>,
    /// 所有占用文件的进程（去重）。
    pub locked_processes: Vec<LockingProcess>,
    pub locked_count: usize,
    pub hot_count: usize,
    pub cloud_count: usize,
    pub link_count: usize,
    /// review 档的项（确认区单独再列一遍“删了有代价”）。
    pub review_items: Vec<ReviewNote>,
    /// 至少有一条能删、并且没有阻止执行的警告。
    pub can_execute: bool,
    /// 可选的隔离区盘（源所在盘以外的固定盘）。
    pub quarantine_choices: Vec<DriveChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewNote {
    pub path: String,
    pub what: String,
    pub lose: String,
    pub bytes: u64,
}

/// 预检：逐条给出 删 / 只删链接 / 跳过（原因），并汇总警告。会修改 `plan` 里的 `decisions`。
pub fn preflight(plan: &mut Plan, protector: &Protector, opts: &PreflightOptions) -> PreflightReport {
    let mut locked_refs: Vec<(usize, usize)> = Vec::new();
    for (ii, item) in plan.items.iter_mut().enumerate() {
        if item.blocked.is_none() {
            // 保护路径再查一次（纵深防御：清单可能是很久以前生成的）
            if let Some(why) = protector.check(&item.path) {
                item.blocked = Some(why);
            }
        }
        if item.blocked.is_some() {
            item.decisions = vec![Decision::Skip { reason: SkipReason::Protected { why: "整项被拦下".into() } }; item.entries.len()];
            continue;
        }
        for (ei, e) in item.entries.iter().enumerate() {
            let d = decide(e, protector, opts);
            if matches!(d, Decision::Skip { reason: SkipReason::Locked { .. } }) {
                locked_refs.push((ii, ei));
            }
            item.decisions[ei] = d;
        }
    }
    // 护栏 7：被占用的文件问 Restart Manager 是谁在用（只报告）。
    let mut all_procs: Vec<LockingProcess> = Vec::new();
    if !locked_refs.is_empty() {
        let paths: Vec<String> = locked_refs.iter().map(|&(i, e)| plan.items[i].entries[e].path.clone()).collect();
        if locked_refs.len() <= opts.rm_per_file_limit {
            for (k, &(i, e)) in locked_refs.iter().enumerate() {
                let procs = winops::rm_locking_processes(std::slice::from_ref(&paths[k])).unwrap_or_default();
                all_procs.extend(procs.iter().cloned());
                plan.items[i].decisions[e] = Decision::Skip { reason: SkipReason::Locked { processes: procs } };
            }
        } else {
            let group: Vec<String> = paths.iter().take(opts.rm_group_limit).cloned().collect();
            let procs = winops::rm_locking_processes(&group).unwrap_or_default();
            all_procs.extend(procs.iter().cloned());
            for &(i, e) in &locked_refs {
                plan.items[i].decisions[e] = Decision::Skip { reason: SkipReason::Locked { processes: procs.clone() } };
            }
        }
        all_procs.sort_by_key(|p| p.pid);
        all_procs.dedup_by_key(|p| p.pid);
    }
    plan.preflight_done = true;
    report(plan, all_procs)
}

fn decide(e: &super::Entry, protector: &Protector, opts: &PreflightOptions) -> Decision {
    // 护栏 2（条目级）：文件夹里面的 .ssh 之类
    if let Some(why) = protector.entry_blocked(&e.path) {
        return Decision::Skip { reason: SkipReason::Protected { why } };
    }
    // 护栏 5：云
    if e.kind == EntryKind::CloudDir || (e.kind == EntryKind::File && e.fp.is_none() && is_cloud_attrs(e.attrs, e.reparse_tag)) {
        return Decision::Skip { reason: SkipReason::Cloud };
    }
    // 护栏 6：链接只删自身
    if e.kind == EntryKind::Link {
        return match winops::delete_probe(&e.path, e.link_is_dir) {
            Ok(()) => Decision::RemoveLink,
            Err(c) if winops::is_sharing_violation(c) => Decision::Skip { reason: SkipReason::Locked { processes: vec![] } },
            Err(2) | Err(3) => Decision::Skip { reason: SkipReason::Missing },
            Err(c) => Decision::Skip { reason: SkipReason::NoPermission { code: c } },
        };
    }
    let Some(fp) = e.fp else {
        return Decision::Skip { reason: SkipReason::Unreadable { code: 0 } };
    };
    // 护栏 4：热文件
    let mtime = winops::ft_to_unix(fp.mtime_ft);
    let age = opts.now - mtime;
    if age < opts.hot_window_secs {
        return Decision::Skip { reason: SkipReason::Hot { minutes_ago: (age.max(0) / 60) } };
    }
    // 护栏 7：占用探测（DELETE 权限 + 全共享打开，立刻关闭，不改文件）
    match winops::delete_probe(&e.path, false) {
        Ok(()) => Decision::Delete,
        Err(c) if winops::is_sharing_violation(c) => Decision::Skip { reason: SkipReason::Locked { processes: vec![] } },
        Err(2) | Err(3) => Decision::Skip { reason: SkipReason::Missing },
        Err(c) => Decision::Skip { reason: SkipReason::NoPermission { code: c } },
    }
}

/// 隔离区位置的检查：完整路径、不在任何清单项里面、不在 Windows / Program Files 里、能写。
///
/// 能不能写是实际试出来的：建一次 `<dest>\DiskWhy-Quarantine`，原来没有就马上删掉（空文件夹）。
/// 例如很多机器上普通用户不能在 `D:\` 根目录建文件夹（ACL 只给 Everyone 读），这时要换成盘上的某个文件夹。
pub fn quarantine_dest_problem(dest_root: &str, plan: &Plan) -> Option<String> {
    let n = match super::protect::normalize(dest_root) {
        Ok(n) => n,
        Err(e) => return Some(format!("隔离区位置不对：{e}")),
    };
    for it in plan.items.iter().filter(|i| i.blocked.is_none()) {
        let a = it.path.to_lowercase();
        let d = n.path.to_lowercase();
        if d == a || d.starts_with(&format!("{}\\", a.trim_end_matches('\\'))) {
            return Some(format!("隔离区位置 {} 在要清理的「{}」里面，换一个位置", n.path, it.path));
        }
    }
    let s = &n.segs;
    if s.len() >= 2 && (s[1] == "windows" || s[1].starts_with("program files")) {
        return Some("隔离区不能放在 Windows 或 Program Files 里".into());
    }
    let q = crate::fs::join(&n.path, super::quarantine::DIR_NAME);
    let ql = crate::win::long_path(&q);
    let existed = std::path::Path::new(&ql).is_dir();
    match std::fs::create_dir_all(&ql) {
        Ok(()) => {
            if !existed {
                let _ = std::fs::remove_dir(&ql);
            }
            None
        }
        Err(e) => Some(format!(
            "在 {} 里建不了隔离区文件夹（{e}）。普通用户常常不能在盘的根目录（如 D:\\）建文件夹，请在“删到哪里”里选“其他文件夹…”，换成这个盘上你能写的文件夹。",
            n.path
        )),
    }
}

fn is_cloud_attrs(a: u32, tag: u32) -> bool {
    use crate::fs::{attrs, reparse};
    a & attrs::CLOUD_PLACEHOLDER_MASK != 0 || (a & attrs::REPARSE_POINT != 0 && reparse::is_cloud(tag))
}

fn report(plan: &Plan, locked_processes: Vec<LockingProcess>) -> PreflightReport {
    let mut items = Vec::new();
    let (mut total_entries, mut total_bytes, mut total_skipped) = (0usize, 0u64, 0usize);
    let (mut locked_count, mut hot_count, mut cloud_count, mut link_count) = (0, 0, 0, 0);
    let mut review_items = Vec::new();
    for it in &plan.items {
        let mut skipped = Vec::new();
        let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
        let (mut sk_count, mut sk_bytes, mut links) = (0usize, 0u64, 0usize);
        if it.blocked.is_none() {
            for (e, d) in it.entries.iter().zip(&it.decisions) {
                match d {
                    Decision::Skip { reason } => {
                        sk_count += 1;
                        sk_bytes += e.bytes;
                        *by_reason.entry(reason.code().to_string()).or_default() += 1;
                        match reason {
                            SkipReason::Locked { .. } => locked_count += 1,
                            SkipReason::Hot { .. } => hot_count += 1,
                            SkipReason::Cloud => cloud_count += 1,
                            _ => {}
                        }
                        if skipped.len() < PREVIEW_LIMIT {
                            let processes = if let SkipReason::Locked { processes } = reason { processes.clone() } else { vec![] };
                            skipped.push(SkipView {
                                path: e.path.clone(),
                                kind: e.kind,
                                bytes: e.bytes,
                                code: reason.code().into(),
                                reason: reason.text(),
                                processes,
                            });
                        }
                    }
                    Decision::RemoveLink => links += 1,
                    _ => {}
                }
            }
        }
        let wd_count = if it.blocked.is_none() { it.entry_count() } else { 0 };
        let wd_bytes = if it.blocked.is_none() { it.bytes() } else { 0 };
        total_entries += wd_count;
        total_bytes += wd_bytes;
        total_skipped += sk_count;
        link_count += links;
        if it.blocked.is_none() && it.tier == Tier::Review && wd_count > 0 {
            review_items.push(ReviewNote { path: it.path.clone(), what: it.what.clone(), lose: it.lose.clone(), bytes: wd_bytes });
        }
        items.push(ItemReport {
            path: it.path.clone(),
            is_dir: it.is_dir,
            tier: it.tier,
            unknown: it.unknown,
            source_label: it.source_label.clone(),
            rule_id: it.rule_id.clone(),
            what: it.what.clone(),
            owner: it.owner.clone(),
            lose: it.lose.clone(),
            scan_allocated: it.scan_allocated,
            blocked: it.blocked.clone(),
            will_delete: if it.blocked.is_none() { it.entries_preview(PREVIEW_LIMIT) } else { vec![] },
            will_delete_count: wd_count,
            will_delete_bytes: wd_bytes,
            link_count: links,
            skipped,
            skipped_count: sk_count,
            skipped_bytes: sk_bytes,
            skipped_by_reason: by_reason,
            dirs_to_remove: if it.blocked.is_none() { it.dirs.len() } else { 0 },
            enum_errors: it.enum_errors.clone(),
        });
    }

    let mut warnings = Vec::new();
    for w in &plan.warnings {
        warnings.push(Warning { code: "merged".into(), message: w.clone(), blocking: false });
    }
    // 源所在的卷
    let mut src_vols: Vec<String> = plan
        .items
        .iter()
        .filter(|i| i.blocked.is_none() && i.entry_count() > 0)
        .filter_map(|i| crate::scan::volume_root_of(&i.path))
        .collect();
    src_vols.sort();
    src_vols.dedup();
    let drives = crate::win::list_fixed_drives();
    // 护栏 8：空间提示
    match &plan.strategy {
        Strategy::RecycleBin => {
            for v in &src_vols {
                if let Ok(sp) = crate::win::volume_space(v) {
                    if sp.total_bytes > 0 && (sp.free_bytes as f64) < sp.total_bytes as f64 * LOW_SPACE_RATIO {
                        warnings.push(Warning {
                            code: "recycle_low_space".into(),
                            message: format!(
                                "{v} 只剩 {}（不到 10%）。回收站就在同一个盘上：删除后空间不会立刻腾出来，要等你清空回收站才行。想马上腾出空间，可以改成“隔离到其他盘”。",
                                crate::view::human_bytes(sp.free_bytes)
                            ),
                            blocking: false,
                        });
                    }
                }
            }
        }
        Strategy::Quarantine { dest_root } => {
            if let Some(why) = quarantine_dest_problem(dest_root, plan) {
                warnings.push(Warning { code: "quarantine_bad_dest".into(), message: why, blocking: true });
            }
            let dest_vol = crate::scan::volume_root_of(dest_root).unwrap_or_default();
            if src_vols.iter().any(|v| v.eq_ignore_ascii_case(&dest_vol)) {
                warnings.push(Warning {
                    code: "quarantine_same_volume".into(),
                    message: format!("隔离区在 {dest_vol}，和要清理的东西在同一个盘上：只是挪了位置，这个盘不会腾出空间。"),
                    blocking: false,
                });
            }
            match crate::win::volume_space(&dest_vol) {
                Ok(sp) if sp.free_bytes < total_bytes.saturating_add(64 * 1024 * 1024) => warnings.push(Warning {
                    code: "quarantine_no_space".into(),
                    message: format!(
                        "隔离区所在的 {dest_vol} 只剩 {}，放不下这次要隔离的 {}（另留 64 MB 余量）。请换一个盘或改用回收站。",
                        crate::view::human_bytes(sp.free_bytes),
                        crate::view::human_bytes(total_bytes)
                    ),
                    blocking: true,
                }),
                Err(e) => warnings.push(Warning { code: "quarantine_bad_dest".into(), message: format!("隔离区位置不可用：{e}"), blocking: true }),
                _ => {}
            }
        }
    }
    if locked_count > 0 {
        warnings.push(Warning {
            code: "locked".into(),
            message: format!(
                "有 {locked_count} 个文件正在被程序占用{}。DiskWhy 不会关掉任何程序：你可以“跳过被占用的”继续，或者取消、自己关掉程序后重新预览。",
                if locked_processes.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", locked_processes.iter().map(|p| format!("{} PID {}", p.name, p.pid)).collect::<Vec<_>>().join("、"))
                }
            ),
            blocking: false,
        });
    }
    if hot_count > 0 {
        warnings.push(Warning { code: "hot".into(), message: format!("有 {hot_count} 个文件是最近 30 分钟内改过的，可能正在用，已跳过。"), blocking: false });
    }
    if cloud_count > 0 {
        warnings.push(Warning { code: "cloud".into(), message: format!("有 {cloud_count} 个云端文件 / 占位文件（OneDrive 等），已跳过：删了可能连云端的一起删。"), blocking: false });
    }
    let quarantine_choices = drives
        .iter()
        .filter(|d| !src_vols.iter().any(|v| v.eq_ignore_ascii_case(&d.root)))
        .map(|d| DriveChoice { root: d.root.clone(), label: d.label.clone(), free_bytes: d.free_bytes, total_bytes: d.total_bytes })
        .collect();
    let can_execute = total_entries > 0 && !warnings.iter().any(|w| w.blocking);
    PreflightReport {
        plan_id: plan.id.clone(),
        strategy: plan.strategy.clone(),
        items,
        total_entries,
        total_bytes,
        total_skipped,
        warnings,
        locked_processes,
        locked_count,
        hot_count,
        cloud_count,
        link_count,
        review_items,
        can_execute,
        quarantine_choices,
    }
}
