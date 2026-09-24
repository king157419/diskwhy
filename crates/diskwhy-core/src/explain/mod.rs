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
pub mod tools;

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
    /// `how` 里有 `BuiltinTool` 时，工具叫什么（从 how_detail 认出来，见 [`tools::tool_name`]；
    /// 对应 PLAN 第 2 节的 `BuiltinTool(name)`）。由 [`enforce_guards`] 统一填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// 能腾出的字节（safe/review 且可删时 = 实际占用；keep 为 0）。
    pub gain_bytes: u64,
    /// “能腾出”一栏的大白话。
    pub gain_note: String,
    pub source: Source,
    /// 0~1。规则 1.0，启发式 0.5~0.7，未知 0。
    pub confidence: f32,
    pub refs: Vec<String>,
    /// 规则没有官方出处、只在本机观察过（界面上标“本机观察”）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub observed_only: bool,
    pub verified_on: Option<String>,
    /// 附加提示（继承、年龄、档位被下调等）。
    pub notes: Vec<String>,
    /// 线索（启发式 / 未知时列出看到了什么）。
    pub clues: Vec<String>,
    /// 护栏：这一整项**不能整块删**的原因（里面混着像是用户数据的东西、下层有“需判断 / 保留”的规则、
    /// 内容太多没看全、名字像缓存但格式不对）。这种项的档位已经降成 review，`how` 里可能仍有 delete
    /// （指“挑出可清理的部分再删”），所以阶段 D 的删除必须看这个字段，见 [`delete_block_reason`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<String>,
}

impl Explanation {
    pub fn is_unknown(&self) -> bool {
        self.source == Source::Unknown
    }

    /// 来自规则库（精确命中或继承上层）。
    pub fn is_rule(&self) -> bool {
        matches!(self.source, Source::Rule { .. })
    }

    pub fn summary(&self) -> ExplainSummary {
        ExplainSummary { tier: self.tier, owner: self.owner.clone(), source: self.source.clone(), what: self.what.clone() }
    }
}

/// 解释摘要：`diskwhy-cli scan --json` 给树里每个目录节点附上的那一小段
/// （档位、属于谁、说明从哪来、一句话）。完整解释用 `diskwhy-cli explain`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExplainSummary {
    pub tier: Tier,
    pub owner: Option<String>,
    pub source: Source,
    pub what: String,
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
    /// 往下找用户数据陷阱时最多看多少个节点（默认 [`heuristics::TRAP_VISIT`]；测试可调小）。
    pub trap_visit: usize,
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
        ExplainCtx { env, rules, registry, settings, protected, now: now_unix(), trap_visit: heuristics::TRAP_VISIT }
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
        let mut ctx = ExplainCtx::new(env, rules, registry, settings);
        // 个人文件夹的真实位置（被移到 D 盘等别处时也保护）。
        ctx.add_protected(&crate::win::known_personal_folders());
        (ctx, warnings)
    }

    /// 再加几个受保护目录（比如 `SHGetKnownFolderPath` 读到的“文档”真实位置）。
    pub fn add_protected(&mut self, paths: &[String]) {
        for p in paths {
            let segs = split_lower(p);
            if segs.len() >= 2 && !self.protected.contains(&segs) {
                self.protected.push(segs);
            }
        }
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
    e.tool = if e.how.contains(&How::BuiltinTool) { tools::tool_name(&e.how_detail, e.owner.as_deref()) } else { None };
    e
}

/// 这一项能不能被“删除”动作整块删掉；不能时返回原因（给阶段 D 的删除清单 / 批量删除用）。
///
/// 一律拦下：来源是 AI 或未认出；保留档；`how` 里没有 delete（要用工具清、要迁移、要手动处理）；
/// 护栏标了 [`Explanation::hold`]（整块删会连带用户数据）。review 档其余情况放行（用户逐项自己决定）。
pub fn delete_block_reason(e: &Explanation) -> Option<String> {
    match &e.source {
        Source::Unknown => return Some("还没认出来是什么，不能删".into()),
        Source::Llm { .. } => return Some("说明来自 AI 推测，不能作为删除依据".into()),
        _ => {}
    }
    if e.tier == Tier::Keep {
        return Some(format!("建议保留的东西不能删（{}）", e.what));
    }
    if !e.how.contains(&How::Delete) {
        return Some("这一项不该直接删，按“怎么腾”里写的办法处理".into());
    }
    e.hold.clone()
}

/// 能不能进“一键清理可清理项”这种批量删除：只有 safe、`how` 含 delete、没有被护栏扣下的。
pub fn batch_deletable(e: &Explanation) -> bool {
    e.tier == Tier::Safe && delete_block_reason(e).is_none()
}

/// 同 [`explain`]，但继承命中由调用方预先算好（列目录时对所有子项只算一次）。
pub fn explain_with_inherited(t: &Target, inherited: Option<&InheritedHit>, ctx: &ExplainCtx) -> Explanation {
    let node = t.node;
    if node.kind == NodeKind::Rest {
        return enforce_guards(guard_rule_hit(explain_rest(t, inherited, ctx), t, ctx), t.path, ctx);
    }
    let segs = split_lower(t.path);
    let redirect = msix_redirect(t.path, ctx);
    let exact = ctx.rules.best_exact(&segs, Some(node));
    // 链接 / 挂载点：不跟进去，本身不占空间（除非规则库专门写了它）。
    if node.reparse_skipped && exact.is_none() {
        return enforce_guards(heuristics::link_item(t), t.path, ctx);
    }
    let e = if let Some(idx) = exact {
        from_rule(&ctx.rules.rules[idx], t, &segs, None, ctx)
    } else if let Some(e) = redirect.as_ref().and_then(|r| explain_redirected(t, r, ctx)) {
        e
    } else if let (Some(r), Some(h)) = (redirect.as_ref(), inherited.filter(|h| seg_len(&h.from_path) <= redirect.as_ref().map(|r| r.real_base_len).unwrap_or(0))) {
        // 沙箱里重定向过来、按原位置又认不出的东西：不再沿用包规则的说法（“这个应用的登录信息和设置”），
        // 因为它很可能是这个应用启动的别的程序写的（第 1 轮评审第 1 条）。
        let _ = h;
        let mut e = heuristics::unknown(t, &heuristics::subtree_stats(t.node), ctx);
        e.what = "商店应用沙箱里的东西，还没认出是谁的".into();
        e.clues.insert(
            0,
            format!(
                "它在商店应用「{}」的沙箱里：本来要写到「{}」，被 Windows 挪到了这里。可能是这个应用自己的，也可能是它启动的别的程序（命令行工具、编译器等）写的",
                r.package, r.virtual_path
            ),
        );
        e
    } else if let Some(h) = inherited {
        from_rule(&ctx.rules.rules[h.rule], t, &segs, Some(&h.from_path), ctx)
    } else {
        heuristics::explain_unmatched(t, ctx)
    };
    enforce_guards(guard_rule_hit(e, t, ctx), t.path, ctx)
}

/// 路径的段数。
fn seg_len(p: &str) -> usize {
    split_lower(p).len()
}

/// 规则命中（精确、继承、沙箱重定向都算）之后，按内容再核一遍（第 1 轮安全评审第 1、4、8 条）：
///
/// 1. 规则写了 `verify = "chromium-disk-cache"`（`**` 通配的 Chromium 缓存规则）：命中的那个文件夹必须通过
///    和启发式一样严格的签名检查，否则 safe 降为 review；
/// 2. 自己就是用户数据（[`heuristics::trap_here`]）：改用那个说明（keep 或 review）；
/// 3. 在规则覆盖范围内的某个上层文件夹是用户数据（比如 Temp 里某个 git 仓库的 src）：同上；
/// 4. 下面有用户数据：大部分字节是 keep 级的（虚拟磁盘、聊天、conda 环境）→ 整个按 keep；
///    否则 safe 降为 review，并在“能腾多少”里分开写。
///
/// keep 档不动；启发式、AI、未知来源不经过这里（启发式只有签名严格的 Chromium 缓存能 safe）。
fn guard_rule_hit(mut e: Explanation, t: &Target, ctx: &ExplainCtx) -> Explanation {
    let Source::Rule { id, inherited_from, .. } = &e.source else { return e };
    if e.tier == Tier::Keep {
        return e;
    }
    let Some(rule) = ctx.rules.by_id(id) else { return e };
    let rule_what = rule.what.clone();
    let from = inherited_from.clone();
    // review 规则只在“继承来的、而且不是专门讲虚拟机的规则”时才查（商店版 WSL 的 LocalState 挂在
    // store-app-package 下面）；专门为这个位置写的 review 规则（比如 Claude 的虚拟机镜像）已经考虑过里面有什么。
    if e.tier == Tier::Review && (from.is_none() || rule.category == Category::Vm) {
        return e;
    }
    // “其余小文件合计”：扫描时记下的名字标志里有文档 / 虚拟磁盘 / 密钥，或者根本没记（第 2 轮安全评审 S1）。
    if t.node.kind == NodeKind::Rest {
        if let Some(tr) = heuristics::trap_here(t.path, t.node, e.tier, ctx) {
            let mut x = heuristics::trap_explanation(&tr, t, &rule_what, ctx);
            x.hold = Some("被合并显示的小文件里可能有你的文件".into());
            return x;
        }
        return e;
    }

    // 1. 严格核验
    if e.tier == Tier::Safe && rule.verify.as_deref() == Some("chromium-disk-cache") {
        let (mnode, mparent) = match &from {
            None => (Some(t.node), t.parent),
            Some(f) => {
                let i = t.ancestors.iter().position(|a| a.path.eq_ignore_ascii_case(f));
                (i.map(|i| t.ancestors[i]), i.and_then(|i| i.checked_sub(1)).map(|j| t.ancestors[j]))
            }
        };
        let ok = mnode.is_some_and(|n| heuristics::verify_chromium_disk_cache(n, mparent));
        if !ok {
            e.tier = Tier::Review;
            let (g, note) = gain(Tier::Review, &e.how, t.node.allocated);
            e.gain_bytes = g;
            e.gain_note = note;
            e.hold = Some("名字像 Chromium 缓存，但里面混着别的文件".into());
            e.notes.push(format!(
                "规则按名字认出这是 Chromium 类程序的缓存，但{}里的文件不全是 Chromium 缓存的标准格式，混着别的东西，所以标成“需要你判断”。",
                from.as_deref().map(|f| format!("「{f}」")).unwrap_or_else(|| "这个文件夹".into())
            ));
        }
    }

    // 2. 自己就是用户数据
    if let Some(tr) = heuristics::trap_here(t.path, t.node, e.tier, ctx) {
        return heuristics::trap_explanation(&tr, t, &rule_what, ctx);
    }
    // 3. 规则范围内的上层文件夹是用户数据
    if let Some(f) = &from {
        let root_len = seg_len(f);
        for a in t.ancestors.iter().filter(|a| !a.path.is_empty() && seg_len(&a.path) >= root_len) {
            if let Some(tr) = heuristics::trap_here(&a.path, a, e.tier, ctx) {
                let mut x = heuristics::trap_explanation(&tr, t, &rule_what, ctx);
                x.notes.insert(0, format!("它在「{}」里面，那里看起来是你的数据。", a.path));
                return x;
            }
        }
    }
    // 4. 下面有用户数据
    if t.node.kind == NodeKind::Dir {
        let (traps, truncated) = heuristics::traps_below(t.node, e.tier, ctx);
        if traps.is_empty() {
            if truncated && e.tier == Tier::Safe {
                // 看不全就往保守的方向走（第 2 轮安全评审 S1）：不再只加一句提示照样标“可清理”。
                e.tier = Tier::Review;
                let (g, note) = gain(Tier::Review, &e.how, t.node.allocated);
                e.gain_bytes = g;
                e.gain_note = note;
                e.hold = Some(format!("内容太多（超过 {} 项），没法确认里面全是可以清理的东西", ctx.trap_visit));
                e.notes.insert(
                    0,
                    format!(
                        "这里面的东西太多（超过 {} 项），DiskWhy 没能全部看一遍，没法确认里面没有混着你的数据，所以标成“需要你判断”。点进去看下一层会更准。",
                        ctx.trap_visit
                    ),
                );
            }
            return e;
        }
        let total: u64 = traps.iter().map(|x| x.bytes).sum();
        let keep_bytes: u64 = traps.iter().filter(|x| x.tier == Tier::Keep).map(|x| x.bytes).sum();
        let shown: Vec<String> = traps
            .iter()
            .take(3)
            .map(|x| {
                let rel = x.path.get(t.path.trim_end_matches('\\').len()..).unwrap_or(&x.path).trim_start_matches('\\');
                let what = match x.name {
                    "hidden-files" if !x.note.is_empty() => format!("，里面有{}", x.note),
                    "unchecked-files" => "，没记下文件名".to_string(),
                    "nested-rule" => x.rule.and_then(|i| ctx.rules.rules.get(i)).map(|r| format!("，{}", r.what)).unwrap_or_default(),
                    _ => String::new(),
                };
                format!("{}（{}{}）", rel, human_bytes(x.bytes), what)
            })
            .collect();
        let list = format!(
            "{}{}",
            shown.join("、"),
            if traps.len() > 3 { format!(" 等 {} 处", traps.len()) } else { String::new() }
        );
        if t.node.allocated > 0 && keep_bytes * 2 >= t.node.allocated {
            let biggest = traps.iter().filter(|x| x.tier == Tier::Keep).max_by_key(|x| x.bytes).unwrap();
            let mut x = heuristics::trap_explanation(biggest, t, &rule_what, ctx);
            x.what = format!("这个文件夹里大部分是：{}", x.what);
            x.notes.insert(0, format!("里面看起来是你的数据的地方：{list}。"));
            x.hold = Some(format!("里面有看起来是你的数据的东西：{list}"));
            return x;
        }
        if e.tier == Tier::Safe {
            e.tier = Tier::Review;
            let rest = t.node.allocated.saturating_sub(total);
            e.gain_bytes = rest;
            e.gain_note = format!(
                "其中约 {} 可以清理；另有 {} 处（共 {}）看起来是你的数据，别跟着一起删",
                human_bytes(rest),
                traps.len(),
                human_bytes(total)
            );
            e.notes.insert(
                0,
                format!("这里整体是可以清理的，但里面混着看起来是你的数据的东西：{list}。所以整个文件夹标成“需要你判断”，点进去逐项看。"),
            );
            e.hold = Some(format!("里面混着看起来是你的数据的东西：{list}"));
        } else {
            e.notes.push(format!("里面有看起来是你的数据的东西：{list}。"));
            e.hold = Some(format!("里面有看起来是你的数据的东西：{list}"));
        }
    }
    e
}

/// 商店应用（MSIX）沙箱重定向：打包的桌面应用（以及它启动的程序）在 `AppData` 里**新建**的文件和文件夹，
/// Windows 会写到 `%LOCALAPPDATA%\Packages\<包名>\LocalCache\{Local,Roaming,LocalLow}\…`，
/// 应用自己看到的还是 `AppData\…`（learn.microsoft.com：Understanding how packaged desktop apps run on Windows）。
/// 所以沙箱里的 `LocalCache\Local\pip\cache` 其实就是 pip 缓存，应该按 `%LOCALAPPDATA%\pip\cache` 来说明。
pub struct MsixRedirect {
    /// 包的文件夹名，如 `Claude_pzs8sxrjxfjjc`。
    pub package: String,
    /// 应用以为自己写到的位置，如 `C:\Users\me\AppData\Local\pip\cache`。
    pub virtual_path: String,
    /// `virtual_path` 在真实路径里从第几段开始对应（真实路径前 `real_base_len` 段 = `…\LocalCache\Local`）。
    pub real_base_len: usize,
    /// 虚拟路径里 `AppData\Local` 这一层有几段。
    pub virtual_base_len: usize,
}

pub fn msix_redirect(path: &str, ctx: &ExplainCtx) -> Option<MsixRedirect> {
    let local = ctx.env.get("LOCALAPPDATA")?;
    let lsegs = split_lower(local);
    let segs = split_lower(path);
    // …\Packages\<包>\LocalCache\<Local|Roaming|LocalLow>\<至少一层>
    if segs.len() < lsegs.len() + 5 || segs[..lsegs.len()] != lsegs[..] || segs[lsegs.len()] != "packages" || segs[lsegs.len() + 2] != "localcache" {
        return None;
    }
    let base = match segs[lsegs.len() + 3].as_str() {
        "local" => local.to_string(),
        "roaming" => ctx.env.get("APPDATA")?.to_string(),
        "locallow" => format!("{}\\AppData\\LocalLow", ctx.env.get("USERPROFILE")?),
        _ => return None,
    };
    let orig: Vec<&str> = path.trim_end_matches('\\').split('\\').filter(|s| !s.is_empty()).collect();
    if orig.len() != segs.len() {
        return None;
    }
    let real_base_len = lsegs.len() + 4;
    let virtual_path = format!("{}\\{}", base.trim_end_matches('\\'), orig[real_base_len..].join("\\"));
    Some(MsixRedirect { package: orig[lsegs.len() + 1].to_string(), virtual_path, real_base_len, virtual_base_len: split_lower(&base).len() })
}

/// 按重定向前的位置找规则：精确 → 上层 tree 规则 → 启发式。都认不出返回 None（调用方退回包自己的规则）。
fn explain_redirected(t: &Target, r: &MsixRedirect, ctx: &ExplainCtx) -> Option<Explanation> {
    let vsegs = split_lower(&r.virtual_path);
    let note = format!(
        "它在商店应用「{}」的沙箱里：Windows 把写到「{}」的新文件重定向到了这里（应用自己看到的还是原位置），所以按原位置来说明。",
        r.package, r.virtual_path
    );
    let mut e = if let Some(idx) = ctx.rules.best_exact(&vsegs, Some(t.node)) {
        from_rule(&ctx.rules.rules[idx], t, &vsegs, None, ctx)
    } else {
        // 上层：只看沙箱 LocalCache\<种类> 以下的祖先（再往上就是包自己的规则了）。
        let hit = t.ancestors.iter().rev().filter(|a| split_lower(&a.path).len() > r.real_base_len).find_map(|a| {
            let asegs = split_lower(&a.path);
            let mut v = vsegs[..r.virtual_base_len].to_vec();
            v.extend_from_slice(&asegs[r.real_base_len..]);
            ctx.rules.best_tree_exact(&v, Some(a)).map(|idx| (idx, a.path.clone()))
        });
        match hit {
            Some((idx, from)) => from_rule(&ctx.rules.rules[idx], t, &vsegs, Some(&from), ctx),
            None => {
                let h = heuristics::explain_unmatched(t, ctx);
                if h.is_unknown() {
                    return None;
                }
                h
            }
        }
    };
    e.notes.insert(0, note);
    Some(e)
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
        tool: None,
        gain_bytes,
        gain_note,
        source: Source::Rule {
            id: rule.id.clone(),
            group: rule.group.clone(),
            inherited_from: inherited_from.map(str::to_string),
        },
        confidence: 1.0,
        refs: rule.refs.clone(),
        observed_only: rule.observed_only,
        hold: None,
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
    /// 这条规则命中的最大的一处（规则直接命中的那个文件夹 / 文件），界面副标题用它的名字。
    #[serde(default)]
    pub top_path: String,
    #[serde(default)]
    pub top_path_bytes: u64,
}

/// 全盘“可清理”清单里的一项：一条 safe 规则命中的一处（文件夹或文件），按字节从大到小。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeItem {
    pub path: String,
    pub is_dir: bool,
    /// 这一处里算“可清理”的字节（护栏挑出来的部分已经扣掉）。
    pub bytes: u64,
    pub rule_id: String,
    pub owner: String,
    pub what: String,
    /// 规则的 how 里没有 delete：要用写明的工具清，别直接删文件夹。
    #[serde(default)]
    pub tool_only: bool,
    /// 这一处里面还混着别的东西（像是用户数据、下层“需判断 / 保留”的规则、没记名字的小文件），
    /// 所以它在列表里整体显示成“需判断”，可清理的只是其中一部分。
    #[serde(default)]
    pub mixed: bool,
}

/// 按规则库统计的各档合计（不含启发式；启发式和未知在列表里逐项看）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TierSummary {
    pub safe_bytes: u64,
    pub review_bytes: u64,
    pub keep_bytes: u64,
    /// 规则没覆盖到的字节。
    pub uncovered_bytes: u64,
    /// safe 里“要用工具清、不能直接删文件夹”的部分（规则的 how 里没有 delete，比如 conda clean、pnpm store prune）。
    #[serde(default)]
    pub safe_tool_bytes: u64,
    /// 规则本来是 safe / review，但内容核对后改记到 review / keep 的字节（缓存目录里像用户数据的部分、
    /// 不合 Chromium 缓存格式的通配命中）。已经算进 review_bytes / keep_bytes。
    #[serde(default)]
    pub guarded_bytes: u64,
    /// 护栏改记的每一类（按字节从大到小），方便核对是哪些东西被从 safe 里拿出来的。
    #[serde(default)]
    pub guard_items: Vec<RuleBytes>,
    /// 命中过至少一次的规则数 / 规则总数。
    pub rules_hit: usize,
    pub rules_total: usize,
    /// 各档里最大的几项（按规则聚合；护栏改记的部分 id 以 `guard:` 开头）。
    pub top_safe: Vec<RuleBytes>,
    pub top_review: Vec<RuleBytes>,
    pub top_keep: Vec<RuleBytes>,
    /// 全盘可清理清单（最多 [`SAFE_ITEMS_MAX`] 项，按字节从大到小）。界面上点“可清理”卡片时列出来。
    #[serde(default)]
    pub safe_items: Vec<SafeItem>,
    /// 可清理的一共有几处（清单可能只列了前面一部分）。
    #[serde(default)]
    pub safe_items_total: usize,
    pub duration_ms: u64,
}

/// 可清理清单最多列多少项。
pub const SAFE_ITEMS_MAX: usize = 300;

/// 商店应用沙箱 `…\Packages\<包>\LocalCache\{Local,Roaming,LocalLow}` 这一层对应的“应用以为的位置”（小写段）。
fn msix_virtual_base(segs: &[String], ctx: &ExplainCtx) -> Option<Vec<String>> {
    let lsegs = split_lower(ctx.env.get("LOCALAPPDATA")?);
    let n = lsegs.len();
    if segs.len() != n + 4 || segs[..n] != lsegs[..] || segs[n] != "packages" || segs[n + 2] != "localcache" {
        return None;
    }
    let base = match segs[n + 3].as_str() {
        "local" => ctx.env.get("LOCALAPPDATA")?.to_string(),
        "roaming" => ctx.env.get("APPDATA")?.to_string(),
        "locallow" => format!("{}\\AppData\\LocalLow", ctx.env.get("USERPROFILE")?),
        _ => return None,
    };
    Some(split_lower(&base))
}

/// 遍历时的累加器。
struct SumAcc<'a> {
    rs: &'a RuleSet,
    ctx: &'a ExplainCtx,
    /// (规则, 命中根路径下标) → 字节
    per_root: std::collections::HashMap<(usize, usize), u64>,
    /// 命中根：(路径, 是不是文件夹)
    roots: Vec<(String, bool)>,
    /// 里面被护栏挑出过东西、或下层有非 safe 规则的命中根（下标）。
    mixed: std::collections::HashSet<usize>,
    uncovered: u64,
    /// (护栏名, 改记到的档) → (字节, 最大一处路径, 最大一处字节)
    guard: std::collections::HashMap<(&'static str, Tier), (u64, String, u64)>,
}

impl SumAcc<'_> {
    fn add(&mut self, hit: Option<(usize, usize)>, bytes: u64) {
        match hit {
            Some(k) => *self.per_root.entry(k).or_default() += bytes,
            None => self.uncovered += bytes,
        }
    }
    fn root(&mut self, path: String, is_dir: bool) -> usize {
        self.roots.push((path, is_dir));
        self.roots.len() - 1
    }
    /// 记一处护栏改记，并把它所在的命中根标成“混着别的东西”。
    fn guard_in(&mut self, root: usize, name: &'static str, tier: Tier, bytes: u64, path: &str) {
        if root != usize::MAX {
            self.mixed.insert(root);
        }
        self.guard(name, tier, bytes, path);
    }
    fn guard(&mut self, name: &'static str, tier: Tier, bytes: u64, path: &str) {
        let g = self.guard.entry((name, tier)).or_insert((0, String::new(), 0));
        g.0 += bytes;
        if bytes > g.2 {
            g.1 = path.to_string();
            g.2 = bytes;
        }
    }
}

/// 遍历整棵树，按规则把每个字节归到 safe / review / keep / 未覆盖。
///
/// 目录命中规则后仍会往下走（子目录可能有更具体的规则，比如浏览器 Profile 是 keep，
/// 里面的 Cache 是 safe）；用模式前缀剪枝，不可能再命中的子树整块结算。
/// 和逐项解释用同一套护栏：safe / review 规则覆盖的地方里像用户数据的部分改记到 review / keep
/// （[`heuristics::trap_here`]）；`verify` 核不过的通配命中记到 review；商店应用沙箱按原位置找规则。
pub fn tier_summary(tree: &Node, ctx: &ExplainCtx) -> TierSummary {
    let start = std::time::Instant::now();
    let rs = &ctx.rules;
    let root_segs = split_lower(&tree.path);
    let active: Vec<u32> = (0..rs.patterns.len() as u32)
        .filter(|&i| {
            let p = &rs.patterns[i as usize].1;
            p.matches(&root_segs) || p.could_match_below(&root_segs)
        })
        .collect();
    let mut acc = SumAcc {
        rs,
        ctx,
        per_root: Default::default(),
        roots: Vec::new(),
        mixed: Default::default(),
        uncovered: 0,
        guard: Default::default(),
    };
    let mut segs = root_segs;
    walk_summary(tree, &mut segs, &active, None, &mut acc);

    let mut s = TierSummary { uncovered_bytes: acc.uncovered, rules_total: rs.rules.len(), ..Default::default() };
    // 按规则聚合，同时记下每条规则最大的一处。
    let mut per_rule: Vec<(u64, usize, u64)> = vec![(0, usize::MAX, 0); rs.rules.len()];
    for (&(ri, root), &b) in &acc.per_root {
        let r = &mut per_rule[ri];
        r.0 += b;
        if b > r.2 {
            r.1 = root;
            r.2 = b;
        }
    }
    // 全盘可清理清单：safe 规则的每一处命中。
    let mut safe_items: Vec<SafeItem> = acc
        .per_root
        .iter()
        .filter(|(&(ri, _), &b)| b > 0 && rs.rules[ri].tier == Tier::Safe)
        .filter_map(|(&(ri, root), &b)| {
            let (path, is_dir) = acc.roots.get(root)?.clone();
            let r = &rs.rules[ri];
            Some(SafeItem {
                path,
                is_dir,
                bytes: b,
                rule_id: r.id.clone(),
                owner: r.owner.clone(),
                what: r.what.clone(),
                tool_only: !r.how.contains(&How::Delete),
                mixed: acc.mixed.contains(&root),
            })
        })
        .collect();
    safe_items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    s.safe_items_total = safe_items.len();
    safe_items.truncate(SAFE_ITEMS_MAX);
    s.safe_items = safe_items;

    let mut items: Vec<RuleBytes> = Vec::new();
    for (i, &(b, root, rb)) in per_rule.iter().enumerate() {
        if b == 0 {
            continue;
        }
        let r = &rs.rules[i];
        s.rules_hit += 1;
        match r.tier {
            Tier::Safe => {
                s.safe_bytes += b;
                if !r.how.contains(&How::Delete) {
                    s.safe_tool_bytes += b;
                }
            }
            Tier::Review => s.review_bytes += b,
            Tier::Keep => s.keep_bytes += b,
        }
        let top_path = acc.roots.get(root).map(|r| r.0.clone()).unwrap_or_default();
        items.push(RuleBytes { id: r.id.clone(), owner: r.owner.clone(), what: r.what.clone(), tier: r.tier, bytes: b, top_path, top_path_bytes: rb });
    }
    for (&(name, tier), (b, path, pb)) in &acc.guard {
        s.guarded_bytes += b;
        match tier {
            Tier::Keep => s.keep_bytes += b,
            _ => s.review_bytes += b,
        }
        let (owner, what) = match name {
            "verify-failed" => ("护栏", "名字像 Chromium 缓存、但内容不全是缓存格式的文件夹"),
            "virtual-disk" => ("护栏", "缓存 / 临时文件夹里的虚拟磁盘"),
            "chat-data" => ("护栏", "缓存 / 临时文件夹里的聊天数据"),
            "conda-env" => ("护栏", "缓存 / 临时文件夹里的 conda 环境"),
            "source-repo" => ("护栏", "缓存 / 临时文件夹里的代码仓库"),
            "secret-key" => ("护栏", "缓存 / 临时文件夹里的密钥 / 密码库"),
            "hidden-files" => ("护栏", "合并显示的小文件里有文档 / 密钥 / 虚拟磁盘"),
            "unchecked-files" => ("护栏", "合并显示、没记下名字的小文件"),
            _ => ("护栏", "缓存 / 临时文件夹里的文档"),
        };
        let rb = RuleBytes {
            id: format!("guard:{name}"),
            owner: owner.into(),
            what: what.into(),
            tier,
            bytes: *b,
            top_path: path.clone(),
            top_path_bytes: *pb,
        };
        s.guard_items.push(rb.clone());
        items.push(rb);
    }
    s.guard_items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
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

/// 这条规则覆盖的地方要不要查用户数据陷阱：safe 要查；review 除了专门讲虚拟机的规则也查（只查虚拟磁盘）；keep 不查。
fn guarded(r: &Rule) -> bool {
    r.tier == Tier::Safe || (r.tier == Tier::Review && r.category != Category::Vm)
}

/// safe / review 规则覆盖、又没有更具体规则的子树：整块记给这条规则，但先把像用户数据的部分挑出来。
fn settle_guarded(node: &Node, hit: (usize, usize), tier: Tier, acc: &mut SumAcc) {
    for c in &node.children {
        match c.kind {
            NodeKind::Rest => match heuristics::rest_trap(c, tier) {
                Some((name, to, _)) => acc.guard_in(hit.1, name, to, c.allocated, &node.path),
                None => acc.add(Some(hit), c.allocated),
            },
            NodeKind::File => {
                // 文件的陷阱只看名字（聊天文件夹在上面的文件夹那一层就会被认出来）。
                match heuristics::file_trap(&c.name, tier) {
                    Some((name, to)) => acc.guard_in(hit.1, name, to, c.allocated, &format!("{}\\{}", node.path.trim_end_matches('\\'), c.name)),
                    None => acc.add(Some(hit), c.allocated),
                }
            }
            NodeKind::Dir => {
                if let Some(tr) = heuristics::trap_here(&c.path, c, tier, acc.ctx) {
                    acc.guard_in(hit.1, tr.name, tr.tier, c.allocated, &c.path);
                } else if c.children.is_empty() {
                    acc.add(Some(hit), c.allocated);
                } else {
                    settle_guarded(c, hit, tier, acc);
                }
            }
        }
    }
}

fn walk_summary(node: &Node, segs: &mut Vec<String>, active: &[u32], inherited: Option<(usize, usize)>, acc: &mut SumAcc) {
    let rs = acc.rs;
    let exact = rs.best_exact_among(active, segs, Some(node));
    // self 规则只说明文件夹本身，里面的东西不归它。
    let child_inherited = match exact {
        Some(i) if rs.rules[i].scope == Scope::Tree => {
            let r = acc.root(node.path.clone(), true);
            Some((i, r))
        }
        _ => inherited,
    };
    for c in &node.children {
        match c.kind {
            NodeKind::Rest => match child_inherited {
                Some((i, r)) if guarded(&rs.rules[i]) => match heuristics::rest_trap(c, rs.rules[i].tier) {
                    Some((name, to, _)) => acc.guard_in(r, name, to, c.allocated, &node.path),
                    None => acc.add(child_inherited, c.allocated),
                },
                _ => acc.add(child_inherited, c.allocated),
            },
            NodeKind::File => {
                segs.push(c.name.to_lowercase());
                let f = rs.best_exact_among(active, segs, Some(c));
                segs.pop();
                let hit = match f {
                    Some(i) => {
                        let r = acc.root(format!("{}\\{}", node.path.trim_end_matches('\\'), c.name), false);
                        Some((i, r))
                    }
                    None => child_inherited,
                };
                match hit {
                    Some((i, r)) if guarded(&rs.rules[i]) => {
                        // 单个文件：按名字认的陷阱（虚拟磁盘、密钥、文档）
                        let p = format!("{}\\{}", node.path.trim_end_matches('\\'), c.name);
                        match heuristics::file_trap(&c.name, rs.rules[i].tier) {
                            Some((name, to)) => acc.guard_in(r, name, to, c.allocated, &p),
                            None => acc.add(hit, c.allocated),
                        }
                    }
                    _ => acc.add(hit, c.allocated),
                }
            }
            NodeKind::Dir => {
                segs.push(c.name.to_lowercase());
                // 商店应用沙箱：LocalCache\Local 等下面的东西按“应用以为的位置”找规则。
                if let Some(mut vbase) = msix_virtual_base(segs, acc.ctx) {
                    let vactive: Vec<u32> = (0..rs.patterns.len() as u32)
                        .filter(|&i| {
                            let p = &rs.patterns[i as usize].1;
                            p.matches(&vbase) || p.could_match_below(&vbase)
                        })
                        .collect();
                    walk_summary(c, &mut vbase, &vactive, None, acc);
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
                let own = if child_active.is_empty() { None } else { rs.best_exact_among(&child_active, segs, Some(c)) };
                // safe 规则的地盘里，下层文件夹命中了 review / keep 规则（Temp\Rar$EX*、INetCache\Content.Outlook）：
                // 字节归下层规则（下面照常走），上层那一处在清单里标“混着别的东西”（第 2 轮安全评审 S2）。
                if let (Some(o), Some((pi, pr))) = (own, child_inherited) {
                    if rs.rules[pi].tier == Tier::Safe && rs.rules[o].tier != Tier::Safe && pr != usize::MAX {
                        acc.mixed.insert(pr);
                    }
                }
                // 这个文件夹里面归哪条规则管（tree 规则 = 自己；self 规则 = 沿用上层）。
                let eff = match own {
                    Some(i) if rs.rules[i].scope == Scope::Tree => Some((i, usize::MAX)),
                    _ => child_inherited,
                };
                // verify 核不过：整块记到 review。
                if let Some(i) = own {
                    let r = &rs.rules[i];
                    if r.tier == Tier::Safe && r.verify.as_deref() == Some("chromium-disk-cache") && !heuristics::verify_chromium_disk_cache(c, Some(node)) {
                        acc.guard_in(child_inherited.map(|h| h.1).unwrap_or(usize::MAX), "verify-failed", Tier::Review, c.allocated, &c.path);
                        segs.pop();
                        continue;
                    }
                }
                // 自己就是用户数据：整块改记。
                if let Some((i, _)) = eff {
                    let tier = rs.rules[i].tier;
                    if guarded(&rs.rules[i]) {
                        if let Some(tr) = heuristics::trap_here(&c.path, c, tier, acc.ctx) {
                            let root = match own {
                                Some(o) if rs.rules[o].scope == Scope::Tree => usize::MAX,
                                _ => child_inherited.map(|h| h.1).unwrap_or(usize::MAX),
                            };
                            acc.guard_in(root, tr.name, tr.tier, c.allocated, &c.path);
                            segs.pop();
                            continue;
                        }
                    }
                }
                if child_active.is_empty() || c.children.is_empty() {
                    let hit = match own {
                        Some(i) if rs.rules[i].scope == Scope::Tree => {
                            let r = acc.root(c.path.clone(), true);
                            Some((i, r))
                        }
                        _ => child_inherited,
                    };
                    match hit {
                        Some((i, _)) if guarded(&rs.rules[i]) && !c.children.is_empty() => {
                            settle_guarded(c, hit.unwrap(), rs.rules[i].tier, acc);
                        }
                        _ => acc.add(hit, c.allocated),
                    }
                } else {
                    walk_summary(c, segs, &child_active, child_inherited, acc);
                }
                segs.pop();
            }
        }
    }
}
