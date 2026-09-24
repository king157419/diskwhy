//! 阶段 B-3：`diskwhy-cli scan --json` / `explain` 的端到端测试。
//!
//! 在临时目录里造一棵假树，真的跑编译出来的 diskwhy-cli（子进程），检查输出的 JSON。
//! 子进程的 %LOCALAPPDATA% / %APPDATA% / %USERPROFILE% / %TEMP% 都指到假树里：
//! 规则按假环境展开（pip 缓存、Temp 能命中规则），也不会读到本机真实的 DiskWhy 设置和用户规则。
//! LLM 用本机 127.0.0.1 上的假服务器，不联网。

use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const BIN: &str = env!("CARGO_BIN_EXE_diskwhy-cli");
const SECRET: &str = "B3-SECRET-CONTENT-mUst-nOt-LeAk";
static SEQ: AtomicUsize = AtomicUsize::new(0);

struct Fake {
    base: PathBuf,
    root: PathBuf,
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// 造假树：
/// - `Local\pip\cache\http-v2\a\blob.bin`：规则 pip-cache（safe），http-v2 继承上层；
/// - `Local\Temp\*.tmp`：规则 user-temp（safe）；
/// - `proj\package.json` + `proj\node_modules\left-pad`：启发式 node-modules，说出项目「proj」；
/// - `apps\AppOne\Cache\Cache_Data\…`：签名严格的 Chromium 缓存，启发式 safe；
/// - `logsdir\{a\b,c\d,e\f}\*.log`：日志藏在第 3 层，JSON 截断后要回磁盘补读才认得出；
/// - `qzxv-mystery\blob.qzx`：谁也认不出（内容是机密串，绝不能发给 AI）。
fn fake_tree(tag: &str) -> Fake {
    let base = std::env::temp_dir().join(format!("diskwhy-b3-{}-{}-{}", tag, std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("t");
    put(&root, r"Local\pip\cache\http-v2\a\blob.bin", &[7u8; 60_000]);
    put(&root, r"Local\pip\cache\selfcheck\x.json", b"{}");
    for i in 0..3 {
        put(&root, &format!(r"Local\Temp\x{i}.tmp"), &[0u8; 20_000]);
    }
    put(&root, r"proj\package.json", br#"{"name":"proj"}"#);
    put(&root, r"proj\node_modules\left-pad\index.js", &[b'j'; 45_000]);
    put(&root, r"apps\AppOne\Cache\Cache_Data\index", &[0u8; 2_000]);
    put(&root, r"apps\AppOne\Cache\Cache_Data\data_0", &[0u8; 8_000]);
    put(&root, r"apps\AppOne\Cache\Cache_Data\data_1", &[0u8; 8_000]);
    put(&root, r"apps\AppOne\Cache\Cache_Data\f_00001a", &[0u8; 30_000]);
    put(&root, r"logsdir\a\b\x.log", &[b'l'; 30_000]);
    put(&root, r"logsdir\c\d\y.log", &[b'l'; 30_000]);
    put(&root, r"logsdir\e\f\z.log", &[b'l'; 30_000]);
    put(&root, r"qzxv-mystery\blob.qzx", SECRET.repeat(2_000).as_bytes());
    std::fs::create_dir_all(root.join(r"Roaming")).unwrap();
    std::fs::create_dir_all(root.join(r"home")).unwrap();
    Fake { base, root }
}

impl Fake {
    fn p(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.root.to_string_lossy().to_string()
        } else {
            self.root.join(rel).to_string_lossy().to_string()
        }
    }

    fn cmd(&self, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(BIN)
            .args(args)
            .env("LOCALAPPDATA", self.p("Local"))
            .env("APPDATA", self.p("Roaming"))
            .env("USERPROFILE", self.p("home"))
            .env("TEMP", self.p(r"Local\Temp"))
            .env("TMP", self.p(r"Local\Temp"))
            .output()
            .expect("运行 diskwhy-cli 失败");
        (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
    }

    /// 跑 explain 并把 `--json -` 的标准输出解析成数组。
    fn explain_json(&self, args: &[&str]) -> Vec<Value> {
        let mut all = vec!["explain"];
        all.extend_from_slice(args);
        all.extend_from_slice(&["--json", "-", "--quiet"]);
        let (ok, stdout, stderr) = self.cmd(&all);
        assert!(ok, "explain 失败：{stderr}");
        let v: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("标准输出不是 JSON（{e}）：{stdout}\n{stderr}"));
        v.as_array().expect("应是数组").clone()
    }
}

fn by_path<'a>(rows: &'a [Value], path: &str) -> &'a Value {
    rows.iter().find(|r| r["path"].as_str().is_some_and(|p| p.eq_ignore_ascii_case(path))).unwrap_or_else(|| {
        panic!("输出里没有 {path}；有：{:?}", rows.iter().map(|r| r["path"].as_str().unwrap_or("")).collect::<Vec<_>>())
    })
}

fn src_kind(v: &Value) -> &str {
    v["source"]["kind"].as_str().unwrap_or("")
}

/// 在 JSON 目录树里按路径找节点。
fn tree_find<'a>(n: &'a Value, path: &str) -> Option<&'a Value> {
    if n["path"].as_str().is_some_and(|p| p.eq_ignore_ascii_case(path)) {
        return Some(n);
    }
    n["children"].as_array()?.iter().find_map(|c| tree_find(c, path))
}

/// 目录树里每个目录节点都必须带解释摘要（tier / owner / source / what）。
fn assert_every_dir_has_summary(n: &Value, count: &mut usize) {
    if n["kind"] == "Dir" {
        let e = &n["explanation"];
        assert!(e.is_object(), "目录节点没有 explanation：{}", n["path"]);
        assert!(["safe", "review", "keep"].contains(&e["tier"].as_str().unwrap_or("")), "{}", e);
        assert!(e.get("owner").is_some(), "摘要里应有 owner 字段（可以是 null）");
        assert!(!src_kind(e).is_empty(), "摘要里应有 source.kind");
        assert!(!e["what"].as_str().unwrap_or("").is_empty(), "摘要里应有一句话 what");
        assert!(e.get("lose").is_none(), "树里只放摘要，不放完整解释");
        *count += 1;
    } else {
        assert!(n.get("explanation").is_none(), "文件节点不该有 explanation");
    }
    for c in n["children"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
        assert_every_dir_has_summary(c, count);
    }
}

fn check_expected(rows: &[Value], f: &Fake) {
    let pip = by_path(rows, &f.p(r"Local\pip\cache"));
    assert_eq!(pip["source"]["id"], "pip-cache");
    assert_eq!(pip["tier"], "safe");
    let http = by_path(rows, &f.p(r"Local\pip\cache\http-v2"));
    assert_eq!(http["source"]["id"], "pip-cache");
    assert!(http["source"]["inherited_from"].is_string(), "http-v2 应继承上层 pip-cache：{http}");
    assert_eq!(by_path(rows, &f.p(r"Local\Temp"))["source"]["id"], "user-temp");

    let nm = by_path(rows, &f.p(r"proj\node_modules"));
    assert_eq!(src_kind(nm), "heuristic");
    assert_eq!(nm["source"]["name"], "node-modules");
    assert_eq!(nm["tier"], "review");
    assert!(nm.to_string().contains("「proj」"), "node_modules 应说出项目名：{nm}");

    let cache = by_path(rows, &f.p(r"apps\AppOne\Cache"));
    assert_eq!(cache["source"]["name"], "chromium-cache");
    assert_eq!(cache["tier"], "safe");

    let logs = by_path(rows, &f.p("logsdir"));
    assert_eq!(logs["source"]["name"], "logs-dumps", "日志在第 4 层也要认出来：{logs}");

    let m = by_path(rows, &f.p("qzxv-mystery"));
    assert_eq!(src_kind(m), "unknown");
    assert_eq!(m["tier"], "review");
    assert_eq!(m["what"], "还没认出来，先别动");
    assert!(!m["clues"].as_array().unwrap().is_empty(), "未知项要列线索");

    for r in rows {
        // 完整 Explanation 的字段都在。
        for k in ["tier", "owner", "category", "what", "why", "lose", "how", "how_detail", "gain_bytes", "source", "confidence", "notes", "clues"] {
            assert!(r.get(k).is_some(), "{} 缺字段 {k}", r["path"]);
        }
        // 护栏：safe 只能来自规则或 chromium-cache 启发式。
        if r["tier"] == "safe" {
            let k = src_kind(r);
            assert!(k == "rule" || (k == "heuristic" && r["source"]["name"] == "chromium-cache"), "不该是 safe：{r}");
        }
    }
}

#[test]
fn b3_scan_json_carries_summaries_and_explain_from_json_matches_live() {
    let f = fake_tree("json");
    let scan_json = f.base.join("scan.json").to_string_lossy().to_string();
    // --json-depth 1：故意截得很浅，检验“前 N 大目录及其上层照样保留”和“截断后回磁盘补读”。
    let (ok, stdout, stderr) = f.cmd(&["scan", &f.p(""), "--top", "40", "--json", &scan_json, "--json-depth", "1", "--quiet"]);
    assert!(ok, "scan 失败：{stderr}");
    assert!(stdout.contains("每个目录附解释摘要"), "{stdout}");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&scan_json).unwrap()).unwrap();

    // 1. 目录树：每个目录节点都有摘要；深处的前 N 大目录连同上层都在。
    let mut dirs = 0;
    assert_every_dir_has_summary(&v["tree"], &mut dirs);
    assert!(dirs >= 15, "树里的目录太少（{dirs}），前 N 大目录应被保留");
    let deep = tree_find(&v["tree"], &f.p(r"logsdir\a\b")).expect("第 3 层的前 N 大目录应保留在 JSON 里");
    assert_eq!(deep["explanation"]["source"]["name"], "logs-dumps");
    assert!(tree_find(&v["tree"], &f.p(r"Local\pip\cache\http-v2\a")).is_some(), "第 5 层的前 N 大目录也在");
    let pip = tree_find(&v["tree"], &f.p(r"Local\pip\cache")).unwrap();
    assert_eq!(pip["explanation"]["tier"], "safe");
    assert_eq!(pip["explanation"]["source"]["id"], "pip-cache");
    assert_eq!(pip["explanation"]["owner"], "pip");
    let mystery = tree_find(&v["tree"], &f.p("qzxv-mystery")).unwrap();
    assert_eq!(mystery["explanation"]["source"]["kind"], "unknown");

    // 2. top_dirs 带完整解释。
    let tops = v["top_dirs"].as_array().unwrap();
    assert!(!tops.is_empty());
    for t in tops {
        assert!(t["explanation"]["lose"].is_string(), "top_dirs 应带完整解释：{t}");
    }
    assert!(v["explain"]["rules_loaded"].as_u64().unwrap() >= 80);

    // 3. explain --from JSON 和现扫结果一致。
    let from = f.explain_json(&["--from", &scan_json, "--top", "40"]);
    let live = f.explain_json(&[&f.p(""), "--top", "40"]);
    assert_eq!(from[0]["rank"], 0);
    assert!(from[0]["path"].as_str().unwrap().eq_ignore_ascii_case(&f.p("")), "第 0 行应是目标本身");
    check_expected(&from, &f);
    check_expected(&live, &f);
    assert_eq!(from.len(), live.len());
    for (a, b) in from.iter().zip(&live) {
        assert_eq!(a["path"], b["path"]);
        assert_eq!(a["tier"], b["tier"], "{}", a["path"]);
        assert_eq!(a["source"], b["source"], "{}", a["path"]);
        assert_eq!(a["what"], b["what"], "{}", a["path"]);
    }
    assert!(from.iter().all(|r| r.get("reread").is_none()), "前 N 大目录都完整保留时不用补读");

    // 4. 截得更狠（只留前 3 大目录）：logsdir 的孙子层不在 JSON 里，要回磁盘补读；--no-reread 时就认不出了。
    let small_json = f.base.join("small.json").to_string_lossy().to_string();
    let (ok, _, stderr) = f.cmd(&["scan", &f.p(""), "--top", "3", "--json", &small_json, "--json-depth", "1", "--quiet"]);
    assert!(ok, "{stderr}");
    let small: Value = serde_json::from_str(&std::fs::read_to_string(&small_json).unwrap()).unwrap();
    assert!(tree_find(&small["tree"], &f.p(r"logsdir\a\b")).is_none(), "前提：孙子层被截掉了");
    assert_eq!(tree_find(&small["tree"], &f.p("logsdir")).unwrap()["explanation"]["source"]["name"], "logs-dumps", "摘要是在完整树上算的");
    let re = f.explain_json(&["--from", &small_json, "--top", "40"]);
    let logs = by_path(&re, &f.p("logsdir"));
    assert_eq!(logs["reread"], true, "logsdir 在 JSON 里被截断，应回磁盘补读：{logs}");
    assert_eq!(logs["source"]["name"], "logs-dumps");
    assert!(logs["notes"].to_string().contains("重新读了一遍磁盘"));
    for a in &re {
        // 截断 JSON 里剩下的每一行，都和现扫的结论一样。
        let b = by_path(&live, a["path"].as_str().unwrap());
        assert_eq!((&a["tier"], &a["source"]), (&b["tier"], &b["source"]), "{}", a["path"]);
    }
    let no = f.explain_json(&["--from", &small_json, "--top", "40", "--no-reread"]);
    assert_ne!(by_path(&no, &f.p("logsdir"))["source"]["name"], "logs-dumps", "不补读时只看得到截断后的样子");

    // 5. --from 加路径：从 JSON 里的某个文件夹开始。
    let sub = f.explain_json(&["--from", &scan_json, &f.p("Local"), "--top", "3"]);
    assert!(sub[0]["path"].as_str().unwrap().eq_ignore_ascii_case(&f.p("Local")));
    assert!(sub.len() <= 4 && sub.len() >= 2);

    // 6. --report 和 --md：统计数字和表格。
    let rep = f.base.join("report.json").to_string_lossy().to_string();
    let md = f.base.join("cov.md").to_string_lossy().to_string();
    let (ok, stdout, stderr) = f.cmd(&["explain", "--from", &scan_json, "--top", "40", "--report", &rep, "--md", &md, "--quiet"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("统计（"), "{stdout}");
    let r: Value = serde_json::from_str(&std::fs::read_to_string(&rep).unwrap()).unwrap();
    let c = &r["counts"];
    let total = c["total"].as_u64().unwrap();
    let sum = ["rule_exact", "rule_inherited", "heuristic", "llm", "unknown"].iter().map(|k| c[*k].as_u64().unwrap()).sum::<u64>();
    assert_eq!(total, sum, "各来源个数之和应等于总数：{c}");
    assert_eq!(total as usize + 1, r["rows"].as_array().unwrap().len(), "统计不含第 0 行");
    assert!(c["unknown"].as_u64().unwrap() >= 1);
    assert_eq!(r["llm_calls"], 0);
    let mdtext = std::fs::read_to_string(&md).unwrap();
    assert!(mdtext.contains("| # | 目录 | 大小 | 档位 | 来源 | 一句话说明 |"));
    assert!(mdtext.contains("规则没命中的前 10 个目录"));
    assert!(mdtext.contains("qzxv-mystery"));
}

#[test]
fn b3_explain_single_paths_and_files() {
    let f = fake_tree("single");
    // --top 0：只解释路径本身。
    let nm = f.explain_json(&[&f.p(r"proj\node_modules"), "--top", "0"]);
    assert_eq!(nm.len(), 1);
    assert_eq!(nm[0]["rank"], 0);
    assert_eq!(nm[0]["source"]["name"], "node-modules", "只扫 node_modules 也要能看到旁边的 package.json：{}", nm[0]);

    // 只扫 pip\cache 下面一层：靠扫描根以上的祖先认出继承的规则。
    let http = f.explain_json(&[&f.p(r"Local\pip\cache\http-v2"), "--top", "0"]);
    assert_eq!(http[0]["source"]["id"], "pip-cache");
    assert!(http[0]["source"]["inherited_from"].as_str().unwrap().eq_ignore_ascii_case(&f.p(r"Local\pip\cache")));

    // 文件也能解释。
    let file = f.p(r"qzxv-mystery\blob.qzx");
    let fe = f.explain_json(&[&file, "--top", "0"]);
    assert_eq!(fe.len(), 1);
    assert!(fe[0]["path"].as_str().unwrap().eq_ignore_ascii_case(&file));
    assert_eq!(src_kind(&fe[0]), "unknown");

    // 不存在的路径：失败退出，不 panic。
    let (ok, _, stderr) = f.cmd(&["explain", &f.p("no-such-dir"), "--quiet"]);
    assert!(!ok);
    assert!(stderr.contains("扫描失败"), "{stderr}");
    // --llm 不能用在 scan 上。
    let (ok, _, stderr) = f.cmd(&["scan", &f.p(""), "--llm"]);
    assert!(!ok && stderr.contains("--llm"));
}

// ───────────────────────── --llm ─────────────────────────

/// 本机假 OpenAI 服务：记下每个请求的原始字节，回一个固定答案。
fn fake_llm() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let got = Arc::new(Mutex::new(Vec::new()));
    let g = got.clone();
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(mut sock) = sock else { continue };
            sock.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                let n = sock.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                let s = String::from_utf8_lossy(&buf);
                if let Some(end) = s.find("\r\n\r\n") {
                    let cl = s[..end]
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + cl {
                        break;
                    }
                }
            }
            g.lock().unwrap().push(String::from_utf8_lossy(&buf).to_string());
            let answer = r#"{"what":"像是某个小工具留下的数据文件","why":"不清楚","lose":"不确定","tier":"safe","owner":"某公司","confidence":0.99}"#;
            let body = format!("{{\"choices\":[{{\"message\":{{\"role\":\"assistant\",\"content\":{}}}}}]}}", serde_json::to_string(answer).unwrap());
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = sock.write_all(resp.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{port}/v1"), got)
}

#[test]
fn b3_llm_flag_is_off_by_default_and_only_asks_for_unknowns() {
    let f = fake_tree("llm");
    let (url, got) = fake_llm();
    // 设置里启用了 AI（假服务器）。
    let settings = serde_json::json!({"enabled": true, "base_url": url, "model": "fake-model", "api_key": ""});
    put(&f.root, r"Local\DiskWhy\settings.json", settings.to_string().as_bytes());

    // 1. 不加 --llm：即使设置里启用了，也一次都不问。
    let rows = f.explain_json(&[&f.p(""), "--top", "40"]);
    assert_eq!(src_kind(by_path(&rows, &f.p("qzxv-mystery"))), "unknown");
    assert_eq!(got.lock().unwrap().len(), 0, "没加 --llm 却发了请求");

    // 2. 加 --llm，但目标是规则认得的：不问。
    let pip = f.explain_json(&[&f.p(r"Local\pip\cache"), "--top", "5", "--llm"]);
    assert!(pip.iter().all(|r| src_kind(r) == "rule"));
    assert_eq!(got.lock().unwrap().len(), 0, "规则认得的目录不该问 AI");

    // 3. 加 --llm，对认不出的：问一次，结果只能是 review、低可信度，AI 说的 tier / owner 不算数。
    let m = f.explain_json(&[&f.p("qzxv-mystery"), "--top", "0", "--llm"]);
    let reqs = got.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "应该正好问一次");
    let e = &m[0];
    assert_eq!(src_kind(e), "llm");
    assert_eq!(e["source"]["model"], "fake-model");
    assert_eq!(e["tier"], "review", "AI 说 safe 也不算数");
    assert!(e["confidence"].as_f64().unwrap() < 0.5);
    assert!(e["owner"].is_null(), "归属不让 AI 猜：{e}");
    assert_eq!(e["what"], "像是某个小工具留下的数据文件");
    assert!(!e["clues"].as_array().unwrap().is_empty(), "保留 DiskWhy 自己看到的线索");
    let raw = &reqs[0];
    assert!(raw.starts_with("POST /v1/chat/completions "), "{}", &raw[..raw.len().min(200)]);
    assert!(!raw.contains(SECRET), "请求里出现了文件内容！");
    // 第 1 轮安全评审后：文件只发扩展名和个数，不发文件名
    assert!(raw.contains("qzxv-mystery") && raw.contains(".qzx 文件 × 1") && !raw.contains("blob.qzx"), "应发送名字和文件类型、不发文件名");

    // 4. 整棵树 + --llm：只有认不出的那几个会问。
    got.lock().unwrap().clear();
    let rep = f.base.join("r.json").to_string_lossy().to_string();
    let (ok, stdout, stderr) = f.cmd(&["explain", &f.p(""), "--top", "40", "--llm", "--report", &rep, "--quiet"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("--llm：问了 AI"), "{stdout}");
    let r: Value = serde_json::from_str(&std::fs::read_to_string(&rep).unwrap()).unwrap();
    let calls = r["llm_calls"].as_u64().unwrap() as usize;
    assert_eq!(calls, got.lock().unwrap().len());
    let asked: Vec<&Value> = r["rows"].as_array().unwrap().iter().filter(|x| src_kind(x) == "llm").collect();
    assert_eq!(asked.len(), calls, "每问一次就应有一行是 AI 推测");
    assert!(calls >= 1);
    for x in &asked {
        assert_eq!(x["tier"], "review");
    }

    // 5. 设置里没启用时，--llm 只给提示，不联网。
    put(&f.root, r"Local\DiskWhy\settings.json", serde_json::json!({"enabled": false, "base_url": url, "model": "fake-model"}).to_string().as_bytes());
    got.lock().unwrap().clear();
    let (ok, stdout, _) = f.cmd(&["explain", &f.p("qzxv-mystery"), "--top", "0", "--llm", "--quiet"]);
    assert!(ok);
    assert!(stdout.contains("本次不会联网"), "{stdout}");
    assert_eq!(got.lock().unwrap().len(), 0);
}
