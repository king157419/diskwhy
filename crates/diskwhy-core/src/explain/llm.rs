//! LLM 兜底（默认关闭）：只在“规则和启发式都没认出来”且用户打开了开关时才用。
//!
//! - 设置保存在 `%LOCALAPPDATA%\DiskWhy\settings.json`：`enabled, base_url, model, api_key`，默认 `enabled = false`。
//! - 接口是 OpenAI 兼容的 `POST {base_url}/chat/completions`（本机 Ollama 填 `http://localhost:11434/v1`，
//!   key 可以留空）。
//! - **只发送四样东西**（[`LlmInput`]）：名字、父路径（`X:\Users\<用户名>` 换成 `~`，名字里出现的用户名也换掉）、
//!   最多 20 项“子项摘要”、总大小。**绝不读取、绝不发送文件内容**。第 1 轮安全评审后收紧：父路径只留最近两层；
//!   子文件夹发名字，文件只发“扩展名 × 个数”不发文件名；名字里的用户名（≥2 个字）、邮箱、wxid_…、5 位以上的数字都打码。
//! - 只有“规则和启发式都没认出来”的项能问（[`ask`] 会先检查）；远程地址必须 https，只有本机地址允许 http。
//! - 回复只取 `{what, why, lose}` 三项；模型多给的 tier / owner / confidence 一律丢掉。
//!   结果 `tier = review`、`source = Llm`、可信度固定 [`LLM_CONFIDENCE`]（低），界面标“AI 推测”。
//! - 网络部分藏在 [`LlmOracle`] 后面：生产用 [`HttpOracle`]，测试用假的 oracle，不联网。

use super::heuristics::owner_from_path;
use super::{Category, ExplainCtx, Explanation, How, Source, Target, Tier};
use crate::model::NodeKind;
use crate::view::human_bytes;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// 最多发多少个子项名。
pub const MAX_CHILDREN: usize = 20;
/// AI 结果的可信度（固定的低值：它只看到了名字和大小）。
pub const LLM_CONFIDENCE: f32 = 0.3;
/// AI 回复每一项最多保留多少个字。
const MAX_FIELD_CHARS: usize = 300;

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

// ───────────────────────── 发什么 ─────────────────────────

/// 发给模型的全部信息，只有四个字段。结构体里没有任何“文件内容”字段——测试会序列化后逐项检查。
#[derive(Debug, Clone, Serialize)]
pub struct LlmInput {
    /// 文件夹 / 文件名（用户名已换成 `~`）。
    pub name: String,
    /// 父路径（用户目录已替换成 `~`）。
    pub parent: String,
    /// 最多 20 项：子文件夹名（按大小，后面带 `\`，已脱敏），再加“.扩展名 文件 × 个数”（文件不发名字）。
    pub children: Vec<String>,
    /// 总大小（如 "1.23 GB"）。
    pub size: String,
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

/// 不分大小写地把 `hay` 里出现的 `needle` 换成 `rep`（逐字符比较，中文用户名也行）。
pub fn replace_ci(hay: &str, needle: &str, rep: &str) -> String {
    let low = |c: char| c.to_lowercase().next().unwrap_or(c);
    let n: Vec<char> = needle.chars().map(low).collect();
    if n.is_empty() {
        return hay.to_string();
    }
    let h: Vec<char> = hay.chars().collect();
    let mut out = String::with_capacity(hay.len());
    let mut i = 0;
    while i < h.len() {
        if i + n.len() <= h.len() && h[i..i + n.len()].iter().map(|&c| low(c)).eq(n.iter().copied()) {
            out.push_str(rep);
            i += n.len();
        } else {
            out.push(h[i]);
            i += 1;
        }
    }
    out
}

/// 要隐去的用户名：路径里 `X:\Users\<名字>` 的那个名字，加上调用方给的（比如 %USERPROFILE% 的最后一段）。
/// `Public`、`Default` 这类公共目录名不算。
fn usernames(path: &str, extra: &[String]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let parts: Vec<&str> = path.split('\\').collect();
    if parts.len() >= 3 && parts[1].eq_ignore_ascii_case("users") && !parts[2].is_empty() {
        v.push(parts[2].to_string());
    }
    v.extend(extra.iter().cloned());
    v.retain(|u| {
        let l = u.trim().to_lowercase();
        !l.is_empty() && !matches!(l.as_str(), "public" | "default" | "all users" | "default user")
    });
    v.sort();
    v.dedup();
    v
}

/// 用户名在名字里出现时替换的最短长度：2 个字（两个字的中文名很常见，第 1 轮安全评审第 9 条）。
/// 1 个字的不替换，否则名字里每个同样的字母都会被换掉。
const MIN_USER_CHARS: usize = 2;

/// 名字里的敏感片段打码：邮箱 → `<邮箱>`；`wxid_…` → `wxid_<隐藏>`；连续 5 位以上数字（手机号、QQ 号、
/// 身份证号、企业微信的长编号）→ `<数字>`。
pub fn mask_sensitive(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        // 邮箱：往前后找 [A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}
        if chars[i] == '@' {
            let is_local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
            let is_dom = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
            let mut back = 0;
            let out_chars: Vec<char> = out.chars().collect();
            while back < out_chars.len() && is_local(out_chars[out_chars.len() - 1 - back]) {
                back += 1;
            }
            let mut j = i + 1;
            while j < chars.len() && is_dom(chars[j]) {
                j += 1;
            }
            let dom: String = chars[i + 1..j].iter().collect();
            if back > 0 && dom.contains('.') && !dom.starts_with('.') {
                let keep: String = out_chars[..out_chars.len() - back].iter().collect();
                out = keep;
                out.push_str("<邮箱>");
                // 邮箱后面常跟 .ost / .pst 扩展名：域名里最后一个“.后缀”如果像扩展名就保留
                if let Some((_, ext)) = dom.rsplit_once('.') {
                    if ["ost", "pst", "txt", "json", "db", "log"].contains(&ext.to_ascii_lowercase().as_str()) {
                        out.push('.');
                        out.push_str(ext);
                    }
                }
                i = j;
                continue;
            }
        }
        // wxid_xxx
        if chars[i..].iter().take(5).collect::<String>().eq_ignore_ascii_case("wxid_") && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut j = i + 5;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            if j > i + 5 {
                out.push_str("wxid_<隐藏>");
                i = j;
                continue;
            }
        }
        // 连续数字
        if chars[i].is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j - i >= 5 {
                out.push_str("<数字>");
            } else {
                out.extend(&chars[i..j]);
            }
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 名字脱敏：整个名字就是用户名 → `~`；名字里含用户名（≥ 2 个字）→ 那一段换成 `~`；再打码敏感片段。
fn redact_name(name: &str, users: &[String]) -> String {
    let mut out = name.to_string();
    for u in users {
        if out.to_lowercase() == u.to_lowercase() {
            return "~".into();
        }
        if u.chars().count() >= MIN_USER_CHARS {
            out = replace_ci(&out, u, "~");
        }
    }
    mask_sensitive(&out)
}

/// 父路径只留最近两层（更上面换成 `…`），每一段也脱敏。`~` 开头的保留 `~`。
fn short_parent(parent: &str, users: &[String]) -> String {
    let red = redact_path(parent);
    let segs: Vec<&str> = red.split('\\').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return red;
    }
    let tail: Vec<String> = segs.iter().rev().take(2).rev().map(|s| if *s == "~" { "~".to_string() } else { redact_name(s, users) }).collect();
    if segs.len() <= 2 {
        let joined = tail.join("\\");
        if segs.len() == 1 && joined.ends_with(':') {
            return format!("{joined}\\");
        }
        return joined;
    }
    format!("…\\{}", tail.join("\\"))
}

/// 从节点收集要发的信息。只用扫描树里的名字和大小，不碰文件。
pub fn build_input(t: &Target) -> LlmInput {
    build_input_with(t, &[])
}

/// 同 [`build_input`]，额外指定要隐去的用户名。
pub fn build_input_with(t: &Target, extra_users: &[String]) -> LlmInput {
    let n = t.node;
    let users = usernames(t.path, extra_users);
    let trimmed = t.path.trim_end_matches('\\');
    let parent_raw = match trimmed.rsplit_once('\\') {
        Some((p, _)) if p.len() == 2 && p.ends_with(':') => format!("{p}\\"),
        Some((p, _)) => p.to_string(),
        None => String::new(),
    };
    let parent = short_parent(&parent_raw, &users);
    let name = if redact_path(trimmed) == "~" { "~".to_string() } else { redact_name(&n.name, &users) };
    // 子文件夹发（脱敏后的）名字；文件只发“扩展名 × 个数”，不发文件名（文件名最容易带出个人信息，
    // 比如“张三的简历.docx”“身份证正反面.jpg”，第 1 轮安全评审第 9 条）。
    let mut children: Vec<String> = n
        .children
        .iter()
        .filter(|c| c.kind == NodeKind::Dir)
        .take(MAX_CHILDREN)
        .map(|c| format!("{}\\", redact_name(&c.name, &users)))
        .collect();
    let mut exts: Vec<(String, u64, usize)> = Vec::new();
    for c in n.children.iter().filter(|c| c.kind == NodeKind::File) {
        let e = super::heuristics::ext_of(&c.name);
        let key = if e.is_empty() { "（无扩展名）".to_string() } else { format!(".{}", mask_sensitive(&e)) };
        match exts.iter_mut().find(|x| x.0 == key) {
            Some(x) => {
                x.1 += c.allocated;
                x.2 += 1;
            }
            None => exts.push((key, c.allocated, 1)),
        }
    }
    exts.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, _, cnt) in exts {
        if children.len() >= MAX_CHILDREN {
            break;
        }
        children.push(format!("{k} 文件 × {cnt}"));
    }
    LlmInput { name, parent, children, size: human_bytes(n.allocated) }
}

pub const SYSTEM_PROMPT: &str = "你是 Windows 磁盘清理助手。用户会给你一个 Windows 上的文件夹或文件的名字、所在位置、最多 20 项子项摘要（子文件夹名；文件只给扩展名和个数）和总大小（没有文件内容，个人信息已打码）。\
请推断它可能是什么、为什么会有、删了会失去什么。\
要求：1) 用给普通人看的中文大白话，每项一两句；2) 这是推断不是确认，拿不准就直说“不确定”，并在 lose 里提醒用户先确认再删；3) 不要判断“能不能删”，不要建议直接删除；\
4) 只输出一个 JSON 对象，不要任何多余文字，只有三个字段：{\"what\":\"这是什么\",\"why\":\"为什么会有\",\"lose\":\"删了会失去什么、会不会丢数据\"}";

/// 请求体（OpenAI 兼容 chat/completions）。
pub fn build_request_body(model: &str, input: &LlmInput) -> serde_json::Value {
    let user = format!("请推断下面这一项是什么（这是推断，不是确认）：\n{}", serde_json::to_string_pretty(input).unwrap_or_default());
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

// ───────────────────────── 收什么 ─────────────────────────

/// 模型的回答：只认这三项。模型多给的字段（tier、owner、confidence……）反序列化时直接丢掉。
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct LlmAnswer {
    pub what: String,
    pub why: String,
    pub lose: String,
}

/// 去掉控制字符（保留换行）和首尾空白，最多保留 [`MAX_FIELD_CHARS`] 个字。
fn clean_field(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control() || *c == '\n').collect();
    let t = t.trim();
    if t.chars().count() > MAX_FIELD_CHARS {
        format!("{}…", t.chars().take(MAX_FIELD_CHARS).collect::<String>())
    } else {
        t.to_string()
    }
}

/// 从模型返回的文本里抠出 JSON（有的模型会包一层 ```json ... ``` 或多说几句）。
pub fn parse_answer(content: &str) -> Result<LlmAnswer, String> {
    let start = content.find('{').ok_or("AI 的回复里没有 JSON")?;
    let end = content.rfind('}').ok_or("AI 的回复里没有 JSON")?;
    if end <= start {
        return Err("AI 的回复里没有完整的 JSON".into());
    }
    let a: LlmAnswer = serde_json::from_str(&content[start..=end]).map_err(|e| format!("AI 回复的 JSON 解析失败：{e}"))?;
    let a = LlmAnswer { what: clean_field(&a.what), why: clean_field(&a.why), lose: clean_field(&a.lose) };
    if a.what.is_empty() {
        return Err("AI 没有给出“这是什么”".into());
    }
    Ok(a)
}

// ───────────────────────── 网络 ─────────────────────────

fn agent(timeout: Duration) -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .build()
        .new_agent()
}

/// 地址检查：必须 http(s)；http 只允许本机（localhost / 127.x / [::1]），远程地址必须 https，
/// 否则 API Key 和请求内容会明文走网络（第 1 轮安全评审第 16 条）。
pub fn check_url(url: &str) -> Result<(), String> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    let Some(rest) = lower.strip_prefix("http://") else {
        return Err("地址要以 https://（或本机的 http://）开头".into());
    };
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some(h) = host_port.strip_prefix('[') {
        h.split(']').next().unwrap_or("")
    } else {
        host_port.rsplit_once(':').map(|(h, _)| h).unwrap_or(host_port)
    };
    let local = host == "localhost" || host == "::1" || host.starts_with("127.");
    if local {
        Ok(())
    } else {
        Err(format!("远程地址必须用 https://（现在是 http://{host}，API Key 和请求内容会明文传输）；只有本机地址（localhost / 127.0.0.1）可以用 http://"))
    }
}

/// 发一次 chat 请求，返回 assistant 的文本。所有错误都转成中文字符串，不 panic。
fn chat(settings: &LlmSettings, body: &serde_json::Value, timeout: Duration) -> Result<String, String> {
    let url = chat_url(&settings.base_url);
    check_url(&url)?;
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

/// 能回答问题的模型。生产用 [`HttpOracle`]（真的发 HTTP），测试用假的（不联网）。
pub trait LlmOracle {
    /// 模型名（写进 `Source::Llm`）。
    fn model(&self) -> String;
    /// 发一次 chat/completions 请求体，返回 assistant 的文本。
    fn complete(&self, body: &serde_json::Value) -> Result<String, String>;
}

/// 走 OpenAI 兼容 HTTP 接口的 oracle。
pub struct HttpOracle<'a> {
    pub settings: &'a LlmSettings,
}

impl LlmOracle for HttpOracle<'_> {
    fn model(&self) -> String {
        self.settings.model.trim().to_string()
    }
    fn complete(&self, body: &serde_json::Value) -> Result<String, String> {
        chat(self.settings, body, Duration::from_secs(90))
    }
}

// ───────────────────────── 问 ─────────────────────────

/// 问 AI（走 HTTP）：返回一条 `source = Llm`、`tier = review` 的解释。用户点“问 AI”时调用。
/// 只有规则和启发式都没认出来的项才能问（界面上也只对这种项显示按钮，这里再挡一次）。
pub fn ask(t: &Target, ctx: &ExplainCtx) -> Result<Explanation, String> {
    ensure_askable(t, ctx)?;
    ask_with(t, ctx, &HttpOracle { settings: &ctx.settings })
}

/// 能不能问 AI：只有“还没认出来”的文件夹 / 文件可以；规则库或启发式已经说明了的不发出去。
pub fn ensure_askable(t: &Target, ctx: &ExplainCtx) -> Result<(), String> {
    if t.node.kind == NodeKind::Rest {
        return Err("“其余小文件”不能问 AI".into());
    }
    if !super::explain(t, ctx).is_unknown() {
        return Err("这一项 DiskWhy 已经认出来了，不需要问 AI（只有“还没认出来”的项才能问，避免把不必要的信息发出去）".into());
    }
    Ok(())
}

/// 问 AI（oracle 由调用方给）。开关没打开时直接返回错误，不会调用 oracle。
pub fn ask_with(t: &Target, ctx: &ExplainCtx, oracle: &dyn LlmOracle) -> Result<Explanation, String> {
    if !ctx.settings.usable() {
        return Err("AI 推测还没启用：请在“设置”里打开开关，并填写地址和模型".into());
    }
    let extra: Vec<String> =
        ctx.env.get("USERPROFILE").and_then(|p| p.trim_end_matches('\\').rsplit('\\').next().map(str::to_string)).into_iter().collect();
    let input = build_input_with(t, &extra);
    let model = oracle.model();
    let body = build_request_body(&model, &input);
    let content = oracle.complete(&body)?;
    let a = parse_answer(&content)?;
    let mut e = to_explanation(a, &model, t.node.allocated);
    // 归属不让 AI 猜：只用注册表里对得上的已安装软件。
    e.owner = owner_from_path(t.path, ctx).filter(|o| o.installed).map(|o| o.label());
    Ok(super::enforce_guards(e, t.path, ctx))
}

/// 模型回答 → 解释。永远 review，可信度固定低值，永远不能变成 safe。
pub fn to_explanation(a: LlmAnswer, model: &str, allocated: u64) -> Explanation {
    Explanation {
        tier: Tier::Review,
        owner: None,
        category: Category::Unknown,
        what: a.what,
        why: if a.why.is_empty() { "AI 没说。".into() } else { a.why },
        lose: if a.lose.is_empty() { "AI 没说清楚。请先确认再决定。".into() } else { a.lose },
        how: vec![How::Manual],
        how_detail: "这是 AI 根据名字和大小做的推测，不是确认。删之前请自己再核实一下（搜一下文件夹名、看看里面的文件）。".into(),
        tool: None,
        gain_bytes: 0,
        gain_note: format!("占 {}；AI 推测不作为能否删除的依据", human_bytes(allocated)),
        source: Source::Llm { model: model.to_string() },
        confidence: LLM_CONFIDENCE,
        refs: Vec::new(),
        observed_only: false,
        verified_on: None,
        notes: vec!["AI 推测：只发送了名字、最近两层位置、子文件夹名和各类文件的个数、总大小（用户名、邮箱、长数字已打码），没有发送文件名和文件内容。".into()],
        clues: Vec::new(),
    }
}
