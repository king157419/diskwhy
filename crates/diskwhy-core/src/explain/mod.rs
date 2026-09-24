//! 解释引擎（阶段 B）：告诉用户每个文件夹 / 文件是什么、属于谁、为什么在、删了会怎样、怎么腾。
//!
//! 顺序（[`explain_full`]；[`explain`] 是不带 LLM 的同一条链）：
//! 1. 规则库 `rules/*.toml`（[`rules`]）：精确命中当前路径；没命中再看上层文件夹有没有
//!    `scope = "tree"` 的规则（“继承”）。`source = Rule`，可信度 1.0。
//! 2. 启发式（[`heuristics`]）：目录内容签名 → 注册表归属 → 年龄提示。`source = Heuristic`，可信度 0.4~0.8。
//!    只有签名严格的 Chromium 缓存能给 safe，其余只能 review / keep。
//! 3. （可选）LLM 兜底（[`llm`]）：只有调用方传了 [`llm::LlmOracle`] **并且**设置里打开了开关，
//!    而且前两步都没认出来时才问。`source = Llm`，档位强制 review，可信度固定为低值。
//!    列目录时不传（不联网、不卡界面），用户点“问 AI”时才传。
//! 4. 都不行：`source = Unknown`，把看到的线索列出来。
//!
//! 最后统一过一遍 [`enforce_guards`]：LLM / 未知永远 review；启发式的 safe 只认白名单；
//! safe 落在桌面 / 文档等个人文件夹下一律降为 review。

pub mod env;
pub mod heuristics;
pub mod llm;
pub mod pattern;
pub mod registry;
pub mod rules;

use crate::model::{Node, NodeKind};
use crate::view::human_bytes;
use env::EnvMap;
use llm::LlmSettings;
use pattern::split_lower;
use registry::RegistrySnapshot;
use rules::{InheritedHit, Rule, RuleSet, Scope};
use serde::{Deserialize, Serialize};

/// 三档。未知目录也是 `Review`（界面上用 `source = Unknown` 区分成灰色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// 缓存类，删了会自动重建，不丢数据。
    Safe,
    /// 删了有代价，需要用户判断。
    Review,
    /// 用户数据或系统关键，不能删（可建议迁移或用系统工具）。
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Cache,
    Log,
    Installer,
    Build,
    Model,
    Userdata,
    System,
    Vm,
    Trash,
    /// 软件本体（安装目录）。
    App,
    Unknown,
}

/// 该怎么腾。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    Delete,
    BuiltinTool,
    Migrate,
    Manual,
    None,
}

/// 说明从哪来。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    Rule {
        id: String,
        group: String,
        /// 继承自哪个上层文件夹（精确命中时为 None）。
        inherited_from: Option<String>,
    },
    Heuristic {
        name: String,
    },
    Llm {
        model: String,
    },
    Unknown,
}

/// 一条解释（对应 PLAN 第 2 节）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Explanation {
    pub tier: Tier,
    pub owner: Option<String>,
    pub category: Category,
    pub what: String,
    pub why: String,
    pub lose: String,
    pub how: Vec<How>,
    pub how_detail: String,
    /// 能腾出的字节（safe/review 且可删时 = 实际占用；keep 为 0）。
    pub gain_bytes: u64,
    /// “能腾出”一栏的大白话。
    pub gain_note: String,
    pub source: Source,
    /// 0~1。规则 1.0，启发式 0.5~0.7，未知 0。
    pub confidence: f32,
    pub refs: Vec<String>,
    pub verified_on: Option<String>,
    /// 附加提示（继承、年龄、档位被下调等）。
    pub notes: Vec<String>,
    /// 线索（启发式 / 未知时列出看到了什么）。
    pub clues: Vec<String>,
}

impl Explanation {
    pub fn is_unknown(&self) -> bool {
        self.source == Source::Unknown
    }
}

/// 解释用的上下文：展开后的环境变量、规则库、注册表快照、LLM 设置。启动时建一次。
#[derive(Debug, Clone)]
pub struct ExplainCtx {
    pub env: EnvMap,
    pub rules: RuleSet,
    pub registry: RegistrySnapshot,
    pub settings: LlmSettings,
    /// 受保护的用户目录（小写段），safe 规则落在这些目录下会被降为 review。
    protected: Vec<Vec<String>>,
    /// 当前时间（Unix 秒），测试可固定。
    pub now: i64,
}

/// 受保护的用户目录名（%USERPROFILE% 之下）。
pub const PROTECTED_USER_DIRS: &[&str] = &["Desktop", "Documents", "Pictures", "Videos", "Music", "Downloads", "OneDrive"];

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl ExplainCtx {
    pub fn new(env: EnvMap, rules: RuleSet, registry: RegistrySnapshot, settings: LlmSettings) -> Self {
        let mut protected = Vec::new();
        if let Some(home) = env.get("USERPROFILE") {
            for d in PROTECTED_USER_DIRS {
                protected.push(split_lower(&format!("{home}\\{d}")));
            }
        }
        for v in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(p) = env.get(v) {
                protected.push(split_lower(p));
            }
        }
        if let Some(public) = env.get("PUBLIC") {
            for d in PROTECTED_USER_DIRS {
                protected.push(split_lower(&format!("{public}\\{d}")));
            }
        }
        ExplainCtx { env, rules, registry, settings, protected, now: now_unix() }
    }

    /// 生产用：当前进程环境变量 + 内置规则 + 用户自定义规则目录 + 注册表 + settings.json。
    /// 返回 (ctx, 警告列表)。内置规则校验失败会被当作警告返回（测试会拦住这种情况）。
    pub fn load_default() -> (Self, Vec<String>) {
        let env = EnvMap::from_process();
        let (mut rules, mut warnings) = RuleSet::embedded(&env);
        if let Some(dir) = llm::app_data_dir().map(|d| d.join("rules")) {
            warnings.extend(rules.load_user_dir(&dir, &env));
        }
        let registry = RegistrySnapshot::load();
        let settings = LlmSettings::load();
        (ExplainCtx::new(env, rules, registry, settings), warnings)
    }

    /// 路径是否落在受保护用户目录之下（含自身）。
    pub fn is_protected(&self, segs: &[String]) -> bool {
        self.protected.iter().any(|p| segs.len() >= p.len() && segs[..p.len()] == p[..])
    }
}

/// 要解释的对象。
pub struct Target<'a> {
    pub node: &'a Node,
    /// 完整路径（File 由父路径拼出；Rest 为父目录路径）。
    pub path: &'a str,
    /// 父目录节点（用于看“旁边有没有 Cargo.toml”等）。
    pub parent: Option<&'a Node>,
    /// 从扫描根到父目录的链（不含自身），用于规则继承。
    pub ancestors: &'a [&'a Node],
}

/// 解释一个节点。继承命中会自己沿 `ancestors` 往上找。
pub fn explain(t: &Target, ctx: &ExplainCtx) -> Explanation {
    let inherited = ctx.rules.best_inherited(t.ancestors);
    explain_with_inherited(t, inherited.as_ref(), ctx)
}

/// 完整的链：规则 → 启发式 → （可选）LLM → 未知。
///
/// `llm = None` 时和 [`explain`] 完全一样。传了 oracle 也只在“前两步都没认出来”且
/// `ctx.settings.usable()`（默认关闭）时才调用；LLM 出错时退回未知，并把错误写进 notes。
pub fn explain_full(t: &Target, ctx: &ExplainCtx, llm: Option<&dyn llm::LlmOracle>) -> Explanation {
    let mut e = explain(t, ctx);
    if !e.is_unknown() || t.node.kind == NodeKind::Rest {
        return e;
    }
    let Some(oracle) = llm else { return e };
    if !ctx.settings.usable() {
        return e;
    }
    match llm::ask_with(t, ctx, oracle) {
        Ok(mut ai) => {
            // 保留 DiskWhy 自己看到的线索，方便对照 AI 说得对不对。
            ai.clues = std::mem::take(&mut e.clues);
            enforce_guards(ai, t.path, ctx)
        }
        Err(msg) => {
            e.notes.push(format!("问 AI 没成功：{msg}"));
            e
        }
    }
}

/// 最后一道护栏（对所有来源都跑一遍）：
/// - `Llm` / `Unknown`：档位一律 review（LLM 给什么都不算数）；
/// - `Heuristic`：safe 只认 [`heuristics::SAFE_HEURISTICS`]；
/// - 任何来源的 safe 落在桌面 / 文档 / 下载 / OneDrive 等个人文件夹下：降为 review。
pub fn enforce_guards(mut e: Explanation, path: &str, ctx: &ExplainCtx) -> Explanation {
    let protected_safe = e.tier == Tier::Safe && ctx.is_protected(&split_lower(path));
    let demote = match &e.source {
        Source::Llm { .. } | Source::Unknown => e.tier != Tier::Review,
        Source::Heuristic { name } => e.tier == Tier::Safe && (!heuristics::SAFE_HEURISTICS.contains(&name.as_str()) || protected_safe),
        Source::Rule { .. } => protected_safe,
    };
    if demote {
        e.tier = Tier::Review;
        match e.source {
            Source::Llm { .. } | Source::Unknown => {
                e.gain_bytes = 0;
            }
            _ => {
                // 只有 safe 会走到这里，safe 的 gain_bytes 就是实际占用。
                let alloc = e.gain_bytes;
                let (g, note) = gain(Tier::Review, &e.how, alloc);
                e.gain_bytes = g;
                e.gain_note = note;
            }
        }
        e.notes.push("档位被 DiskWhy 的护栏改成了“需要你判断”（AI 推测、没认出来的东西、个人文件夹里的东西都不会标成可放心清理）。".into());
    }
    e
}

/// 同 [`explain`]，但继承命中由调用方预先算好（列目录时对所有子项只算一次）。
pub fn explain_with_inherited(t: &Target, inherited: Option<&InheritedHit>, ctx: &ExplainCtx) -> Explanation {
    let node = t.node;
    if node.kind == NodeKind::Rest {
        return enforce_guards(explain_rest(t, inherited, ctx), t.path, ctx);
    }
    let segs = split_lower(t.path);
    let e = if let Some(idx) = ctx.rules.best_exact(&segs, Some(node)) {
        from_rule(&ctx.rules.rules[idx], t, &segs, None, ctx)
    } else if let Some(h) = inherited {
        from_rule(&ctx.rules.rules[h.rule], t, &segs, Some(&h.from_path), ctx)
    } else {
        heuristics::explain_unmatched(t, ctx)
    };
    enforce_guards(e, t.path, ctx)
}

/// 目录的“其余小文件合计”节点：跟随所在目录的说明。
fn explain_rest(t: &Target, inherited: Option<&InheritedHit>, ctx: &ExplainCtx) -> Explanation {
    let segs = split_lower(t.path);
    // 所在目录自己精确命中的规则（只认 tree；self 规则只说明那个文件夹本身，不管里面的散文件）。
    let dir_rule = t
        .parent
        .and_then(|p| ctx.rules.best_exact(&segs, Some(p)))
        .filter(|&i| ctx.rules.rules[i].scope == Scope::Tree)
        .map(|i| (i, None))
        .or_else(|| inherited.map(|h| (h.rule, Some(h.from_path.as_str()))));
    if let Some((idx, from)) = dir_rule {
        let mut e = from_rule(&ctx.rules.rules[idx], t, &segs, from.or(Some(t.path)), ctx);
        e.notes.insert(
            0,
            format!("这是「{}」里除最大的几个文件外其余 {} 个小文件的合计，跟随所在文件夹的说明。", t.path, t.node.files),
        );
        return e;
    }
    let mut e = heuristics::unknown(t, &heuristics::subtree_stats(t.node), ctx);
    e.what = format!("「{}」里其余 {} 个小文件的合计（为省内存合并显示）", t.path, t.node.files);
    e
}

/// 规则 → 解释。
fn from_rule(rule: &Rule, t: &Target, segs: &[String], inherited_from: Option<&str>, ctx: &ExplainCtx) -> Explanation {
    let mut notes = Vec::new();
    let mut tier = rule.tier;
    if let Some(from) = inherited_from {
        if t.node.kind != NodeKind::Rest {
            notes.push(format!("它在「{from}」里面，按上层文件夹的规则来说明。"));
        }
    }
    if tier == Tier::Safe && ctx.is_protected(segs) {
        tier = Tier::Review;
        notes.push("它在桌面 / 文档 / 下载 / OneDrive 等个人文件夹下面，为稳妥起见从“可放心清理”降为“需要你判断”。".into());
    }
    if rule.category == Category::Cache && t.node.kind != NodeKind::Rest {
        let newest = heuristics::newest_mtime(t.node, 5_000);
        if let Some(n) = heuristics::long_unused_note(newest, ctx.now) {
            notes.push(n);
        }
    }
    let owner = if rule.owner_from_path {
        match heuristics::owner_from_path(t.path, ctx) {
            Some(o) => Some(format!("{}（{}）", o.label(), rule.owner)),
            None => Some(rule.owner.clone()),
        }
    } else {
        Some(rule.owner.clone())
    };
    let how = rule.how.clone();
    let (gain_bytes, gain_note) = gain(tier, &how, t.node.allocated);
    Explanation {
        tier,
        owner,
        category: rule.category,
        what: rule.what.clone(),
        why: rule.why.clone(),
        lose: rule.lose.clone(),
        how,
        how_detail: rule.how_detail.clone(),
        gain_bytes,
        gain_note,
        source: Source::Rule {
            id: rule.id.clone(),
            group: rule.group.clone(),
            inherited_from: inherited_from.map(str::to_string),
        },
        confidence: 1.0,
        refs: rule.refs.clone(),
        verified_on: Some(rule.verified_on.clone()),
        notes,
        clues: Vec::new(),
    }
}

/// “能腾出多少”。
pub fn gain(tier: Tier, how: &[How], allocated: u64) -> (u64, String) {
    let sz = human_bytes(allocated);
    let can_delete = how.contains(&How::Delete);
    match tier {
        Tier::Safe if can_delete => (allocated, format!("删掉约可腾出 {sz}（会自动重新生成，过段时间可能又长回来一部分）")),
        Tier::Safe => (allocated, format!("用建议的工具清理，最多可腾出约 {sz}")),
        Tier::Review if can_delete => (allocated, format!("删掉最多可腾出 {sz}，但有代价，见“删了会失去什么”")),
        Tier::Review if how.contains(&How::BuiltinTool) => (allocated, format!("用系统工具处理，最多可腾出约 {sz}，具体由工具决定")),
        Tier::Review if how.contains(&How::Migrate) => (0, format!("不建议直接删；迁到别的盘可让 C 盘腾出约 {sz}")),
        Tier::Review => (0, format!("占 {sz}；得先弄清楚是什么再决定，这里不估算能腾多少")),
        Tier::Keep if how.contains(&How::Migrate) => (0, format!("不能删；迁到别的盘可让 C 盘腾出约 {sz}")),
        Tier::Keep if how.contains(&How::BuiltinTool) => {
            (0, format!("不能手动删；用系统自带工具清理能腾出一部分（最多 {sz}），多少由工具决定"))
        }
        Tier::Keep => (0, format!("占 {sz}，但不能删")),
    }
}

// ───────────────────────── 汇总：本次扫描各档合计 ─────────────────────────

/// 一条规则在本次扫描里覆盖的字节。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleBytes {
    pub id: String,
    pub owner: String,
    pub what: String,
    pub tier: Tier,
    pub bytes: u64,
}

/// 按规则库统计的各档合计（不含启发式；启发式和未知在列表里逐项看）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TierSummary {
    pub safe_bytes: u64,
    pub review_bytes: u64,
    pub keep_bytes: u64,
    /// 规则没覆盖到的字节。
    pub uncovered_bytes: u64,
    /// 命中过至少一次的规则数 / 规则总数。
    pub rules_hit: usize,
    pub rules_total: usize,
    /// 各档里最大的几项（按规则聚合）。
    pub top_safe: Vec<RuleBytes>,
    pub top_review: Vec<RuleBytes>,
    pub top_keep: Vec<RuleBytes>,
    pub duration_ms: u64,
}

/// 遍历整棵树，按规则把每个字节归到 safe / review / keep / 未覆盖。
///
/// 目录命中规则后仍会往下走（子目录可能有更具体的规则，比如浏览器 Profile 是 keep，
/// 里面的 Cache 是 safe）；用模式前缀剪枝，不可能再命中的子树整块结算。
pub fn tier_summary(tree: &Node, ctx: &ExplainCtx) -> TierSummary {
    let start = std::time::Instant::now();
    let rs = &ctx.rules;
    let mut per_rule = vec![0u64; rs.rules.len()];
    let mut uncovered = 0u64;
    let root_segs = split_lower(&tree.path);
    let active: Vec<u32> = (0..rs.patterns.len() as u32)
        .filter(|&i| {
            let p = &rs.patterns[i as usize].1;
            p.matches(&root_segs) || p.could_match_below(&root_segs)
        })
        .collect();
    let mut segs = root_segs;
    walk_summary(tree, &mut segs, &active, None, rs, &mut per_rule, &mut uncovered);

    let mut s = TierSummary { uncovered_bytes: uncovered, rules_total: rs.rules.len(), ..Default::default() };
    let mut items: Vec<RuleBytes> = Vec::new();
    for (i, &b) in per_rule.iter().enumerate() {
        if b == 0 {
            continue;
        }
        let r = &rs.rules[i];
        s.rules_hit += 1;
        match r.tier {
            Tier::Safe => s.safe_bytes += b,
            Tier::Review => s.review_bytes += b,
            Tier::Keep => s.keep_bytes += b,
        }
        items.push(RuleBytes { id: r.id.clone(), owner: r.owner.clone(), what: r.what.clone(), tier: r.tier, bytes: b });
    }
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    for it in items {
        let v = match it.tier {
            Tier::Safe => &mut s.top_safe,
            Tier::Review => &mut s.top_review,
            Tier::Keep => &mut s.top_keep,
        };
        if v.len() < 8 {
            v.push(it);
        }
    }
    s.duration_ms = start.elapsed().as_millis() as u64;
    s
}

fn add(per_rule: &mut [u64], uncovered: &mut u64, rule: Option<usize>, bytes: u64) {
    match rule {
        Some(i) => per_rule[i] += bytes,
        None => *uncovered += bytes,
    }
}

fn walk_summary(
    node: &Node,
    segs: &mut Vec<String>,
    active: &[u32],
    inherited: Option<usize>,
    rs: &RuleSet,
    per_rule: &mut [u64],
    uncovered: &mut u64,
) {
    let exact = rs.best_exact_among(active, segs, Some(node));
    // self 规则只说明文件夹本身，里面的东西不归它。
    let child_inherited = match exact {
        Some(i) if rs.rules[i].scope == Scope::Tree => Some(i),
        _ => inherited,
    };
    for c in &node.children {
        match c.kind {
            NodeKind::Rest => add(per_rule, uncovered, child_inherited, c.allocated),
            NodeKind::File => {
                segs.push(c.name.to_lowercase());
                let f = rs.best_exact_among(active, segs, Some(c));
                segs.pop();
                add(per_rule, uncovered, f.or(child_inherited), c.allocated);
            }
            NodeKind::Dir => {
                segs.push(c.name.to_lowercase());
                let child_active: Vec<u32> = active
                    .iter()
                    .copied()
                    .filter(|&i| {
                        let p = &rs.patterns[i as usize].1;
                        p.matches(segs) || p.could_match_below(segs)
                    })
                    .collect();
                if child_active.is_empty() || c.children.is_empty() {
                    let own = if child_active.is_empty() { None } else { rs.best_exact_among(&child_active, segs, Some(c)) };
                    add(per_rule, uncovered, own.or(child_inherited), c.allocated);
                } else {
                    walk_summary(c, segs, &child_active, child_inherited, rs, per_rule, uncovered);
                }
                segs.pop();
            }
        }
    }
}
