//! 清理候选（阶段 F）：扫描完就把“能清理 / 需要你判断”的东西按规则（或启发式）分好组，
//! 清理页直接列出来，不用用户自己一个个加进清单。
//!
//! 规则：
//! - 规则候选：`safe` / `review` 档、`scope = tree`、`how` 不只是 `none` 的规则，在它**精确命中**的文件夹 / 文件处
//!   出一个候选路径（和解释、预检时 `best_exact` 的结果一致，所以预检的“规则一致性”能通过）。
//! - 能直接删（`how` 含 `delete`、节点本身没有“系统保留 / 映射 / 无权限 / 云占位”、保护路径没拦）而且
//!   **里面没有别的更严格的规则**（keep、不能直接删的、或 safe 里面套着 review）时，整个文件夹就是一个候选，
//!   不再往里看；里面套着别的可删规则时，它们的字节算进这个候选（删这个文件夹时它们本来就会一起删）。
//! - 里面有更严格的东西、或者本身不能直接删：只记它自己的那部分字节（不含里面别的规则命中的），标成不能直接删，
//!   再往里找更小的候选。
//! - 启发式候选：规则完全没覆盖到的地方，名字像 `node_modules` / `target` / `bin` / `obj` 或里面有
//!   `.rustc_info.json` 的文件夹，按启发式解释；结果是可删的 `rust-target` / `node-modules` / `dotnet-build`
//!   时出候选（review 档，默认不选）。
//! - 商店应用重定向的 LocalCache 区域不列（那里的路径预检时按真实 AppData 路径解释，规则一致性对不上）；
//!   例外：`keep_latest`（旧版本）规则自己写了 LocalCache 路径的模式时，只按这些模式往里找（比如商店版 Claude 自带的
//!   Claude Code 旧版本），规则一致性按写明的模式对得上。
//! - `keep_latest` 规则里“正在用的版本”（[`ExplainCtx::is_current_version`]）当 keep：不出候选，外层文件夹里有它也算
//!   “里面有更严格的”。
//!
//! 同一条规则 / 启发式的所有路径合成一组，界面上一行。

use super::pattern::split_lower;
use super::rules::Scope;
use super::{explain, Category, ExplainCtx, How, LoseKind, Source, Target, Tier};
use crate::fs::join;
use crate::model::{Node, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 候选里的一个路径。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidatePath {
    pub path: String,
    pub bytes: u64,
    /// 能不能直接删（能的话清理页可以勾选，预检时会再判断一遍）。
    pub deletable: bool,
    /// 硬拦（保护路径 / 系统保留 / 映射 / 读不了 / 云占位 / 硬链接）的短原因（≤ 6 个字）：这种路径怎么都不能勾。
    /// None 而且 `deletable = false`：规则说不建议直接删（要手动 / 用自带工具 / 里面有更严格的），界面上可以
    /// 二次确认后强制勾选（预检时按 `forced` 处理，只进回收站 / 暂存区）。
    #[serde(default)]
    pub hard: Option<String>,
}

/// 一组候选（同一条规则或同一种启发式）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateGroup {
    /// 规则 id，或 `h:<启发式名>`。
    pub key: String,
    /// 规则候选才有；预检时作为 expect_rule 传回去。
    pub rule_id: Option<String>,
    /// 规则文件分组（windows-system / browsers / …），启发式为 `heuristic`。
    pub group: String,
    /// 启发式名字（规则候选为 None）。
    pub heuristic: Option<String>,
    /// 短名字（界面上一行的名字）。
    pub name: String,
    pub owner: String,
    pub category: Category,
    pub tier: Tier,
    pub lose_kind: Option<LoseKind>,
    /// 删了具体会怎样（≤ 8 个字，行上的标签；阶段 H）。
    #[serde(default)]
    pub lose_short: Option<String>,
    pub how: Vec<How>,
    pub what: String,
    pub lose: String,
    pub how_detail: String,
    /// 所有路径合计。
    pub bytes: u64,
    /// 其中能直接删的合计。
    pub deletable_bytes: u64,
    pub paths: Vec<CandidatePath>,
}

/// 启发式候选的名字（界面短名）。
const HEURISTIC_NAMES: &[(&str, &str)] = &[("rust-target", "Rust 编译产物"), ("node-modules", "node_modules"), ("dotnet-build", ".NET 编译产物")];

/// 分类的中文短后缀（拼成“npm 缓存”这种短名字）。
fn category_suffix(c: Category) -> &'static str {
    match c {
        Category::Cache => "缓存",
        Category::Log => "日志",
        Category::Installer => "安装包",
        Category::Build => "编译产物",
        Category::Model => "模型",
        Category::Userdata => "数据",
        Category::System => "系统文件",
        Category::Vm => "虚拟机",
        Category::Trash => "",
        Category::App => "程序",
        Category::Unknown => "",
    }
}

/// 内置规则在清理页上的短名字（界面标签，不是规则文案；没列的用 [`short_name`] 从 owner 拼）。
const RULE_LABELS: &[(&str, &str)] = &[
    ("huggingface-hub", "Hugging Face 模型"),
    ("huggingface-datasets", "Hugging Face 数据集"),
    ("ollama-models", "Ollama 模型"),
    ("ollama-local-data", "Ollama 日志和更新包"),
    ("ollama-updates", "Ollama 更新包"),
    ("torch-hub", "PyTorch 模型"),
    ("modelscope-cache", "魔搭模型"),
    ("insightface-models", "InsightFace 模型"),
    ("paddle-models", "飞桨模型"),
    ("lm-studio-models", "LM Studio 模型"),
    ("whisper-models", "Whisper 模型"),
    ("keras-models", "Keras 模型"),
    ("nvidia-ngx-models", "NVIDIA AI 模型"),
    ("chrome-cache", "Chrome 缓存"),
    ("chrome-on-device-model", "Chrome AI 模型"),
    ("edge-cache", "Edge 缓存"),
    ("chromium-browsers-cache", "浏览器缓存"),
    ("firefox-cache", "Firefox 缓存"),
    ("chromium-generic-cache", "网页缓存"),
    ("chromium-generic-code-cache", "脚本缓存"),
    ("chromium-generic-gpu-cache", "着色器缓存"),
    ("chromium-generic-sw-cache", "网页离线缓存"),
    ("crashpad-reports", "崩溃报告"),
    ("wechat-files-cache", "微信文件缓存"),
    ("wechat-radium", "微信小程序数据"),
    ("wechat-logs", "微信日志"),
    ("wechat-update", "微信更新包"),
    ("wechat-appdata", "微信程序数据"),
    ("qq-appdata", "QQ 程序数据"),
    ("tencent-logs", "腾讯软件日志"),
    ("wemeet-data", "腾讯会议数据"),
    ("feishu-update", "飞书更新包"),
    ("feishu-larkshell", "飞书本地数据"),
    ("feishu-sdk-storage", "飞书本地数据库"),
    ("feishu-logs", "飞书日志"),
    ("feishu-embedded-browser", "飞书内置浏览器"),
    ("dingtalk-data", "钉钉数据"),
    ("baidu-netdisk-ui-data", "百度网盘界面数据"),
    ("thunder-data", "迅雷数据"),
    ("netease-music-cache", "网易云音乐缓存"),
    ("netease-music-logs", "网易云音乐日志"),
    ("netease-music-data", "网易云音乐数据"),
    ("qqmusic-data", "QQ 音乐数据"),
    ("kugou-data", "酷狗音乐数据"),
    ("bilibili-data", "哔哩哔哩数据"),
    ("wps-roaming", "WPS 设置和缓存"),
    ("npm-cache", "npm 缓存"),
    ("npm-global", "npm 全局工具"),
    ("pnpm-store", "pnpm 仓库"),
    ("yarn-cache", "Yarn 缓存"),
    ("yarn-berry-cache", "Yarn 新版缓存"),
    ("bun-cache", "Bun 缓存"),
    ("playwright-browsers", "Playwright 浏览器"),
    ("electron-download-cache", "Electron 下载缓存"),
    ("pip-cache", "pip 缓存"),
    ("uv-cache", "uv 缓存"),
    ("uv-python-tools", "uv 工具和 Python"),
    ("conda-pkgs", "conda 包缓存"),
    ("cargo-registry", "Cargo 依赖缓存"),
    ("cargo-home", "Cargo 主目录"),
    ("rustup-toolchains", "Rust 工具链"),
    ("go-cache", "Go 缓存"),
    ("gradle-cache", "Gradle 缓存"),
    ("maven-repo", "Maven 仓库"),
    ("nuget-cache", "NuGet 缓存"),
    ("docker-logs", "Docker 日志"),
    ("vscode-family-cached", "编辑器缓存"),
    ("vscode-family-root", "编辑器数据"),
    ("vscode-family-extensions", "编辑器扩展"),
    ("jetbrains-caches", "JetBrains 缓存"),
    ("jetbrains-local", "JetBrains 本机数据"),
    ("visual-studio-cache", "VS 缓存"),
    ("visual-studio-package-cache", "VS 安装包"),
    ("claude-desktop-vm", "Claude 工作区"),
    ("claude-desktop-data", "Claude 桌面数据"),
    ("claude-code-data", "Claude Code 数据"),
    ("codex-cli-data", "Codex 数据"),
    ("office-file-cache", "Office 文件缓存"),
    ("outlook-ost", "Outlook 离线邮件"),
    ("gpu-shader-cache", "显卡着色器缓存"),
    ("nvidia-app-updates", "NVIDIA 更新包"),
    ("nvidia-driver-installers", "NVIDIA 驱动安装包"),
    ("nvidia-app-data", "NVIDIA App 数据"),
    ("steam-caches", "Steam 缓存"),
    ("logitech-ghub", "G HUB 数据"),
    ("logitech-ghub-cache", "G HUB 缓存"),
    ("raspberry-pi-imager-cache", "树莓派镜像缓存"),
    ("adobe-media-cache", "Adobe 媒体缓存"),
    ("squirrel-temp", "安装临时文件"),
    ("discord-data", "Discord 数据"),
    ("codex-desktop-local", "Codex 桌面程序"),
    ("codex-runtimes-cache", "Codex 运行时"),
    ("codex-runtimes-previous", "Codex 旧运行时"),
    ("claude-code-old-versions", "Claude Code 旧版本"),
    ("codex-command-runner-old", "Codex 沙盒旧版本"),
    ("codex-cua-node-old", "Codex 旧 Node 运行时"),
    ("wps-office-old-versions", "WPS 旧版本"),
    ("aclos-game-recorder", "ACLOS 数据"),
    ("aclos-highlight-videos", "无畏时刻录像"),
    ("aclos-old-installer", "ACLOS 旧安装包"),
    ("electron-updater-cache", "软件更新包"),
    ("pandoc-local", "Pandoc"),
    ("windows-temp", "系统临时文件"),
    ("user-temp", "临时文件"),
    ("windows-update-download", "更新安装包"),
    ("delivery-optimization", "传递优化缓存"),
    ("windows-old", "旧版 Windows"),
    ("hiberfil", "休眠文件"),
    ("recycle-bin", "回收站"),
    ("thumbnail-cache", "缩略图缓存"),
    ("explorer-cache-dir", "资源管理器缓存"),
    ("inetcache", "IE 缓存"),
    ("directx-shader-cache", "DirectX 缓存"),
    ("windows-error-reports", "错误报告"),
    ("memory-dumps", "内存转储"),
    ("windows-logs", "Windows 日志"),
    ("package-cache", "安装包副本"),
    ("store-app-package", "商店应用数据"),
    ("store-app-temp", "商店应用临时文件"),
    ("root-tmp-dir", "C:\\tmp"),
    ("root-temp-dir", "C:\\Temp"),
    ("amd-installer-files", "AMD 驱动安装包"),
];

/// 规则 id 对应的界面短名（内置表里没有时返回 None）。
pub fn rule_label(id: &str) -> Option<&'static str> {
    RULE_LABELS.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
}

/// 规则的短名字：owner 去掉括号里的补充说明 + 分类后缀。
pub fn short_name(owner: &str, c: Category) -> String {
    let mut o = owner.trim().to_string();
    for open in ['（', '('] {
        if let Some(i) = o.find(open) {
            o.truncate(i);
        }
    }
    let o = o.trim();
    let suf = category_suffix(c);
    if suf.is_empty() || o.ends_with(suf) {
        o.to_string()
    } else if o.is_ascii() || o.chars().last().is_some_and(|ch| ch.is_ascii_alphanumeric()) {
        format!("{o} {suf}")
    } else {
        format!("{o}{suf}")
    }
}

fn rule_deletable_kind(how: &[How]) -> bool {
    how.contains(&How::Delete)
}

/// 节点本身能不能动（和预检的 node_gate 一致）。
fn node_ok(n: &Node) -> bool {
    node_hard(n).is_none()
}

/// 节点本身怎么都不能动的短原因（≤ 6 个字）；能动为 None。
pub fn node_hard(n: &Node) -> Option<&'static str> {
    if n.kind == NodeKind::Rest {
        Some("零散文件")
    } else if n.system_reserved {
        Some("系统保留")
    } else if n.mapped {
        Some("映射目录")
    } else if n.inaccessible {
        Some("没有权限")
    } else if n.cloud_placeholder {
        Some("云端文件")
    } else if n.hardlink_dup {
        Some("不腾空间")
    } else {
        None
    }
}

struct Walker<'a> {
    ctx: &'a ExplainCtx,
    blocked: &'a dyn Fn(&str) -> Option<String>,
    groups: HashMap<String, CandidateGroup>,
    /// 正在商店应用的 LocalCache 区域里（只找旧版本规则，不做启发式）。
    in_msix: bool,
}

/// 扫描树 → 清理候选（按 bytes 降序）。`blocked(path)` 返回 true 表示保护路径拦下（不能直接删）。
pub fn cleanup_candidates(tree: &Node, ctx: &ExplainCtx, blocked: &dyn Fn(&str) -> bool) -> Vec<CandidateGroup> {
    cleanup_candidates_with(tree, ctx, &|p: &str| blocked(p).then(|| "系统保护".to_string()))
}

/// 同 [`cleanup_candidates`]，`blocked(path)` 返回保护路径的短原因（≤ 6 个字，见 `protect::short_reason`），
/// 填进 [`CandidatePath::hard`]。
pub fn cleanup_candidates_with(tree: &Node, ctx: &ExplainCtx, blocked: &dyn Fn(&str) -> Option<String>) -> Vec<CandidateGroup> {
    let rs = &ctx.rules;
    let root_segs = split_lower(&tree.path);
    let active: Vec<u32> = (0..rs.patterns.len() as u32)
        .filter(|&i| {
            let p = &rs.patterns[i as usize].1;
            p.matches(&root_segs) || p.could_match_below(&root_segs)
        })
        .collect();
    let mut w = Walker { ctx, blocked, groups: HashMap::new(), in_msix: false };
    let mut segs = root_segs;
    let mut chain: Vec<&Node> = Vec::new();
    w.walk(tree, &mut segs, &active, None, None, &mut chain);
    let mut out: Vec<CandidateGroup> = w.groups.into_values().filter(|g| g.bytes > 0).collect();
    for g in &mut out {
        g.paths.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.key.cmp(&b.key)));
    out
}

impl<'a> Walker<'a> {
    fn filter_active(&self, active: &[u32], segs: &[String]) -> Vec<u32> {
        let rs = &self.ctx.rules;
        active
            .iter()
            .copied()
            .filter(|&i| {
                let p = &rs.patterns[i as usize].1;
                p.matches(segs) || p.could_match_below(segs)
            })
            .collect()
    }

    /// 规则 `ri` 能不能作为候选（safe / review、tree、how 不只是 none）。
    fn rule_candidate(&self, ri: usize) -> bool {
        let r = &self.ctx.rules.rules[ri];
        r.tier != Tier::Keep && r.scope == Scope::Tree && r.how.iter().any(|h| *h != How::None)
    }

    /// keep_latest：`parent` 的第 `idx` 个子项（精确命中规则 `ri`）是不是正在用的版本（同一父目录同一规则只算一次）。
    fn is_current(&self, cache: &mut Vec<(usize, Vec<bool>)>, ri: usize, parent: &Node, idx: usize) -> bool {
        if self.ctx.rules.rules[ri].keep_latest.is_none() {
            return false;
        }
        if let Some((_, v)) = cache.iter().find(|(r, _)| *r == ri) {
            return v[idx];
        }
        let v = self.ctx.current_versions(ri, parent, &self.ctx.match_segs(&parent.path));
        let f = v[idx];
        cache.push((ri, v));
        f
    }

    /// 这个节点的“实际档位”：safe 规则落在个人文件夹下降为 review（和解释一致）。
    fn effective_tier(&self, ri: usize, segs: &[String]) -> Tier {
        let r = &self.ctx.rules.rules[ri];
        if r.tier == Tier::Safe && self.ctx.is_protected(segs) {
            Tier::Review
        } else {
            r.tier
        }
    }

    /// `node` 里面（不含自身）别的规则的情况：返回（被别的规则精确命中的最上层节点字节合计，有没有“更严格”的）。
    /// 更严格 = keep、不能直接删、或者外层是 safe 而里面是 review。
    fn inside(&self, node: &Node, segs: &mut Vec<String>, active: &[u32], outer: usize, outer_tier: Tier) -> (u64, bool) {
        let rs = &self.ctx.rules;
        let mut other = 0u64;
        let mut strict = false;
        let mut cur_cache: Vec<(usize, Vec<bool>)> = Vec::new();
        for (ci, c) in node.children.iter().enumerate() {
            if c.kind == NodeKind::Rest {
                continue;
            }
            // 里面有读不了的文件夹、云端占位文件、系统保留项都不算：预检时逐个文件判断，这些本来就会被跳过或拦下。
            segs.push(c.name.to_lowercase());
            let ca = self.filter_active(active, segs);
            if !ca.is_empty() {
                if let Some(ri) = rs.best_exact_among(&ca, segs, Some(c)) {
                    if ri != outer {
                        other += c.allocated;
                        let r = &rs.rules[ri];
                        // 旧版本规则里正在用的版本当 keep（和解释一致）。
                        let t = if self.is_current(&mut cur_cache, ri, node, ci) { Tier::Keep } else { self.effective_tier(ri, segs) };
                        if t == Tier::Keep || !rule_deletable_kind(&r.how) || (outer_tier == Tier::Safe && t == Tier::Review) {
                            strict = true;
                        }
                    }
                }
                if c.kind == NodeKind::Dir && !c.children.is_empty() {
                    let (o, s) = self.inside(c, segs, &ca, outer, outer_tier);
                    // 已经算进上面 “other” 的子树不重复加字节，但“更严格”要一直往下看。
                    if rs.best_exact_among(&ca, segs, Some(c)).map(|ri| ri == outer).unwrap_or(true) {
                        other += o;
                    }
                    strict |= s;
                }
            }
            segs.pop();
        }
        (other, strict)
    }

    fn add(&mut self, key: String, make: impl FnOnce() -> CandidateGroup, path: String, bytes: u64, deletable: bool, hard: Option<String>) {
        let g = self.groups.entry(key).or_insert_with(make);
        g.bytes += bytes;
        if deletable {
            g.deletable_bytes += bytes;
        }
        let hard = if deletable { None } else { hard };
        g.paths.push(CandidatePath { path, bytes, deletable, hard });
    }

    fn add_rule(&mut self, ri: usize, tier: Tier, path: String, bytes: u64, deletable: bool, hard: Option<String>) {
        let r = &self.ctx.rules.rules[ri];
        // 降档（safe → review）的路径单独成组，免得一组里档位不一致。
        let key = if tier == r.tier { r.id.clone() } else { format!("{}@review", r.id) };
        let lose_kind = match (tier, r.lose_kind) {
            (Tier::Review, None) => Some(LoseKind::Unsure),
            (_, k) => k,
        };
        let lose_short = match (tier, &r.lose_short) {
            (_, Some(s)) => Some(s.clone()),
            (Tier::Review, None) => Some(super::DOWNGRADED_SHORT.to_string()),
            _ => None,
        };
        let make = || CandidateGroup {
            key: key.clone(),
            rule_id: Some(r.id.clone()),
            group: r.group.clone(),
            heuristic: None,
            // 用户自己加的规则没有内置短名：“这是什么”够短就直接用（同一个 owner 的几条规则才不会重名），否则 owner + 分类
            name: rule_label(&r.id).map(str::to_string).unwrap_or_else(|| {
                if r.what.chars().count() <= 12 {
                    r.what.clone()
                } else {
                    short_name(&r.owner, r.category)
                }
            }),
            owner: r.owner.clone(),
            category: r.category,
            tier,
            lose_kind,
            lose_short: lose_short.clone(),
            how: r.how.clone(),
            what: r.what.clone(),
            lose: r.lose.clone(),
            how_detail: r.how_detail.clone(),
            bytes: 0,
            deletable_bytes: 0,
            paths: Vec::new(),
        };
        self.add(key.clone(), make, path, bytes, deletable, hard);
    }

    /// 规则没覆盖到的文件夹：看看是不是可删的编译产物 / 依赖。命中返回 true（不再往里走）。
    fn try_heuristic(&mut self, node: &'a Node, chain: &[&'a Node]) -> bool {
        if node.kind != NodeKind::Dir || !node_ok(node) {
            return false;
        }
        let nm = node.name.to_lowercase();
        let looks = matches!(nm.as_str(), "node_modules" | "target" | "bin" | "obj")
            || node.children.iter().any(|c| c.name.eq_ignore_ascii_case(".rustc_info.json"));
        if !looks {
            return false;
        }
        let t = Target { node, path: &node.path, parent: chain.last().copied(), ancestors: chain };
        let e = explain(&t, self.ctx);
        let Source::Heuristic { name } = &e.source else { return false };
        let Some((_, label)) = HEURISTIC_NAMES.iter().find(|(n, _)| n == name) else { return false };
        if e.tier == Tier::Keep || !e.how.contains(&How::Delete) {
            return false;
        }
        let hard = (self.blocked)(&node.path);
        let deletable = hard.is_none();
        let key = format!("h:{name}");
        let (name_s, label_s) = (name.clone(), label.to_string());
        let make = || CandidateGroup {
            key: key.clone(),
            rule_id: None,
            group: "heuristic".into(),
            heuristic: Some(name_s),
            name: label_s,
            owner: String::new(),
            category: e.category,
            tier: Tier::Review,
            lose_kind: e.lose_kind,
            lose_short: e.lose_short.clone(),
            how: e.how.clone(),
            what: e.what.clone(),
            lose: e.lose.clone(),
            how_detail: e.how_detail.clone(),
            bytes: 0,
            deletable_bytes: 0,
            paths: Vec::new(),
        };
        self.add(key.clone(), make, node.path.clone(), node.allocated, deletable, hard);
        true
    }

    /// `inherited`：上层 tree 规则（没有 = 规则完全没覆盖到）；`inside_rule`：正在某个不能直接删的候选规则里面
    /// （同一条规则在里面再次命中时不重复出候选）。
    fn walk(
        &mut self,
        node: &'a Node,
        segs: &mut Vec<String>,
        active: &[u32],
        inherited: Option<usize>,
        inside_rule: Option<usize>,
        chain: &mut Vec<&'a Node>,
    ) {
        let rs = &self.ctx.rules;
        chain.push(node);
        let mut cur_cache: Vec<(usize, Vec<bool>)> = Vec::new();
        for (ci, c) in node.children.iter().enumerate() {
            if c.kind == NodeKind::Rest {
                continue;
            }
            segs.push(c.name.to_lowercase());
            // 商店应用的 LocalCache\{Local,Roaming,LocalLow}：不列；只有写明了 LocalCache 路径的旧版本规则往里找。
            if !self.in_msix && c.kind == NodeKind::Dir && self.ctx.msix_redirect(&join(&c.path, "x")).is_some() {
                let ka: Vec<u32> =
                    self.filter_active(active, segs).into_iter().filter(|&i| rs.rules[rs.patterns[i as usize].0].keep_latest.is_some()).collect();
                if !ka.is_empty() && !c.children.is_empty() {
                    self.in_msix = true;
                    self.walk(c, segs, &ka, inherited, inside_rule, chain);
                    self.in_msix = false;
                }
                segs.pop();
                continue;
            }
            let ca = self.filter_active(active, segs);
            let exact = if ca.is_empty() { None } else { rs.best_exact_among(&ca, segs, Some(c)) };
            let full = if c.kind == NodeKind::Dir { c.path.clone() } else { join(&node.path, &c.name) };
            let mut descend = c.kind == NodeKind::Dir && !c.children.is_empty();
            let mut next_inside = inside_rule;
            // 旧版本规则里正在用的版本：keep，不出候选；里面也不再按这条规则 / 启发式找。
            let current = exact.is_some_and(|ri| self.is_current(&mut cur_cache, ri, node, ci));
            match exact {
                Some(ri) if current => next_inside = Some(ri),
                Some(ri) if Some(ri) != inside_rule && self.rule_candidate(ri) => {
                    let tier = self.effective_tier(ri, segs);
                    let r = &rs.rules[ri];
                    let (other, strict) = if c.kind == NodeKind::Dir { self.inside(c, segs, &ca, ri, tier) } else { (0, false) };
                    let hard = node_hard(c).map(str::to_string).or_else(|| (self.blocked)(&full));
                    let can_delete = rule_deletable_kind(&r.how) && hard.is_none();
                    // 按年龄挑的规则（临时文件夹）：只算 N 天以上没改过的直接子项（预检 / 执行时按真实修改时间再挑一遍）。
                    let size = match r.min_age_days {
                        Some(n) if c.kind == NodeKind::Dir => super::old_children_bytes(c, self.ctx.now - n as i64 * 86_400),
                        _ => c.allocated,
                    };
                    if can_delete && !strict {
                        // 整个文件夹 / 文件就是一个候选，不再往里看。
                        if size > 0 {
                            self.add_rule(ri, tier, full, size, true, None);
                        }
                        descend = false;
                    } else {
                        self.add_rule(ri, tier, full, size.min(c.allocated.saturating_sub(other)), false, hard);
                        next_inside = Some(ri);
                    }
                }
                Some(ri) if Some(ri) == inside_rule => {}
                None if inherited.is_none() && inside_rule.is_none() && !self.in_msix => {
                    if self.try_heuristic(c, chain) {
                        descend = false;
                    }
                }
                _ => {}
            }
            if descend {
                let child_inherited = match exact {
                    Some(i) if rs.rules[i].scope == Scope::Tree => Some(i),
                    _ => inherited,
                };
                // 下面还可能有规则命中，或者这里完全没被规则覆盖（可能有启发式候选）才往下走。
                if !ca.is_empty() || (child_inherited.is_none() && next_inside.is_none()) {
                    self.walk(c, segs, &ca, child_inherited, next_inside, chain);
                }
            }
            segs.pop();
        }
        chain.pop();
    }
}
