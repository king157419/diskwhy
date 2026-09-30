//! LLM 兜底（默认关闭）：用户在详情面板点“问 AI”时才调用。
//!
//! - 设置保存在 `%LOCALAPPDATA%\DiskWhy\settings.json`：`enabled, base_url, model, api_key`。
//! - 接口是 OpenAI 兼容的 `POST {base_url}/chat/completions`（本机 Ollama 填 `http://localhost:11434/v1`，
//!   key 可以留空）。
//! - **只发送**：文件夹名、父路径（`X:\Users\<用户名>` 换成 `~`）、前 10 大子项名和大小、扩展名分布、
//!   总大小、文件/文件夹个数、最近修改日期。**绝不读取、绝不发送文件内容。**
//! - 回复要求是 JSON `{what, why, lose, confidence}`；结果一律 `tier = review`、`source = Llm`，界面标“AI 推测”。

use super::heuristics::{ext_of, subtree_stats, ymd};
use super::{Category, ExplainCtx, Explanation, How, Source, Target, Tier};
use crate::model::NodeKind;
use crate::view::human_bytes;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// LLM 设置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmSettings {
    pub enabled: bool,
    /// 例如 `http://localhost:11434/v1`、`https://api.openai.com/v1`。
    pub base_url: String,
    pub model: String,
    /// 可空（本机 Ollama 不需要）。
    pub api_key: String,
}

/// `%LOCALAPPDATA%\DiskWhy`。
pub fn app_data_dir() -> Option<PathBuf> {
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

    /// 能不能用（开关打开且填了地址和模型）。
    pub fn usable(&self) -> bool {
        self.enabled && !self.base_url.trim().is_empty() && !self.model.trim().is_empty()
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

/// 从节点收集要发的信息。只用扫描树里的名字、大小、时间，不碰文件。
pub fn build_input(t: &Target) -> LlmInput {
    let n = t.node;
    let parent = match t.path.trim_end_matches('\\').rsplit_once('\\') {
        Some((p, _)) => redact_path(if p.len() == 2 { &t.path[..3] } else { p }),
        None => String::new(),
    };
    let stats = subtree_stats(n);
    let top_children = n
        .children
        .iter()
        .take(10)
        .map(|c| {
            let nm = match c.kind {
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

pub const SYSTEM_PROMPT: &str = "你是 Windows 磁盘清理助手。用户会给你一个 Windows 上的文件夹或文件的名字、所在位置、最大的几个子项名、扩展名分布、大小和修改日期（没有文件内容）。\
请推断它可能是什么、属于哪个软件、为什么会有、删了会失去什么。\
要求：1) 用给普通人看的中文大白话，每项一两句；2) 这是推断不是确认，拿不准就直说“不确定”，并在 lose 里提醒用户先确认再删；3) 不要建议用户直接删除系统文件夹；\
4) 只输出一个 JSON 对象，不要任何多余文字，格式：{\"what\":\"这是什么\",\"owner\":\"可能属于哪个软件，不知道就写空字符串\",\"why\":\"为什么会有\",\"lose\":\"删了会失去什么、会不会丢数据\",\"confidence\":0到1之间的小数}";

/// 请求体（OpenAI 兼容 chat/completions）。
pub fn build_request_body(model: &str, input: &LlmInput) -> serde_json::Value {
    let user = format!(
        "请推断下面这个{}是什么（这是推断，不是确认）：\n{}",
        input.kind,
        serde_json::to_string_pretty(input).unwrap_or_default()
    );
    serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "stream": false,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user}
        ]
    })
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

/// 从模型返回的文本里抠出 JSON（有的模型会包一层 ```json ... ``` 或多说几句）。
pub fn parse_answer(content: &str) -> Result<LlmAnswer, String> {
    let start = content.find('{').ok_or("AI 的回复里没有 JSON")?;
    let end = content.rfind('}').ok_or("AI 的回复里没有 JSON")?;
    if end <= start {
        return Err("AI 的回复里没有完整的 JSON".into());
    }
    let mut a: LlmAnswer = serde_json::from_str(&content[start..=end]).map_err(|e| format!("AI 回复的 JSON 解析失败：{e}"))?;
    if a.what.trim().is_empty() {
        return Err("AI 没有给出“这是什么”".into());
    }
    a.confidence = if a.confidence.is_finite() { a.confidence.clamp(0.0, 1.0) } else { 0.0 };
    Ok(a)
}

fn agent(timeout: Duration) -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .build()
        .new_agent()
}

/// 发一次 chat 请求，返回 assistant 的文本。所有错误都转成中文字符串，不 panic。
fn chat(settings: &LlmSettings, body: &serde_json::Value, timeout: Duration) -> Result<String, String> {
    let url = chat_url(&settings.base_url);
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("地址要以 http:// 或 https:// 开头".into());
    }
    let mut req = agent(timeout).post(&url).header("Content-Type", "application/json");
    if !settings.api_key.trim().is_empty() {
        req = req.header("Authorization", &format!("Bearer {}", settings.api_key.trim()));
    }
    let mut resp = req.send(body.to_string()).map_err(|e| format!("连不上 {url}：{e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().map_err(|e| format!("读取回复失败：{e}"))?;
    if !(200..300).contains(&status) {
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("服务返回 HTTP {status}：{snippet}"));
    }
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("回复不是 JSON：{e}"))?;
    v.pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .ok_or_else(|| "回复里没有 choices[0].message.content，确认这是 OpenAI 兼容接口".into())
}

/// 测试连接：发一句很短的话，返回耗时和回复开头。
pub fn test_connection(settings: &LlmSettings) -> Result<String, String> {
    if settings.base_url.trim().is_empty() || settings.model.trim().is_empty() {
        return Err("请先填写地址和模型名".into());
    }
    let body = serde_json::json!({
        "model": settings.model.trim(),
        "stream": false,
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "只回复两个字：你好"}]
    });
    let start = std::time::Instant::now();
    let reply = chat(settings, &body, Duration::from_secs(30))?;
    let head: String = reply.trim().chars().take(40).collect();
    Ok(format!("连接成功，用时 {:.1} 秒，模型回复：{head}", start.elapsed().as_secs_f64()))
}

/// 问 AI：返回一条 `source = Llm`、`tier = review` 的解释。
pub fn ask(t: &Target, ctx: &ExplainCtx) -> Result<Explanation, String> {
    let s = &ctx.settings;
    if !s.usable() {
        return Err("AI 推测还没启用：请在“设置”里打开开关，并填写地址和模型".into());
    }
    let input = build_input(t);
    let body = build_request_body(s.model.trim(), &input);
    let content = chat(s, &body, Duration::from_secs(90))?;
    let a = parse_answer(&content)?;
    Ok(to_explanation(a, s.model.trim(), t.node.allocated))
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
    }
}
