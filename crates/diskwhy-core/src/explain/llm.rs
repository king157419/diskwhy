//! LLM 兜底（默认关闭）。打开后：扫描完自动把认不出的大项问 AI（[`super::ai_batch`]，一次一个），
//! 用户也可以在详情里手动点“问 AI”。答案存进 [`super::ai_store::AiStore`]，界面标「AI 推测」。
//!
//! - 设置保存在 `%LOCALAPPDATA%\DiskWhy\settings.json`：`enabled, base_url, model, api_key`。
//! - 接口是 OpenAI 兼容的 `POST {base_url}/chat/completions`（本机 Ollama 填 `http://127.0.0.1:11434/v1`——别写 localhost：Windows 上先试 ::1 被拒，每个请求白等约 2 秒；
//!   本机 / 局域网的地址 key 可以留空；云服务没填 key 时不算“能用”，见 [`key_optional`]）。
//! - **只发送**：文件夹名、父路径（`X:\Users\<用户名>` 换成 `~`）、前 10 大子项名和大小、扩展名分布、
//!   总大小、文件/文件夹个数、最近修改日期。**绝不读取、绝不发送文件内容。**问 `X:\Users` 本身时，子文件夹名（= 用户名）
//!   换成「（某个用户）」；自动识别不进别的账号的用户文件夹，也不问主要装着文档 / 照片 / 视频 / 音乐的文件夹和这些类型的大文件
//!   （`ai_store::ai_candidates` / `ai_store::personal_content`），不对打开的快照自动跑。
//! - 回复要求是 JSON `{what, owner, why, lose, confidence}`；AI 的说法永远不改变档位（见 `ai_store` 的铁律）。
//! - 按服务商（看 base_url 的域名）补几个参数，见 [`provider_extras`]（2026-10-11 核对各家官方文档）：
//!   智谱 / DeepSeek / Kimi / 火山方舟的模型默认开“深度思考”，批量问时又慢又费钱 → `"thinking":{"type":"disabled"}`；
//!   通义（百炼）/ 硅基流动的 qwen3 系 → `"enable_thinking": false`；智谱 / DeepSeek / 通义 支持
//!   `"response_format":{"type":"json_object"}`（提示词里有 “json” 字样）。OpenAI、本机 Ollama 等别的地址什么都不加。
//!   哪家不认这些参数、回 400 并点名时，去掉再试（[`params_to_drop`]）。
//! - 出错分三类（[`AskError`]）：等一会儿再试 / 配置不对整批停下 / 只是这一项失败。

use super::ai_store::{today, AiNote};
use super::heuristics::{ext_of, subtree_stats, ymd};
use super::{Category, ExplainCtx, Explanation, How, Source, Target, Tier};
use crate::model::NodeKind;
use crate::view::human_bytes;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// LLM 设置。
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmSettings {
    pub enabled: bool,
    /// 例如 `http://127.0.0.1:11434/v1`、`https://api.openai.com/v1`。
    pub base_url: String,
    pub model: String,
    /// 本机 / 局域网的地址可空（见 [`key_optional`]），云服务必填。
    pub api_key: String,
}

/// `{:?}` 不打出密钥（`ExplainCtx` 也是 Debug，以后谁顺手打一行日志不会把密钥写进 crash.log）。
impl std::fmt::Debug for LlmSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmSettings")
            .field("enabled", &self.enabled)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &if self.api_key.trim().is_empty() { "" } else { "<已填>" })
            .finish()
    }
}

/// 这个地址可以不填密钥：本机 / 局域网（`127.0.0.1`、`localhost`、`::1`、10.x / 172.16–31.x / 192.168.x / 169.254.x、
/// fc00::/7、fe80::/10、`*.localhost` / `*.local` / `*.lan` / `*.internal` / `*.home.arpa`、没有点的主机名）。
/// 别的（各家云服务）都要密钥——没填就不算“能用”，免得自动识别先把最大的那个文件夹的信息不带密钥发出去、才收到 401。
pub fn key_optional(base_url: &str) -> bool {
    let h = host_of(base_url);
    if h.is_empty() || (!h.contains('.') && !h.contains(':')) {
        return true; // localhost、单名字的内网主机
    }
    if let Ok(v4) = h.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified();
    }
    if let Ok(v6) = h.parse::<std::net::Ipv6Addr>() {
        let s0 = v6.segments()[0];
        return v6.is_loopback() || (s0 & 0xfe00) == 0xfc00 || (s0 & 0xffc0) == 0xfe80;
    }
    [".localhost", ".local", ".lan", ".internal", ".home.arpa"].iter().any(|s| h.ends_with(s))
}

/// 测试用：设了这个环境变量，DiskWhy 自己的所有文件（settings.json、ui.json、ai-cache.json、perf.json、
/// last-run.json、actions.log、crash.log / crash\、rules\、quarantine-roots.json）都放到它指的文件夹，
/// 而规则展开仍然用真实的 %LOCALAPPDATA%。界面壳另外把 WebView2 的资料夹也放进去（见 src-tauri 的 lib.rs 文件头）。
pub const DATA_DIR_ENV: &str = "DISKWHY_DATA_DIR";

/// `%LOCALAPPDATA%\DiskWhy`（设了 [`DATA_DIR_ENV`] 时用它）。
pub fn app_data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()).map(|v| PathBuf::from(v).join("DiskWhy"))
}

impl LlmSettings {
    pub fn settings_path() -> Option<PathBuf> {
        app_data_dir().map(|d| d.join("settings.json"))
    }

    /// 读取设置；文件不存在或坏了就用默认（关闭）。
    pub fn load() -> Self {
        Self::settings_path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let p = Self::settings_path().ok_or("找不到 %LOCALAPPDATA%")?;
        self.save_to(&p)?;
        Ok(p)
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{e}"))?;
        }
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, s).map_err(|e| format!("写设置失败：{e}"))
    }

    /// 能不能用：开关打开、填了地址和模型，云服务还要填了密钥（[`key_optional`]）。
    pub fn usable(&self) -> bool {
        self.enabled && !self.base_url.trim().is_empty() && !self.model.trim().is_empty() && !self.missing_key()
    }

    /// 地址要密钥、却没填。
    pub fn missing_key(&self) -> bool {
        self.api_key.trim().is_empty() && !key_optional(&self.base_url)
    }

    /// 和 `other` 发给的是同一个地方、同一个账号、同一个模型（开关、地址、模型名、密钥都一样，忽略首尾空白）。
    /// 排队等请求锁的那段时间里用户可能换了设置：拿到锁后用它核对，变了就不发（见 `ai_batch::recheck_batch` / `ask_manual`）。
    pub fn same_target(&self, other: &LlmSettings) -> bool {
        self.enabled == other.enabled
            && self.base_url.trim() == other.base_url.trim()
            && self.model.trim() == other.model.trim()
            && self.api_key.trim() == other.api_key.trim()
    }

    /// 不能用时的错误（[`ask_input`] 和手动“问 AI”拿到锁后的核对共用）：云服务没填密钥 → [`MSG_NO_KEY`]，别的 → [`MSG_OFF`]。
    pub fn unusable_error(&self) -> AskError {
        if self.enabled && self.missing_key() {
            AskError::fatal(MSG_NO_KEY, "这个地址要密钥，请在“设置”里填上")
        } else {
            AskError::fatal(MSG_OFF, "请在“设置”里打开开关，并填写地址和模型")
        }
    }
}

/// 发给模型的全部信息。结构体里没有任何“文件内容”字段——测试会序列化后逐项检查。
#[derive(Debug, Clone, Serialize)]
pub struct LlmInput {
    /// 文件夹 / 文件名。
    pub name: String,
    /// 父路径（用户目录已替换成 `~`）。
    pub parent: String,
    /// "文件夹" / "文件"
    pub kind: String,
    pub total_size: String,
    pub files: u64,
    pub dirs: u64,
    /// 前 10 大子项：(名字, 大小)。文件夹名后面带 `\`。
    pub top_children: Vec<(String, String)>,
    /// 扩展名分布：(".ext", 占比)。
    pub extensions: Vec<(String, String)>,
    pub modified: String,
}

/// 把 `X:\Users\<用户名>` 换成 `~`（大小写不敏感，任何用户都换）。
pub fn redact_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('\\').collect();
    if parts.len() >= 3 && parts[0].len() == 2 && parts[0].ends_with(':') && parts[1].eq_ignore_ascii_case("users") && !parts[2].is_empty() {
        let rest = parts[3..].join("\\");
        return if rest.is_empty() { "~".into() } else { format!("~\\{rest}") };
    }
    path.to_string()
}

/// 名字本身也可能带用户名（比如一个叫 king 的文件夹）：扫描到的是 `X:\Users\<名字>` 时整体换成 `~`。
fn redact_name(t: &Target) -> String {
    let full = redact_path(t.path);
    if full == "~" {
        "~".into()
    } else {
        t.node.name.clone()
    }
}

/// `X:\Users` 本身（任何盘）：它的子文件夹名就是这台电脑上各个账号的用户名。
pub fn is_users_root(path: &str) -> bool {
    let parts: Vec<&str> = path.split('\\').filter(|s| !s.is_empty()).collect();
    parts.len() == 2 && parts[0].len() == 2 && parts[0].ends_with(':') && parts[1].eq_ignore_ascii_case("users")
}

/// 问 `X:\Users` 时子文件夹名（= 用户名）换成这个。
const REDACTED_USER: &str = "（某个用户）";

/// 从节点收集要发的信息。只用扫描树里的名字、大小、时间，不碰文件。
pub fn build_input(t: &Target) -> LlmInput {
    let n = t.node;
    let parent = match t.path.trim_end_matches('\\').rsplit_once('\\') {
        Some((p, _)) => redact_path(if p.len() == 2 { &t.path[..3] } else { p }),
        None => String::new(),
    };
    let stats = subtree_stats(n);
    let users_root = is_users_root(t.path);
    let top_children = n
        .children
        .iter()
        .take(10)
        .map(|c| {
            let nm = match c.kind {
                NodeKind::Dir if users_root => format!("{REDACTED_USER}\\"),
                NodeKind::Dir => format!("{}\\", c.name),
                NodeKind::File => c.name.clone(),
                NodeKind::Rest => "（其余小文件合计）".into(),
            };
            (nm, human_bytes(c.allocated))
        })
        .collect();
    let extensions = if n.kind == NodeKind::File {
        let e = ext_of(&n.name);
        vec![(if e.is_empty() { "无扩展名".into() } else { format!(".{e}") }, "100%".into())]
    } else {
        stats
            .ext
            .iter()
            .take(8)
            .map(|(e, b, _)| {
                (
                    if e.is_empty() { "无扩展名".into() } else { format!(".{e}") },
                    format!("{:.0}%", if stats.bytes_seen > 0 { *b as f64 * 100.0 / stats.bytes_seen as f64 } else { 0.0 }),
                )
            })
            .collect()
    };
    let newest = if stats.newest_mtime > 0 { stats.newest_mtime } else { n.mtime };
    LlmInput {
        name: redact_name(t),
        parent,
        kind: if n.kind == NodeKind::File { "文件".into() } else { "文件夹".into() },
        total_size: human_bytes(n.allocated),
        files: n.files,
        dirs: n.dirs,
        top_children,
        extensions,
        modified: ymd(newest),
    }
}

/// 系统提示词。界面只显示 `what`（悬停卡片 / 列表）和 `lose`（展开行），所以要求 what 自己说清“哪个软件的什么”、
/// 两者都短（2026-10-12 集成测试：本机 qwen3-vl 4B 把真实 C 盘上的 40 个问题各问一遍，改之前 what 中位 38 字、
/// lose 中位 50 字、22 条说“不会丢数据”，软件名常常只写在界面不显示的 owner 里；改之后 what 中位 20 字、lose 24 字、
/// “不会丢”2 条，软件名都写进了 what，40/40 仍是合法 JSON，还快了约 40%）。举例特意不用常见的缓存文件夹（小模型会照抄
/// 例子里的软件名：拿 pip 举例时它把 uv 的缓存也说成 pip 的）；“先从名字认”让它少编（uv 的 archive-v0 不再说成“Uvicom”）。
pub const SYSTEM_PROMPT: &str = "你是 Windows 磁盘清理助手。用户会给你一个 Windows 上的文件夹或文件的名字、所在位置、最大的几个子项名、扩展名分布、大小和修改日期（没有文件内容）。\
请推断它可能是什么、属于哪个软件、为什么会有、删了会失去什么。\
要求：1) 全用给普通人看的中文大白话，不要写英文句子；名字本身最要紧：文件夹名常常就是软件或工具的名字，先从名字认，再参考位置和子项；\
2) what 一句话、不超过 30 个字，说清是什么东西、属于哪个软件（例如“微信的聊天图片和视频缓存”“Steam 下载的游戏更新包”）；\
3) why、lose 各一句话、不超过 40 个字；lose 具体说删了会怎样（例如“下次打开要重新下载这些内容”“聊天里的旧图片会打不开”）；\
4) 这是推断不是确认，拿不准就直说“不确定”，并在 lose 里提醒用户先确认再删；5) 不要建议用户直接删除系统文件夹；\
6) 只输出一个 JSON 对象，不要任何多余文字，格式：{\"what\":\"这是什么\",\"owner\":\"可能属于哪个软件，不知道就写空字符串\",\"why\":\"为什么会有\",\"lose\":\"删了会失去什么、会不会丢数据\",\"confidence\":0到1之间的小数}";

// ───────────────────────── 按服务商补的参数 ─────────────────────────

/// `base_url` 里的主机名（小写，不含端口 / 用户名）。
pub fn host_of(base_url: &str) -> String {
    let s = base_url.trim();
    let s = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
    let s = s.split(['/', '?', '#']).next().unwrap_or("");
    let s = s.rsplit_once('@').map(|(_, h)| h).unwrap_or(s);
    let h = match s.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => s.split(':').next().unwrap_or(""),
    };
    h.trim_end_matches('.').to_ascii_lowercase()
}

/// 主机是这个域名本身或它的子域名（`evil-bigmodel.cn` 不算）。
fn host_is(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.'))
}

/// 默认开“深度思考”、认 `"thinking":{"type":"disabled"}` 的：智谱（含国际版 z.ai）、DeepSeek、Kimi（开放平台 moonshot.cn / .ai，
/// Kimi Code 的 api.kimi.com / api.kimi.ai：文档里 effort = none 就是 `thinking.type = disabled`）、火山方舟（豆包）。
/// Kimi 各家都**不加** `response_format`（不在 [`JSON_MODE_HOSTS`] 里）。
const THINKING_TYPE_HOSTS: &[&str] = &["bigmodel.cn", "z.ai", "deepseek.com", "moonshot.cn", "moonshot.ai", "kimi.com", "kimi.ai", "volces.com"];
/// 用 `"enable_thinking": false` 关思考的：阿里百炼（通义千问）、硅基流动。
const ENABLE_THINKING_HOSTS: &[&str] = &["dashscope.aliyuncs.com", "dashscope-intl.aliyuncs.com", "siliconflow.cn", "siliconflow.com"];
/// 支持 `"response_format":{"type":"json_object"}` 的：智谱、DeepSeek、通义。
const JSON_MODE_HOSTS: &[&str] = &["bigmodel.cn", "z.ai", "deepseek.com", "dashscope.aliyuncs.com", "dashscope-intl.aliyuncs.com"];

/// 按 `base_url` 的域名要补进请求体的参数（OpenAI、本机 Ollama 和别的地址：空）。
pub fn provider_extras(base_url: &str) -> serde_json::Map<String, serde_json::Value> {
    let h = host_of(base_url);
    let any = |ds: &[&str]| ds.iter().any(|d| host_is(&h, d));
    let mut m = serde_json::Map::new();
    if any(THINKING_TYPE_HOSTS) {
        m.insert("thinking".into(), serde_json::json!({"type": "disabled"}));
    }
    if any(ENABLE_THINKING_HOSTS) {
        m.insert("enable_thinking".into(), serde_json::json!(false));
    }
    if any(JSON_MODE_HOSTS) {
        m.insert("response_format".into(), serde_json::json!({"type": "json_object"}));
    }
    m
}

fn merge_into(body: &mut serde_json::Value, extras: serde_json::Map<String, serde_json::Value>) {
    if let Some(o) = body.as_object_mut() {
        for (k, v) in extras {
            o.insert(k, v);
        }
    }
}

/// 请求体（OpenAI 兼容 chat/completions），按 `base_url` 补上 [`provider_extras`]。
///
/// **不发 `temperature` / `top_p` / `max_tokens`**：Kimi K2.6 及以后的模型把 temperature、top_p 固定死，
/// 文档说传其他值会报错、不要显式传（<https://platform.kimi.ai/docs/api/models-overview>）；
/// OpenAI 的 GPT-5 系推理模型也只接受默认 temperature、且不认 `max_tokens`。各家默认值都能用，
/// 所以干脆一个采样参数都不发。万一将来有人加回来，[`chat`] 遇到点名这些参数的 400 会去掉再试。
pub fn build_request_body(base_url: &str, model: &str, input: &LlmInput) -> serde_json::Value {
    let user = format!(
        "请推断下面这个{}是什么（这是推断，不是确认），只回一个 json 对象：\n{}",
        input.kind,
        serde_json::to_string_pretty(input).unwrap_or_default()
    );
    let mut b = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user}
        ]
    });
    merge_into(&mut b, provider_extras(base_url));
    b
}

/// 测试连接的请求体：一句很短的话，按 `base_url` 补上关思考的参数。
/// **不加 `response_format`**：这句话不是要 JSON 的（提示词里也没有 “json”），开 JSON 模式有的服务会 400、
/// 有的会让模型一直输出空白直到超时。
pub fn test_request_body(base_url: &str, model: &str) -> serde_json::Value {
    let mut b = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": [{"role": "user", "content": "只回复两个字：你好"}]
    });
    let mut extras = provider_extras(base_url);
    extras.remove("response_format");
    merge_into(&mut b, extras);
    b
}

/// `base_url` → 完整的 chat/completions 地址。
pub fn chat_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/chat/completions") {
        b.to_string()
    } else {
        format!("{b}/chat/completions")
    }
}

/// 模型的回答。
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct LlmAnswer {
    pub what: String,
    pub owner: String,
    pub why: String,
    pub lose: String,
    pub confidence: f32,
}

/// 去掉推理模型的思考段：`<think>…</think>`（本机 Ollama 的 qwen3 / deepseek-r1 会放在 content 里）。
/// 没闭合的 `<think>`（回复被截断）就整段丢掉。
fn strip_think(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let Some(i) = rest.find("<think>") else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..i]);
        match rest[i..].find("</think>") {
            Some(j) => rest = &rest[i + j + "</think>".len()..],
            None => break,
        }
    }
    out
}

/// 数字或字符串（"0.7"、"70%"）都认；0–100 的百分数也认；认不出就 0。
fn lenient_confidence(v: Option<&serde_json::Value>) -> f32 {
    let x = match v {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(serde_json::Value::String(t)) => {
            let t = t.trim();
            match t.strip_suffix('%') {
                Some(p) => p.trim().parse::<f64>().map(|x| x / 100.0).unwrap_or(0.0),
                None => t.parse::<f64>().unwrap_or(0.0),
            }
        }
        _ => 0.0,
    };
    let x = if x > 1.0 && x <= 100.0 { x / 100.0 } else { x };
    if x.is_finite() {
        x.clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

fn lenient_str(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> String {
    match o.get(k) {
        Some(serde_json::Value::String(t)) => t.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// 一个 JSON 对象 → 回答；必须有非空的 `what`。
fn answer_from_value(v: &serde_json::Value) -> Option<LlmAnswer> {
    let o = v.as_object()?;
    let what = lenient_str(o, "what");
    if what.trim().is_empty() {
        return None;
    }
    Some(LlmAnswer {
        what,
        owner: lenient_str(o, "owner"),
        why: lenient_str(o, "why"),
        lose: lenient_str(o, "lose"),
        confidence: lenient_confidence(o.get("confidence")),
    })
}

/// 从模型返回的文本里抠出 JSON 回答。模型常见的“不听话”都要能吃下：
/// - 包一层 ```` ```json … ``` ````；
/// - JSON 前面先说一段推理（推理里可能也有 `{`、`}`），或后面再补几句；
/// - `<think>…</think>` 思考段；
/// - confidence 写成字符串 / 百分数，owner 写成 null。
///
/// 做法：去掉思考段后，从每个 `{` 处试着用 serde 读出**一个完整的** JSON 值（后面多余的文字不管），
/// 取最后一个带非空 `what` 的对象（推理里举的例子在前，正式答案在后）。
pub fn parse_answer(content: &str) -> Result<LlmAnswer, String> {
    let text = strip_think(content);
    let mut found: Option<LlmAnswer> = None;
    let mut saw_json = false;
    let mut i = 0;
    while let Some(off) = text[i..].find('{') {
        let at = i + off;
        let mut it = serde_json::Deserializer::from_str(&text[at..]).into_iter::<serde_json::Value>();
        if let Some(Ok(v)) = it.next() {
            saw_json = true;
            if let Some(a) = answer_from_value(&v) {
                found = Some(a);
                // 跳过整个对象，里面嵌套的 `{` 不再单独试
                i = at + it.byte_offset().max(1);
                continue;
            }
        }
        i = at + 1;
    }
    match found {
        Some(a) => Ok(a),
        None if saw_json => Err("AI 没有给出“这是什么”".into()),
        None => Err("AI 的回复里没有 JSON".into()),
    }
}

/// 证书用 Windows 自己的根证书库（`RootCerts::PlatformVerifier`）。ureq 默认是 Mozilla 的根证书列表（WebPki），
/// 但 Windows 拼证书链时可能拼到列表外的根：2026-10-12 真机测试 Kimi Code（api.kimi.com：GTS Root R4 由
/// GlobalSign Root CA 交叉签名）每次都报 “unable to find any user-specified roots in the final cert chain”，界面显示“连不上”。
fn agent(timeout: Duration) -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .new_agent()
}

/// 可以不发的参数（采样参数 + 按服务商补的）。服务器 400 并点名其中某个时，去掉它重试。
const OPTIONAL_PARAMS: [&str; 8] =
    ["temperature", "top_p", "max_tokens", "presence_penalty", "frequency_penalty", "response_format", "thinking", "enable_thinking"];

/// 400 的报错里点名了请求体里的哪些可选参数（全部要去掉）。
pub fn params_to_drop(body: &serde_json::Value, status: u16, err_text: &str) -> Vec<&'static str> {
    if status != 400 {
        return Vec::new();
    }
    let low = err_text.to_ascii_lowercase();
    OPTIONAL_PARAMS.iter().copied().filter(|k| body.get(*k).is_some() && low.contains(k)).collect()
}

/// `choices[0].message.content`：字符串，或（少数兼容实现）`[{type:"text",text:"…"}]` 数组。
fn extract_content(v: &serde_json::Value) -> Option<String> {
    match v.pointer("/choices/0/message/content")? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(parts) => {
            let s: String = parts.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        _ => None,
    }
}

// ───────────────────────── 出错时怎么办 ─────────────────────────

/// 问 AI 失败。三类（加上批量执行时的两种“没发请求”）：
/// - `retryable`：等一会儿再试可能就好（HTTP 429、5xx、智谱业务码 1302 / 1303 / 1305、超时、连接被重置）；
/// - `fatal`：配置不对，整批停下（401 / 403 密钥、404 / 模型不存在、地址不对、余额 / 额度用完、连不上、没填密钥）；
/// - 都不是：只是这一项失败（回答看不懂、内容被拦……），接着问下一项；
/// - `skipped`：这一项不用问了（扫描结果里已经没有它，比如刚被清理掉）：算问完，**不算失败**；
/// - `cancelled`：这一批已经取消（排队等请求的时候换了设置 / 关了 AI / 开始了新的扫描），没发请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskError {
    pub retryable: bool,
    pub fatal: bool,
    pub skipped: bool,
    pub cancelled: bool,
    /// 给界面看的短中文（见 [`ASK_MESSAGES`]）。
    pub message: String,
    /// 技术细节（HTTP 状态码、服务器报错片段）；测试连接时一起显示，自动识别只用 `message`。
    pub detail: String,
}

pub const MSG_KEY: &str = "密钥不对";
pub const MSG_NO_PERM: &str = "没有权限";
pub const MSG_MODEL: &str = "模型名不对";
pub const MSG_URL: &str = "地址不对";
pub const MSG_BALANCE: &str = "余额不足";
pub const MSG_QUOTA: &str = "额度用完了";
pub const MSG_CONNECT: &str = "连不上";
pub const MSG_RATE: &str = "请求太频繁";
pub const MSG_BUSY: &str = "服务繁忙";
pub const MSG_TIMEOUT: &str = "超时";
pub const MSG_NET: &str = "网络断了";
pub const MSG_BAD_ANSWER: &str = "回答看不懂";
pub const MSG_NO_ANSWER: &str = "没有回答";
pub const MSG_REJECTED: &str = "请求被拒";
pub const MSG_OFF: &str = "AI 没打开";
/// 云服务的地址没填密钥（[`LlmSettings::missing_key`]）。
pub const MSG_NO_KEY: &str = "没填密钥";
/// 扫描结果里已经没有这个路径了（界面壳用）。
pub const MSG_NOT_FOUND: &str = "找不到了";
/// 自动识别自己出了意外（界面壳用；详情在 crash.log）。
pub const MSG_FAILED: &str = "出错了";
/// 这一批取消了；手动“问 AI”排队时换了服务商 / 模型 / 密钥，也是这个（没发请求）。
pub const MSG_CANCELLED: &str = "已取消";
/// 排队等请求的时候这一项已经被问过了（自动识别里算跳过，不显示）。
pub const MSG_ASKED: &str = "问过了";

/// `AskError.message` 可能的全部取值（界面上会原样显示，前端字体子集要能显示这些字）。
pub const ASK_MESSAGES: &[&str] = &[
    MSG_KEY,
    MSG_NO_PERM,
    MSG_MODEL,
    MSG_URL,
    MSG_BALANCE,
    MSG_QUOTA,
    MSG_CONNECT,
    MSG_RATE,
    MSG_BUSY,
    MSG_TIMEOUT,
    MSG_NET,
    MSG_BAD_ANSWER,
    MSG_NO_ANSWER,
    MSG_REJECTED,
    MSG_OFF,
    MSG_NO_KEY,
    MSG_NOT_FOUND,
    MSG_FAILED,
    MSG_CANCELLED,
    MSG_ASKED,
];

impl AskError {
    fn new(message: &str, detail: impl Into<String>) -> Self {
        AskError { retryable: false, fatal: false, skipped: false, cancelled: false, message: message.into(), detail: detail.into() }
    }
    pub fn retry(message: &str, detail: impl Into<String>) -> Self {
        AskError { retryable: true, ..Self::new(message, detail) }
    }
    pub fn fatal(message: &str, detail: impl Into<String>) -> Self {
        AskError { fatal: true, ..Self::new(message, detail) }
    }
    /// 只是这一项失败。
    pub fn item(message: &str, detail: impl Into<String>) -> Self {
        Self::new(message, detail)
    }
    /// 这一项不用问了（不算失败）。
    pub fn skip(message: &str, detail: impl Into<String>) -> Self {
        AskError { skipped: true, ..Self::new(message, detail) }
    }
    /// 这一批取消了，没发请求。
    pub fn cancelled() -> Self {
        AskError { cancelled: true, ..Self::new(MSG_CANCELLED, "") }
    }
    /// “短中文（细节）”。
    pub fn detailed(&self) -> String {
        if self.detail.trim().is_empty() {
            self.message.clone()
        } else {
            format!("{}（{}）", self.message, self.detail)
        }
    }
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 报错 JSON 里的业务码：`error.code` / `code`（数字或字符串都认）。
fn error_code(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let c = v.pointer("/error/code").or_else(|| v.get("code")).filter(|c| !c.is_null())?;
    Some(match c {
        serde_json::Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    })
}

fn snippet(s: &str) -> String {
    let t: String = s.trim().chars().take(200).collect();
    t.replace(['\r', '\n'], " ")
}

/// 报错文字像不像“没有这个模型”。
fn mentions_missing_model(low: &str) -> bool {
    low.contains("model_not_found")
        || low.contains("模型不存在")
        || (low.contains("model")
            && (low.contains("not found")
                || low.contains("not exist")
                || low.contains("not_found")
                || low.contains("invalid model")
                || low.contains("unknown model")))
}

/// 非 2xx 的回复 → 哪一类错误（纯函数）。
///
/// 智谱业务码（docs.bigmodel.cn「错误码」）：1000–1004 鉴权；1113 欠费；1211 模型不存在；1220 无权访问模型；
/// 1221 / 1222 API 下线 / 不存在；1301 内容不安全；1302 并发太高、1303 频率太高、1305 请求过多（都可以等等再试）；
/// 1304 / 1308 / 1310 次数 / 额度用完。
pub fn classify_http(status: u16, body: &str) -> AskError {
    let detail = format!("HTTP {status}：{}", snippet(body));
    let low = body.to_lowercase();
    if let Some(code) = error_code(body) {
        match code.as_str() {
            "1302" | "1303" => return AskError::retry(MSG_RATE, detail),
            "1305" => return AskError::retry(MSG_BUSY, detail),
            "1113" => return AskError::fatal(MSG_BALANCE, detail),
            "1304" | "1308" | "1310" => return AskError::fatal(MSG_QUOTA, detail),
            "1000" | "1001" | "1002" | "1003" | "1004" => return AskError::fatal(MSG_KEY, detail),
            "1211" => return AskError::fatal(MSG_MODEL, detail),
            "1220" => return AskError::fatal(MSG_NO_PERM, detail),
            "1221" | "1222" => return AskError::fatal(MSG_URL, detail),
            "1301" => return AskError::item(MSG_REJECTED, detail),
            // 通义（百炼）/ OpenAI 的字符串码
            "InvalidApiKey" | "invalid_api_key" => return AskError::fatal(MSG_KEY, detail),
            "Arrearage" => return AskError::fatal(MSG_BALANCE, detail),
            "insufficient_quota" => return AskError::fatal(MSG_QUOTA, detail),
            "model_not_found" => return AskError::fatal(MSG_MODEL, detail),
            _ => {}
        }
    }
    match status {
        401 => AskError::fatal(MSG_KEY, detail),
        403 if low.contains("free tier") || low.contains("quota") => AskError::fatal(MSG_QUOTA, detail),
        403 => AskError::fatal(MSG_NO_PERM, detail),
        402 => AskError::fatal(MSG_BALANCE, detail),
        404 if low.contains("model") => AskError::fatal(MSG_MODEL, detail),
        404 | 405 => AskError::fatal(MSG_URL, detail),
        408 => AskError::retry(MSG_TIMEOUT, detail),
        429 if low.contains("insufficient_quota") || low.contains("exceeded your current quota") => AskError::fatal(MSG_QUOTA, detail),
        429 if low.contains("balance") || low.contains("欠费") || low.contains("余额") => AskError::fatal(MSG_BALANCE, detail),
        429 => AskError::retry(MSG_RATE, detail),
        500..=599 => AskError::retry(MSG_BUSY, detail),
        _ if mentions_missing_model(&low) => AskError::fatal(MSG_MODEL, detail),
        400 | 422 if low.contains("api key") || low.contains("apikey") || low.contains("api_key") || low.contains("authentication") => {
            AskError::fatal(MSG_KEY, detail)
        }
        _ => AskError::item(MSG_REJECTED, detail),
    }
}

/// 发请求时的网络错误 → 哪一类。超时 / 连接被重置 / 中途断开可以重试；连不上（拒绝连接、找不到服务器、TLS 失败）整批停下。
pub fn classify_transport(e: &ureq::Error, url: &str) -> AskError {
    use std::io::ErrorKind as K;
    let detail = format!("{url}：{e}");
    match e {
        ureq::Error::Timeout(_) => AskError::retry(MSG_TIMEOUT, detail),
        ureq::Error::Io(io) => match io.kind() {
            K::TimedOut | K::WouldBlock => AskError::retry(MSG_TIMEOUT, detail),
            K::ConnectionReset | K::ConnectionAborted | K::UnexpectedEof | K::BrokenPipe | K::Interrupted => AskError::retry(MSG_NET, detail),
            _ => AskError::fatal(MSG_CONNECT, detail),
        },
        ureq::Error::Protocol(_) => AskError::retry(MSG_NET, detail),
        ureq::Error::BadUri(_) | ureq::Error::RedirectFailed | ureq::Error::TooManyRedirects => AskError::fatal(MSG_URL, detail),
        _ => AskError::fatal(MSG_CONNECT, detail),
    }
}

/// 发一次 HTTP，返回 (状态码, 回复文本)。
fn post_once(settings: &LlmSettings, url: &str, body: &serde_json::Value, timeout: Duration) -> Result<(u16, String), AskError> {
    let mut req = agent(timeout).post(url).header("Content-Type", "application/json");
    if !settings.api_key.trim().is_empty() {
        req = req.header("Authorization", &format!("Bearer {}", settings.api_key.trim()));
    }
    let mut resp = req.send(body.to_string()).map_err(|e| classify_transport(&e, url))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().map_err(|e| classify_transport(&e, url))?;
    Ok((status, text))
}

/// 发 chat 请求，返回 assistant 的文本。所有错误都分好类（[`AskError`]），不 panic。
/// 服务器回 400 且点名了请求里的某个可选参数（如 Kimi 的 “invalid temperature”、不认 `thinking` 的服务）时，
/// 去掉它再试（最多 3 轮，每轮去掉这次点名的）。
pub fn chat(settings: &LlmSettings, body: &serde_json::Value, timeout: Duration) -> Result<String, AskError> {
    let url = chat_url(&settings.base_url);
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AskError::fatal(MSG_URL, "地址要以 http:// 或 https:// 开头"));
    }
    let mut body = body.clone();
    let (mut status, mut text) = post_once(settings, &url, &body, timeout)?;
    for _ in 0..3 {
        let drop = params_to_drop(&body, status, &text);
        if drop.is_empty() {
            break;
        }
        if let Some(o) = body.as_object_mut() {
            for k in &drop {
                o.remove(*k);
            }
        }
        (status, text) = post_once(settings, &url, &body, timeout)?;
    }
    if !(200..300).contains(&status) {
        return Err(classify_http(status, &text));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AskError::fatal(MSG_URL, format!("回复不是 JSON（{e}），确认这是 OpenAI 兼容接口：{}", snippet(&text))))?;
    if v.get("choices").is_none() {
        // 有的服务 HTTP 200 里也放报错
        if v.get("error").is_some() || v.get("code").is_some() {
            return Err(classify_http(if status == 200 { 400 } else { status }, &text));
        }
        return Err(AskError::fatal(MSG_URL, format!("回复里没有 choices，确认这是 OpenAI 兼容接口：{}", snippet(&text))));
    }
    extract_content(&v).ok_or_else(|| AskError::item(MSG_NO_ANSWER, "回复里没有 choices[0].message.content"))
}

/// 测试连接：发一句很短的话，返回耗时和回复开头。出错时是“短中文（细节）”。
pub fn test_connection(settings: &LlmSettings) -> Result<String, String> {
    if settings.base_url.trim().is_empty() || settings.model.trim().is_empty() {
        return Err("请先填写地址和模型名".into());
    }
    // 不发 max_tokens：OpenAI 推理模型不认它（要 max_completion_tokens），而且推理模型
    // 16 个 token 可能全花在思考上、正文为空。
    let body = test_request_body(&settings.base_url, settings.model.trim());
    let start = std::time::Instant::now();
    let reply = chat(settings, &body, Duration::from_secs(60)).map_err(|e| e.detailed())?;
    let head: String = strip_think(&reply).trim().chars().take(40).collect();
    Ok(format!("连接成功，用时 {:.1} 秒，模型回复：{head}", start.elapsed().as_secs_f64()))
}

/// 问一次 AI 的超时。
pub const ASK_TIMEOUT: Duration = Duration::from_secs(90);

/// 问 AI（自动识别和手动“问 AI”都走这里）：返回 AI 的推测。**不写缓存**（调用方 `ctx.ai.put`）。
pub fn ask_note(t: &Target, ctx: &ExplainCtx) -> Result<AiNote, AskError> {
    ask_input(&build_input(t), t.node.allocated, &ctx.settings, ASK_TIMEOUT)
}

/// 同 [`ask_note`]，但要发的信息已经从扫描树里取好了（界面壳先取信息、放开扫描树，再慢慢等网络）。
pub fn ask_input(input: &LlmInput, size: u64, settings: &LlmSettings, timeout: Duration) -> Result<AiNote, AskError> {
    if !settings.usable() {
        return Err(settings.unusable_error());
    }
    let model = settings.model.trim();
    let body = build_request_body(&settings.base_url, model, input);
    let content = chat(settings, &body, timeout)?;
    let a = parse_answer(&content).map_err(|e| AskError::item(MSG_BAD_ANSWER, e))?;
    Ok(to_note(a, model, size))
}

/// 一段 AI 文字最多留多少个字（防止模型啰嗦个没完）。
pub const NOTE_FIELD_MAX: usize = 600;

/// 控制字符和改写显示方向的字符（U+202A–202E、U+2066–2069、U+200E / 200F、U+061C）：模型的回答里不该有，
/// 有的话会让界面上的字倒着排、或者藏起一段——换成空格 / 去掉。
fn is_display_trick(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

fn clip_field(s: &str) -> String {
    let clean: String = s.chars().filter(|c| !is_display_trick(*c)).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = clean.trim();
    if t.chars().count() <= NOTE_FIELD_MAX {
        t.to_string()
    } else {
        let mut x: String = t.chars().take(NOTE_FIELD_MAX).collect();
        x.push('…');
        x
    }
}

/// 模型回答 → 存起来的推测。owner 写成“未知 / 不知道”的当空；confidence 最多 0.9；日期 = 今天。
pub fn to_note(a: LlmAnswer, model: &str, size: u64) -> AiNote {
    let owner = a.owner.trim();
    let owner = match owner.to_lowercase().as_str() {
        "未知" | "不知道" | "不确定" | "无" | "unknown" | "n/a" | "none" | "null" => "",
        _ => owner,
    };
    AiNote {
        what: clip_field(&a.what),
        owner: clip_field(owner),
        why: clip_field(&a.why),
        lose: clip_field(&a.lose),
        confidence: if a.confidence.is_finite() { a.confidence.clamp(0.0, 0.9) } else { 0.0 },
        model: model.to_string(),
        at: today(),
        size,
    }
}

/// 问 AI（旧接口，返回一条 `source = Llm`、`tier = review` 的解释）。答案也会存进 `ctx.ai`。
pub fn ask(t: &Target, ctx: &ExplainCtx) -> Result<Explanation, String> {
    let note = ask_note(t, ctx).map_err(|e| e.detailed())?;
    let _ = ctx.ai.put(t.path, note.clone());
    let a = LlmAnswer { what: note.what, owner: note.owner, why: note.why, lose: note.lose, confidence: note.confidence };
    Ok(to_explanation(a, &note.model, t.node.allocated))
}

/// AI 推测结果的标签。
pub const LLM_SHORT: &str = "还没确认是什么";

/// 模型回答 → 解释。永远 review，永远不能变成 safe。
pub fn to_explanation(a: LlmAnswer, model: &str, allocated: u64) -> Explanation {
    let how = vec![How::Manual];
    Explanation {
        tier: Tier::Review,
        owner: if a.owner.trim().is_empty() { None } else { Some(format!("{}（AI 推测）", a.owner.trim())) },
        category: Category::Unknown,
        what: a.what.trim().to_string(),
        why: a.why.trim().to_string(),
        lose: if a.lose.trim().is_empty() { "AI 没说清楚。请先确认再决定。".into() } else { a.lose.trim().to_string() },
        lose_kind: Some(super::LoseKind::Unsure),
        lose_short: Some(LLM_SHORT.to_string()),
        how,
        how_detail: "这是 AI 根据名字和大小做的推测，不是确认。删之前请自己再核实一下（搜一下文件夹名、看看里面的文件）。".into(),
        gain_bytes: 0,
        gain_note: format!("占 {}；AI 推测不作为能否删除的依据", human_bytes(allocated)),
        source: Source::Llm { model: model.to_string() },
        confidence: a.confidence.min(0.9),
        refs: Vec::new(),
        verified_on: None,
        notes: vec!["AI 推测：只发送了名字、位置（用户名已隐去）、子项名、大小和修改日期，没有发送任何文件内容。".into()],
        clues: Vec::new(),
        ai: None,
    }
}
