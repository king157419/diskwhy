//! 解释引擎（阶段 B）：告诉用户每个文件夹 / 文件是什么、属于谁、为什么在、删了会怎样、怎么腾。
//!
//! 顺序：
//! 1. 规则库 `rules/*.toml`（[`rules`]）：精确命中当前路径；没命中再看上层文件夹有没有
//!    `scope = "tree"` 的规则（“继承”）。
//! 2. 启发式（[`heuristics`]）：目录内容签名 → 注册表归属 → 年龄提示。结果只能是 review / keep。
//! 3. 都不行：`source = Unknown`，把看到的线索列出来。
//! 4. LLM 兜底（[`llm`]）只在用户点“问 AI”时调用，不在这里自动跑。

pub mod big_items;
pub mod candidates;
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

/// 删了最坏会怎样（界面上的“后果标签”）。一项只挂一个，取最坏的那个。
///
/// 声明顺序就是严重程度（从最坏到最轻），所以 `Ord` 比较时越小越坏：
/// 找不回来 > 影响修复 > 需你确认 > 要重装 > 要重登录 > 功能变少 > 要重下载 > 删了变慢。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoseKind {
    /// 删了就没了 / 可能丢本机独有的内容。
    Irreversible,
    /// 以后卸载 / 修复 / 回滚会失败或要重新下安装包。
    Repair,
    /// 规则自己也说不准，或者里面混着能删和不能删的。
    Unsure,
    /// 删了这个工具 / 扩展就用不了，要重新装。
    Reinstall,
    /// 要重新登录、丢本地设置（数据在云端）。
    Relogin,
    /// 某个功能会关掉或暂时不能用。
    Feature,
    /// 能下回来，但很大、很慢。
    Redownload,
    /// 会自动重建，只是下次要等。
    Slower,
}

impl LoseKind {
    pub const ALL: [LoseKind; 8] = [
        LoseKind::Irreversible,
        LoseKind::Repair,
        LoseKind::Unsure,
        LoseKind::Reinstall,
        LoseKind::Relogin,
        LoseKind::Feature,
        LoseKind::Redownload,
        LoseKind::Slower,
    ];

    /// TOML / JSON 里的写法（snake_case）。
    pub fn as_str(self) -> &'static str {
        match self {
            LoseKind::Irreversible => "irreversible",
            LoseKind::Repair => "repair",
            LoseKind::Unsure => "unsure",
            LoseKind::Reinstall => "reinstall",
            LoseKind::Relogin => "relogin",
            LoseKind::Feature => "feature",
            LoseKind::Redownload => "redownload",
            LoseKind::Slower => "slower",
        }
    }

    pub fn parse(s: &str) -> Option<LoseKind> {
        LoseKind::ALL.into_iter().find(|k| k.as_str() == s.trim())
    }
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
    /// 删了最坏会怎样（review 档一定有；safe 可能没有；keep 可以没有）。
    #[serde(default)]
    pub lose_kind: Option<LoseKind>,
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

// ───────────────────────── 商店应用（MSIX）重定向的 AppData ─────────────────────────

/// `%LOCALAPPDATA%\Packages\<包>\LocalCache\{Local,Roaming,LocalLow}\…` 其实是“打包成商店应用的桌面程序
/// （以及它启动的子程序）往 AppData 写的新文件”，Windows 把它们存在应用包里、同时在真实 AppData 位置显示同一份
/// （learn.microsoft.com/windows/msix/desktop/desktop-to-uwp-behind-the-scenes）。
/// 解释这种路径时按它对应的真实 AppData 路径去匹配规则，而不是笼统地说成“某商店应用的数据”。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsixRedirect {
    /// 包文件夹名（如 `Claude_pzs8sxrjxfjjc`）。
    pub package: String,
    /// 对应的真实 AppData 路径（保留原大小写）。
    pub mapped: String,
    /// `AppData\Local` / `AppData\Roaming` / `AppData\LocalLow`。
    pub area: &'static str,
}

impl MsixRedirect {
    /// 包名里的应用名：`Publisher.AppName_发布者哈希` → `AppName`；`Claude_xxx` → `Claude`。
    pub fn app_name(&self) -> String {
        let first = self.package.split('_').next().unwrap_or(&self.package);
        first.rsplit_once('.').map(|(_, a)| a).unwrap_or(first).to_string()
    }
}

impl ExplainCtx {
    /// 小写段是否形如 `<LOCALAPPDATA>\packages\<包>\localcache\<区>…`；返回 (LOCALAPPDATA 段数, 区对应的根变量, 子路径, 区名)。
    fn msix_parts(&self, segs: &[String]) -> Option<(usize, &'static str, &'static str, &'static str)> {
        let base = split_lower(self.env.get("LOCALAPPDATA")?);
        let n = base.len();
        if segs.len() < n + 4 || segs[..n] != base[..] || segs[n] != "packages" || segs[n + 2] != "localcache" {
            return None;
        }
        match segs[n + 3].as_str() {
            "local" => Some((n, "LOCALAPPDATA", "", "AppData\\Local")),
            "roaming" => Some((n, "APPDATA", "", "AppData\\Roaming")),
            "locallow" => Some((n, "USERPROFILE", "\\AppData\\LocalLow", "AppData\\LocalLow")),
            _ => None,
        }
    }

    /// 路径在某个商店应用的 LocalCache\{Local,Roaming,LocalLow} **之下**（不含这三个文件夹本身）时，给出对应的真实 AppData 路径。
    pub fn msix_redirect(&self, path: &str) -> Option<MsixRedirect> {
        let segs = split_lower(path);
        let (n, var, sub, area) = self.msix_parts(&segs)?;
        if segs.len() < n + 5 {
            return None;
        }
        let orig: Vec<&str> = path.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
        if orig.len() != segs.len() {
            return None;
        }
        let root = self.env.get(var)?;
        Some(MsixRedirect { package: orig[n + 1].to_string(), mapped: format!("{root}{sub}\\{}", orig[n + 4..].join("\\")), area })
    }

    /// 小写段正好是某个 LocalCache\{Local,Roaming,LocalLow} 文件夹本身时，返回它对应的真实 AppData 根（汇总遍历用）。
    fn msix_area_root(&self, segs: &[String]) -> Option<String> {
        let (n, var, sub, _) = self.msix_parts(segs)?;
        if segs.len() != n + 4 {
            return None;
        }
        Some(format!("{}{sub}", self.env.get(var)?))
    }

    /// 沿祖先链从近到远找第一个命中 `scope = tree` 规则的文件夹；商店应用重定向的路径按真实 AppData 路径匹配，
    /// 而且只从同样是重定向的祖先继承（不会越过 LocalCache 边界继承“某商店应用的数据”）。
    pub fn inherited(&self, ancestors: &[&Node]) -> Option<InheritedHit> {
        let mut started_mapped: Option<bool> = None;
        for a in ancestors.iter().rev() {
            if a.path.is_empty() {
                continue;
            }
            let m = self.msix_redirect(&a.path);
            match started_mapped {
                None => started_mapped = Some(m.is_some()),
                Some(true) if m.is_none() => return None,
                _ => {}
            }
            let segs = split_lower(m.as_ref().map(|x| x.mapped.as_str()).unwrap_or(&a.path));
            if let Some(ri) = self.rules.best_tree_rule(&segs, a) {
                return Some(InheritedHit { rule: ri, from_path: a.path.clone() });
            }
        }
        None
    }
}

/// 解释一个节点。继承命中会自己沿 `ancestors` 往上找。
pub fn explain(t: &Target, ctx: &ExplainCtx) -> Explanation {
    let inherited = ctx.inherited(t.ancestors);
    explain_with_inherited(t, inherited.as_ref(), ctx)
}

/// 同 [`explain`]，但继承命中由调用方预先算好（列目录时对所有子项只算一次，用 [`ExplainCtx::inherited`]）。
pub fn explain_with_inherited(t: &Target, inherited: Option<&InheritedHit>, ctx: &ExplainCtx) -> Explanation {
    if t.node.kind != NodeKind::Rest {
        if let Some(m) = ctx.msix_redirect(t.path) {
            return explain_redirected(t, inherited, &m, ctx);
        }
    }
    explain_plain(t, inherited, ctx)
}

/// 商店应用重定向的路径：按真实 AppData 路径解释，再补一句“它其实是哪里、卸载时会怎样”。
fn explain_redirected(t: &Target, inherited: Option<&InheritedHit>, m: &MsixRedirect, ctx: &ExplainCtx) -> Explanation {
    // 只接受同样在 LocalCache 里面的上层规则，不越界继承“某商店应用的数据”。
    let inh = inherited.filter(|h| ctx.msix_redirect(&h.from_path).is_some());
    let mt = Target { node: t.node, path: &m.mapped, parent: t.parent, ancestors: t.ancestors };
    let mut e = explain_plain(&mt, inh, ctx);
    let app = m.app_name();
    e.notes.insert(
        0,
        format!(
            "它和「{}」是同一份文件：商店版「{app}」（以及它启动的其他程序，比如在它里面跑的命令行工具）往 {} 写的新文件，Windows 会存进这个应用包的 LocalCache，同时在原位置显示。所以这里按「{}」来说明。注意：卸载「{app}」时，Windows 会把这里的文件一起删掉。",
            m.mapped, m.area, m.mapped
        ),
    );
    if e.is_unknown() {
        e.clues.insert(0, format!("它其实是「{}」（商店版「{app}」重定向过来的 {}）", m.mapped, m.area));
    }
    e
}

fn explain_plain(t: &Target, inherited: Option<&InheritedHit>, ctx: &ExplainCtx) -> Explanation {
    let node = t.node;
    if node.kind == NodeKind::Rest {
        return explain_rest(t, inherited, ctx);
    }
    let segs = split_lower(t.path);
    if let Some(idx) = ctx.rules.best_exact(&segs, Some(node)) {
        return from_rule(&ctx.rules.rules[idx], t, &segs, None, ctx);
    }
    if let Some(h) = inherited {
        return from_rule(&ctx.rules.rules[h.rule], t, &segs, Some(&h.from_path), ctx);
    }
    heuristics::explain_unmatched(t, ctx)
}

/// 目录的“其余小文件合计”节点：跟随所在目录的说明。
fn explain_rest(t: &Target, inherited: Option<&InheritedHit>, ctx: &ExplainCtx) -> Explanation {
    // 所在目录是商店应用重定向的 AppData 时，按真实路径匹配，且不越界继承。
    let redirect = ctx.msix_redirect(t.path);
    let inherited = if redirect.is_some() { inherited.filter(|h| ctx.msix_redirect(&h.from_path).is_some()) } else { inherited };
    let segs = split_lower(redirect.as_ref().map(|m| m.mapped.as_str()).unwrap_or(t.path));
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
    // safe 规则因为落在个人文件夹下被降成 review 时，规则本身没写后果类型：按“需你确认”算。
    let lose_kind = match (tier, rule.lose_kind) {
        (Tier::Review, None) => Some(LoseKind::Unsure),
        (_, k) => k,
    };
    Explanation {
        tier,
        owner,
        category: rule.category,
        what: rule.what.clone(),
        why: rule.why.clone(),
        lose: rule.lose.clone(),
        lose_kind,
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
    walk_summary(tree, &mut segs, &active, None, ctx, &mut per_rule, &mut uncovered);

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
    ctx: &ExplainCtx,
    per_rule: &mut [u64],
    uncovered: &mut u64,
) {
    let rs = &ctx.rules;
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
                // 商店应用重定向的 AppData：里面的东西按真实 AppData 路径归档（和解释时一致）。
                if let Some(root) = ctx.msix_area_root(segs) {
                    let mut msegs = split_lower(&root);
                    let mactive: Vec<u32> = (0..rs.patterns.len() as u32)
                        .filter(|&i| {
                            let p = &rs.patterns[i as usize].1;
                            p.matches(&msegs) || p.could_match_below(&msegs)
                        })
                        .collect();
                    walk_summary(c, &mut msegs, &mactive, None, ctx, per_rule, uncovered);
                    segs.pop();
                    continue;
                }
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
                    walk_summary(c, segs, &child_active, child_inherited, ctx, per_rule, uncovered);
                }
                segs.pop();
            }
        }
    }
}
