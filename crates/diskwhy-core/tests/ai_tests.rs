//! 阶段 M：AI 自动识别（2026-10-11「免费智谱 + 自动识别」）。
//!
//! - 按服务商补的参数（智谱 / DeepSeek / Kimi / 火山 → thinking disabled；通义 / 硅基 → enable_thinking false；
//!   智谱 / DeepSeek / 通义 → JSON 模式；OpenAI / 本机 Ollama 什么都不加）；400 点名时去掉再试；
//! - 出错分类（[`AskError`]）：本地假 HTTP 服务器（127.0.0.1）回 429 / 5xx / 智谱业务码 / 401 / 404 / 超时 / 断线……；
//! - AI 缓存 [`AiStore`]：读写、坏文件、上限、大小写；
//! - `explain()` 只在“未知”时附上 AI 的说法，档位 / 来源 / 各档合计 / 清理候选一概不变；
//! - `ai_candidates`：门槛、上限、跳过规则、排序、问过的不再问；
//! - 批量执行器 `run_batch`：重试等待、配置错误整批停、连续 3 项失败停、取消、请求之间停一下、一次只问一个。
//!
//! 不连任何外网（唯一例外是 `#[ignore]` 的智谱连通性探测，手动跑，用假密钥）。

use diskwhy_core::explain::ai_batch::{ask_manual, ask_one, recheck_batch, run_batch, AiStatus, BatchConfig, BatchEnd, BatchSlot, ManualAnswer};
use diskwhy_core::explain::ai_store::{ai_candidates, personal_content, today, AiNote, AiStore, AI_STORE_CAP};
use diskwhy_core::explain::candidates::cleanup_candidates;
use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::{self, AskError, LlmInput, LlmSettings};
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::RuleSet;
use diskwhy_core::explain::{tier_summary, ExplainCtx, Explanation, Source, Target, Tier};
use diskwhy_core::view::{self, explain_path, listing};
use diskwhy_core::*;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ───────────────────────── 公共工具 ─────────────────────────

const MB: u64 = 1024 * 1024;

fn fake_env() -> EnvMap {
    EnvMap::from_pairs(&[
        ("SystemDrive", "C:"),
        ("SystemRoot", r"C:\Windows"),
        ("WINDIR", r"C:\Windows"),
        ("ProgramFiles", r"C:\Program Files"),
        ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
        ("ProgramData", r"C:\ProgramData"),
        ("USERPROFILE", r"C:\Users\bob"),
        ("LOCALAPPDATA", r"C:\Users\bob\AppData\Local"),
        ("APPDATA", r"C:\Users\bob\AppData\Roaming"),
        ("TEMP", r"C:\Users\bob\AppData\Local\Temp"),
        ("PUBLIC", r"C:\Users\Public"),
    ])
}

fn embedded_ctx() -> ExplainCtx {
    let env = fake_env();
    let (rules, errs) = RuleSet::embedded(&env);
    assert!(errs.is_empty(), "内置规则有错误：{errs:#?}");
    ExplainCtx::new(env, rules, RegistrySnapshot::default(), LlmSettings::default())
}

fn empty_ctx() -> ExplainCtx {
    ExplainCtx::new(fake_env(), RuleSet::default(), RegistrySnapshot::default(), LlmSettings::default())
}

fn file_node(name: &str, sz: u64) -> Node {
    Node {
        name: name.to_string(),
        path: String::new(),
        kind: NodeKind::File,
        logical: sz,
        allocated: sz,
        mtime: 1_700_000_000,
        attrs: 0x20,
        reparse_tag: 0,
        inaccessible: false,
        error_code: 0,
        reparse_skipped: false,
        cloud_placeholder: false,
        system_reserved: false,
        hardlink_dup: false,
        mapped: false,
        files: 1,
        dirs: 0,
        children: vec![],
    }
}

/// 内存里的树：文件路径 → 大小；以 `\` 结尾的是空目录。根固定为 `C:\`。
fn tree(entries: &[(&str, u64)]) -> Node {
    #[derive(Default)]
    struct D {
        dirs: BTreeMap<String, D>,
        files: Vec<(String, u64)>,
    }
    let mut root = D::default();
    for (p, sz) in entries {
        let rel = p.strip_prefix(r"C:\").expect("路径必须以 C:\\ 开头");
        let is_dir = rel.ends_with('\\');
        let parts: Vec<&str> = rel.split('\\').filter(|s| !s.is_empty()).collect();
        let mut cur = &mut root;
        let n = if is_dir { parts.len() } else { parts.len() - 1 };
        for part in &parts[..n] {
            cur = cur.dirs.entry(part.to_string()).or_default();
        }
        if !is_dir {
            cur.files.push((parts[n].to_string(), *sz));
        }
    }
    fn build(name: &str, path: &str, d: &D) -> Node {
        let mut children: Vec<Node> = Vec::new();
        for (n, sub) in &d.dirs {
            let p = if path.ends_with('\\') { format!("{path}{n}") } else { format!("{path}\\{n}") };
            children.push(build(n, &p, sub));
        }
        for (n, sz) in &d.files {
            children.push(file_node(n, *sz));
        }
        children.sort_by(|a, b| b.allocated.cmp(&a.allocated));
        Node {
            name: name.to_string(),
            path: path.to_string(),
            kind: NodeKind::Dir,
            logical: children.iter().map(|c| c.logical).sum(),
            allocated: children.iter().map(|c| c.allocated).sum(),
            mtime: 1_700_000_000,
            attrs: 0x10,
            reparse_tag: 0,
            inaccessible: false,
            error_code: 0,
            reparse_skipped: false,
            cloud_placeholder: false,
            system_reserved: false,
            hardlink_dup: false,
            mapped: false,
            files: children.iter().map(|c| c.files).sum(),
            dirs: children.iter().map(|c| if c.kind == NodeKind::Dir { 1 + c.dirs } else { 0 }).sum(),
            children,
        }
    }
    build(r"C:\", r"C:\", &root)
}

fn find_mut<'a>(n: &'a mut Node, path: &str) -> &'a mut Node {
    if n.path.eq_ignore_ascii_case(path) {
        return n;
    }
    let i = n
        .children
        .iter()
        .position(|c| c.kind == NodeKind::Dir && path.to_lowercase().starts_with(&c.path.to_lowercase()))
        .unwrap_or_else(|| panic!("找不到 {path}"));
    find_mut(&mut n.children[i], path)
}

fn result_of(t: Node) -> ScanResult {
    let mut r: ScanResult = serde_json::from_value(serde_json::json!({
        "root": "C:\\", "total_bytes": 0, "used_bytes": 0, "free_bytes": 0, "cluster_size": 4096,
        "buckets": Attribution::default(), "duration_ms": 0, "files": 0, "dirs": 0, "elevated": false,
        "inaccessible_total": 0, "inaccessible_dirs": [], "size_mode": "Exact", "top_files_per_dir": 50,
        "tree": file_node("x", 0), "mapped_dirs": []
    }))
    .unwrap_or_else(|e| panic!("ScanResult 结构变了，改一下这个测试：{e}"));
    r.used_bytes = t.allocated;
    r.total_bytes = t.allocated * 2;
    r.tree = t;
    r
}

fn note(what: &str) -> AiNote {
    AiNote {
        what: what.into(),
        owner: "某软件".into(),
        why: "自动生成".into(),
        lose: "删了要重新下".into(),
        confidence: 0.6,
        model: "glm-4.7-flash".into(),
        at: "2026-10-11".into(),
        size: 123,
    }
}

fn input() -> LlmInput {
    LlmInput {
        name: "qzx".into(),
        parent: "~".into(),
        kind: "文件夹".into(),
        total_size: "1 GB".into(),
        files: 3,
        dirs: 1,
        top_children: vec![("a.bin".into(), "1 GB".into())],
        extensions: vec![(".bin".into(), "100%".into())],
        modified: "2026-01-01".into(),
    }
}

fn settings(url: &str) -> LlmSettings {
    LlmSettings { enabled: true, base_url: url.into(), model: "fake-model".into(), api_key: "sk-test".into() }
}

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("diskwhy-ai-{tag}-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

// ───────────────────────── 本地假 HTTP 服务器 ─────────────────────────

/// 假服务器对一个连接怎么回。
#[derive(Clone)]
enum Resp {
    /// 回 (状态码, 原样的 body)。
    Raw(u16, String),
    /// 回 200 + OpenAI 格式，content = 给的文字。
    Chat(String),
    /// 读完请求，一直不回（客户端超时）。
    Hang(Duration),
    /// 读完请求，直接关掉连接（不回任何东西）。
    Close,
}

fn read_request(sock: &mut std::net::TcpStream) -> String {
    sock.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let n = sock.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        let s = String::from_utf8_lossy(&buf);
        if let Some(hdr_end) = s.find("\r\n\r\n") {
            let cl = s[..hdr_end]
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            if buf.len() >= hdr_end + 4 + cl {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// 按顺序回 `script` 里的每一条（一个连接一条），返回 (base_url, 收到的原始请求)。
fn fake_http(script: Vec<Resp>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        let mut got = Vec::new();
        for r in script {
            let (mut sock, _) = listener.accept().unwrap();
            got.push(read_request(&mut sock));
            let (status, body) = match r {
                Resp::Raw(s, b) => (s, b),
                Resp::Chat(c) => (200, serde_json::json!({"choices":[{"message":{"role":"assistant","content":c}}]}).to_string()),
                Resp::Hang(d) => {
                    std::thread::sleep(d);
                    continue;
                }
                Resp::Close => {
                    let _ = sock.shutdown(std::net::Shutdown::Both);
                    continue;
                }
            };
            let resp = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes());
        }
        got
    });
    (format!("http://127.0.0.1:{port}/v1"), h)
}

fn body_of(raw: &str) -> serde_json::Value {
    let i = raw.find("\r\n\r\n").expect("没有请求体");
    serde_json::from_str(&raw[i + 4..]).expect("请求体不是 JSON")
}

const GOOD: &str = r#"{"what":"像是某个游戏平台自动录下的视频","owner":"未知","why":"自动录制","lose":"录像就没了","confidence":0.95}"#;

// ───────────────────────── 1. 按服务商补的参数 ─────────────────────────

#[test]
fn provider_extras_per_host() {
    let keys = |u: &str| -> Vec<String> {
        let mut k: Vec<String> = llm::provider_extras(u).keys().cloned().collect();
        k.sort();
        k
    };
    let v = |xs: &[&str]| -> Vec<String> { xs.iter().map(|s| s.to_string()).collect() };
    // 智谱（官方 base_url）、国际版
    assert_eq!(keys("https://open.bigmodel.cn/api/paas/v4"), v(&["response_format", "thinking"]));
    assert_eq!(keys("https://api.z.ai/api/paas/v4"), v(&["response_format", "thinking"]));
    // DeepSeek
    assert_eq!(keys("https://api.deepseek.com"), v(&["response_format", "thinking"]));
    assert_eq!(keys("https://api.deepseek.com/v1"), v(&["response_format", "thinking"]));
    // Kimi / 火山方舟：只关思考
    assert_eq!(keys("https://api.moonshot.cn/v1"), v(&["thinking"]));
    assert_eq!(keys("https://api.kimi.com/v1"), v(&["thinking"]));
    // Kimi Code（api.kimi.com / api.kimi.ai）：effort none = thinking.type disabled；不加 response_format
    assert_eq!(keys("https://api.kimi.com/coding/v1"), v(&["thinking"]));
    assert_eq!(keys("https://api.kimi.ai/coding/v1"), v(&["thinking"]));
    assert_eq!(keys("https://api.moonshot.ai/v1"), v(&["thinking"]));
    assert!(keys("https://notkimi.ai/v1").is_empty());
    assert_eq!(keys("https://ark.cn-beijing.volces.com/api/v3"), v(&["thinking"]));
    // 通义（百炼）：enable_thinking + JSON 模式；硅基流动：只 enable_thinking
    assert_eq!(keys("https://dashscope.aliyuncs.com/compatible-mode/v1"), v(&["enable_thinking", "response_format"]));
    assert_eq!(keys("https://api.siliconflow.cn/v1"), v(&["enable_thinking"]));
    // OpenAI、本机 Ollama、别的：什么都不加
    for u in ["https://api.openai.com/v1", "http://localhost:11434/v1", "http://127.0.0.1:11434/v1", "http://[::1]:11434/v1", "", "not a url"] {
        assert!(keys(u).is_empty(), "{u} 不该加参数：{:?}", keys(u));
    }
    // 只认域名本身和子域名（冒名的不算）；大小写、端口、用户名、结尾斜杠都不影响
    assert!(keys("https://notbigmodel.cn/v1").is_empty());
    assert!(keys("https://open.bigmodel.cn.evil.com/v1").is_empty());
    assert!(keys("https://evil.com/open.bigmodel.cn/v1").is_empty());
    assert_eq!(keys(" HTTPS://user:pw@Open.BigModel.CN:443/api/paas/v4/ "), v(&["response_format", "thinking"]));
    // 值
    let e = llm::provider_extras("https://open.bigmodel.cn/api/paas/v4");
    assert_eq!(e["thinking"], serde_json::json!({"type": "disabled"}));
    assert_eq!(e["response_format"], serde_json::json!({"type": "json_object"}));
    assert_eq!(llm::provider_extras("https://dashscope.aliyuncs.com/compatible-mode/v1")["enable_thinking"], serde_json::json!(false));
    assert_eq!(llm::host_of("https://Open.BigModel.cn:8443/x"), "open.bigmodel.cn");
    assert_eq!(llm::host_of("localhost:11434"), "localhost");
}

#[test]
fn request_bodies_merge_extras() {
    let b = llm::build_request_body("https://open.bigmodel.cn/api/paas/v4", "glm-4.7-flash", &input());
    assert_eq!(b["model"], "glm-4.7-flash");
    assert_eq!(b["thinking"], serde_json::json!({"type": "disabled"}));
    assert_eq!(b["response_format"], serde_json::json!({"type": "json_object"}));
    // JSON 模式要求提示词里有 “json”（DeepSeek / 通义 / OpenAI 的规定）
    assert!(b.to_string().to_lowercase().contains("json"));
    for k in ["temperature", "top_p", "max_tokens"] {
        assert!(b.get(k).is_none(), "不该发 {k}");
    }
    // Kimi Code / Kimi 开放平台：关思考，没有 JSON 模式
    for u in ["https://api.kimi.com/coding/v1", "https://api.kimi.ai/coding/v1", "https://api.moonshot.cn/v1"] {
        let k = llm::build_request_body(u, "kimi-for-coding", &input());
        assert_eq!(k["thinking"], serde_json::json!({"type": "disabled"}), "{u}");
        assert!(k.get("response_format").is_none(), "{u} 不该开 JSON 模式");
        assert!(k.get("enable_thinking").is_none(), "{u}");
        let t = llm::test_request_body(u, "kimi-for-coding");
        assert_eq!(t["thinking"], serde_json::json!({"type": "disabled"}), "{u}");
        assert!(t.get("response_format").is_none(), "{u}");
    }
    let q = llm::build_request_body("https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen-flash", &input());
    assert_eq!(q["enable_thinking"], false);
    assert!(q.get("thinking").is_none());
    // 本机 / OpenAI：和以前一样，只有 model / stream / messages
    let o = llm::build_request_body("http://localhost:11434/v1", "qwen3.5:9b", &input());
    let mut ks: Vec<&String> = o.as_object().unwrap().keys().collect();
    ks.sort();
    assert_eq!(ks, vec!["messages", "model", "stream"]);
    // 测试连接：关思考，但不开 JSON 模式（那句话不是要 JSON 的）
    let t = llm::test_request_body("https://open.bigmodel.cn/api/paas/v4", "glm-4.7-flash");
    assert_eq!(t["thinking"], serde_json::json!({"type": "disabled"}));
    assert!(t.get("response_format").is_none());
    assert_eq!(t["messages"][0]["content"], "只回复两个字：你好");
    let t = llm::test_request_body("https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen-flash");
    assert_eq!(t["enable_thinking"], false);
    assert!(t.get("response_format").is_none());
    let t = llm::test_request_body("http://localhost:11434/v1", "qwen3.5:9b");
    assert_eq!(t.as_object().unwrap().len(), 3);
}

#[test]
fn params_to_drop_knows_thinking_and_json_mode() {
    let b = serde_json::json!({"model":"m","thinking":{"type":"disabled"},"response_format":{"type":"json_object"},"messages":[]});
    let e = r#"{"error":{"message":"Unrecognized request argument supplied: thinking","type":"invalid_request_error"}}"#;
    assert_eq!(llm::params_to_drop(&b, 400, e), vec!["thinking"]);
    let e = r#"{"error":{"message":"response_format type json_object is not supported by this model"}}"#;
    assert_eq!(llm::params_to_drop(&b, 400, e), vec!["response_format"]);
    let q = serde_json::json!({"model":"m","enable_thinking":false,"messages":[]});
    let e = r#"{"error":{"message":"parameter.enable_thinking must be set to false for non-streaming calls"}}"#;
    assert_eq!(llm::params_to_drop(&q, 400, e), vec!["enable_thinking"]);
    // 不是 400 / 请求里没有 → 不去
    assert!(llm::params_to_drop(&b, 500, "thinking").is_empty());
    let plain = serde_json::json!({"model":"m","messages":[]});
    assert!(llm::params_to_drop(&plain, 400, "thinking response_format enable_thinking").is_empty());
}

/// 真的发出去：服务器先 400 点名 thinking，再 400 点名 response_format，第三次才成功——两个都去掉、其余不动。
#[test]
fn chat_retries_without_params_the_server_named() {
    let (url, h) = fake_http(vec![
        Resp::Raw(400, r#"{"error":{"message":"Unknown parameter: 'thinking'."}}"#.into()),
        Resp::Raw(400, r#"{"error":{"message":"'response_format' is not supported"}}"#.into()),
        Resp::Chat("好".into()),
    ]);
    let body = serde_json::json!({"model":"m","stream":false,"thinking":{"type":"disabled"},"response_format":{"type":"json_object"},"messages":[{"role":"user","content":"x"}]});
    let r = llm::chat(&settings(&url), &body, Duration::from_secs(10)).expect("第三次应成功");
    assert_eq!(r, "好");
    let got = h.join().unwrap();
    assert_eq!(got.len(), 3);
    let b0 = body_of(&got[0]);
    let b1 = body_of(&got[1]);
    let b2 = body_of(&got[2]);
    assert!(b0.get("thinking").is_some() && b0.get("response_format").is_some());
    assert!(b1.get("thinking").is_none() && b1.get("response_format").is_some());
    assert!(b2.get("thinking").is_none() && b2.get("response_format").is_none());
    assert_eq!(b2["model"], "m");
    assert!(got[0].contains("Bearer sk-test"));
}

// ───────────────────────── 2. 出错分类 ─────────────────────────

fn ask_against(script: Vec<Resp>, timeout: Duration) -> (Result<AiNote, AskError>, Vec<String>) {
    let (url, h) = fake_http(script);
    let r = llm::ask_input(&input(), 5 * MB, &settings(&url), timeout);
    (r, h.join().unwrap())
}

fn assert_kind(r: &Result<AiNote, AskError>, retryable: bool, fatal: bool, msg: &str) {
    let e = r.as_ref().expect_err("应该失败");
    assert_eq!((e.retryable, e.fatal, e.message.as_str()), (retryable, fatal, msg), "{e:?}");
    assert!(llm::ASK_MESSAGES.contains(&e.message.as_str()), "{} 不在 ASK_MESSAGES 里", e.message);
}

#[test]
fn ask_error_classification_against_fake_server() {
    let t = Duration::from_secs(10);
    // 可以等等再试
    assert_kind(&ask_against(vec![Resp::Raw(429, r#"{"error":{"message":"Rate limit reached"}}"#.into())], t).0, true, false, llm::MSG_RATE);
    assert_kind(&ask_against(vec![Resp::Raw(500, "oops".into())], t).0, true, false, llm::MSG_BUSY);
    assert_kind(&ask_against(vec![Resp::Raw(503, r#"{"error":{"message":"Server overloaded"}}"#.into())], t).0, true, false, llm::MSG_BUSY);
    // 智谱业务码在报错 JSON 里（不管 HTTP 状态码是多少都认）
    let z1302 = r#"{"error":{"code":"1302","message":"您当前使用该API的并发数过高，请降低并发，或联系客服增加限额。"}}"#;
    assert_kind(&ask_against(vec![Resp::Raw(429, z1302.into())], t).0, true, false, llm::MSG_RATE);
    assert_kind(&ask_against(vec![Resp::Raw(400, z1302.into())], t).0, true, false, llm::MSG_RATE);
    let z1305 = r#"{"error":{"code":1305,"message":"当前API请求过多，请稍后重试。"}}"#;
    assert_kind(&ask_against(vec![Resp::Raw(429, z1305.into())], t).0, true, false, llm::MSG_BUSY);
    // 配置不对：整批停下
    assert_kind(&ask_against(vec![Resp::Raw(401, r#"{"error":{"code":"1002","message":"Authorization Token非法，请确认Authorization Token正确传递。"}}"#.into())], t).0, false, true, llm::MSG_KEY);
    assert_kind(&ask_against(vec![Resp::Raw(401, r#"{"error":{"message":"Authentication Fails"}}"#.into())], t).0, false, true, llm::MSG_KEY);
    assert_kind(&ask_against(vec![Resp::Raw(403, "{}".into())], t).0, false, true, llm::MSG_NO_PERM);
    assert_kind(&ask_against(vec![Resp::Raw(404, r#"{"error":{"message":"model \"qwen3.5:9b\" not found, try pulling it first"}}"#.into())], t).0, false, true, llm::MSG_MODEL);
    assert_kind(&ask_against(vec![Resp::Raw(400, r#"{"error":{"code":"1211","message":"模型不存在，请检查模型代码。"}}"#.into())], t).0, false, true, llm::MSG_MODEL);
    assert_kind(&ask_against(vec![Resp::Raw(400, r#"{"error":{"message":"Model Not Exist"}}"#.into())], t).0, false, true, llm::MSG_MODEL);
    assert_kind(&ask_against(vec![Resp::Raw(404, "<html>404 page not found</html>".into())], t).0, false, true, llm::MSG_URL);
    assert_kind(&ask_against(vec![Resp::Raw(402, r#"{"error":{"message":"Insufficient Balance"}}"#.into())], t).0, false, true, llm::MSG_BALANCE);
    assert_kind(&ask_against(vec![Resp::Raw(429, r#"{"error":{"code":"1113","message":"您的账户已欠费"}}"#.into())], t).0, false, true, llm::MSG_BALANCE);
    assert_kind(&ask_against(vec![Resp::Raw(429, r#"{"error":{"code":"insufficient_quota","message":"You exceeded your current quota"}}"#.into())], t).0, false, true, llm::MSG_QUOTA);
    // 200 但不是 JSON / 没有 choices：地址填成了网页
    assert_kind(&ask_against(vec![Resp::Raw(200, "<html>hello</html>".into())], t).0, false, true, llm::MSG_URL);
    assert_kind(&ask_against(vec![Resp::Raw(200, r#"{"data":1}"#.into())], t).0, false, true, llm::MSG_URL);
    // 只是这一项失败
    assert_kind(&ask_against(vec![Resp::Chat("我不知道这是什么".into())], t).0, false, false, llm::MSG_BAD_ANSWER);
    assert_kind(&ask_against(vec![Resp::Raw(400, r#"{"error":{"code":"1301","message":"系统检测到输入或生成内容可能包含不安全或敏感内容"}}"#.into())], t).0, false, false, llm::MSG_REJECTED);
    assert_kind(&ask_against(vec![Resp::Raw(400, r#"{"error":{"message":"bad request"}}"#.into())], t).0, false, false, llm::MSG_REJECTED);
    assert_kind(&ask_against(vec![Resp::Raw(200, r#"{"choices":[{"message":{"role":"assistant","content":null}}]}"#.into())], t).0, false, false, llm::MSG_NO_ANSWER);
}

#[test]
fn ask_error_timeout_reset_refused_and_bad_url() {
    // 超时：服务器收了请求一直不回
    let start = Instant::now();
    let (r, _) = ask_against(vec![Resp::Hang(Duration::from_secs(4))], Duration::from_secs(1));
    assert_kind(&r, true, false, llm::MSG_TIMEOUT);
    assert!(start.elapsed() < Duration::from_secs(8));
    // 收了请求直接断开
    let (r, _) = ask_against(vec![Resp::Close], Duration::from_secs(5));
    let e = r.expect_err("应该失败");
    assert!(e.retryable && !e.fatal, "断线应能重试：{e:?}");
    // 连不上（端口没人听）：整批停
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let r = llm::ask_input(&input(), 1, &settings(&format!("http://127.0.0.1:{port}/v1")), Duration::from_secs(5));
    assert_kind(&r, false, true, llm::MSG_CONNECT);
    // 地址不对 / 没打开
    assert_kind(&llm::ask_input(&input(), 1, &settings("ftp://x/v1"), Duration::from_secs(5)), false, true, llm::MSG_URL);
    let mut off = settings("http://127.0.0.1:9/v1");
    off.enabled = false;
    assert_kind(&llm::ask_input(&input(), 1, &off, Duration::from_secs(5)), false, true, llm::MSG_OFF);
}

#[test]
fn classify_http_pure() {
    let c = |s: u16, b: &str| {
        let e = llm::classify_http(s, b);
        (e.retryable, e.fatal, e.message)
    };
    assert_eq!(c(429, ""), (true, false, llm::MSG_RATE.into()));
    assert_eq!(c(502, ""), (true, false, llm::MSG_BUSY.into()));
    assert_eq!(c(408, ""), (true, false, llm::MSG_TIMEOUT.into()));
    assert_eq!(c(401, ""), (false, true, llm::MSG_KEY.into()));
    assert_eq!(c(400, r#"{"code":"InvalidApiKey","message":"Invalid API-key provided."}"#), (false, true, llm::MSG_KEY.into()));
    assert_eq!(c(400, r#"{"code":"Arrearage","message":"Access denied"}"#), (false, true, llm::MSG_BALANCE.into()));
    assert_eq!(c(429, r#"{"error":{"code":"1304","message":"该API的今日调用次数限额"}}"#), (false, true, llm::MSG_QUOTA.into()));
    assert_eq!(c(429, r#"{"error":{"code":"1303","message":"频率过高"}}"#), (true, false, llm::MSG_RATE.into()));
    assert_eq!(c(418, "teapot"), (false, false, llm::MSG_REJECTED.into()));
    let e = llm::classify_http(401, r#"{"error":{"message":"bad key"}}"#);
    assert!(e.detail.contains("HTTP 401") && e.detail.contains("bad key"), "{}", e.detail);
    assert_eq!(e.to_string(), llm::MSG_KEY, "Display = 短中文");
    assert!(e.detailed().starts_with(llm::MSG_KEY) && e.detailed().contains("HTTP 401"));
}

/// test_connection 出错时是“短中文（细节）”，成功时照旧。
#[test]
fn test_connection_errors_are_short_plus_detail() {
    let (url, h) = fake_http(vec![Resp::Raw(401, r#"{"error":{"message":"令牌已过期或验证不正确"}}"#.into())]);
    let e = llm::test_connection(&settings(&url)).unwrap_err();
    h.join().unwrap();
    assert!(e.starts_with(llm::MSG_KEY) && e.contains("HTTP 401") && e.contains("令牌已过期"), "{e}");
    let (url, h) = fake_http(vec![Resp::Chat("<think>嗯</think>你好".into())]);
    let ok = llm::test_connection(&settings(&url)).unwrap();
    let raw = h.join().unwrap();
    assert!(ok.contains("你好") && !ok.contains("think"), "{ok}");
    let b = body_of(&raw[0]);
    assert!(b.get("response_format").is_none() && b.get("thinking").is_none(), "本机地址不加参数：{b}");
}

// ───────────────────────── 3. 成功的答案 ─────────────────────────

#[test]
fn ask_input_returns_note() {
    let (r, got) = ask_against(vec![Resp::Chat(format!("```json\n{GOOD}\n```"))], Duration::from_secs(10));
    let n = r.expect("应成功");
    assert!(n.what.contains("游戏平台"));
    assert_eq!(n.owner, "", "“未知”当空");
    assert_eq!(n.model, "fake-model");
    assert_eq!(n.size, 5 * MB);
    assert_eq!(n.at, today());
    assert_eq!(n.at.len(), 10);
    assert!(n.confidence <= 0.9 + 1e-6, "AI 推测最多 0.9：{}", n.confidence);
    let b = body_of(&got[0]);
    assert_eq!(b["model"], "fake-model");
    // 很长的回答截到 600 字
    let long = "很".repeat(2000);
    let (r, _) = ask_against(vec![Resp::Chat(format!(r#"{{"what":"{long}","lose":"x"}}"#))], Duration::from_secs(10));
    let n = r.unwrap();
    assert_eq!(n.what.chars().count(), llm::NOTE_FIELD_MAX + 1);
    assert!(n.what.ends_with('…'));
}

/// 手动“问 AI”的旧接口 ask()：答案也存进 ctx.ai，之后解释“未知”项时带上。
#[test]
fn manual_ask_stores_into_ctx_ai() {
    let t = tree(&[(r"C:\qzxdata\blob.qzx", 300 * MB), (r"C:\qzxdata\b2.qzx", 100 * MB)]);
    let (url, h) = fake_http(vec![Resp::Chat(GOOD.into())]);
    let mut ctx = empty_ctx();
    ctx.settings = settings(&url);
    let (chain, node) = view::find_node(&t, r"C:\qzxdata").unwrap();
    let tg = Target { node, path: r"C:\qzxdata", parent: chain.last().copied(), ancestors: &chain };
    let e = llm::ask(&tg, &ctx).expect("应成功");
    h.join().unwrap();
    assert_eq!(e.tier, Tier::Review);
    assert_eq!(e.source, Source::Llm { model: "fake-model".into() });
    let stored = ctx.ai.get(r"c:\QZXDATA").expect("存进了 ctx.ai（大小写不敏感）");
    assert!(stored.what.contains("游戏平台"));
    assert_eq!(stored.size, 400 * MB);
    let (_, ex) = explain_path(&t, r"C:\qzxdata", &ctx).unwrap();
    assert!(ex.is_unknown());
    assert_eq!(ex.ai.as_ref().map(|a| a.what.as_str()), Some(stored.what.as_str()));
}

// ───────────────────────── 4. AI 缓存 ─────────────────────────

#[test]
fn ai_store_roundtrip_corrupt_and_case() {
    let d = temp_dir("store");
    let p = d.join("sub").join("ai-cache.json");
    // 文件不存在 → 空的；第一次 put 建目录、写文件
    let s = AiStore::load_from(&p);
    assert!(s.is_empty());
    s.put(r"C:\Users\Bob\Mystery", note("某游戏的录像")).unwrap();
    s.put(r"C:\big.bin", note("虚拟机磁盘")).unwrap();
    assert!(p.is_file());
    let left: Vec<String> = std::fs::read_dir(d.join("sub")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["ai-cache.json".to_string()], "临时文件改名后不留");
    let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(raw["version"], 1);
    assert_eq!(raw["items"][r"c:\users\bob\mystery"]["what"], "某游戏的录像", "键是小写完整路径：{raw}");
    assert_eq!(raw["items"][r"c:\big.bin"]["size"], 123);
    // 读回来，大小写不敏感
    let s2 = AiStore::load_from(&p);
    assert_eq!(s2.len(), 2);
    assert_eq!(s2.get(r"c:\users\BOB\mystery"), Some(note("某游戏的录像")));
    assert!(s2.contains(r"C:\BIG.BIN"));
    assert!(s2.get(r"C:\nope").is_none());
    // 同一路径覆盖
    s2.put(r"C:\BIG.bin", note("新的说法")).unwrap();
    assert_eq!(AiStore::load_from(&p).get(r"c:\big.bin").unwrap().what, "新的说法");
    assert_eq!(AiStore::load_from(&p).len(), 2);
    // 坏文件 / 版本不认识 / 结构不对 → 空的，不 panic；之后照样能写
    for bad in ["not json", "{\"version\":2,\"items\":{}}", "[1,2,3]", "{\"version\":1,\"items\":[]}", ""] {
        std::fs::write(&p, bad).unwrap();
        let s3 = AiStore::load_from(&p);
        assert!(s3.is_empty(), "坏文件 {bad:?} 应当成空的");
        s3.put(r"C:\x", note("x")).unwrap();
        assert_eq!(AiStore::load_from(&p).len(), 1);
    }
    // 缺字段的条目用默认值；what 为空的条目丢掉
    std::fs::write(&p, r#"{"version":1,"items":{"C:\\A":{"what":"只有这个"},"c:\\b":{"what":""}}}"#).unwrap();
    let s4 = AiStore::load_from(&p);
    assert_eq!(s4.len(), 1);
    let a = s4.get(r"c:\a").unwrap();
    assert_eq!((a.what.as_str(), a.size, a.at.as_str()), ("只有这个", 0, ""));
    // 只在内存里的：put 不写任何文件
    let m = AiStore::memory();
    m.put(r"C:\x", note("x")).unwrap();
    assert_eq!(m.len(), 1);
    assert!(m.path().is_none());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn ai_store_cap_drops_oldest() {
    let m = AiStore::memory();
    for i in 0..AI_STORE_CAP {
        let mut n = note("x");
        n.at = format!("2026-{:02}-{:02}", 1 + (i % 12), 1 + (i % 28));
        n.size = i as u64;
        m.put(&format!(r"C:\d{i}"), n).unwrap();
    }
    assert_eq!(m.len(), AI_STORE_CAP);
    // 最旧的是 2026-01-01（i = 0, 84, 168…，size 最小的是 i = 0）
    let mut fresh = note("新的");
    fresh.at = "2026-10-11".into();
    m.put(r"C:\fresh", fresh).unwrap();
    assert_eq!(m.len(), AI_STORE_CAP);
    assert!(m.contains(r"C:\fresh"));
    assert!(!m.contains(r"C:\d0"), "最旧（同一天里最小）的被丢掉");
    assert!(m.contains(r"C:\d84"));
    // 刚写入的那条即使日期最旧也不丢
    let mut ancient = note("很旧");
    ancient.at = "1999-01-01".into();
    m.put(r"C:\ancient", ancient).unwrap();
    assert!(m.contains(r"C:\ancient"));
    assert_eq!(m.len(), AI_STORE_CAP);
    // 文件里超过上限的，读进来时也裁掉
    let d = temp_dir("cap");
    let p = d.join("ai-cache.json");
    let mut items = serde_json::Map::new();
    for i in 0..AI_STORE_CAP + 5 {
        let mut n = note("x");
        n.at = if i < 5 { "2020-01-01".into() } else { "2026-10-01".into() };
        items.insert(format!(r"c:\f{i}"), serde_json::to_value(n).unwrap());
    }
    std::fs::write(&p, serde_json::json!({"version":1,"items":items}).to_string()).unwrap();
    let s = AiStore::load_from(&p);
    assert_eq!(s.len(), AI_STORE_CAP);
    for i in 0..5 {
        assert!(!s.contains(&format!(r"c:\f{i}")));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn ai_store_shared_across_ctx_clones_and_threads() {
    let ctx = empty_ctx();
    let copy = {
        let mut c = ctx.clone();
        c.settings = settings("http://x");
        c
    };
    copy.ai.put(r"C:\shared", note("共用")).unwrap();
    assert!(ctx.ai.contains(r"c:\SHARED"), "换设置复制出来的上下文和原来共用一份缓存");
    assert!(Arc::ptr_eq(&ctx.ai, &copy.ai));
    // 多线程同时写同一个文件
    let d = temp_dir("threads");
    let s = Arc::new(AiStore::load_from(&d.join("ai-cache.json")));
    let hs: Vec<_> = (0..8)
        .map(|t| {
            let s = s.clone();
            std::thread::spawn(move || {
                for i in 0..10 {
                    s.put(&format!(r"C:\t{t}\{i}"), note("x")).unwrap();
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(AiStore::load_from(&d.join("ai-cache.json")).len(), 80);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn json_shapes_for_the_frontend() {
    let n = note("这是什么");
    let v = serde_json::to_value(&n).unwrap();
    let mut ks: Vec<&String> = v.as_object().unwrap().keys().collect();
    ks.sort();
    assert_eq!(ks, vec!["at", "confidence", "lose", "model", "owner", "size", "what", "why"]);
    let s = serde_json::to_value(AiStatus::default()).unwrap();
    assert_eq!(s, serde_json::json!({"running": false, "done": 0, "total": 0, "failed": 0, "error": null}));
    let s = serde_json::to_value(AiStatus { running: true, done: 3, total: 12, failed: 1, error: Some("密钥不对".into()) }).unwrap();
    assert_eq!(s, serde_json::json!({"running": true, "done": 3, "total": 12, "failed": 1, "error": "密钥不对"}));
}

// ───────────────────────── 5. explain() 只附上，不改别的 ─────────────────────────

fn strip_ai(e: &Explanation) -> serde_json::Value {
    let mut v = serde_json::to_value(e).unwrap();
    v.as_object_mut().unwrap().remove("ai");
    v
}

#[test]
fn explain_attaches_ai_only_for_unknown_and_changes_nothing_else() {
    let t = tree(&[
        (r"C:\Users\bob\qzxmystery\blob1.qzx", 300 * MB),
        (r"C:\Users\bob\qzxmystery\blob2.qzx", 200 * MB),
        (r"C:\Users\bob\AppData\Local\Temp\junk.tmp", 150 * MB),
        (r"C:\Users\bob\AppData\Local\Microsoft\Windows\INetCache\IE\x.dat", 120 * MB),
        (r"C:\Users\bob\qzxsmall\a.qzx", 10 * MB),
    ]);
    let r = result_of(t);
    let ctx = embedded_ctx();
    let unk = r"C:\Users\bob\qzxmystery";
    let unk_file = r"C:\Users\bob\qzxmystery\blob1.qzx";
    let temp = r"C:\Users\bob\AppData\Local\Temp";
    let paths = [unk, unk_file, temp, r"C:\Users\bob\AppData\Local\Microsoft\Windows\INetCache", r"C:\Users\bob"];
    let before: Vec<Explanation> = paths.iter().map(|p| explain_path(&r.tree, p, &ctx).unwrap().1).collect();
    assert!(before[0].is_unknown() && before[1].is_unknown(), "前提：这两个是未知");
    assert!(!before[2].is_unknown(), "前提：Temp 有规则");
    assert!(before.iter().all(|e| e.ai.is_none()));
    // “未知”项在序列化时没有 ai 字段（前端靠有没有来判断）
    assert!(serde_json::to_value(&before[0]).unwrap().get("ai").is_none());
    let sum_before = tier_summary(&r.tree, &ctx);
    let cand_before = cleanup_candidates(&r.tree, &ctx, &|_| false);
    let tm_before = view::treemap(&r, r"C:\Users\bob", 3, Some(&ctx)).unwrap();

    // 每个路径都存一条 AI 答案（包括有规则的：不该被用上）
    for p in paths {
        ctx.ai.put(p, note(&format!("AI 说 {p} 是缓存，可以放心删"))).unwrap();
    }
    let after: Vec<Explanation> = paths.iter().map(|p| explain_path(&r.tree, p, &ctx).unwrap().1).collect();
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        assert_eq!(strip_ai(b), strip_ai(a), "{} 除了 ai 之外有变化", paths[i]);
        assert_eq!(a.ai.is_some(), b.is_unknown(), "{}：只有未知项才附上 AI", paths[i]);
    }
    let a0 = after[0].ai.as_ref().unwrap();
    assert!(a0.what.contains("可以放心删"));
    // AI 说“可以放心删”也没用：档位、来源、后果、能腾多少照旧
    assert_eq!(after[0].tier, Tier::Review);
    assert_eq!(after[0].source, Source::Unknown);
    assert_eq!(after[0].gain_bytes, 0);
    assert!(serde_json::to_value(&after[0]).unwrap().get("ai").is_some());
    // 各档合计、清理候选一字不差
    let mut s1 = serde_json::to_value(&sum_before).unwrap();
    let mut s2 = serde_json::to_value(tier_summary(&r.tree, &ctx)).unwrap();
    s1.as_object_mut().unwrap().remove("duration_ms");
    s2.as_object_mut().unwrap().remove("duration_ms");
    assert_eq!(s1, s2);
    assert_eq!(serde_json::to_value(&cand_before).unwrap(), serde_json::to_value(cleanup_candidates(&r.tree, &ctx, &|_| false)).unwrap());
    assert!(!cleanup_candidates(&r.tree, &ctx, &|_| false).iter().any(|g| g.paths.iter().any(|p| p.path.eq_ignore_ascii_case(unk))));

    // 列表：子项和当前目录都带上
    let l = listing(&r, r"C:\Users\bob", 100, Some(&ctx)).unwrap();
    let row = l.children.iter().find(|c| c.path.eq_ignore_ascii_case(unk)).unwrap();
    assert!(row.explanation.as_ref().unwrap().ai.is_some());
    let me = l.node.explanation.as_ref().unwrap();
    assert_eq!(me.ai.is_some(), me.is_unknown(), "当前目录：未知才附");
    let l2 = listing(&r, unk, 100, Some(&ctx)).unwrap();
    assert!(l2.node.explanation.as_ref().unwrap().ai.is_some(), "当前目录（列表顶上那行线索）也带上");
    let f = l2.children.iter().find(|c| c.name == "blob1.qzx").unwrap();
    assert!(f.explanation.as_ref().unwrap().ai.is_some(), "文件也带上");

    // 方块图：unknown 的块带 ai，颜色（tier）不变；有规则的块不带；没有 ai 时 JSON 里没有这个字段
    let tm = view::treemap(&r, r"C:\Users\bob", 3, Some(&ctx)).unwrap();
    fn walk<'a>(n: &'a view::TreemapNode, out: &mut Vec<&'a view::TreemapNode>) {
        out.push(n);
        for c in &n.children {
            walk(c, out);
        }
    }
    let (mut a, mut b) = (Vec::new(), Vec::new());
    walk(&tm_before, &mut b);
    walk(&tm, &mut a);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!((&x.path, &x.tier, &x.what, x.category, x.lose_kind), (&y.path, &y.tier, &y.what, y.category, y.lose_kind));
        assert_eq!(x.ai.is_some(), x.tier == "unknown" && x.kind != NodeKind::Rest && ctx.ai.contains(&x.path), "{}", x.path);
    }
    let unk_tm = a.iter().find(|n| n.path.eq_ignore_ascii_case(unk)).unwrap();
    assert_eq!(unk_tm.tier, "unknown");
    assert!(unk_tm.ai.is_some());
    let tm_json = serde_json::to_value(&tm).unwrap();
    assert_eq!(tm_json.get("ai").is_some(), tm.tier == "unknown", "根：只有未知时才有 ai 字段");
    assert!(serde_json::to_value(&tm_before).unwrap().get("ai").is_none(), "没问过时 JSON 里没有 ai 字段");
    // “其余小文件合计”的 path 是所在目录：不能把目录的答案附到它身上
    let mut t2 = r.tree.clone();
    let dir = find_mut(&mut t2, unk);
    let mut rest = file_node("（其余）", 5 * MB);
    rest.kind = NodeKind::Rest;
    rest.files = 7;
    dir.children.push(rest);
    let chain = view::find_dir(&t2, unk).unwrap();
    let dirn = *chain.last().unwrap();
    let restn = dirn.children.iter().find(|c| c.kind == NodeKind::Rest).unwrap();
    let e = diskwhy_core::explain::explain(&Target { node: restn, path: unk, parent: Some(dirn), ancestors: &chain }, &ctx);
    assert!(e.is_unknown() && e.ai.is_none(), "Rest 节点不附 AI");
}

// ───────────────────────── 6. 值得问的项 ─────────────────────────

#[test]
fn ai_candidates_threshold_cap_skip_sort_cached() {
    let mut t = tree(&[
        (r"C:\qzxbig\a.qzx", 900 * MB),
        (r"C:\qzxbig\inner\b.qzx", 300 * MB),
        (r"C:\qzxbig\inner\small.qzx", 20 * MB),
        (r"C:\qzxmid\c.qzx", 150 * MB),
        (r"C:\qzxsmall\d.qzx", 99 * MB),
        (r"C:\qzxnoacc\e.qzx", 500 * MB),
        (r"C:\qzxsys\f.qzx", 500 * MB),
        (r"C:\qzxcloud\g.qzx", 400 * MB),
        (r"C:\qzxlinks\h.qzx", 400 * MB),
        (r"C:\qzxmapped\i.qzx", 400 * MB),
        (r"C:\lonely.qzxbin", 250 * MB),
    ]);
    find_mut(&mut t, r"C:\qzxnoacc").inaccessible = true;
    // 系统保留：子树都是
    {
        let s = find_mut(&mut t, r"C:\qzxsys");
        s.system_reserved = true;
        for c in &mut s.children {
            c.system_reserved = true;
        }
    }
    for (dir, which) in [(r"C:\qzxcloud", 0), (r"C:\qzxlinks", 1), (r"C:\qzxmapped", 2)] {
        let d = find_mut(&mut t, dir);
        match which {
            0 => d.children[0].cloud_placeholder = true,
            1 => d.children[0].hardlink_dup = true,
            _ => d.mapped = true,
        }
    }
    let ctx = empty_ctx();
    let got = ai_candidates(&t, &ctx, 100 * MB, 40);
    let paths: Vec<&str> = got.iter().map(|(p, _)| p.as_str()).collect();
    // 从大到小；文件和文件夹都有；根本身不算；小于门槛的不算；无权限 / 系统保留 / 云占位 / 硬链接 / 映射都不算
    assert_eq!(got[0], (r"C:\qzxbig".to_string(), 1220 * MB), "{paths:#?}");
    for p in [r"C:\qzxbig\a.qzx", r"C:\qzxbig\inner", r"C:\qzxmid", r"C:\lonely.qzxbin"] {
        assert!(paths.contains(&p), "应包含 {p}：{paths:#?}");
    }
    for p in [r"C:\", r"C:\qzxsmall", r"C:\qzxsmall\d.qzx", r"C:\qzxbig\inner\small.qzx", r"C:\qzxnoacc", r"C:\qzxsys", r"C:\qzxsys\f.qzx", r"C:\qzxcloud\g.qzx", r"C:\qzxlinks\h.qzx", r"C:\qzxmapped"] {
        assert!(!paths.contains(&p), "不该包含 {p}：{paths:#?}");
    }
    // 占上级 ≥ 90% 的（b.qzx 是 inner 的 94%，c.qzx 是 qzxmid 的全部）和上级算同一样东西，不单独问
    for p in [r"C:\qzxbig\inner\b.qzx", r"C:\qzxmid\c.qzx"] {
        assert!(!paths.contains(&p), "套娃里的不该再问 {p}：{paths:#?}");
    }
    // 无权限目录里认得出的子项还是会看（e.qzx 本身不是无权限）
    assert!(paths.contains(&r"C:\qzxnoacc\e.qzx"));
    assert!(got.windows(2).all(|w| w[0].1 >= w[1].1), "从大到小：{got:?}");
    // 上限
    assert_eq!(ai_candidates(&t, &ctx, 100 * MB, 3).len(), 3);
    assert_eq!(ai_candidates(&t, &ctx, 100 * MB, 3)[0].0, r"C:\qzxbig");
    assert!(ai_candidates(&t, &ctx, 100 * MB, 0).is_empty());
    // 问过的不再问（大小写不敏感）
    ctx.ai.put(r"c:\QZXBIG", note("x")).unwrap();
    let got2 = ai_candidates(&t, &ctx, 100 * MB, 40);
    assert!(!got2.iter().any(|(p, _)| p == r"C:\qzxbig"));
    assert!(got2.iter().any(|(p, _)| p == r"C:\qzxbig\inner"), "问过的文件夹里面的照样看");
    assert_eq!(got2.len(), got.len() - 1);
    // 有规则 / 启发式认得出的不问
    let e = embedded_ctx();
    let t3 = tree(&[(r"C:\Users\bob\AppData\Local\Temp\x.tmp", 500 * MB), (r"C:\Windows\System32\big.dll", 500 * MB), (r"C:\Users\bob\qzxmine\x.qzx", 200 * MB)]);
    let g3 = ai_candidates(&t3, &e, 100 * MB, 40);
    let p3: Vec<&str> = g3.iter().map(|(p, _)| p.as_str()).collect();
    assert!(p3.contains(&r"C:\Users\bob\qzxmine"), "{p3:#?}");
    assert!(!p3.iter().any(|p| p.contains("Temp") || p.contains("Windows")), "{p3:#?}");
    // 个人文件夹（文档、桌面、下载……）和里面的东西不自动问（会把私人文件名发出去）
    let t4 = tree(&[
        (r"C:\Users\bob\Documents\qzxpriv\secret-plan.qzx", 500 * MB),
        (r"C:\Users\bob\Desktop\qzxstuff\x.qzx", 300 * MB),
        (r"C:\Users\bob\Downloads\qzxsetup.exe", 300 * MB),
        (r"C:\Users\bob\qzxother\y.qzx", 200 * MB),
    ]);
    let g4 = ai_candidates(&t4, &empty_ctx(), 100 * MB, 40);
    let p4: Vec<&str> = g4.iter().map(|(p, _)| p.as_str()).collect();
    assert!(p4.contains(&r"C:\Users\bob\qzxother"), "{p4:#?}");
    for bad in ["Documents", "Desktop", "Downloads", "secret", "qzxsetup"] {
        assert!(!p4.iter().any(|p| p.contains(bad)), "不该自动问 {bad}：{p4:#?}");
    }
}

/// 一层套一层、大小几乎一样的链（真机上的 pip\cache\http-v2\8\c\7\8\9\x.body）只问最上面那个；
/// 链里明显小一截（< 90%）的是另一样东西，照问；中间隔着认得出来的文件夹重新算；问过的上级也算数。
#[test]
fn ai_candidates_ask_each_nesting_chain_once() {
    let t = tree(&[
        // 套娃：qzxpip 1000 → cache 1000 → http 990 → 8 → c → x.body 950（都 ≥ 90%）
        (r"C:\qzxpip\cache\http\8\c\x.body", 950 * MB),
        (r"C:\qzxpip\cache\http\8\c\y.body", 30 * MB),
        (r"C:\qzxpip\cache\http\z.body", 10 * MB),
        (r"C:\qzxpip\cache\wheels.qzx", 10 * MB),
        // 链里小一截的：qzxapp 1000 → data 800（80%，问）→ db 790（data 的 99%，不问）→ big.qzx 780（不问）
        (r"C:\qzxapp\data\db\big.qzx", 780 * MB),
        (r"C:\qzxapp\data\db\small.qzx", 10 * MB),
        (r"C:\qzxapp\data\other.qzx", 10 * MB),
        (r"C:\qzxapp\bin\app.qzx", 200 * MB),
    ]);
    let ctx = empty_ctx();
    let got = ai_candidates(&t, &ctx, 100 * MB, 40);
    let paths: Vec<&str> = got.iter().map(|(p, _)| p.as_str()).collect();
    // 一样大时按路径排（qzxapp 在 qzxpip 前面）
    assert_eq!(paths, vec![r"C:\qzxapp", r"C:\qzxpip", r"C:\qzxapp\data", r"C:\qzxapp\bin"], "{got:#?}");
    // 上级问过了（在缓存里）：链里的照样不问，小一截的照问
    ctx.ai.put(r"C:\qzxpip", note("pip 的缓存")).unwrap();
    ctx.ai.put(r"C:\qzxapp", note("某个软件")).unwrap();
    let got2 = ai_candidates(&t, &ctx, 100 * MB, 40);
    let p2: Vec<&str> = got2.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(p2, vec![r"C:\qzxapp\data", r"C:\qzxapp\bin"], "{got2:#?}");
    // 中间隔着认得出来的文件夹（只说明自己这一层的“容器”规则，scope = self）：它下面认不出来的是另一样东西，照问
    const CONTAINER: &str = r#"
[[rule]]
id = "t-container"
owner = "测试"
category = "app"
tier = "review"
lose_kind = "unsure"
lose_short = "测试"
scope = "self"
match = ['%ProgramData%\qzxouter\qzxknown']
what = "测试用容器"
why = "测试"
lose = "测试"
how = "manual"
how_detail = ""
refs = ["https://example.com/"]
verified_on = "2026-10-12"
"#;
    let env = fake_env();
    let (rules, errs) = RuleSet::from_sources(&[("t", CONTAINER)], &env);
    assert!(errs.is_empty(), "{errs:?}");
    let c3 = ExplainCtx::new(env, rules, RegistrySnapshot::default(), LlmSettings::default());
    let t3 = tree(&[(r"C:\ProgramData\qzxouter\qzxknown\qzxinner\blob.qzx", 500 * MB)]);
    let src = |p: &str| view::explain_path(&t3, p, &c3).map(|(_, e)| e.source);
    assert!(!matches!(src(r"C:\ProgramData\qzxouter\qzxknown"), Some(Source::Unknown)), "{:?}", src(r"C:\ProgramData\qzxouter\qzxknown"));
    assert!(matches!(src(r"C:\ProgramData\qzxouter\qzxknown\qzxinner"), Some(Source::Unknown)), "{:?}", src(r"C:\ProgramData\qzxouter\qzxknown\qzxinner"));
    let g3 = ai_candidates(&t3, &c3, 100 * MB, 40);
    let p3: Vec<&str> = g3.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(p3, vec![r"C:\ProgramData", r"C:\ProgramData\qzxouter\qzxknown\qzxinner"], "{g3:#?}");
}

/// 大树上要快：小于门槛的子树整个跳过。20 万个文件、2000 个文件夹（只有少数 ≥ 100 MB）。
#[test]
fn ai_candidates_is_fast_on_a_big_tree() {
    let mut entries: Vec<(String, u64)> = Vec::new();
    for d in 0..2000 {
        for f in 0..100 {
            // 前 20 个文件夹各 ~150 MB，其余都很小
            let sz = if d < 20 { 1536 * 1024 } else { 4096 };
            entries.push((format!(r"C:\qzx{}\sub{}\f{f}.qzx", d / 50, d), sz));
        }
    }
    let refs: Vec<(&str, u64)> = entries.iter().map(|(p, s)| (p.as_str(), *s)).collect();
    let t = tree(&refs);
    let ctx = empty_ctx();
    let start = Instant::now();
    let got = ai_candidates(&t, &ctx, 100 * MB, 40);
    let el = start.elapsed();
    println!("ai_candidates on {} nodes: {:?}, {} candidates", 200_000 + 2000, el, got.len());
    assert!(!got.is_empty());
    assert!(el < Duration::from_secs(3), "太慢：{el:?}");
}

// ───────────────────────── 7. 批量执行器 ─────────────────────────

fn fast_cfg() -> BatchConfig {
    BatchConfig { backoff: vec![Duration::from_millis(5), Duration::from_millis(10), Duration::from_millis(20)], pause: Duration::from_millis(1), max_consecutive_failures: 3 }
}

fn items(n: usize) -> Vec<(String, u64)> {
    (0..n).map(|i| (format!(r"C:\item{i}"), (100 - i) as u64 * MB)).collect()
}

struct Rec {
    statuses: Vec<AiStatus>,
    answers: Vec<String>,
    asked: Vec<String>,
}

fn run(
    its: &[(String, u64)],
    cfg: &BatchConfig,
    cancel: &AtomicBool,
    store: &AiStore,
    mut f: impl FnMut(&str, usize) -> Result<AiNote, AskError>,
) -> (AiStatus, BatchEnd, Rec) {
    let rec = Mutex::new(Rec { statuses: vec![], answers: vec![], asked: vec![] });
    let (st, end) = run_batch(
        its,
        cfg,
        cancel,
        store,
        |p| {
            let n = {
                let mut r = rec.lock().unwrap();
                r.asked.push(p.to_string());
                r.asked.len()
            };
            f(p, n)
        },
        |s| rec.lock().unwrap().statuses.push(s.clone()),
        |p, n| {
            assert!(store.contains(p), "on_answer 之前已经存好");
            assert!(!n.what.is_empty());
            rec.lock().unwrap().answers.push(p.to_string());
        },
    );
    (st, end, rec.into_inner().unwrap())
}

#[test]
fn batch_all_succeed_and_skips_cached() {
    let store = AiStore::memory();
    store.put(r"C:\item1", note("早就问过")).unwrap();
    let cancel = AtomicBool::new(false);
    let (st, end, rec) = run(&items(3), &fast_cfg(), &cancel, &store, |p, _| Ok(note(&format!("答 {p}"))));
    assert_eq!(end, BatchEnd::Finished);
    assert_eq!(st, AiStatus { running: false, done: 3, total: 3, failed: 0, error: None });
    assert_eq!(rec.asked, vec![r"C:\item0", r"C:\item2"], "问过的不再发请求");
    assert_eq!(rec.answers, vec![r"C:\item0", r"C:\item2"]);
    assert_eq!(store.get(r"C:\item1").unwrap().what, "早就问过", "不覆盖");
    assert_eq!(store.get(r"C:\item2").unwrap().what, r"答 C:\item2");
    // 状态：开始（running, 0/3）→ 1/3 → 2/3 → 3/3 → 结束（running = false）
    let s = &rec.statuses;
    assert_eq!(s.first().unwrap(), &AiStatus { running: true, done: 0, total: 3, failed: 0, error: None });
    assert_eq!(s.last().unwrap(), &st);
    assert!(s[..s.len() - 1].iter().all(|x| x.running));
    let dones: Vec<usize> = s.iter().map(|x| x.done).collect();
    assert_eq!(dones, vec![0, 1, 2, 3, 3]);
    // 没有要问的：直接结束，不进入 running
    let (st, end, rec) = run(&[], &fast_cfg(), &cancel, &store, |_, _| panic!("不该问"));
    assert_eq!(end, BatchEnd::Finished);
    assert_eq!(st, AiStatus::default());
    assert_eq!(rec.statuses, vec![AiStatus::default()]);
}

#[test]
fn batch_retries_with_backoff_then_succeeds_or_fails_item() {
    let store = AiStore::memory();
    let cancel = AtomicBool::new(false);
    let cfg = BatchConfig { backoff: vec![Duration::from_millis(30), Duration::from_millis(60), Duration::from_millis(90)], pause: Duration::from_millis(1), max_consecutive_failures: 3 };
    // 第 1 项：两次 429 之后成功（等了 30 + 60 ms）
    let start = Instant::now();
    let tries = AtomicUsize::new(0);
    let (st, end, rec) = run(&items(1), &cfg, &cancel, &store, |_, _| {
        if tries.fetch_add(1, Ordering::SeqCst) < 2 {
            Err(AskError::retry(llm::MSG_RATE, "HTTP 429"))
        } else {
            Ok(note("终于答了"))
        }
    });
    assert!(start.elapsed() >= Duration::from_millis(90), "{:?}", start.elapsed());
    assert_eq!(end, BatchEnd::Finished);
    assert_eq!((st.done, st.failed), (1, 0));
    assert_eq!(rec.asked.len(), 3);
    // 一直 1302：1 次 + 重试 2 次 = 3 次（每项最多 3 次请求），然后这一项算失败，接着问下一项
    let cfg = BatchConfig { backoff: vec![Duration::from_millis(30), Duration::from_millis(60)], ..cfg };
    assert_eq!(1 + cfg.backoff.len(), diskwhy_core::explain::ai_batch::AI_MAX_ATTEMPTS);
    let store = AiStore::memory();
    let start = Instant::now();
    let (st, end, rec) = run(&items(2), &cfg, &cancel, &store, |p, _| {
        if p.ends_with("item0") {
            Err(AskError::retry(llm::MSG_RATE, "1302"))
        } else {
            Ok(note("ok"))
        }
    });
    assert!(start.elapsed() >= Duration::from_millis(90), "等了 30 + 60 ms：{:?}", start.elapsed());
    assert_eq!(rec.asked.iter().filter(|p| p.ends_with("item0")).count(), 3, "每项最多 3 次请求");
    assert_eq!(end, BatchEnd::Finished);
    assert_eq!(st, AiStatus { running: false, done: 2, total: 2, failed: 1, error: None });
    assert!(!store.contains(r"C:\item0") && store.contains(r"C:\item1"));
}

#[test]
fn batch_fatal_stops_immediately() {
    let store = AiStore::memory();
    let cancel = AtomicBool::new(false);
    let (st, end, rec) = run(&items(5), &fast_cfg(), &cancel, &store, |_, n| {
        if n == 2 {
            Err(AskError::fatal(llm::MSG_KEY, "HTTP 401"))
        } else {
            Ok(note("ok"))
        }
    });
    assert_eq!(end, BatchEnd::Stopped(llm::MSG_KEY.into()));
    assert_eq!(st, AiStatus { running: false, done: 1, total: 5, failed: 0, error: Some(llm::MSG_KEY.into()) });
    assert_eq!(rec.asked.len(), 2, "密钥不对不重试、不再问后面的");
    assert_eq!(rec.statuses.last().unwrap(), &st);
}

#[test]
fn batch_stops_after_three_consecutive_failures() {
    let store = AiStore::memory();
    let cancel = AtomicBool::new(false);
    // 失败、成功、失败、失败、成功……：没有连续 3 次 → 做完
    let (st, end, _) = run(&items(6), &fast_cfg(), &cancel, &store, |_, n| {
        if [1, 3, 4].contains(&n) {
            Err(AskError::item(llm::MSG_BAD_ANSWER, "x"))
        } else {
            Ok(note("ok"))
        }
    });
    assert_eq!(end, BatchEnd::Finished);
    assert_eq!((st.done, st.failed, st.error.clone()), (6, 3, None));
    // 连续 3 项失败（重试用完也算一项失败）→ 停下，error = 最后一项的原因
    let store = AiStore::memory();
    let (st, end, rec) = run(&items(6), &fast_cfg(), &cancel, &store, |p, _| {
        if p.ends_with("item0") {
            Ok(note("ok"))
        } else if p.ends_with("item1") {
            Err(AskError::retry(llm::MSG_BUSY, "503"))
        } else {
            Err(AskError::item(llm::MSG_BAD_ANSWER, "x"))
        }
    });
    assert_eq!(end, BatchEnd::Stopped(llm::MSG_BAD_ANSWER.into()));
    assert_eq!(st, AiStatus { running: false, done: 4, total: 6, failed: 3, error: Some(llm::MSG_BAD_ANSWER.into()) });
    assert!(!rec.asked.iter().any(|p| p.ends_with("item4") || p.ends_with("item5")));
}

#[test]
fn batch_cancel_between_items_and_during_waits() {
    // 第一项答完后取消：第二项不问
    let store = AiStore::memory();
    let cancel = AtomicBool::new(false);
    let (st, end, rec) = run(&items(3), &fast_cfg(), &cancel, &store, |_, _| {
        cancel.store(true, Ordering::SeqCst);
        Ok(note("ok"))
    });
    assert_eq!(end, BatchEnd::Cancelled);
    assert_eq!(rec.asked.len(), 1);
    assert_eq!((st.running, st.done, st.error.clone()), (false, 1, None));
    assert!(store.contains(r"C:\item0"), "取消前答上来的照样存");
    // 重试等待中（10 秒）取消：马上停
    let store = AiStore::memory();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    let cfg = BatchConfig { backoff: vec![Duration::from_secs(10); 3], pause: Duration::from_millis(1), max_consecutive_failures: 3 };
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        c2.store(true, Ordering::SeqCst);
    });
    let start = Instant::now();
    let (st, end, rec) = run(&items(3), &cfg, &cancel, &store, |_, _| Err(AskError::retry(llm::MSG_RATE, "429")));
    assert!(start.elapsed() < Duration::from_secs(3), "等待中要能取消：{:?}", start.elapsed());
    assert_eq!(end, BatchEnd::Cancelled);
    assert_eq!(rec.asked.len(), 1);
    assert!(!st.running);
    // 请求之间停一下的时候取消
    let store = AiStore::memory();
    let cancel = Arc::new(AtomicBool::new(false));
    let c3 = cancel.clone();
    let cfg = BatchConfig { backoff: vec![], pause: Duration::from_secs(10), max_consecutive_failures: 3 };
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        c3.store(true, Ordering::SeqCst);
    });
    let start = Instant::now();
    let (_, end, rec) = run(&items(3), &cfg, &cancel, &store, |_, _| Ok(note("ok")));
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(end, BatchEnd::Cancelled);
    assert_eq!(rec.asked.len(), 1);
    // 一开始就取消：一个都不问
    let store = AiStore::memory();
    let cancel = AtomicBool::new(true);
    let (st, end, rec) = run(&items(3), &fast_cfg(), &cancel, &store, |_, _| panic!("不该问"));
    assert_eq!(end, BatchEnd::Cancelled);
    assert!(rec.asked.is_empty());
    assert!(!st.running);
}

#[test]
fn batch_pauses_between_requests_and_never_overlaps() {
    let store = AiStore::memory();
    let cancel = AtomicBool::new(false);
    let cfg = BatchConfig { backoff: vec![], pause: Duration::from_millis(40), max_consecutive_failures: 3 };
    let in_flight = AtomicUsize::new(0);
    let last_end: Mutex<Option<Instant>> = Mutex::new(None);
    let gaps: Mutex<Vec<Duration>> = Mutex::new(vec![]);
    let start = Instant::now();
    let (st, _, _) = run(&items(4), &cfg, &cancel, &store, |_, _| {
        assert_eq!(in_flight.fetch_add(1, Ordering::SeqCst), 0, "同时只能有一个请求");
        if let Some(t) = *last_end.lock().unwrap() {
            gaps.lock().unwrap().push(t.elapsed());
        }
        std::thread::sleep(Duration::from_millis(5));
        in_flight.fetch_sub(1, Ordering::SeqCst);
        *last_end.lock().unwrap() = Some(Instant::now());
        Ok(note("ok"))
    });
    assert_eq!(st.done, 4);
    assert!(start.elapsed() >= Duration::from_millis(120), "{:?}", start.elapsed());
    assert!(gaps.lock().unwrap().iter().all(|g| *g >= Duration::from_millis(35)), "{:?}", gaps.lock().unwrap());
}

/// 默认节奏就是约定的：每项最多 3 次请求（中间等 3 / 8 秒）、请求之间 300 毫秒、连续 3 项失败停。
#[test]
fn batch_default_config_matches_contract() {
    let c = BatchConfig::default();
    assert_eq!(c.backoff, vec![Duration::from_secs(3), Duration::from_secs(8)]);
    assert_eq!(1 + c.backoff.len(), diskwhy_core::explain::ai_batch::AI_MAX_ATTEMPTS);
    assert_eq!(diskwhy_core::explain::ai_batch::AI_MAX_ATTEMPTS, 3);
    assert_eq!(c.pause, Duration::from_millis(300));
    assert_eq!(c.max_consecutive_failures, 3);
    assert_eq!(diskwhy_core::explain::ai_store::AI_MIN_BYTES, 100 * MB);
    assert_eq!(diskwhy_core::explain::ai_store::AI_MAX_ITEMS, 40);
}

/// 端到端：假服务器 + run_batch + llm::ask_input（批量里一次只发一个请求、答案存进缓存、401 整批停）。
#[test]
fn batch_end_to_end_against_fake_server() {
    let (url, h) = fake_http(vec![
        Resp::Chat(GOOD.into()),
        Resp::Raw(429, r#"{"error":{"code":"1302","message":"并发数过高"}}"#.into()),
        Resp::Chat(r#"{"what":"第二个","lose":""}"#.into()),
        Resp::Raw(401, r#"{"error":{"code":"1002","message":"Authorization Token非法"}}"#.into()),
    ]);
    let s = settings(&url);
    let store = AiStore::memory();
    let cancel = AtomicBool::new(false);
    let (st, end, rec) = run(&items(4), &fast_cfg(), &cancel, &store, |_, _| llm::ask_input(&input(), 7, &s, Duration::from_secs(10)));
    let got = h.join().unwrap();
    assert_eq!(got.len(), 4);
    assert_eq!(end, BatchEnd::Stopped(llm::MSG_KEY.into()));
    assert_eq!(st, AiStatus { running: false, done: 2, total: 4, failed: 0, error: Some(llm::MSG_KEY.into()) });
    assert_eq!(rec.answers, vec![r"C:\item0", r"C:\item1"]);
    assert_eq!(store.get(r"C:\item1").unwrap().what, "第二个");
    assert_eq!(store.get(r"C:\item1").unwrap().lose, "", "AI 没说就留空（界面用自己的兜底）");
}

// ───────────────────────── 7b. 界面壳的那一半：问一项（等扫描结果、排队、取消）和“当前这一批” ─────────────────────────

/// 跳过的项（扫描结果里已经没有了）算问完、不算失败，也不会凑成“连续 3 项失败”；取消的那一项让整批按取消结束。
#[test]
fn batch_skipped_items_are_not_failures_and_cancelled_ends_batch() {
    let store = AiStore::memory();
    let cancel = AtomicBool::new(false);
    let (st, end, rec) = run(&items(5), &fast_cfg(), &cancel, &store, |p, _| {
        if p.ends_with("item4") {
            Ok(note("ok"))
        } else {
            Err(AskError::skip(llm::MSG_NOT_FOUND, "gone"))
        }
    });
    assert_eq!(end, BatchEnd::Finished, "4 项接连跳过也不算连续失败");
    assert_eq!(st, AiStatus { running: false, done: 5, total: 5, failed: 0, error: None });
    assert_eq!(rec.answers, vec![r"C:\item4"]);
    let store = AiStore::memory();
    let (st, end, rec) = run(&items(3), &fast_cfg(), &cancel, &store, |_, n| if n == 2 { Err(AskError::cancelled()) } else { Ok(note("ok")) });
    assert_eq!(end, BatchEnd::Cancelled);
    assert_eq!((st.running, st.done, st.failed, st.error.clone()), (false, 1, 0, None), "取消不是出错");
    assert_eq!(rec.asked.len(), 2);
}

/// 清理 / 还原后的刷新把扫描结果拿走一会儿（审查发现的 must-fix）：自动识别等它回来，不算失败、不发请求，回来后照常问。
#[test]
fn ask_one_waits_while_the_scan_result_is_taken_for_a_refresh() {
    let slot: Arc<Mutex<Option<Arc<String>>>> = Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        *s2.lock().unwrap() = Some(Arc::new("tree".to_string()));
    });
    let http = Mutex::new(());
    let cancel = AtomicBool::new(false);
    let sent = AtomicUsize::new(0);
    let start = Instant::now();
    let r = ask_one(
        &cancel,
        Duration::from_secs(5),
        || slot.lock().unwrap().clone(),
        |t: &Arc<String>| Some(format!("input from {t}")),
        &http,
        || Ok(()),
        |input, ()| {
            sent.fetch_add(1, Ordering::SeqCst);
            assert_eq!(input, "input from tree");
            Ok(note("答了"))
        },
    );
    assert!(r.is_ok(), "{r:?}");
    assert!(start.elapsed() >= Duration::from_millis(140), "等到扫描结果回来才问：{:?}", start.elapsed());
    assert_eq!(sent.load(Ordering::SeqCst), 1);
    // 扫描结果放回来时引用计数没被多占（刷新用 Arc::make_mut，多一个引用就会把整棵树复制一份）
    assert_eq!(Arc::strong_count(slot.lock().unwrap().as_ref().unwrap()), 1);
}

#[test]
fn ask_one_skips_vanished_items_and_gives_up_waiting() {
    let http = Mutex::new(());
    let cancel = AtomicBool::new(false);
    // 扫描结果在，但这一项被清理掉了 → 跳过，不发请求
    let r = ask_one(&cancel, Duration::from_secs(1), || Some(1u8), |_| None::<String>, &http, || Ok(()), |_, ()| panic!("不该发请求"));
    let e = r.unwrap_err();
    assert!(e.skipped && !e.fatal && !e.retryable && !e.cancelled, "{e:?}");
    // 扫描结果一直不回来：等到上限后跳过（不是失败）
    let start = Instant::now();
    let r = ask_one(&cancel, Duration::from_millis(120), || None::<u8>, |_| Some(()), &http, || Ok(()), |_, ()| panic!("不该发请求"));
    assert!(r.unwrap_err().skipped);
    assert!(start.elapsed() >= Duration::from_millis(110) && start.elapsed() < Duration::from_secs(2));
    // 等的时候取消：马上返回“取消”
    let cancel2 = Arc::new(AtomicBool::new(false));
    let c2 = cancel2.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        c2.store(true, Ordering::SeqCst);
    });
    let start = Instant::now();
    let r = ask_one(&cancel2, Duration::from_secs(30), || None::<u8>, |_| Some(()), &http, || Ok(()), |_, ()| panic!("不该发请求"));
    assert!(r.unwrap_err().cancelled);
    assert!(start.elapsed() < Duration::from_secs(3));
}

/// 排队等请求锁的时候换了设置 / 关了 AI（审查发现的 should-fix）：拿到锁后一个请求都不发。
#[test]
fn ask_one_cancelled_while_queued_for_the_http_lock_sends_nothing() {
    let http = Arc::new(Mutex::new(()));
    let cancel = Arc::new(AtomicBool::new(false));
    // 另一个请求（比如手动“问 AI”）正占着锁
    let (h2, c2) = (http.clone(), cancel.clone());
    let holder = std::thread::spawn(move || {
        let _g = h2.lock().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        c2.store(true, Ordering::SeqCst); // 用户这时保存了新设置
        std::thread::sleep(Duration::from_millis(50));
    });
    std::thread::sleep(Duration::from_millis(20));
    let sent = AtomicUsize::new(0);
    let r = ask_one(&cancel, Duration::from_secs(5), || Some(()), |_| Some(()), &http, || Ok(()), |_, ()| {
        sent.fetch_add(1, Ordering::SeqCst);
        Ok(note("不该有"))
    });
    holder.join().unwrap();
    assert!(r.unwrap_err().cancelled);
    assert_eq!(sent.load(Ordering::SeqCst), 0, "旧设置的请求不能发出去");
}

/// run_batch + ask_one 一起：问到一半做了一次清理（扫描结果拿走 200 ms，其中一项被清掉了）——整批照常做完，
/// 不出现「AI 出错 · 找不到了」，失败数 0。
#[test]
fn batch_survives_a_cleanup_refresh_in_the_middle() {
    let names: Vec<String> = (0..6).map(|i| format!(r"C:\item{i}")).collect();
    let result: Arc<Mutex<Option<Arc<Vec<String>>>>> = Arc::new(Mutex::new(Some(Arc::new(names.clone()))));
    let http = Mutex::new(());
    let store = AiStore::memory();
    let cancel = AtomicBool::new(false);
    let sent = AtomicUsize::new(0);
    let its = items(6);
    let (st, end, _) = run(&its, &fast_cfg(), &cancel, &store, |path, n| {
        if n == 2 {
            // 第 2 项开始问之前：用户清理了 item3，刷新把扫描结果拿走 200 ms
            *result.lock().unwrap() = None;
            let r2 = result.clone();
            let left: Vec<String> = names.iter().filter(|p| !p.ends_with("item3")).cloned().collect();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                *r2.lock().unwrap() = Some(Arc::new(left));
            });
        }
        ask_one(
            &cancel,
            Duration::from_secs(5),
            || result.lock().unwrap().clone(),
            |t: &Arc<Vec<String>>| t.iter().find(|p| p.as_str() == path).cloned(),
            &http,
            || Ok(()),
            |p, ()| {
                sent.fetch_add(1, Ordering::SeqCst);
                Ok(note(&format!("答 {p}")))
            },
        )
    });
    assert_eq!(end, BatchEnd::Finished);
    assert_eq!(st, AiStatus { running: false, done: 6, total: 6, failed: 0, error: None });
    assert_eq!(sent.load(Ordering::SeqCst), 5, "被清掉的那一项不发请求");
    assert!(!store.contains(r"C:\item3"));
}

/// “当前这一批”的代号：新的扫描 / 保存设置 / 重试都是先 stop 再 begin；旧的那批的进度丢掉、取消标记被置上。
#[test]
fn batch_slot_generations_drop_stale_status() {
    let mut s = BatchSlot::default();
    // 第一批
    let (g1, changed) = s.stop();
    assert!(!changed, "本来就空闲：不用通知界面");
    let c1 = s.begin(g1).expect("代号没变，可以开始");
    assert!(s.report(g1, &AiStatus { running: true, total: 3, ..Default::default() }));
    assert!(s.status().running);
    // 用户保存了新设置（或开始了新的扫描）：停掉第一批、开始第二批
    let (g2, changed) = s.stop();
    assert!(changed, "从进行中变成空闲：要通知界面");
    assert!(c1.load(Ordering::SeqCst), "第一批被取消");
    assert_eq!(s.status(), &AiStatus::default());
    let c2 = s.begin(g2).unwrap();
    // 第一批的线程还在收尾，报来的进度不能盖掉第二批的
    assert!(!s.report(g1, &AiStatus { running: false, error: Some("密钥不对".into()), ..Default::default() }));
    assert_eq!(s.status(), &AiStatus::default());
    assert!(s.report(g2, &AiStatus { running: true, total: 5, ..Default::default() }));
    assert_eq!(s.status().total, 5);
    // 开始之前又被停了一次（两个线程抢着开始）：旧代号不能再开始
    let (g3, _) = s.stop();
    assert!(c2.load(Ordering::SeqCst));
    assert!(s.begin(g2).is_none());
    assert!(s.begin(g3).is_some());
    assert_eq!(s.gen(), g3);
}

// ───────────────────────── 7c. 隐私：别的账号、云服务没填密钥 ─────────────────────────

fn tree_on(root: &str, entries: &[(&str, u64)]) -> Node {
    // 借用 C:\ 的 tree()，再把盘符换掉
    let mapped: Vec<(String, u64)> = entries.iter().map(|(p, s)| (format!(r"C:\{}", p.strip_prefix(root).unwrap()), *s)).collect();
    let refs: Vec<(&str, u64)> = mapped.iter().map(|(p, s)| (p.as_str(), *s)).collect();
    let mut t = tree(&refs);
    fn re(n: &mut Node, root: &str) {
        if !n.path.is_empty() {
            n.path = format!("{root}{}", &n.path[3..]);
        }
        if n.name == r"C:\" {
            n.name = root.to_string();
        }
        for c in &mut n.children {
            re(c, root);
        }
    }
    re(&mut t, root);
    t
}

/// 管理员身份扫描（读得到别的账号）、打开别的电脑的快照、第二块盘上的旧 Users：别的账号的东西一律不自动问，
/// `X:\Users` 本身不问（子文件夹名就是用户名）；本机当前用户 bob 的 AppData 照问、bob 的文档照样不问。
#[test]
fn ai_candidates_never_enter_other_accounts() {
    let ctx = embedded_ctx(); // USERPROFILE = C:\Users\bob
    let c = tree(&[
        (r"C:\Users\bob\Documents\BobTaxes\scan1.pdf", 300 * MB),
        (r"C:\Users\bob\qzxbobproj\x.qzx", 200 * MB),
        (r"C:\Users\alice\Documents\Alice 离婚协议\协议扫描.pdf", 400 * MB),
        (r"C:\Users\alice\Documents\Alice 离婚协议\聊天截图.zip", 150 * MB),
        (r"C:\Users\alice\Desktop\病历 2025\ct.dcm", 200 * MB),
        (r"C:\Users\alice\AppData\Local\Foo\x.bin", 120 * MB),
        (r"C:\Users\alice\qzxproj\big.qzx", 300 * MB),
        (r"C:\Users\Public\qzxshared\big.qzx", 300 * MB),
    ]);
    let got = ai_candidates(&c, &ctx, 100 * MB, 40);
    let paths: Vec<&str> = got.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&r"C:\Users\bob\qzxbobproj"), "本机用户自己的照问：{paths:#?}");
    for p in &paths {
        let l = p.to_lowercase();
        assert!(!l.starts_with(r"c:\users\alice") && !l.starts_with(r"c:\users\public"), "不该自动问别的账号：{p}");
        assert!(!l.contains("documents") && l != r"c:\users", "{p}");
    }
    // 第二块盘上的旧系统：D:\Users 本身和里面所有账号（包括同名的 bob）都不问；Users 外面的照问
    let d = tree_on(
        r"D:\",
        &[
            (r"D:\Users\carol\Documents\Carol 简历\resume.docx", 200 * MB),
            (r"D:\Users\dave\Pictures\Family\img.jpg", 200 * MB),
            (r"D:\Users\bob\AppData\Local\Old\x.bin", 200 * MB),
            (r"D:\Users\big-file-in-users-root.qzx", 150 * MB),
            (r"D:\qzxgames\x.qzx", 300 * MB),
        ],
    );
    let gd = ai_candidates(&d, &ctx, 100 * MB, 40);
    let pd: Vec<&str> = gd.iter().map(|(p, _)| p.as_str()).collect();
    assert!(pd.contains(&r"D:\qzxgames"), "{pd:#?}");
    assert!(!pd.iter().any(|p| p.eq_ignore_ascii_case(r"D:\Users")), "X:\\Users 本身不问：{pd:#?}");
    assert!(pd.contains(&r"D:\Users\big-file-in-users-root.qzx"), "Users 下面直接放的大文件照问（名字里没有用户名）：{pd:#?}");
    for who in ["carol", "dave", r"\bob"] {
        assert!(!pd.iter().any(|p| p.to_lowercase().contains(who)), "不该自动问 {who}：{pd:#?}");
    }
    // 没有 USERPROFILE（拿不准谁是本机用户）：所有 X:\Users\<某人> 都不问
    let env = EnvMap::from_pairs(&[("SystemDrive", "C:"), ("SystemRoot", r"C:\Windows")]);
    let bare = ExplainCtx::new(env, RuleSet::default(), RegistrySnapshot::default(), LlmSettings::default());
    let gb = ai_candidates(&c, &bare, 100 * MB, 40);
    assert!(!gb.iter().any(|(p, _)| p.to_lowercase().starts_with(r"c:\users\")), "{gb:#?}");
}

/// 手动问 `X:\Users`（认不出来的第二块盘）时，子文件夹名（= 用户名）不发出去。
#[test]
fn build_input_hides_user_names_under_users_root() {
    let d = tree_on(r"D:\", &[(r"D:\Users\carol\a.bin", 200 * MB), (r"D:\Users\dave\b.bin", 100 * MB), (r"D:\Users\note.txt", 1 * MB)]);
    let (chain, node) = view::find_node(&d, r"D:\Users").unwrap();
    let t = Target { node, path: r"D:\Users", parent: chain.last().copied(), ancestors: &chain };
    let input = llm::build_input(&t);
    let sent = serde_json::to_string(&input).unwrap();
    assert!(!sent.contains("carol") && !sent.contains("dave"), "{sent}");
    assert!(sent.contains("note.txt"), "文件名照常：{sent}");
    assert_eq!(input.top_children.iter().filter(|(n, _)| n.starts_with("（某个用户）")).count(), 2, "{sent}");
    assert!(llm::is_users_root(r"D:\Users") && llm::is_users_root(r"c:\USERS\") && !llm::is_users_root(r"D:\Users\carol") && !llm::is_users_root(r"D:\x\Users"));
}

/// 云服务没填密钥不算“能用”（自动识别不开始、不把第一项不带密钥发出去）；本机 / 局域网可以不填。
#[test]
fn cloud_endpoints_need_a_key_local_ones_do_not() {
    for u in [
        "http://127.0.0.1:11434/v1",
        "http://localhost:11434/v1",
        "http://[::1]:11434/v1",
        "http://192.168.1.20:11434/v1",
        "http://10.0.0.5/v1",
        "http://172.20.1.1:8000/v1",
        "http://ollama.local:11434/v1",
        "http://gpu-box:8000/v1",
        "http://[fd00::5]:8000/v1",
    ] {
        assert!(llm::key_optional(u), "{u} 不用密钥");
        let s = LlmSettings { enabled: true, base_url: u.into(), model: "m".into(), api_key: String::new() };
        assert!(s.usable() && !s.missing_key(), "{u}");
    }
    for u in [
        "https://open.bigmodel.cn/api/paas/v4",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "https://api.deepseek.com",
        "https://api.openai.com/v1",
        "http://8.8.8.8/v1",
        "http://172.32.0.1/v1",
    ] {
        assert!(!llm::key_optional(u), "{u} 要密钥");
        let mut s = LlmSettings { enabled: true, base_url: u.into(), model: "m".into(), api_key: "  ".into() };
        assert!(!s.usable() && s.missing_key(), "{u} 没填密钥不能用");
        s.api_key = "sk-x".into();
        assert!(s.usable(), "{u}");
    }
    // 直接调用也不发请求：没填密钥 → 整批停下的「没填密钥」
    let s = LlmSettings { enabled: true, base_url: "https://open.bigmodel.cn/api/paas/v4".into(), model: "glm-4.7-flash".into(), api_key: String::new() };
    assert_kind(&llm::ask_input(&input(), 1, &s, Duration::from_secs(1)), false, true, llm::MSG_NO_KEY);
    // {:?} 不打出密钥
    let s = LlmSettings { api_key: "sk-secret-123".into(), ..s };
    let dbg = format!("{s:?}");
    assert!(!dbg.contains("sk-secret-123") && dbg.contains("open.bigmodel.cn"), "{dbg}");
    let mut ctx = empty_ctx();
    ctx.settings = s;
    assert!(!format!("{ctx:?}").contains("sk-secret-123"));
}

/// 缓存文件：开头有 BOM（记事本另存过）照读；某一条坏了只丢那一条。
#[test]
fn ai_store_tolerates_bom_and_a_bad_entry() {
    let d = temp_dir("bom");
    let p = d.join("ai-cache.json");
    let good = serde_json::to_value(note("好的那条")).unwrap();
    let text = serde_json::json!({"version":1,"items":{"c:\\good": good, "c:\\bad": {"what": 12, "size": "很大"}, "c:\\also": {"what":"也好"}}}).to_string();
    std::fs::write(&p, format!("\u{feff}{text}")).unwrap();
    let s = AiStore::load_from(&p);
    assert_eq!(s.len(), 2, "坏的那条丢掉，其余照读");
    assert_eq!(s.get(r"C:\good").unwrap().what, "好的那条");
    assert!(s.contains(r"c:\also") && !s.contains(r"c:\bad"));
    s.put(r"C:\new", note("新")).unwrap();
    assert_eq!(AiStore::load_from(&p).len(), 3);
    let _ = std::fs::remove_dir_all(&d);
}

/// AI 回答里的控制字符 / 改写显示方向的字符去掉（界面上字不会倒着排）。
#[test]
fn ai_text_is_stripped_of_display_tricks() {
    let a = llm::LlmAnswer { what: "可以\u{202E}删除放心\u{202C}的缓存\u{0007}".into(), lose: "第一行\n第二行".into(), ..Default::default() };
    let n = llm::to_note(a, "m", 1);
    assert_eq!(n.what, "可以删除放心的缓存");
    assert_eq!(n.lose, "第一行 第二行");
}

// ───────────────────────── 8. 智谱连通性探测（手动跑，假密钥，不发任何文件夹信息） ─────────────────────────

/// `cargo test -p diskwhy-core --test ai_tests -- --ignored bigmodel_probe --nocapture`
#[test]
#[ignore]
fn bigmodel_probe_with_dummy_key() {
    let s = LlmSettings {
        enabled: true,
        base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
        model: "glm-4.7-flash".into(),
        api_key: "dummy.invalid-key-for-probe".into(),
    };
    let r = llm::test_connection(&s);
    println!("智谱探测：{r:?}");
    let e = r.expect_err("假密钥不该成功");
    assert!(e.starts_with(llm::MSG_KEY), "假密钥应映射成「{}」：{e}", llm::MSG_KEY);
}

/// 真实快照上的速度（手动跑）：`set DISKWHY_SNAPSHOT=…\c.dwsnap` 然后 `cargo test --release … -- --ignored ai_candidates_on_snapshot --nocapture`
#[test]
#[ignore]
fn ai_candidates_on_snapshot() {
    let p = std::env::var("DISKWHY_SNAPSHOT").expect("DISKWHY_SNAPSHOT");
    let (r, _) = diskwhy_core::snapshot::with_big_stack(move || diskwhy_core::snapshot::load(std::path::Path::new(&p))).unwrap();
    let env = EnvMap::from_process();
    let (rules, _) = RuleSet::embedded(&env);
    let ctx = ExplainCtx::new(env, rules, RegistrySnapshot::load(), LlmSettings::default());
    let start = Instant::now();
    let got = ai_candidates(&r.tree, &ctx, 100 * MB, 40);
    println!("{} 个文件 / {} 个文件夹：ai_candidates 用时 {:?}，{} 项", r.files, r.dirs, start.elapsed(), got.len());
    for (p, b) in &got {
        println!("  {:>10}  {p}", view::human_bytes(*b));
    }
    assert!(start.elapsed() < Duration::from_secs(5));
}

// ───────────────────────── 阶段 M 审查修正 ─────────────────────────

/// 自动识别不问“个人资料”（审查 should-fix）：不在固定的个人文件夹里也一样——`C:\资料`、用户文件夹根上的文件、
/// 别的盘上的 `D:\照片`、搬走了的文档文件夹……文件夹里看得到的字节一半以上是文档 / 图片 / 视频 / 音乐就不问，
/// 这些类型的大文件也不问；里面不是这类东西的子文件夹照问；正好一半不算。
#[test]
fn ai_candidates_skip_personal_content_anywhere() {
    let ctx = empty_ctx(); // USERPROFILE = C:\Users\bob
    let c = tree(&[
        (r"C:\资料\张三合同\合同扫描.pdf", 300 * MB),
        (r"C:\资料\张三合同\签字照片.JPG", 150 * MB),
        (r"C:\资料\张三合同\备注.qzx", 10 * MB),
        (r"C:\Users\bob\张三体检报告.pdf", 200 * MB),
        (r"C:\Users\bob\qzxtool\engine.qzx", 300 * MB),
        (r"C:\qzxmix\相册\IMG_0001.heic", 600 * MB),
        (r"C:\qzxmix\engine\core.qzx", 300 * MB),
        (r"C:\qzxhalf\a.qzx", 200 * MB),
        (r"C:\qzxhalf\b.pdf", 200 * MB),
        (r"C:\qzxapp\data.qzx", 300 * MB),
        (r"C:\qzxapp\icon.png", 100 * MB),
        (r"C:\婚礼录像.MP4", 900 * MB),
        (r"C:\会议录音.m4a", 150 * MB),
        (r"C:\qzxvm.qzxbin", 500 * MB),
    ]);
    let got = ai_candidates(&c, &ctx, 100 * MB, 40);
    let paths: Vec<&str> = got.iter().map(|(p, _)| p.as_str()).collect();
    for bad in [
        r"C:\资料",
        r"C:\资料\张三合同",
        r"C:\资料\张三合同\合同扫描.pdf",
        r"C:\资料\张三合同\签字照片.JPG",
        r"C:\Users\bob\张三体检报告.pdf",
        r"C:\qzxmix",
        r"C:\qzxmix\相册",
        r"C:\qzxmix\相册\IMG_0001.heic",
        r"C:\qzxapp\icon.png",
        r"C:\婚礼录像.MP4",
        r"C:\会议录音.m4a",
    ] {
        assert!(!paths.contains(&bad), "个人资料不该自动问 {bad}：{paths:#?}");
    }
    for good in [r"C:\qzxmix\engine", r"C:\qzxhalf", r"C:\qzxapp", r"C:\qzxvm.qzxbin", r"C:\Users\bob\qzxtool"] {
        assert!(paths.contains(&good), "应该照问 {good}：{paths:#?}");
    }
    // 别的盘：D:\照片、D:\音乐、搬到 D: 的文档文件夹
    let d = tree_on(
        r"D:\",
        &[
            (r"D:\照片\2023 婚礼\DSC_0001.NEF", 500 * MB),
            (r"D:\照片\2023 婚礼\clip.mov", 300 * MB),
            (r"D:\音乐\周杰伦\a.flac", 150 * MB),
            (r"D:\我的文档\年终总结.docx", 120 * MB),
            (r"D:\我的文档\笔记.md", 30 * MB),
            (r"D:\qzxgames\x.qzx", 300 * MB),
        ],
    );
    let gd = ai_candidates(&d, &ctx, 100 * MB, 40);
    let pd: Vec<&str> = gd.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(pd, vec![r"D:\qzxgames"], "{gd:#?}");
    let (_, n) = view::find_node(&d, r"D:\照片").unwrap();
    assert!(personal_content(n));
    let (_, n) = view::find_node(&d, r"D:\qzxgames").unwrap();
    assert!(!personal_content(n));
    // “其余小文件合计”不算看得到的字节；空文件夹不算个人资料
    let mut r = tree(&[(r"C:\qzxrest\a.jpg", 10 * MB), (r"C:\qzxempty\", 0)]);
    {
        let dir = find_mut(&mut r, r"C:\qzxrest");
        let mut rest = file_node("（其余）", 900 * MB);
        rest.kind = NodeKind::Rest;
        dir.children.push(rest);
    }
    assert!(personal_content(find_mut(&mut r, r"C:\qzxrest")), "看得到的全是 .jpg");
    assert!(!personal_content(find_mut(&mut r, r"C:\qzxempty")));
    assert!(!personal_content(&file_node("setup.exe", 1)) && personal_content(&file_node("报告.PDF", 1)));
    assert!(!personal_content(&file_node(".mp4", 1)) && !personal_content(&file_node("mp4", 1)), "没有名字 / 没有扩展名");
}

/// 自动识别拿到请求锁之后的核对（审查 must-fix + nit）：设置变了 / 关了 → 取消；排队时已经问过了 → 跳过；都不发。
#[test]
fn recheck_batch_cancels_on_changed_settings_and_skips_answered() {
    let store = AiStore::memory();
    let s = settings("https://open.bigmodel.cn/api/paas/v4");
    assert_eq!(recheck_batch(&store, r"C:\x", &s, &s).unwrap(), s);
    // 只差首尾空白：算同一个
    let mut ws = s.clone();
    ws.base_url = format!(" {} ", s.base_url);
    ws.api_key = format!("{}  ", s.api_key);
    assert!(recheck_batch(&store, r"C:\x", &s, &ws).is_ok());
    let changed: Vec<LlmSettings> = vec![
        LlmSettings { enabled: false, ..s.clone() },
        LlmSettings { base_url: "https://api.deepseek.com".into(), ..s.clone() },
        LlmSettings { model: "glm-4.5-flash".into(), ..s.clone() },
        LlmSettings { api_key: "sk-other".into(), ..s.clone() },
        LlmSettings { api_key: String::new(), ..s.clone() },
    ];
    for f in &changed {
        let e = recheck_batch(&store, r"C:\x", &s, f).unwrap_err();
        assert!(e.cancelled && !e.fatal && !e.skipped, "{f:?}：{e:?}");
    }
    store.put(r"c:\X", note("问过了")).unwrap();
    let e = recheck_batch(&store, r"C:\x", &s, &s).unwrap_err();
    assert!(e.skipped && !e.cancelled && !e.fatal && !e.retryable, "{e:?}");
    assert_eq!(e.message, llm::MSG_ASKED);
    assert!(llm::ASK_MESSAGES.contains(&llm::MSG_ASKED));
}

/// 同一项不会问两次（审查 nit）：自动识别排队等锁的时候，手动“问 AI”正好把它问完了 → 拿到锁后跳过，不发请求，不算失败。
#[test]
fn batch_item_answered_while_queued_is_not_sent_twice() {
    let http = Arc::new(Mutex::new(()));
    let store = Arc::new(AiStore::memory());
    let s = settings("https://open.bigmodel.cn/api/paas/v4");
    let (h2, st2) = (http.clone(), store.clone());
    let holder = std::thread::spawn(move || {
        let _g = h2.lock().unwrap();
        std::thread::sleep(Duration::from_millis(80));
        st2.put(r"C:\item0", note("手动问到的")).unwrap(); // 手动“问 AI”答完、存好
    });
    std::thread::sleep(Duration::from_millis(20));
    let cancel = AtomicBool::new(false);
    let sent = AtomicUsize::new(0);
    let (st, end, _) = run(&items(2), &fast_cfg(), &cancel, &store, |path, _| {
        ask_one(
            &cancel,
            Duration::from_secs(5),
            || Some(()),
            |_| Some(()),
            &http,
            || recheck_batch(&store, path, &s, &s),
            |_, _| {
                sent.fetch_add(1, Ordering::SeqCst);
                Ok(note("自动问到的"))
            },
        )
    });
    holder.join().unwrap();
    assert_eq!(end, BatchEnd::Finished);
    assert_eq!(st, AiStatus { running: false, done: 2, total: 2, failed: 0, error: None });
    assert_eq!(sent.load(Ordering::SeqCst), 1, "item0 已经问过了，只发 item1");
    assert_eq!(store.get(r"C:\item0").unwrap().what, "手动问到的", "不覆盖");
}

/// 手动“问 AI”排队等锁时关了 AI / 换了服务商（审查 must-fix）：拿到锁后用现在的设置核对，变了一个请求都不发；
/// 排队时已经被问过了 → 直接给缓存里的答案；都没变 → 用现在的设置发。
#[test]
fn ask_manual_rechecks_settings_and_cache_after_the_lock() {
    let http = Mutex::new(());
    let s = settings("https://open.bigmodel.cn/api/paas/v4");
    let never = |_: &LlmSettings| -> Result<AiNote, AskError> { panic!("不该发请求") };
    // 关了 AI
    let off = LlmSettings { enabled: false, ..s.clone() };
    let e = ask_manual(&http, &s, || off.clone(), || None, never).unwrap_err();
    assert_eq!(e.message, llm::MSG_OFF);
    // 云服务把密钥清空了
    let nokey = LlmSettings { api_key: String::new(), ..s.clone() };
    let e = ask_manual(&http, &s, || nokey.clone(), || None, never).unwrap_err();
    assert_eq!(e.message, llm::MSG_NO_KEY);
    // 换了服务商 / 模型 / 密钥 → 已取消
    for f in [
        LlmSettings { base_url: "https://api.deepseek.com".into(), ..s.clone() },
        LlmSettings { model: "other".into(), ..s.clone() },
        LlmSettings { api_key: "sk-other".into(), ..s.clone() },
    ] {
        let e = ask_manual(&http, &s, || f.clone(), || None, never).unwrap_err();
        assert!(e.cancelled, "{e:?}");
        assert_eq!(e.message, llm::MSG_CANCELLED);
    }
    // 点按钮时就没打开：照样是“AI 没打开”，不发
    let e = ask_manual(&http, &off, || off.clone(), || None, never).unwrap_err();
    assert_eq!(e.message, llm::MSG_OFF);
    // 已经问过了 → 缓存里的答案
    let r = ask_manual(&http, &s, || s.clone(), || Some(note("早问过了")), never).unwrap();
    assert_eq!(r, ManualAnswer::Cached(note("早问过了")));
    // 都没变 → 用现在的设置发
    let r = ask_manual(&http, &s, || s.clone(), || None, |now| {
        assert_eq!(now, &s);
        Ok(note("新答案"))
    })
    .unwrap();
    assert_eq!(r, ManualAnswer::Fresh(note("新答案")));
    // 发请求出错：原样返回
    let e = ask_manual(&http, &s, || s.clone(), || None, |_| Err(AskError::fatal(llm::MSG_KEY, "401"))).unwrap_err();
    assert_eq!(e.message, llm::MSG_KEY);

    // 真的排队：自动识别正占着锁，这期间用户关了 AI → 拿到锁后不发
    let http = Arc::new(Mutex::new(()));
    let cur = Arc::new(Mutex::new(s.clone()));
    let (h2, c2) = (http.clone(), cur.clone());
    let holder = std::thread::spawn(move || {
        let _g = h2.lock().unwrap();
        std::thread::sleep(Duration::from_millis(60));
        c2.lock().unwrap().enabled = false; // 用户在设置里关了 AI
        std::thread::sleep(Duration::from_millis(40));
    });
    std::thread::sleep(Duration::from_millis(15));
    let sent = AtomicUsize::new(0);
    let r = ask_manual(&http, &s, || cur.lock().unwrap().clone(), || None, |_| {
        sent.fetch_add(1, Ordering::SeqCst);
        Ok(note("不该有"))
    });
    holder.join().unwrap();
    assert_eq!(r.unwrap_err().message, llm::MSG_OFF);
    assert_eq!(sent.load(Ordering::SeqCst), 0, "关了 AI 之后排队的手动请求不能发出去");
}

/// 同时开着两个 DiskWhy（两份 AiStore 写同一个文件）：临时文件名不撞，每次都写得进去，不留临时文件。
#[test]
fn ai_store_two_instances_do_not_clobber_tmp_files() {
    let d = temp_dir("two");
    let p = d.join("ai-cache.json");
    let a = Arc::new(AiStore::load_from(&p));
    let b = Arc::new(AiStore::load_from(&p));
    let hs: Vec<_> = [a, b]
        .into_iter()
        .enumerate()
        .map(|(k, s)| {
            std::thread::spawn(move || {
                for i in 0..30 {
                    s.put(&format!(r"C:\inst{k}\item{i}"), note("x")).expect("每次都写得进去");
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    let left: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["ai-cache.json".to_string()], "{left:?}");
    assert!(AiStore::load_from(&p).len() >= 30, "最后写的那一份是完整的");
}
