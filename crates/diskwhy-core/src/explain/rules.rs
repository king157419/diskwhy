//! 规则库：`rules/*.toml`，编译期嵌入，运行时可再加载 `%LOCALAPPDATA%\DiskWhy\rules\*.toml`。
//!
//! 每条规则：
//! ```toml
//! [[rule]]
//! id = "npm-cache"
//! owner = "npm"
//! category = "cache"      # cache | log | installer | build | model | userdata | system | vm | trash | app
//! tier = "safe"           # safe | review | keep
//! lose_kind = "slower"    # review 档必填：删了最坏会怎样（见下）
//! lose_short = "下次会慢一点"  # review 档必填：这一项删了具体会怎样，≤ 8 个字，界面上的标签
//! match = ["%LOCALAPPDATA%\\npm-cache"]   # %ENV%、* 和 ** 通配；大小写不敏感
//! what = "..."  why = "..."  lose = "..."
//! how = "delete"          # 或数组：delete | builtin_tool | migrate | manual | none
//! how_detail = ""
//! refs = ["https://..."]
//! verified_on = "2026-09-23"
//! # 可选：
//! scope = "tree"          # tree（默认，子孙也按它说明）| self（只说明这一层，比如 AppData 这种“容器”）
//! kind = "any"            # any（默认）| dir | file
//! when_child = ["Cache_Data"]   # 目录里必须有其中之一（名字，大小写不敏感）才算命中
//! owner_from_path = false # true 时用注册表 / 文件夹名推断具体是哪个软件
//! keep_latest = 1         # 旧版本规则：同一父文件夹里留最新的 N 个版本（见下）
//! min_age_days = 7        # 只删 N 天以上没改过的（临时文件夹用，见下）
//! ```
//!
//! `min_age_days = N`（N ≥ 1，可选，只能用在 `scope = tree`、`how` 含 `delete` 的规则上）：命中的文件夹 F 不再整个算能删，
//! 而是 F 下面每个**直接子项**单独看——文件看它自己的修改时间，子文件夹看它整棵子树里最新的修改时间——N 天内都没改过的
//! 才能删。清理页的“能腾出”只算这些旧的（按扫描树里的修改时间估）；预检和执行开始时按每个文件真实的修改时间再查一遍，
//! 子项里有一个文件是 N 天内改的、被占用、或者有程序正从里面运行，这个子项整个跳过（[`crate::action::preflight::skip_busy_units`]）。
//!
//! `keep_latest = N`（N ≥ 1，可选，“旧版本”规则用）：每个模式的**最后一段必须带通配**（`*` / `?`，不能是 `**`），
//! 同一个父文件夹里会命中好几个兄弟（每个版本一个）。同一个父文件夹里、最具体命中的规则**正是这一条**的兄弟中，
//! 下面这些算“正在用的版本”，一律当 keep（固定说法「正在用的版本」，不能删、不进清理页），其余的才按规则自己的
//! 档位和文案（一般是 safe、“旧版本……”）：
//! - 按版本号最高的 N 个（数字段按数字比：1.10 > 1.9，app-10.0.0 > app-2.0.0；先去掉兄弟们共同的前缀 / 后缀，
//!   剩下的必须以 `1.2`、`v1.2.3`、`1.2.3-beta.1` 这样至少两段数字开头；正式版高于同号的预发布版）；
//! - 有任何一个兄弟的名字不像版本号（哈希、GUID 之类）：改按修改时间，最新的 N 个；
//! - 另外，修改时间最新的那一个兄弟永远算“正在用”。
//!
//! 看不到兄弟（比如扫描根就是这个文件夹）时当“正在用”。正在用的版本文件夹里面的东西也跟着当 keep。
//! 删除时（预检和执行开始各查一次）旧版本文件夹里只要有一个文件被占用（Restart Manager）或 30 分钟内改过，
//! 整个文件夹都跳过，不会只删一半（[`crate::action::preflight::skip_busy_units`]）。
//!
//! `lose_kind`（界面上的后果标签，一项只挂一个，按最坏的选）：
//! irreversible（找不回来）> repair（影响修复）> unsure（需你确认）> reinstall（要重装）>
//! relogin（要重登录）> feature（功能变少）> redownload（要重下载）> slower（删了变慢）。
//! `review` 档必须写；`safe` / `keep` 可以不写。写了不认识的值就是错误。
//!
//! `lose_short`（阶段 H）：清理页标签上显示的具体后果，比如“不能休眠”“微信要重新登录”，一看就懂；
//! 最多 [`LOSE_SHORT_MAX`] 个字（按字符数，空格和字母也算），太长就改短，界面上从不截断。
//! `lose_kind` 仍然决定标签颜色和排序。`review` 档必须写。
//!
//! 校验（[`validate`]）：字段齐全、枚举合法、id 唯一、模式以白名单环境变量开头、
//! `safe` 不许落在桌面/文档/图片/视频/音乐/下载/OneDrive 之下、`keep` 不许 `how = delete`、
//! `builtin_tool`/`migrate` 必须写 `how_detail`、`verified_on` 是 YYYY-MM-DD、
//! `review` 必须写合法的 `lose_kind`。

use super::env::{self, EnvMap};
use super::pattern::{split_lower, Pattern};
use super::{Category, How, LoseKind, Tier};
use crate::model::{Node, NodeKind};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

/// `lose_short` 最多几个字（按字符数）。
pub const LOSE_SHORT_MAX: usize = 8;

/// 编译期嵌入的规则文件（文件名即分组名）。
pub const EMBEDDED: &[(&str, &str)] = &[
    ("windows-system", include_str!("../../../../rules/windows-system.toml")),
    ("browsers", include_str!("../../../../rules/browsers.toml")),
    ("dev-caches", include_str!("../../../../rules/dev-caches.toml")),
    ("ai-models", include_str!("../../../../rules/ai-models.toml")),
    ("cn-apps", include_str!("../../../../rules/cn-apps.toml")),
    ("misc", include_str!("../../../../rules/misc.toml")),
    ("old-versions", include_str!("../../../../rules/old-versions.toml")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    Tree,
    #[serde(rename = "self")]
    SelfOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KindFilter {
    #[default]
    Any,
    Dir,
    File,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(How),
    Many(Vec<How>),
}

/// TOML 里的一条规则（原样）。所有必填字段没有默认值，缺了反序列化就失败。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDef {
    pub id: String,
    pub owner: String,
    pub category: Category,
    pub tier: Tier,
    #[serde(rename = "match")]
    pub patterns: Vec<String>,
    pub what: String,
    pub why: String,
    pub lose: String,
    /// 删了最坏会怎样（字符串，校验时转成 [`LoseKind`]，不认识的值报错）。
    #[serde(default)]
    pub lose_kind: Option<String>,
    /// 这一项删了具体会怎样（≤ 8 个字，界面标签）。review 档必填。
    #[serde(default)]
    pub lose_short: Option<String>,
    how: OneOrMany,
    pub how_detail: String,
    pub refs: Vec<String>,
    pub verified_on: String,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub kind: KindFilter,
    #[serde(default)]
    pub when_child: Vec<String>,
    #[serde(default)]
    pub owner_from_path: bool,
    /// 旧版本规则：同一父文件夹里留最新的几个版本（见文件头）。
    #[serde(default)]
    pub keep_latest: Option<u32>,
    /// 只删 N 天以上没改过的直接子项（见文件头）。
    #[serde(default)]
    pub min_age_days: Option<u32>,
}

impl RuleDef {
    pub fn how(&self) -> Vec<How> {
        match &self.how {
            OneOrMany::One(h) => vec![*h],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    rule: Vec<RuleDef>,
}

/// 加载后的规则。
#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub group: String,
    pub owner: String,
    pub category: Category,
    pub tier: Tier,
    pub raw_patterns: Vec<String>,
    pub what: String,
    pub why: String,
    pub lose: String,
    pub lose_kind: Option<LoseKind>,
    pub lose_short: Option<String>,
    pub how: Vec<How>,
    pub how_detail: String,
    pub refs: Vec<String>,
    pub verified_on: String,
    pub scope: Scope,
    pub kind: KindFilter,
    pub when_child: Vec<String>,
    pub owner_from_path: bool,
    /// 旧版本规则：同一父文件夹里留最新的几个版本（None = 普通规则）。
    pub keep_latest: Option<u32>,
    /// 只删 N 天以上没改过的直接子项（None = 不看年龄）。
    pub min_age_days: Option<u32>,
}

/// 规则集合 + 展平的已编译模式表。
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    /// (规则下标, 模式)。只包含当前机器上环境变量都存在的模式。
    pub patterns: Vec<(usize, Pattern)>,
}

/// 继承命中：上层某个文件夹命中了 `scope = tree` 的规则。
#[derive(Debug, Clone)]
pub struct InheritedHit {
    pub rule: usize,
    pub from_path: String,
}

const PROTECTED_UNDER_PROFILE: &[&str] = &["desktop", "documents", "pictures", "videos", "music", "downloads"];

/// 某个 safe 模式是否会落到个人文件夹里（按原始模式静态判断）。
fn safe_pattern_hits_personal(raw: &str) -> bool {
    let segs = split_lower(raw);
    let Some(first) = segs.first() else { return true };
    let base_is_profile = first == "%userprofile%";
    let base_is_public = first == "%public%";
    if !base_is_profile && !base_is_public {
        return false;
    }
    match segs.get(1) {
        None => true, // 整个用户目录
        Some(s) => {
            s.contains('*')
                || s.contains('?')
                || PROTECTED_UNDER_PROFILE.contains(&s.as_str())
                || s.starts_with("onedrive")
        }
    }
}

fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && (1..=12).contains(&s[5..7].parse::<u32>().unwrap_or(0))
        && (1..=31).contains(&s[8..10].parse::<u32>().unwrap_or(0))
}

/// 校验一条规则，返回所有问题（空 = 通过）。`seen_ids` 用于查重。
pub fn validate(def: &RuleDef, seen_ids: &mut HashSet<String>) -> Vec<String> {
    let mut e = Vec::new();
    let id = def.id.trim();
    if id.is_empty() {
        e.push("id 为空".into());
    } else if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        e.push(format!("id `{id}` 只能用小写字母、数字和 -"));
    } else if !seen_ids.insert(id.to_string()) {
        e.push(format!("id `{id}` 重复"));
    }
    for (name, v) in [("owner", &def.owner), ("what", &def.what), ("why", &def.why), ("lose", &def.lose)] {
        if v.trim().is_empty() {
            e.push(format!("{name} 为空"));
        }
    }
    match def.lose_kind.as_deref() {
        Some(k) if LoseKind::parse(k).is_none() => {
            let all: Vec<&str> = LoseKind::ALL.iter().map(|x| x.as_str()).collect();
            e.push(format!("lose_kind `{k}` 不认识（只能是 {}）", all.join(" / ")));
        }
        None if def.tier == Tier::Review => e.push("review 档必须写 lose_kind（删了最坏会怎样）".into()),
        _ => {}
    }
    match def.lose_short.as_deref().map(str::trim) {
        Some("") | None if def.tier == Tier::Review => e.push("review 档必须写 lose_short（删了具体会怎样，≤ 8 个字）".into()),
        Some(s) if s.chars().count() > LOSE_SHORT_MAX => {
            e.push(format!("lose_short `{s}` 有 {} 个字，最多 {LOSE_SHORT_MAX} 个（改短，界面不截断）", s.chars().count()))
        }
        _ => {}
    }
    if def.category == Category::Unknown {
        e.push("category 不能是 unknown".into());
    }
    if def.patterns.is_empty() {
        e.push("match 为空".into());
    }
    let how = def.how();
    if how.is_empty() {
        e.push("how 为空".into());
    }
    if def.tier == Tier::Keep && how.contains(&How::Delete) {
        e.push("keep 档的 how 不能包含 delete".into());
    }
    if (how.contains(&How::BuiltinTool) || how.contains(&How::Migrate)) && def.how_detail.trim().is_empty() {
        e.push("how 含 builtin_tool / migrate 时必须写 how_detail".into());
    }
    if !valid_date(def.verified_on.trim()) {
        e.push(format!("verified_on `{}` 不是 YYYY-MM-DD", def.verified_on));
    }
    for r in &def.refs {
        if !(r.starts_with("https://") || r.starts_with("http://")) {
            e.push(format!("refs 里 `{r}` 不是网址"));
        }
    }
    if !def.when_child.is_empty() && def.kind == KindFilter::File {
        e.push("when_child 只能用于目录规则".into());
    }
    if let Some(n) = def.keep_latest {
        if n == 0 {
            e.push("keep_latest 至少是 1（至少留一个正在用的版本）".into());
        }
        if def.tier == Tier::Keep {
            e.push("keep 档不用写 keep_latest（本来就都不删）".into());
        }
        for p in &def.patterns {
            let last = split_lower(p).pop().unwrap_or_default();
            if last == "**" || !(last.contains('*') || last.contains('?')) {
                e.push(format!("keep_latest 的模式 `{p}` 最后一段必须带通配（* 或 ?，不能是 **）：一个版本一个兄弟"));
            }
        }
    }
    if let Some(n) = def.min_age_days {
        if n == 0 {
            e.push("min_age_days 至少是 1".into());
        }
        if def.scope != Scope::Tree {
            e.push("min_age_days 只能用在 scope = tree 的规则上（按里面每个子项的年龄挑）".into());
        }
        if def.kind == KindFilter::File {
            e.push("min_age_days 只能用于文件夹规则".into());
        }
        if !how.contains(&How::Delete) {
            e.push("min_age_days 只对能直接删的规则有意义（how 要含 delete）".into());
        }
        if def.keep_latest.is_some() {
            e.push("min_age_days 和 keep_latest 不能写在同一条规则上".into());
        }
    }
    for p in &def.patterns {
        if p.contains("..") {
            e.push(format!("模式 `{p}` 不能含 .."));
        }
        match env::leading_var(p) {
            None => e.push(format!("模式 `{p}` 必须以 %变量% 开头（允许的根见 env::ALLOWED_VARS）")),
            Some(v) if !env::is_allowed_var(v) => e.push(format!("模式 `{p}` 的根变量 %{v}% 不在允许列表")),
            _ => {}
        }
        for v in env::vars_in(p) {
            if !env::is_allowed_var(v) {
                e.push(format!("模式 `{p}` 用了不允许的变量 %{v}%"));
            }
        }
        if def.tier == Tier::Safe && safe_pattern_hits_personal(p) {
            e.push(format!("safe 档的模式 `{p}` 落在了桌面/文档/图片/视频/音乐/下载/OneDrive 等个人文件夹下"));
        }
    }
    e
}

impl RuleSet {
    /// 从若干 (分组名, TOML 文本) 加载。校验不过的规则不加载，错误全部返回。
    pub fn from_sources(sources: &[(&str, &str)], env: &EnvMap) -> (RuleSet, Vec<String>) {
        let mut rs = RuleSet::default();
        let mut seen = HashSet::new();
        let errors = rs.add_sources(sources, env, &mut seen);
        (rs, errors)
    }

    /// 内置规则。
    pub fn embedded(env: &EnvMap) -> (RuleSet, Vec<String>) {
        Self::from_sources(EMBEDDED, env)
    }

    fn add_sources(&mut self, sources: &[(&str, &str)], env: &EnvMap, seen: &mut HashSet<String>) -> Vec<String> {
        let mut errors = Vec::new();
        for (group, text) in sources {
            let file: RuleFile = match toml::from_str(text) {
                Ok(f) => f,
                Err(err) => {
                    errors.push(format!("[{group}] TOML 解析失败：{err}"));
                    continue;
                }
            };
            for def in file.rule {
                let errs = validate(&def, seen);
                if !errs.is_empty() {
                    for x in errs {
                        errors.push(format!("[{group}] 规则 `{}`：{x}", def.id));
                    }
                    continue;
                }
                self.push(def, group, env);
            }
        }
        errors
    }

    fn push(&mut self, def: RuleDef, group: &str, env: &EnvMap) {
        let idx = self.rules.len();
        let how = def.how();
        let rule = Rule {
            id: def.id.trim().to_string(),
            group: group.to_string(),
            owner: def.owner,
            category: def.category,
            tier: def.tier,
            raw_patterns: def.patterns.clone(),
            what: def.what,
            why: def.why,
            lose: def.lose,
            lose_kind: def.lose_kind.as_deref().and_then(LoseKind::parse),
            lose_short: def.lose_short.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            how,
            how_detail: def.how_detail,
            refs: def.refs,
            verified_on: def.verified_on,
            scope: def.scope,
            kind: def.kind,
            when_child: def.when_child,
            owner_from_path: def.owner_from_path,
            keep_latest: def.keep_latest,
            min_age_days: def.min_age_days,
        };
        // 环境变量在本机不存在的模式直接跳过（比如 32 位系统没有 ProgramFiles(x86)）。
        for raw in &rule.raw_patterns {
            if let Ok(expanded) = env.expand(raw) {
                self.patterns.push((idx, Pattern::compile(&expanded)));
            }
        }
        self.rules.push(rule);
    }

    /// 加载用户自定义规则目录（`*.toml`）。不存在就什么都不做。返回警告。
    pub fn load_user_dir(&mut self, dir: &Path, env: &EnvMap) -> Vec<String> {
        let mut warnings = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else { return warnings };
        let mut seen: HashSet<String> = self.rules.iter().map(|r| r.id.clone()).collect();
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml"))).collect();
        files.sort();
        for f in files {
            match std::fs::read_to_string(&f) {
                Ok(text) => {
                    let group = format!("user:{}", f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                    let errs = self.add_sources(&[(group.as_str(), text.as_str())], env, &mut seen);
                    warnings.extend(errs);
                }
                Err(e) => warnings.push(format!("读取 {} 失败：{e}", f.display())),
            }
        }
        warnings
    }

    fn rule_accepts(&self, idx: usize, node: Option<&Node>) -> bool {
        let r = &self.rules[idx];
        let Some(n) = node else { return r.kind == KindFilter::Any && r.when_child.is_empty() };
        match r.kind {
            KindFilter::Dir if n.kind != NodeKind::Dir => return false,
            KindFilter::File if n.kind != NodeKind::File => return false,
            _ => {}
        }
        if !r.when_child.is_empty() {
            if n.kind != NodeKind::Dir {
                return false;
            }
            return n.children.iter().any(|c| r.when_child.iter().any(|w| w.eq_ignore_ascii_case(&c.name)));
        }
        true
    }

    /// 精确命中（所有模式里挑最具体的）。
    pub fn best_exact(&self, segs: &[String], node: Option<&Node>) -> Option<usize> {
        let mut best: Option<(u32, usize)> = None;
        for (ri, p) in &self.patterns {
            if (best.is_none() || p.specificity > best.unwrap().0) && p.matches(segs) && self.rule_accepts(*ri, node) {
                best = Some((p.specificity, *ri));
            }
        }
        best.map(|b| b.1)
    }

    /// 同上，但只看 `active` 里的模式下标（遍历剪枝用）。
    pub fn best_exact_among(&self, active: &[u32], segs: &[String], node: Option<&Node>) -> Option<usize> {
        let mut best: Option<(u32, usize)> = None;
        for &i in active {
            let (ri, p) = &self.patterns[i as usize];
            if (best.is_none() || p.specificity > best.unwrap().0) && p.matches(segs) && self.rule_accepts(*ri, node) {
                best = Some((p.specificity, *ri));
            }
        }
        best.map(|b| b.1)
    }

    /// 沿祖先链从近到远找第一个命中 `scope = tree` 规则的文件夹。
    ///
    /// 只看原始路径；需要考虑商店应用（MSIX）重定向时用 `ExplainCtx::inherited`。
    pub fn best_inherited(&self, ancestors: &[&Node]) -> Option<InheritedHit> {
        for a in ancestors.iter().rev() {
            if a.path.is_empty() {
                continue;
            }
            if let Some(ri) = self.best_tree_rule(&split_lower(&a.path), a) {
                return Some(InheritedHit { rule: ri, from_path: a.path.clone() });
            }
        }
        None
    }

    /// 某个文件夹（按 `segs` 匹配）精确命中的、`scope = tree` 的最具体规则。
    pub fn best_tree_rule(&self, segs: &[String], node: &Node) -> Option<usize> {
        let mut best: Option<(u32, usize)> = None;
        for (ri, p) in &self.patterns {
            if self.rules[*ri].scope != Scope::Tree {
                continue;
            }
            if (best.is_none() || p.specificity > best.unwrap().0) && p.matches(segs) && self.rule_accepts(*ri, Some(node)) {
                best = Some((p.specificity, *ri));
            }
        }
        best.map(|b| b.1)
    }

    pub fn by_id(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }
}
