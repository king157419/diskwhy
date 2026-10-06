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
//! | 8 | 空间提示：有东西要进回收站且所在卷可用空间 < 10%（阶段 J：另查回收站上限 MaxCapacity） | [`preflight`] | 警告 + 可切换到隔离区 |
//! | 9 | 重新校验：执行前每条再取一次文件 ID / 大小 / 修改时间 | [`super::execute`] | 对不上的跳过 |

use super::protect::{ParentCache, Protector};
use super::winops::{self, Fingerprint, LockingProcess};
use super::perf::{PerfModel, Workload};
use super::{ByDest, CountBytes, Decision, Dest, EntryKind, EntryView, Plan, Progress, SkipReason, Strategy, HOT_WINDOW_SECS, LOW_SPACE_RATIO, PREVIEW_LIMIT};
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
    /// 问 Restart Manager 时每组最多几个文件（同一清单项里的被占用文件按这个大小分组，每组一个会话）。
    ///
    /// 阶段 G 实测（本机 259 个被占用文件、18 个清单项）：RmGetList 的耗时基本和“这一组查到几个进程”成正比
    /// （每个进程约 0.2 秒），259 个一组 25 秒；按清单项分、每组 ≤16 个、8 个会话并发 6.7 秒。
    pub rm_chunk: usize,
    /// 同时开几个 Restart Manager 会话（文档：每个用户会话最多 64 个，别的程序也可能在用，留足余量）。
    pub rm_sessions: usize,
    /// 最多把多少个被占用的文件交给 Restart Manager；超过的只标“被占用”、不查进程（防止极端情况下查几分钟）。
    pub rm_max_files: usize,
    /// 阶段 J：估计用时用的耗时模型（None = 读 `%LOCALAPPDATA%\DiskWhy\perf.json`，没有就用默认值）。
    pub perf: Option<PerfModel>,
}

impl Default for PreflightOptions {
    fn default() -> Self {
        PreflightOptions { now: crate::explain::now_unix(), hot_window_secs: HOT_WINDOW_SECS, rm_chunk: 16, rm_sessions: 6, rm_max_files: 2048, perf: None }
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
    /// 阶段 J：这个盘上已经能用的暂存区位置（`Strategy::Quarantine { dest_root }` 直接用它）；
    /// None = 盘根不让建文件夹，要先调 `prepare_quarantine` / `elevate_quarantine` 或让用户选文件夹。
    #[serde(default)]
    pub dest_root: Option<String>,
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
    /// 阶段 G：true = 这只是弹层先用的估算（扫描时的大小），逐个文件的检查还在后台进行，不能执行。
    #[serde(default)]
    pub pending: bool,
    /// 各阶段用时（毫秒），排查慢在哪里用。
    #[serde(default)]
    pub timing_ms: BTreeMap<String, u64>,
    /// 阶段 J：safe 档是否直接删除（见 `Plan::safe_direct`）。
    #[serde(default)]
    pub safe_direct: bool,
    /// 阶段 J：预计用时（秒，按 perf.json 的耗时模型；`pending` 时为 0 = 还不知道）。
    #[serde(default)]
    pub est_secs: f64,
    /// 阶段 J：计划按去向分别多少个 / 多少字节（`pending` 时个数为 0、字节按扫描时的大小）。
    #[serde(default)]
    pub by_dest: ByDest,
    /// 阶段 J：跳过的总字节，和按原因（hot / cloud / locked / hardlink / …）分的个数 + 字节。
    #[serde(default)]
    pub skipped_bytes: u64,
    #[serde(default)]
    pub skipped_by_reason: BTreeMap<String, CountBytes>,
    /// 阶段 J：扫描时的估算总量（= 弹层先显示的 quote 的 `total_bytes`），界面用它解释“数字为什么变小了”。
    #[serde(default)]
    pub scan_total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewNote {
    pub path: String,
    pub what: String,
    pub lose: String,
    pub bytes: u64,
}

/// 预检：逐条给出 删 / 只删链接 / 跳过（原因），并汇总警告。会修改 `plan` 里的 `decisions`、`fp`、`bytes`。
pub fn preflight(plan: &mut Plan, protector: &Protector, opts: &PreflightOptions) -> PreflightReport {
    preflight_with(plan, protector, opts, &Progress::default())
}

/// 确认弹层先用的估算（阶段 G）：只用快速计划（[`super::plan::build_plan_quick`]）的结果，每项按扫描时的大小算，
/// 被拦下的项照常列出、空间提示 / 隔离区检查照常给出；`pending = true`、`can_execute = false`。
pub fn quote(plan: &Plan) -> PreflightReport {
    report_impl(plan, Vec::new(), true, &PerfModel::load_default())
}

enum RmMsg {
    Locked(usize, usize, String),
    ItemDone(usize),
}

/// 一组交给 Restart Manager 的文件和查到的进程。
type RmResult = (Vec<(usize, usize)>, Vec<LockingProcess>);

/// 同 [`preflight`]，带进度和取消。
///
/// 阶段 G 的做法（护栏和判断顺序与阶段 D 相同，只是更快）：
/// 1. 每条的保护路径检查：父目录真实位置按文件夹缓存（[`ParentCache`]），同一文件夹只解析一次；
/// 2. 云占位 / 云文件：只看目录项属性，**不打开**；
/// 3. 其余每个文件 / 链接**只打开一次**（[`winops::probe_and_fingerprint`]）：同时得到占用探测结果和指纹
///    （文件 ID / 大小 / 修改时间，执行前的重新校验用它对比）；30 分钟内改过的按指纹里的修改时间跳过；
/// 4. 以上按文件并行（多线程按顺序分块取活）；
/// 5. 探测到被占用的文件边探测边交给 Restart Manager：同一清单项的按 `rm_chunk` 个一组，最多 `rm_sessions`
///    个会话并发，只调用 RmStartSession / RmRegisterResources / RmGetList / RmEndSession（只报告，不关进程）。
///
/// 取消时尽快返回，`plan.preflight_done` 保持 false（不能执行）。
pub fn preflight_with(plan: &mut Plan, protector: &Protector, opts: &PreflightOptions, prog: &Progress) -> PreflightReport {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    let t0 = std::time::Instant::now();
    let mut timing: BTreeMap<String, u64> = BTreeMap::new();
    for item in plan.items.iter_mut() {
        if item.blocked.is_none() {
            // 保护路径再查一次（纵深防御：清单可能是很久以前生成的）
            if let Some(why) = protector.check(&item.path) {
                item.blocked = Some(why);
            }
        }
        if item.blocked.is_some() {
            item.decisions = vec![Decision::Skip { reason: SkipReason::Protected { why: "整项被拦下".into() } }; item.entries.len()];
        } else if item.decisions.len() != item.entries.len() {
            item.decisions = vec![Decision::Pending; item.entries.len()];
        }
    }
    // 小的清单项排在前面：被占用的文件多半在应用缓存这类小文件夹里（本机 259 个被占用文件全在 18 个小项里，
    // uv / npm / cargo 这些十几万个文件的大缓存里一个都没有），先查小的，Restart Manager 就能在大项的探测期间做完。
    let mut order: Vec<usize> = (0..plan.items.len()).filter(|&ii| plan.items[ii].blocked.is_none()).collect();
    order.sort_by_key(|&ii| plan.items[ii].entries.len());
    let work: Vec<(usize, usize)> = order.iter().flat_map(|&ii| (0..plan.items[ii].entries.len()).map(move |ei| (ii, ei))).collect();
    let remaining: Vec<AtomicUsize> =
        plan.items.iter().map(|it| AtomicUsize::new(if it.blocked.is_none() { it.entries.len() } else { 0 })).collect();
    prog.to_check.store(work.len(), Relaxed);
    prog.set_phase(2);
    // 只有真的按普通文件夹枚举过的清单项才能当“根”（清单项本身是链接时只有它自己一条，不算）。
    let cache = ParentCache::with_roots(
        plan.items
            .iter()
            .filter(|i| i.blocked.is_none() && i.is_dir && !i.entries.iter().any(|e| e.kind == EntryKind::Link && e.path == i.path))
            .map(|i| i.path.as_str()),
    );
    let (tx, rx) = std::sync::mpsc::channel::<RmMsg>();
    let items = &plan.items;
    let (decided, rm_results, check_ms) = std::thread::scope(|s| {
        let coord = s.spawn(|| rm_coordinator(rx, opts, prog));
        let tc = std::time::Instant::now();
        // 按顺序分块取活（不用 rayon 的二分切分，保证大体按上面排好的顺序查）；实测线程再多也快不了多少
        // （16 线程约 31 µs / 个，4 线程约 38 µs / 个），用 CPU 线程数、至少 4 个。
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 16);
        const BLOCK: usize = 64;
        let next = AtomicUsize::new(0);
        let slots: Vec<std::sync::Mutex<Vec<(Decision, Option<Fingerprint>, u64)>>> =
            (0..work.len().div_ceil(BLOCK)).map(|_| std::sync::Mutex::new(Vec::new())).collect();
        std::thread::scope(|s2| {
            for _ in 0..threads {
                let tx = tx.clone();
                let (next, slots, work, cache, remaining) = (&next, &slots, &work, &cache, &remaining);
                s2.spawn(move || loop {
                    let b = next.fetch_add(1, Relaxed);
                    if b >= slots.len() || prog.cancelled() {
                        break;
                    }
                    let mut out = Vec::with_capacity(BLOCK);
                    for &(ii, ei) in &work[b * BLOCK..((b + 1) * BLOCK).min(work.len())] {
                        let item = &items[ii];
                        let e = &item.entries[ei];
                        let r = decide(e, protector, cache, opts, item.cluster);
                        if matches!(r.0, Decision::Skip { reason: SkipReason::Locked { .. } }) {
                            prog.locked.fetch_add(1, Relaxed);
                            let _ = tx.send(RmMsg::Locked(ii, ei, e.path.clone()));
                        }
                        prog.checked.fetch_add(1, Relaxed);
                        if remaining[ii].fetch_sub(1, Relaxed) == 1 {
                            let _ = tx.send(RmMsg::ItemDone(ii));
                        }
                        out.push(r);
                    }
                    *slots[b].lock().unwrap() = out;
                });
            }
        });
        let decided: Vec<(Decision, Option<Fingerprint>, u64)> = slots.into_iter().flat_map(|m| m.into_inner().unwrap()).collect();
        let check_ms = tc.elapsed().as_millis() as u64;
        drop(tx);
        if prog.rm_done.load(Relaxed) < prog.rm_total.load(Relaxed) {
            prog.set_phase(3);
        }
        let rm = coord.join().unwrap_or_default();
        (decided, rm, check_ms)
    });
    timing.insert("check_parallel".into(), check_ms);
    timing.insert("rm_after_check".into(), (t0.elapsed().as_millis() as u64).saturating_sub(check_ms));
    timing.insert("parent_dirs_resolved".into(), cache.len() as u64);
    let perf = opts.perf.clone().unwrap_or_else(PerfModel::load_default);
    if prog.cancelled() {
        return report_impl(plan, Vec::new(), true, &perf);
    }
    for (&(ii, ei), (d, fp, bytes)) in work.iter().zip(decided) {
        let item = &mut plan.items[ii];
        item.decisions[ei] = d;
        let e = &mut item.entries[ei];
        if fp.is_some() {
            e.fp = fp;
        }
        e.bytes = bytes;
    }
    // 护栏 7：被占用的文件问 Restart Manager 是谁在用（只报告）。
    let mut all_procs: Vec<LockingProcess> = Vec::new();
    for (refs, procs) in rm_results {
        all_procs.extend(procs.iter().cloned());
        for (ii, ei) in refs {
            plan.items[ii].decisions[ei] = Decision::Skip { reason: SkipReason::Locked { processes: procs.clone() } };
        }
    }
    // 阶段 J（PHASE_J_CRASH.md 隐患 5）：正在运行的 Chromium / Electron 程序的磁盘缓存——它们常用 FILE_SHARE_DELETE
    // 打开缓存文件，探测不出“被占用”，删了会弄坏正在运行的程序（上次 444 条“回收站操作被中止”就是这些）。
    let tl = std::time::Instant::now();
    let live = skip_live_chromium_caches(plan, opts);
    timing.insert("live_cache_check".into(), tl.elapsed().as_millis() as u64);
    all_procs.extend(live);
    all_procs.sort_by_key(|p| p.pid);
    all_procs.dedup_by_key(|p| p.pid);
    skip_cross_volume_hardlinks(plan);
    skip_unrecyclable(plan);
    plan.preflight_done = true;
    prog.set_phase(4);
    let mut rep = report_impl(plan, all_procs, false, &perf);
    timing.insert("total".into(), t0.elapsed().as_millis() as u64);
    rep.timing_ms = timing;
    rep
}

/// 阶段 J（§3.10 硬链接陷阱）：跨盘暂存时链接数 > 1 的文件跳过——整份拷到别的盘既占那个盘的空间，
/// 这个盘又腾不出空间（别的名字还指着同一份数据）。只看要进暂存区、而且跨盘的条目。
pub fn skip_cross_volume_hardlinks(plan: &mut Plan) {
    for ii in 0..plan.items.len() {
        if plan.items[ii].blocked.is_some() || plan.dest_for(&plan.items[ii]) != Dest::Quarantine {
            continue;
        }
        let hits: Vec<(usize, u32)> = plan.items[ii]
            .entries
            .iter()
            .zip(&plan.items[ii].decisions)
            .enumerate()
            .filter(|(_, (e, d))| matches!(d, Decision::Delete) && e.kind == EntryKind::File && plan.crosses_volume(&e.path))
            .filter_map(|(i, (e, _))| e.fp.filter(|f| f.links > 1).map(|f| (i, f.links)))
            .collect();
        for (i, links) in hits {
            plan.items[ii].decisions[i] = Decision::Skip { reason: SkipReason::Hardlink { links } };
        }
    }
}

/// 阶段 J（PHASE_J_CRASH.md 隐患 2）：要进回收站、但回收站收不下的条目（比该盘回收站上限还大，或该盘回收站设成了
/// “直接删除”）预先标成跳过——送进去 Windows 会直接永久删除。执行时还会按同样的规则再查一次。
pub fn skip_unrecyclable(plan: &mut Plan) {
    let mut bins: Vec<(String, winops::RecycleBinInfo)> = Vec::new();
    for ii in 0..plan.items.len() {
        if plan.items[ii].blocked.is_some() {
            continue;
        }
        let hits: Vec<(usize, String)> = plan.items[ii]
            .entries
            .iter()
            .zip(&plan.items[ii].decisions)
            .enumerate()
            .filter(|(_, (e, d))| matches!(d, Decision::Delete) && plan.entry_dest(&plan.items[ii], e) == Dest::Recycle)
            .filter_map(|(i, (e, _))| {
                let v = crate::scan::volume_root_of(&e.path)?.to_lowercase();
                let info = match bins.iter().find(|(r, _)| *r == v) {
                    Some((_, b)) => b.clone(),
                    None => {
                        let b = winops::RecycleBinInfo::read(&v);
                        bins.push((v, b.clone()));
                        b
                    }
                };
                let size = e.fp.map(|f| f.size).unwrap_or(e.size);
                info.refuse(size).map(|why| (i, why))
            })
            .collect();
        for (i, why) in hits {
            plan.items[ii].decisions[i] = Decision::Skip { reason: SkipReason::RecycleUnavailable { why } };
        }
    }
}

/// 一个 Chromium 磁盘缓存文件夹里拿去问 Restart Manager 的文件：`index`、`data_0..3`（blockfile 格式：GPUCache、
/// 老的 Cache）、`index-dir\the-real-index`（simple cache：Cache_Data、Code Cache\js）。
/// 返回 缓存文件夹（小写）→ 要问的文件。只认清单里真有 `index` 的文件夹。
pub fn chromium_cache_dirs(plan: &Plan) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in plan.items.iter().filter(|i| i.blocked.is_none()) {
        let set: std::collections::HashSet<String> = item.entries.iter().map(|e| e.path.to_lowercase()).collect();
        for e in &item.entries {
            if e.kind != EntryKind::File {
                continue;
            }
            let Some((dir, name)) = e.path.rsplit_once('\\') else { continue };
            if !name.eq_ignore_ascii_case("index") {
                continue;
            }
            let ld = dir.to_lowercase();
            let blockfile = set.contains(&format!("{ld}\\data_0"));
            let real_index = format!("{dir}\\index-dir\\the-real-index");
            let simple = set.contains(&real_index.to_lowercase()) || ld.ends_with("\\cache_data");
            if !(blockfile || simple) {
                continue;
            }
            let mut files = vec![e.path.clone()];
            for k in 0..4 {
                let f = format!("{dir}\\data_{k}");
                if set.contains(&f.to_lowercase()) {
                    files.push(f);
                }
            }
            if set.contains(&real_index.to_lowercase()) {
                files.push(real_index);
            }
            out.entry(ld).or_default().extend(files);
        }
    }
    out
}

/// 正在被某个进程打开的 Chromium 缓存文件夹：整个文件夹里要删的都改成“跳过·被占用（进程名）”。返回查到的进程。
/// 先把所有缓存文件放进**一个** Restart Manager 会话问一次（通常没有程序在跑，一次就完）；有进程在用时再按文件夹
/// 分别问（最多 `rm_sessions` 个会话并发、最多 400 个文件夹），找出是哪些文件夹。只报告，不关进程。
pub fn skip_live_chromium_caches(plan: &mut Plan, opts: &PreflightOptions) -> Vec<LockingProcess> {
    let dirs = chromium_cache_dirs(plan);
    if dirs.is_empty() {
        return Vec::new();
    }
    let all: Vec<String> = dirs.values().flatten().cloned().collect();
    // RmRegisterResources 一次注册几千个文件没问题；太多时只看 index（每个文件夹一个）
    let first: Vec<String> = if all.len() <= 4000 { all } else { dirs.values().filter_map(|v| v.first().cloned()).collect() };
    match winops::rm_locking_processes(&first) {
        Ok(p) if p.is_empty() => return Vec::new(),
        Ok(_) => {}
        Err(_) => return Vec::new(),
    }
    let list: Vec<(&String, &Vec<String>)> = dirs.iter().take(400).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let found: std::sync::Mutex<Vec<(String, Vec<LockingProcess>)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..opts.rm_sessions.clamp(1, 8) {
            s.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((d, files)) = list.get(k) else { break };
                if let Ok(p) = winops::rm_locking_processes(files) {
                    if !p.is_empty() {
                        found.lock().unwrap().push(((*d).clone(), p));
                    }
                }
            });
        }
    });
    let found = found.into_inner().unwrap_or_default();
    let mut procs = Vec::new();
    for (d, p) in &found {
        let pre = format!("{d}\\");
        for item in plan.items.iter_mut().filter(|i| i.blocked.is_none()) {
            for (e, dec) in item.entries.iter().zip(item.decisions.iter_mut()) {
                if matches!(dec, Decision::Delete) && e.path.to_lowercase().starts_with(&pre) {
                    *dec = Decision::Skip { reason: SkipReason::Locked { processes: p.clone() } };
                }
            }
        }
        procs.extend(p.iter().cloned());
    }
    procs
}

/// 收被占用的文件、按清单项分组交给 Restart Manager（最多 `rm_sessions` 个会话并发），和探测同时进行。
fn rm_coordinator(rx: std::sync::mpsc::Receiver<RmMsg>, opts: &PreflightOptions, prog: &Progress) -> Vec<RmResult> {
    use std::sync::atomic::Ordering::Relaxed;
    type Job = Vec<(usize, usize, String)>;
    let (jtx, jrx) = std::sync::mpsc::channel::<Job>();
    let jrx = std::sync::Mutex::new(jrx);
    let out: std::sync::Mutex<Vec<RmResult>> = std::sync::Mutex::new(Vec::new());
    let chunk = opts.rm_chunk.max(1);
    std::thread::scope(|s| {
        for _ in 0..opts.rm_sessions.max(1) {
            s.spawn(|| loop {
                let job = match jrx.lock().unwrap().recv() {
                    Ok(j) => j,
                    Err(_) => break,
                };
                let procs = if prog.cancelled() {
                    Vec::new()
                } else {
                    let paths: Vec<String> = job.iter().map(|x| x.2.clone()).collect();
                    winops::rm_locking_processes(&paths).unwrap_or_default()
                };
                out.lock().unwrap().push((job.iter().map(|x| (x.0, x.1)).collect(), procs));
                prog.rm_done.fetch_add(1, Relaxed);
            });
        }
        let mut bufs: BTreeMap<usize, Job> = BTreeMap::new();
        let mut sent = 0usize;
        let send = |job: Job| {
            if !job.is_empty() {
                prog.rm_total.fetch_add(1, Relaxed);
                let _ = jtx.send(job);
            }
        };
        for msg in rx {
            match msg {
                RmMsg::Locked(ii, ei, path) => {
                    if sent >= opts.rm_max_files {
                        continue; // 超过上限：只标“被占用”，不查进程
                    }
                    sent += 1;
                    let b = bufs.entry(ii).or_default();
                    b.push((ii, ei, path));
                    if b.len() >= chunk {
                        send(std::mem::take(b));
                    }
                }
                RmMsg::ItemDone(ii) => {
                    if let Some(b) = bufs.remove(&ii) {
                        send(b);
                    }
                }
            }
        }
        for (_, b) in std::mem::take(&mut bufs) {
            send(b);
        }
        drop(jtx);
    });
    out.into_inner().unwrap()
}

/// 一条的决定 + 预检时取到的指纹 + 能腾出的字节。判断顺序同阶段 D：保护路径 → 云 → 链接 → 读不到 → 热文件 → 占用。
fn decide(e: &super::Entry, protector: &Protector, cache: &ParentCache, opts: &PreflightOptions, cluster: u64) -> (Decision, Option<Fingerprint>, u64) {
    // 护栏 2（条目级）：文件夹里面的 .ssh 之类
    if let Some(why) = protector.entry_blocked_cached(&e.path, cache) {
        return (Decision::Skip { reason: SkipReason::Protected { why } }, None, e.bytes);
    }
    // 护栏 5：云（只看目录项属性，绝不打开）
    if e.kind == EntryKind::CloudDir || (e.kind == EntryKind::File && is_cloud_attrs(e.attrs, e.reparse_tag)) {
        return (Decision::Skip { reason: SkipReason::Cloud }, None, e.bytes);
    }
    // 护栏 6：链接只删自身（打开的是链接本身，不跟随）
    if e.kind == EntryKind::Link {
        let (probe, fp) = winops::probe_and_fingerprint(&e.path, e.link_is_dir);
        let d = match probe {
            Ok(()) => Decision::RemoveLink,
            Err(c) if winops::is_sharing_violation(c) => Decision::Skip { reason: SkipReason::Locked { processes: vec![] } },
            Err(2) | Err(3) => Decision::Skip { reason: SkipReason::Missing },
            Err(c) => Decision::Skip { reason: SkipReason::NoPermission { code: c } },
        };
        return (d, fp.ok(), 0);
    }
    // 护栏 7 的探测 + 指纹：一次打开（DELETE | 读属性，全共享，不跟随），立刻关闭，不改文件
    let (probe, fp) = winops::probe_and_fingerprint(&e.path, false);
    let fp = match fp {
        Ok(f) => f,
        Err(2) | Err(3) => return (Decision::Skip { reason: SkipReason::Missing }, None, e.bytes),
        Err(c) => return (Decision::Skip { reason: SkipReason::Unreadable { code: c } }, None, e.bytes),
    };
    let bytes = if fp.links > 1 { 0 } else { crate::fs::round_up(fp.on_disk, cluster) };
    // 护栏 4：热文件（按刚取到的指纹里的修改时间）
    let age = opts.now - winops::ft_to_unix(fp.mtime_ft);
    if age < opts.hot_window_secs {
        return (Decision::Skip { reason: SkipReason::Hot { minutes_ago: age.max(0) / 60 } }, Some(fp), bytes);
    }
    let d = match probe {
        Ok(()) => Decision::Delete,
        Err(c) if winops::is_sharing_violation(c) => Decision::Skip { reason: SkipReason::Locked { processes: vec![] } },
        Err(2) | Err(3) => Decision::Skip { reason: SkipReason::Missing },
        Err(c) => Decision::Skip { reason: SkipReason::NoPermission { code: c } },
    };
    (d, Some(fp), bytes)
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

/// 计划要做的工作量（估计用时 / 进度权重用），只算预检后标记为删 / 只删链接的条目。
pub fn workload(plan: &Plan) -> Workload {
    let mut w = Workload::default();
    for it in plan.items.iter().filter(|i| i.blocked.is_none()) {
        let mut any = false;
        for (e, _) in it.will_delete() {
            any = true;
            if e.kind == EntryKind::Link {
                match plan.entry_dest(it, e) {
                    Dest::Quarantine => w.rename += 1,
                    _ => w.links += 1,
                }
                continue;
            }
            match plan.entry_dest(it, e) {
                Dest::Permanent => w.permanent += 1,
                Dest::Recycle => w.recycle += 1,
                Dest::Quarantine if plan.crosses_volume(&e.path) => {
                    w.xvol += 1;
                    w.xvol_bytes += e.size;
                }
                Dest::Quarantine => w.rename += 1,
            }
        }
        if any {
            w.dirs += it.dirs.len();
        }
    }
    w
}

/// 汇总报告。`pending = true`（弹层先用的估算）时每项按扫描时的大小算、不能执行。
fn report_impl(plan: &Plan, locked_processes: Vec<LockingProcess>, pending: bool, perf: &PerfModel) -> PreflightReport {
    let mut items = Vec::new();
    let (mut total_entries, mut total_bytes, mut total_skipped) = (0usize, 0u64, 0usize);
    let (mut locked_count, mut hot_count, mut cloud_count, mut link_count) = (0, 0, 0, 0);
    let mut review_items = Vec::new();
    let mut by_dest = ByDest::default();
    let mut skipped_bytes_all = 0u64;
    let mut skipped_by_reason_all: BTreeMap<String, CountBytes> = BTreeMap::new();
    let mut scan_total_bytes = 0u64;
    // 跨盘暂存要在目的盘上占的空间（逻辑大小之和；硬链接已经跳过）
    let mut xvol_need = 0u64;
    for it in &plan.items {
        let mut skipped = Vec::new();
        let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
        let (mut sk_count, mut sk_bytes, mut links) = (0usize, 0u64, 0usize);
        if it.blocked.is_none() {
            scan_total_bytes += it.scan_allocated;
            for (e, d) in it.entries.iter().zip(&it.decisions) {
                match d {
                    Decision::Skip { reason } => {
                        sk_count += 1;
                        sk_bytes += e.bytes;
                        *by_reason.entry(reason.code().to_string()).or_default() += 1;
                        skipped_by_reason_all.entry(reason.code().to_string()).or_default().add(e.bytes);
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
            if !pending {
                for (e, _) in it.will_delete() {
                    let d = plan.entry_dest(it, e);
                    by_dest.get_mut(d).add(e.bytes);
                    if d == Dest::Quarantine && e.kind == EntryKind::File && plan.crosses_volume(&e.path) {
                        xvol_need += e.size;
                    }
                }
            } else {
                let d = plan.dest_for(it);
                by_dest.get_mut(d).bytes += it.scan_allocated;
                if d == Dest::Quarantine && plan.crosses_volume(&it.path) {
                    xvol_need += it.scan_allocated;
                }
            }
        }
        skipped_bytes_all += sk_bytes;
        let wd_count = if it.blocked.is_none() && !pending { it.entry_count() } else { 0 };
        let wd_bytes = match (&it.blocked, pending) {
            (Some(_), _) => 0,
            (None, true) => it.scan_allocated,
            (None, false) => it.bytes(),
        };
        total_entries += wd_count;
        total_bytes += wd_bytes;
        total_skipped += sk_count;
        link_count += links;
        if it.blocked.is_none() && it.tier == Tier::Review && (wd_count > 0 || (pending && wd_bytes > 0)) {
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
            will_delete: if it.blocked.is_none() && !pending { it.entries_preview(PREVIEW_LIMIT) } else { vec![] },
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
        .filter(|i| i.blocked.is_none() && (pending || i.entry_count() > 0))
        .filter_map(|i| crate::scan::volume_root_of(&i.path))
        .collect();
    src_vols.sort();
    src_vols.dedup();
    let drives = crate::win::list_fixed_drives();
    // 哪些卷有东西要进回收站、各多少字节（护栏 8 只在这时提示）
    let mut recycle_vols: BTreeMap<String, u64> = BTreeMap::new();
    for it in plan.items.iter().filter(|i| i.blocked.is_none()) {
        let (goes, b) = if pending {
            (plan.dest_for(it) == Dest::Recycle && it.scan_allocated > 0, it.scan_allocated)
        } else {
            let mut n = 0usize;
            let mut b = 0u64;
            for (e, _) in it.will_delete() {
                if plan.entry_dest(it, e) == Dest::Recycle {
                    n += 1;
                    b += e.bytes;
                }
            }
            (n > 0, b)
        };
        if goes {
            if let Some(v) = crate::scan::volume_root_of(&it.path) {
                *recycle_vols.entry(v).or_default() += b;
            }
        }
    }
    // 护栏 8：空间提示（只在有东西进回收站时）
    for (v, planned) in &recycle_vols {
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
        // 回收站上限（MaxCapacity）：超了 Windows 会静默删掉最早放进回收站的东西
        // （只在逐个检查做完后查：要列一遍回收站文件夹，本机 26 万项约 1 秒，弹层的估算不等它）
        let cap = if pending { None } else { winops::recycle_bin_max_capacity(v).zip(winops::recycle_bin_usage(v)) };
        if let Some((max, (used, _))) = cap {
            if used.saturating_add(*planned) > max {
                warnings.push(Warning {
                    code: "recycle_over_capacity".into(),
                    message: format!(
                        "{v} 的回收站上限 {}，现在已经有 {}，再放 {} 就超了：Windows 会把回收站里最早的东西直接删掉。",
                        crate::view::human_bytes(max),
                        crate::view::human_bytes(used),
                        crate::view::human_bytes(*planned)
                    ),
                    blocking: false,
                });
            }
        }
    }
    let needs_quarantine = by_dest.quarantine.bytes > 0 || by_dest.quarantine.count > 0;
    if let (Strategy::Quarantine { dest_root }, true) = (&plan.strategy, needs_quarantine) {
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
            Ok(sp) if xvol_need > 0 && sp.free_bytes < xvol_need.saturating_add(64 * 1024 * 1024) => warnings.push(Warning {
                code: "quarantine_no_space".into(),
                message: format!(
                    "隔离区所在的 {dest_vol} 只剩 {}，放不下这次要隔离的 {}（另留 64 MB 余量）。请换一个盘或改用回收站。",
                    crate::view::human_bytes(sp.free_bytes),
                    crate::view::human_bytes(xvol_need)
                ),
                blocking: true,
            }),
            Err(e) => warnings.push(Warning { code: "quarantine_bad_dest".into(), message: format!("隔离区位置不可用：{e}"), blocking: true }),
            _ => {}
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
        .map(|d| DriveChoice {
            root: d.root.clone(),
            label: d.label.clone(),
            free_bytes: d.free_bytes,
            total_bytes: d.total_bytes,
            dest_root: match super::quarantine::prepare_dest(&d.root, false) {
                super::quarantine::DestStatus::Ok { dest_root } => Some(dest_root),
                _ => None,
            },
        })
        .collect();
    let can_execute = !pending && plan.preflight_done && total_entries > 0 && !warnings.iter().any(|w| w.blocking);
    let active_items = plan.items.iter().filter(|i| i.blocked.is_none() && i.entry_count() > 0).count();
    let est_secs = if pending { 0.0 } else { perf.estimate_secs(&workload(plan), super::default_workers(), active_items) };
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
        pending,
        timing_ms: BTreeMap::new(),
        safe_direct: plan.safe_direct,
        est_secs,
        by_dest,
        skipped_bytes: skipped_bytes_all,
        skipped_by_reason: skipped_by_reason_all,
        scan_total_bytes,
    }
}
