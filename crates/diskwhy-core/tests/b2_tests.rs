//! 阶段 B-2（S22）测试：第二批规则 + 启发式 + 归属 + LLM 兜底 + 解释链和护栏。
//!
//! - 规则：任务点名的 AI 模型缓存、国内软件、conda envs、游戏平台缓存都在，档位和办法对；
//! - 启发式：在 %TEMP% 下真实建目录、用真实扫描器扫；Chromium 缓存“签名严格才 safe”，其余永不 safe；
//! - LLM：请求体只有四个字段、用户名全部换成 `~`、只取前 20 个子项；回复只认 what/why/lose；
//!   用假的 oracle（纯函数，不联网）验证解释链的优先级和“默认关闭”。

use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::{self, LlmOracle, LlmSettings};
use diskwhy_core::explain::registry::{InstalledApp, RegistrySnapshot};
use diskwhy_core::explain::rules::RuleSet;
use diskwhy_core::explain::{enforce_guards, explain_full, Category, ExplainCtx, Explanation, How, Source, Target, Tier};
use diskwhy_core::scan::scan;
use diskwhy_core::view::{self, explain_path};
use diskwhy_core::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ───────────────────────── 公共工具 ─────────────────────────

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
    ExplainCtx::new(env, rules, RegistrySnapshot::from_apps(Vec::new()), LlmSettings::default())
}

fn on_settings() -> LlmSettings {
    LlmSettings { enabled: true, base_url: "http://127.0.0.1:1/v1".into(), model: "mock-model".into(), api_key: String::new() }
}

fn leaf(name: &str, size: u64, mtime: i64) -> Node {
    Node {
        name: name.to_string(),
        path: String::new(),
        kind: NodeKind::File,
        logical: size,
        allocated: size,
        mtime,
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

/// 内存里的树：文件路径 → 大小；以 `\` 结尾的是空目录；所有节点的修改时间都是 `mtime`。根固定为 `C:\`。
fn tree_at(entries: &[(&str, u64)], mtime: i64) -> Node {
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
    fn build(name: &str, path: &str, d: &D, mtime: i64) -> Node {
        let mut children: Vec<Node> = Vec::new();
        for (n, sub) in &d.dirs {
            let p = if path.ends_with('\\') { format!("{path}{n}") } else { format!("{path}\\{n}") };
            children.push(build(n, &p, sub, mtime));
        }
        for (n, sz) in &d.files {
            children.push(leaf(n, *sz, mtime));
        }
        children.sort_by(|a, b| b.allocated.cmp(&a.allocated));
        let mut me = leaf(name, 0, mtime);
        me.path = path.to_string();
        me.kind = NodeKind::Dir;
        me.attrs = 0x10;
        me.logical = children.iter().map(|c| c.logical).sum();
        me.allocated = children.iter().map(|c| c.allocated).sum();
        me.files = children.iter().map(|c| c.files).sum();
        me.dirs = children.iter().map(|c| if c.kind == NodeKind::Dir { 1 + c.dirs } else { 0 }).sum();
        me.children = children;
        me
    }
    build(r"C:\", r"C:\", &root, mtime)
}

fn tree(entries: &[(&str, u64)]) -> Node {
    tree_at(entries, 1_700_000_000)
}

fn rule_id(e: &Explanation) -> Option<&str> {
    match &e.source {
        Source::Rule { id, .. } => Some(id.as_str()),
        _ => None,
    }
}

fn heur_name(e: &Explanation) -> Option<&str> {
    match &e.source {
        Source::Heuristic { name } => Some(name.as_str()),
        _ => None,
    }
}

fn ex(t: &Node, path: &str, ctx: &ExplainCtx) -> Explanation {
    explain_path(t, path, ctx).unwrap_or_else(|| panic!("树里找不到 {path}")).1
}

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn temp_root(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("diskwhy-b2-{}-{}-{}", tag, std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn put_big(root: &Path, rel: &str, len: u64) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::File::create(p).unwrap().set_len(len).unwrap();
}

struct Fixture {
    root: PathBuf,
    result: ScanResult,
    ctx: ExplainCtx,
}

impl Fixture {
    fn scan(root: PathBuf, ctx: ExplainCtx) -> Fixture {
        let cancel = AtomicBool::new(false);
        let result = scan(root.to_str().unwrap(), &ScanOptions::default(), &cancel, |_| {}).expect("扫描临时目录失败");
        Fixture { root, result, ctx }
    }
    fn explain(&self, rel: &str) -> Explanation {
        let p = self.root.join(rel);
        explain_path(&self.result.tree, p.to_str().unwrap(), &self.ctx).unwrap_or_else(|| panic!("找不到 {rel}")).1
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 规则库为空的上下文，USERPROFILE = 临时目录根（所以 root\Documents 是受保护目录）。
fn empty_rules_ctx(root: &Path) -> ExplainCtx {
    let r = root.to_str().unwrap();
    let env = EnvMap::from_pairs(&[
        ("SystemDrive", "C:"),
        ("USERPROFILE", r),
        ("LOCALAPPDATA", &format!(r"{r}\Local")),
        ("APPDATA", &format!(r"{r}\Roaming")),
        ("ProgramFiles", &format!(r"{r}\PF")),
        ("ProgramData", &format!(r"{r}\PD")),
    ]);
    ExplainCtx::new(env, RuleSet::default(), RegistrySnapshot::from_apps(Vec::new()), LlmSettings::default())
}

// ───────────────────────── 1. 第二批规则 ─────────────────────────

#[test]
fn b2_rules_total_and_listed_software() {
    let ctx = embedded_ctx();
    assert!(ctx.rules.rules.len() >= 80, "规则总数 {}", ctx.rules.rules.len());
    let get = |id: &str| ctx.rules.rules.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("缺规则 {id}"));
    // 任务点名的软件：规则 id → 期望档位
    let listed: &[(&str, Tier)] = &[
        // AI 模型缓存
        ("huggingface-hub", Tier::Review),
        ("ollama-models", Tier::Review),
        ("torch-hub", Tier::Review),
        ("modelscope-cache", Tier::Review),
        ("insightface-models", Tier::Review),
        ("paddle-models", Tier::Review),
        ("lm-studio-models", Tier::Review),
        ("whisper-models", Tier::Review),
        // 中国软件
        ("wechat-files", Tier::Keep),
        ("qq-files", Tier::Keep),
        ("wecom-files", Tier::Keep),
        ("feishu-larkshell", Tier::Review),
        ("dingtalk-data", Tier::Review),
        ("baidu-netdisk", Tier::Keep),
        ("thunder-data", Tier::Review),
        ("netease-music-cache", Tier::Safe),
        ("qqmusic-data", Tier::Review),
        ("kugou-data", Tier::Review),
        ("bilibili-data", Tier::Review),
        ("wps-roaming", Tier::Review),
        ("sogou-pinyin-userdata", Tier::Keep),
        ("ms-pinyin-userdata", Tier::Keep),
        // conda / 游戏平台
        ("conda-envs", Tier::Keep),
        ("steam-caches", Tier::Safe),
        ("steam-htmlcache", Tier::Review),
        ("epic-launcher-webcache", Tier::Safe),
        ("battlenet-cache", Tier::Safe),
    ];
    for (id, tier) in listed {
        assert_eq!(get(id).tier, *tier, "{id} 的档位");
    }
    // AI 模型：都要同时给“迁移”和“删除”两条路
    for id in ["huggingface-hub", "ollama-models", "torch-hub", "modelscope-cache", "insightface-models", "paddle-models", "lm-studio-models", "whisper-models"] {
        let h = &get(id).how;
        assert!(h.contains(&How::Migrate) && h.contains(&How::Delete), "{id} 的办法 {h:?}");
    }
    // 聊天文件 / conda envs：keep + migrate，且绝不给 delete
    for id in ["wechat-files", "qq-files", "wecom-files", "conda-envs"] {
        let h = &get(id).how;
        assert!(h.contains(&How::Migrate) && !h.contains(&How::Delete), "{id} 的办法 {h:?}");
    }
}

#[test]
fn b2_listed_items_real_paths() {
    let ctx = embedded_ctx();
    let cases: &[(&str, &str, Tier)] = &[
        (r"C:\Users\bob\.cache\huggingface\hub\models--bert\blobs\x", "huggingface-hub", Tier::Review),
        (r"C:\Users\bob\.cache\huggingface\transformers\abc.bin", "huggingface-hub", Tier::Review),
        (r"C:\Users\bob\.ollama\models\blobs\sha256-1", "ollama-models", Tier::Review),
        (r"C:\Users\bob\.cache\torch\hub\checkpoints\r.pth", "torch-hub", Tier::Review),
        (r"C:\Users\bob\.cache\modelscope\hub\m\w.bin", "modelscope-cache", Tier::Review),
        (r"C:\Users\bob\.insightface\models\buffalo_l\a.onnx", "insightface-models", Tier::Review),
        (r"C:\Users\bob\.paddleocr\whl\det\m.pdparams", "paddle-models", Tier::Review),
        (r"C:\Users\bob\.lmstudio\models\q\m.gguf", "lm-studio-models", Tier::Review),
        (r"C:\Users\bob\.cache\whisper\large-v3.pt", "whisper-models", Tier::Review),
        // 微信 / QQ 的聊天文件与 FileStorage：keep + migrate
        (r"C:\Users\bob\Documents\WeChat Files\wxid_abc\FileStorage\File\2024-01\a.pdf", "wechat-files", Tier::Keep),
        (r"C:\Users\bob\Documents\WeChat Files\wxid_abc\FileStorage\Video\2024-01\v.mp4", "wechat-files", Tier::Keep),
        (r"C:\Users\bob\Documents\xwechat_files\wxid_abc\msg\file\2025-01\a.docx", "wechat-files", Tier::Keep),
        (r"C:\Users\bob\Documents\Tencent Files\12345\FileRecv\a.zip", "qq-files", Tier::Keep),
        (r"C:\Users\bob\Documents\Tencent Files\nt_qq\nt_data\Pic\p.jpg", "qq-files", Tier::Keep),
        (r"C:\Users\bob\Documents\WXWork\1688\Cache\File\a.xlsx", "wecom-files", Tier::Keep),
        (r"C:\Users\bob\AppData\Roaming\LarkShell\sdk_storage\x", "feishu-larkshell", Tier::Review),
        (r"C:\Users\bob\AppData\Roaming\DingTalk\x.db", "dingtalk-data", Tier::Review),
        (r"C:\Users\bob\AppData\Local\NetEase\CloudMusic\Cache\c.uc", "netease-music-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Roaming\Tencent\QQMusic\x", "qqmusic-data", Tier::Review),
        (r"C:\Users\bob\AppData\Roaming\KuGou8\x", "kugou-data", Tier::Review),
        (r"C:\Users\bob\AppData\Roaming\bilibili\x", "bilibili-data", Tier::Review),
        (r"C:\Users\bob\AppData\Roaming\kingsoft\office6\x", "wps-roaming", Tier::Review),
        (r"C:\Users\bob\AppData\Roaming\SogouPY\x", "sogou-pinyin-userdata", Tier::Keep),
        (r"C:\Users\bob\AppData\Roaming\Microsoft\InputMethod\Chs\x.dat", "ms-pinyin-userdata", Tier::Keep),
        (r"C:\Users\bob\miniconda3\envs\d2l\python.exe", "conda-envs", Tier::Keep),
        (r"C:\Users\bob\.conda\envs\tf\python.exe", "conda-envs", Tier::Keep),
        (r"C:\Program Files (x86)\Steam\steamapps\shadercache\730\x.bin", "steam-caches", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Steam\htmlcache\Local State", "steam-htmlcache", Tier::Review),
        (r"C:\Users\bob\AppData\Local\EpicGamesLauncher\Saved\webcache_4147\x", "epic-launcher-webcache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\EpicGamesLauncher\Saved\webcache\x", "epic-launcher-webcache", Tier::Safe),
        (r"C:\ProgramData\Blizzard Entertainment\Battle.net\Cache\x", "battlenet-cache", Tier::Safe),
    ];
    let entries: Vec<(&str, u64)> = cases.iter().map(|(p, _, _)| (*p, 4096u64)).collect();
    let t = tree(&entries);
    for (p, id, tier) in cases {
        let e = ex(&t, p, &ctx);
        assert_eq!(rule_id(&e), Some(*id), "{p} 应由 {id} 说明，实际 {:?}", e.source);
        assert_eq!(e.tier, *tier, "{p} 档位");
        assert!(!e.what.is_empty() && !e.lose.is_empty(), "{p} 文案不能空");
        if *tier == Tier::Keep {
            assert!(!e.how.contains(&How::Delete) && e.gain_bytes == 0, "{p}：keep 不给删除、能腾出 = 0");
        }
    }
    // 聊天文件的具体办法：迁移
    for p in [r"C:\Users\bob\Documents\WeChat Files\wxid_abc\FileStorage\File\2024-01\a.pdf", r"C:\Users\bob\Documents\Tencent Files\12345\FileRecv\a.zip"] {
        assert!(ex(&t, p, &ctx).how.contains(&How::Migrate), "{p}");
    }
}

// ───────────────────────── 2. 启发式 ─────────────────────────

#[test]
fn b2_chromium_cache_strict_signature_is_the_only_safe_heuristic() {
    let root = temp_root("chromium");
    // 严格签名：新版 Cache\Cache_Data、GPUCache、Code Cache\js+wasm
    put(&root, r"Local\AppOne\Cache\Cache_Data\index", &[0u8; 2000]);
    put(&root, r"Local\AppOne\Cache\Cache_Data\data_0", &[0u8; 8000]);
    put(&root, r"Local\AppOne\Cache\Cache_Data\data_1", &[0u8; 8000]);
    put(&root, r"Local\AppOne\Cache\Cache_Data\f_00001a", &[0u8; 30_000]);
    put(&root, r"Local\AppOne\Cache\No_Vary_Search\journal.baj", &[0u8; 4]);
    put(&root, r"Local\AppOne\GPUCache\index", &[0u8; 2000]);
    put(&root, r"Local\AppOne\GPUCache\data_0", &[0u8; 8000]);
    put(&root, r"Local\AppOne\Code Cache\js\index", &[0u8; 100]);
    put(&root, r"Local\AppOne\Code Cache\js\0123456789abcdef_0", &[0u8; 9000]);
    put(&root, r"Local\AppOne\Code Cache\js\index-dir\the-real-index", &[0u8; 200]);
    put(&root, r"Local\AppOne\Code Cache\wasm\index", &[0u8; 100]);
    put(&root, r"Local\AppOne\Code Cache\wasm\fedcba9876543210_0", &[0u8; 5000]);
    // 名字对但混了别的文件
    put(&root, r"Local\AppTwo\Cache\Cache_Data\index", &[0u8; 2000]);
    put(&root, r"Local\AppTwo\Cache\Cache_Data\data_0", &[0u8; 8000]);
    put(&root, r"Local\AppTwo\Cache\my_notes.docx", &[0u8; 3000]);
    put(&root, r"Local\AppTwo\GPUCache\index", &[0u8; 2000]);
    put(&root, r"Local\AppTwo\GPUCache\data_0", &[0u8; 8000]);
    put(&root, r"Local\AppTwo\GPUCache\readme.txt", &[0u8; 300]);
    // 内容像缓存但文件夹名不是 Chromium 的缓存目录名
    put(&root, r"Local\AppThree\Stuff\index", &[0u8; 2000]);
    put(&root, r"Local\AppThree\Stuff\data_0", &[0u8; 8000]);
    // 在“文档”里（受保护）
    put(&root, r"Documents\SomeApp\Cache\Cache_Data\index", &[0u8; 2000]);
    put(&root, r"Documents\SomeApp\Cache\Cache_Data\data_0", &[0u8; 8000]);
    // 其他启发式的样本（护栏扫描用）
    put(&root, r"web\package.json", b"{}");
    put(&root, r"web\node_modules\x\index.js", b"x");
    put_big(&root, r"vm\disk.vhdx", 4 * 1024 * 1024);
    put(&root, r"logs\a.log", &[b'l'; 90_000]);
    for i in 0..4 {
        put(&root, &format!(r"tmpdir\x{i}.tmp"), &[0u8; 1000]);
    }

    let mut ctx = empty_rules_ctx(&root);
    ctx.now = diskwhy_core::explain::now_unix() + 400 * 86_400;
    let fx = Fixture::scan(root, ctx);

    for rel in [r"Local\AppOne\Cache", r"Local\AppOne\Cache\Cache_Data", r"Local\AppOne\GPUCache", r"Local\AppOne\Code Cache", r"Local\AppOne\Code Cache\js"] {
        let e = fx.explain(rel);
        assert_eq!(heur_name(&e), Some("chromium-cache"), "{rel}: {:?} {}", e.source, e.what);
        assert_eq!(e.tier, Tier::Safe, "{rel} 签名严格应为 safe");
        assert_eq!(e.category, Category::Cache);
        assert!(e.how.contains(&How::Delete) && e.gain_bytes > 0, "{rel}");
        assert!(e.notes.iter().any(|n| n.contains("很久没用")), "{rel} 缓存超过 180 天应提示很久没用：{:?}", e.notes);
    }
    for rel in [r"Local\AppTwo\Cache", r"Local\AppTwo\GPUCache", r"Local\AppThree\Stuff"] {
        let e = fx.explain(rel);
        assert_eq!(heur_name(&e), Some("chromium-cache-mixed"), "{rel}: {:?}", e.source);
        assert_eq!(e.tier, Tier::Review, "{rel} 混了别的东西或名字不对，不能 safe");
    }
    let doc = fx.explain(r"Documents\SomeApp\Cache");
    assert_eq!(heur_name(&doc), Some("chromium-cache"));
    assert_eq!(doc.tier, Tier::Review, "个人文件夹下的缓存不能 safe");
    assert!(doc.notes.iter().any(|n| n.contains("个人文件夹")), "{:?}", doc.notes);

    // 护栏扫描：整棵树每个节点都解释一遍，safe 只能来自 chromium-cache
    let mut stack: Vec<(Vec<&Node>, &Node)> = vec![(Vec::new(), &fx.result.tree)];
    let mut n_checked = 0;
    while let Some((chain, node)) = stack.pop() {
        if node.kind == NodeKind::Dir {
            let t = Target { node, path: &node.path, parent: chain.last().copied(), ancestors: &chain };
            let e = diskwhy_core::explain::explain(&t, &fx.ctx);
            n_checked += 1;
            if e.tier == Tier::Safe {
                assert_eq!(heur_name(&e), Some("chromium-cache"), "{} 被判成 safe，来源 {:?}", node.path, e.source);
                assert!(!node.path.to_lowercase().contains(r"\documents\"), "{}", node.path);
            }
            for c in &node.children {
                let mut ch = chain.clone();
                ch.push(node);
                stack.push((ch, c));
            }
        }
    }
    assert!(n_checked > 20, "只检查了 {n_checked} 个目录");
}

#[test]
fn b2_heuristics_project_hints_chat_data_conda_and_inheritance() {
    let root = temp_root("hints");
    // node_modules：本身、里面的一个包、嵌套的 node_modules、没有 package.json 的遗留
    put(&root, r"proj\web\package.json", b"{}");
    put(&root, r"proj\web\node_modules\lodash\index.js", &[0u8; 4000]);
    put(&root, r"proj\web\node_modules\lodash\node_modules\tiny\i.js", &[0u8; 1000]);
    put(&root, r"old\gone\node_modules\left-pad\index.js", b"x");
    // Rust target 里面的子目录
    put(&root, r"proj\rs\Cargo.toml", b"[package]");
    put(&root, r"proj\rs\target\debug\deps\libx.rlib", &[0u8; 9000]);
    // site-packages 里面的一个库
    put(&root, r"py\Lib\site-packages\numpy\core\x.pyd", &[0u8; 5000]);
    // 换了位置的微信聊天数据：视频占大头也必须是 keep（不能被“媒体文件”抢走）
    put_big(&root, r"moved\WeChat Files\wxid_1\FileStorage\Video\2024-01\a.mp4", 20 * 1024 * 1024);
    put(&root, r"moved\WeChat Files\wxid_1\Msg\MicroMsg.db", &[0u8; 5000]);
    // 程序目录下的 WXWork 是程序本体，不是聊天数据
    put(&root, r"PF\WXWork\WXWork.exe", &[0u8; 5000]);
    // 装在规则库不认识的位置的 conda
    put(&root, r"conda\conda-meta\history", b"h");
    put(&root, r"conda\envs\d2l\conda-meta\history", b"h");
    put(&root, r"conda\envs\d2l\python.exe", &[0u8; 5000]);

    let ctx = empty_rules_ctx(&root);
    let fx = Fixture::scan(root.clone(), ctx);

    let nm = fx.explain(r"proj\web\node_modules");
    assert_eq!(heur_name(&nm), Some("node-modules"));
    assert!(nm.what.contains("「web」") && nm.owner.as_deref().unwrap_or("").contains("web"), "{} / {:?}", nm.what, nm.owner);
    let inner = fx.explain(r"proj\web\node_modules\lodash");
    assert_eq!(heur_name(&inner), Some("node-modules"), "node_modules 里面的包也要认出来：{:?}", inner.source);
    assert!(inner.what.contains("「web」"), "{}", inner.what);
    let nested = fx.explain(r"proj\web\node_modules\lodash\node_modules");
    assert!(nested.what.contains("「web」"), "嵌套的 node_modules 也指向最外层项目：{}", nested.what);
    let orphan = fx.explain(r"old\gone\node_modules");
    assert_eq!(heur_name(&orphan), Some("node-modules-orphan"));
    assert!(orphan.clues.iter().any(|c| c.contains("没有 package.json")), "{:?}", orphan.clues);

    let rt = fx.explain(r"proj\rs\target\debug");
    assert_eq!(heur_name(&rt), Some("rust-target"), "{:?}", rt.source);
    assert!(rt.what.contains("「rs」"), "{}", rt.what);

    let sp = fx.explain(r"py\Lib\site-packages\numpy");
    assert_eq!(heur_name(&sp), Some("python-env"));
    assert_eq!(sp.tier, Tier::Review);

    for rel in [r"moved\WeChat Files", r"moved\WeChat Files\wxid_1\FileStorage", r"moved\WeChat Files\wxid_1\FileStorage\Video"] {
        let e = fx.explain(rel);
        assert_eq!(heur_name(&e), Some("chat-data"), "{rel}: {:?}", e.source);
        assert_eq!(e.tier, Tier::Keep, "{rel}");
        assert!(e.how == vec![How::Migrate] && e.gain_bytes == 0, "{rel}: {:?}", e.how);
    }
    assert_ne!(heur_name(&fx.explain(r"PF\WXWork")), Some("chat-data"), "Program Files 下的 WXWork 是程序本体");

    let env = fx.explain(r"conda\envs\d2l");
    assert_eq!(heur_name(&env), Some("conda-env"));
    assert_eq!(env.tier, Tier::Keep);
    assert!(env.how.contains(&How::Migrate) && !env.how.contains(&How::Delete));
    let envs = fx.explain(r"conda\envs");
    assert_eq!(heur_name(&envs), Some("conda-env"), "{:?}", envs.source);
    assert_eq!(envs.tier, Tier::Keep);
}

#[test]
fn b2_long_unused_note_on_cache_rules() {
    let ctx0 = embedded_ctx();
    let mtime = 1_700_000_000;
    let t = tree_at(&[(r"C:\Users\bob\AppData\Local\pip\cache\http\a", 1000), (r"C:\Users\bob\.cache\huggingface\hub\m\w.bin", 1000)], mtime);
    let mut ctx = ctx0.clone();
    ctx.now = mtime + 400 * 86_400;
    let pip = ex(&t, r"C:\Users\bob\AppData\Local\pip\cache", &ctx);
    assert_eq!(rule_id(&pip), Some("pip-cache"));
    assert!(pip.notes.iter().any(|n| n.contains("很久没用")), "{:?}", pip.notes);
    // 模型不是 cache 类，不写“很久没用”（模型很久不动是正常的）
    let hf = ex(&t, r"C:\Users\bob\.cache\huggingface\hub", &ctx);
    assert!(!hf.notes.iter().any(|n| n.contains("很久没用")), "{:?}", hf.notes);
    // 才 10 天：不提示
    ctx.now = mtime + 10 * 86_400;
    let pip2 = ex(&t, r"C:\Users\bob\AppData\Local\pip\cache", &ctx);
    assert!(!pip2.notes.iter().any(|n| n.contains("很久没用")), "{:?}", pip2.notes);
}

#[test]
fn b2_registry_owner_by_install_location_prefix_and_name() {
    // 安装目录前缀匹配：InstallLocation 为空时用 DisplayIcon 的 exe 目录；目录名匹配 DisplayName
    let env = fake_env();
    let apps = vec![
        InstalledApp::new("Acme Studio 3.2 (64-bit)", "Acme Corp", "", r#""D:\Tools\AcmeStudio\acme.exe",0"#, "HKLM32"),
        InstalledApp::new("Zeta Notes", "Zeta Ltd", "", "", "HKCU"),
    ];
    let ctx = ExplainCtx::new(env, RuleSet::default(), RegistrySnapshot::from_apps(apps), LlmSettings::default());
    let t = tree(&[(r"C:\Users\bob\AppData\Roaming\Zeta Notes\db\x.q", 4096)]);
    let z = ex(&t, r"C:\Users\bob\AppData\Roaming\Zeta Notes", &ctx);
    assert_eq!(heur_name(&z), Some("registry-appdata"));
    assert!(z.owner.as_deref().unwrap_or("").contains("Zeta Notes"), "{:?}", z.owner);
    let g = diskwhy_core::explain::heuristics::owner_from_path(r"D:\Tools\AcmeStudio\plugins\p", &ctx).unwrap();
    assert!(g.installed && g.name.contains("Acme Studio") && g.via == "安装目录", "{g:?}");
}

#[cfg(windows)]
#[test]
fn b2_real_registry_reads_three_views() {
    let snap = RegistrySnapshot::load();
    let mut by_hive: BTreeMap<&str, usize> = BTreeMap::new();
    for a in &snap.apps {
        *by_hive.entry(a.hive.as_str()).or_default() += 1;
    }
    println!("本机注册表 Uninstall：{by_hive:?}，读取 {} ms", snap.load_ms);
    assert!(!snap.apps.is_empty());
    assert!(by_hive.keys().all(|h| ["HKLM64", "HKLM32", "HKCU"].contains(h)), "{by_hive:?}");
}

// ───────────────────────── 3. LLM：发什么、收什么 ─────────────────────────

#[test]
fn b2_llm_input_four_fields_top20_and_username_redacted() {
    let mut entries: Vec<(String, u64)> = (0..30).map(|i| (format!(r"C:\Users\bob\AppData\Local\bobtools\item{i:02}.dat"), 10_000 + i as u64)).collect();
    entries.push((r"C:\Users\bob\AppData\Local\bobtools\Bob-Notes.txt".into(), 99_999));
    entries.push((r"C:\Users\bob\AppData\Local\bobtools\BOB\x".into(), 88_888));
    let refs: Vec<(&str, u64)> = entries.iter().map(|(p, s)| (p.as_str(), *s)).collect();
    let t = tree(&refs);
    let p = r"C:\Users\bob\AppData\Local\bobtools";
    let (chain, node) = view::find_node(&t, p).unwrap();
    let tg = Target { node, path: p, parent: chain.last().copied(), ancestors: &chain };
    let input = llm::build_input(&tg);
    let v = serde_json::to_value(&input).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["children", "name", "parent", "size"], "只发四个字段");
    assert_eq!(input.children.len(), llm::MAX_CHILDREN, "只发前 20 个子项");
    assert_eq!(input.parent, r"~\AppData\Local");
    assert_eq!(input.name, "~tools");
    assert!(input.children.contains(&"~-Notes.txt".to_string()), "{:?}", input.children);
    assert!(input.children.contains(&"~\\".to_string()), "{:?}", input.children);
    let body = llm::build_request_body("m", &input).to_string();
    assert!(!body.to_lowercase().contains("bob"), "请求体里还有用户名：{body}");
    assert!(!body.contains("mtime") && !body.contains("extensions") && !body.contains("modified"), "{body}");

    // 扫的就是用户目录本身：名字换成 ~，父路径是盘符
    let t2 = tree(&[(r"C:\Users\bob\x.txt", 10)]);
    let (chain, node) = view::find_node(&t2, r"C:\Users\bob").unwrap();
    let tg2 = Target { node, path: r"C:\Users\bob", parent: chain.last().copied(), ancestors: &chain };
    let i2 = llm::build_input(&tg2);
    assert_eq!(i2.name, "~");
    assert_eq!(i2.parent, r"C:\Users");
    // 用户目录不在 C:\Users 下（比如 D:\Profiles\bob）：靠调用方给的用户名脱敏
    let t3 = tree(&[(r"C:\Profiles\bob\stuff\bob.cfg", 10)]);
    let (chain, node) = view::find_node(&t3, r"C:\Profiles\bob\stuff").unwrap();
    let tg3 = Target { node, path: r"C:\Profiles\bob\stuff", parent: chain.last().copied(), ancestors: &chain };
    let i3 = llm::build_input_with(&tg3, &["bob".to_string()]);
    assert_eq!(i3.parent, r"C:\Profiles\~");
    assert_eq!(i3.children, vec!["~.cfg".to_string()]);
}

#[test]
fn b2_llm_answer_only_what_why_lose() {
    let long = "很".repeat(1000);
    let raw = format!(
        "```json\n{{\"what\":\"某个工具的下载缓存\",\"why\":\"自动下载\\u0007\",\"lose\":\"{long}\",\"tier\":\"safe\",\"owner\":\"某软件\",\"confidence\":0.99,\"how\":[\"delete\"]}}\n```"
    );
    let a = llm::parse_answer(&raw).unwrap();
    assert_eq!(a.why, "自动下载", "控制字符要去掉");
    assert!(a.lose.chars().count() <= 301, "超长字段要截断：{}", a.lose.chars().count());
    let e = llm::to_explanation(a, "m", 5_000_000);
    assert_eq!(e.tier, Tier::Review, "模型说 safe 也不算数");
    assert_eq!(e.owner, None, "模型给的 owner 不用");
    assert!((e.confidence - llm::LLM_CONFIDENCE).abs() < f32::EPSILON && e.confidence <= 0.3, "可信度固定低值");
    assert_eq!(e.how, vec![How::Manual], "模型给的 how 不用");
    assert_eq!(e.gain_bytes, 0);
    assert_eq!(e.source, Source::Llm { model: "m".into() });
}

// ───────────────────────── 4. 解释链 + 护栏（假 oracle，不联网）─────────────────────────

struct MockOracle {
    reply: Result<String, String>,
    calls: RefCell<Vec<String>>,
}

impl MockOracle {
    fn ok(s: &str) -> Self {
        MockOracle { reply: Ok(s.to_string()), calls: RefCell::new(Vec::new()) }
    }
    fn err(s: &str) -> Self {
        MockOracle { reply: Err(s.to_string()), calls: RefCell::new(Vec::new()) }
    }
    fn n(&self) -> usize {
        self.calls.borrow().len()
    }
}

impl LlmOracle for MockOracle {
    fn model(&self) -> String {
        "mock-model".into()
    }
    fn complete(&self, body: &serde_json::Value) -> Result<String, String> {
        self.calls.borrow_mut().push(body.to_string());
        self.reply.clone()
    }
}

const GREEDY_REPLY: &str = r#"{"what":"像是某个工具留下的数据","why":"工具运行时生成","lose":"不确定，先确认再删","tier":"safe","confidence":1.0,"owner":"随便猜的"}"#;

fn full(t: &Node, path: &str, ctx: &ExplainCtx, o: Option<&dyn LlmOracle>) -> Explanation {
    let (chain, node) = view::find_node(t, path).unwrap_or_else(|| panic!("找不到 {path}"));
    let tg = Target { node, path, parent: chain.last().copied(), ancestors: &chain };
    explain_full(&tg, ctx, o)
}

#[test]
fn b2_explain_chain_priority_rule_heuristic_llm_unknown() {
    let t = tree(&[
        (r"C:\Users\bob\AppData\Local\Temp\a.tmp", 5000),
        (r"C:\Users\bob\code\web\package.json", 10),
        (r"C:\Users\bob\code\web\node_modules\x\i.js", 5000),
        (r"C:\Users\bob\AppData\Local\zzqx\blob.xyz", 70_000),
        (r"C:\Users\bob\AppData\Local\zzqx\sub\q.abc", 30_000),
    ]);
    let mut ctx = embedded_ctx();
    ctx.settings = on_settings();
    let o = MockOracle::ok(GREEDY_REPLY);

    // 1. 规则命中：不问 AI
    let r = full(&t, r"C:\Users\bob\AppData\Local\Temp", &ctx, Some(&o));
    assert_eq!(rule_id(&r), Some("user-temp"));
    assert!((r.confidence - 1.0).abs() < f32::EPSILON);
    assert_eq!(o.n(), 0, "规则命中时不该问 AI");

    // 2. 启发式命中：不问 AI
    let h = full(&t, r"C:\Users\bob\code\web\node_modules", &ctx, Some(&o));
    assert_eq!(heur_name(&h), Some("node-modules"));
    assert!(h.confidence > 0.3 && h.confidence < 1.0);
    assert_eq!(o.n(), 0, "启发式命中时不该问 AI");

    // 3. 都没认出来 + 开关打开：问一次 AI，结果 review / Llm / 低可信度，保留线索
    let a = full(&t, r"C:\Users\bob\AppData\Local\zzqx", &ctx, Some(&o));
    assert_eq!(o.n(), 1);
    assert_eq!(a.source, Source::Llm { model: "mock-model".into() });
    assert_eq!(a.tier, Tier::Review, "LLM 永不 safe");
    assert!((a.confidence - llm::LLM_CONFIDENCE).abs() < f32::EPSILON);
    assert_eq!(a.owner, None, "注册表对不上时不写归属，AI 猜的不用");
    assert!(!a.clues.is_empty(), "保留 DiskWhy 自己看到的线索");
    let sent = o.calls.borrow()[0].clone();
    assert!(sent.contains("blob.xyz") && sent.contains("sub\\\\"), "应发子项名：{sent}");
    assert!(!sent.to_lowercase().contains("bob"), "发出去的请求里有用户名：{sent}");

    // 4. 开关关闭（默认）：不问，结果是未知
    let mut off = ctx.clone();
    off.settings = LlmSettings::default();
    let o2 = MockOracle::ok(GREEDY_REPLY);
    let u = full(&t, r"C:\Users\bob\AppData\Local\zzqx", &off, Some(&o2));
    assert_eq!(u.source, Source::Unknown);
    assert_eq!(o2.n(), 0, "默认关闭时绝不调用");
    // 没给 oracle：同 explain()
    let u2 = full(&t, r"C:\Users\bob\AppData\Local\zzqx", &ctx, None);
    assert_eq!(u2.source, Source::Unknown);

    // 5. AI 出错：退回未知，错误写进 notes，不 panic
    let bad = MockOracle::err("连不上 http://127.0.0.1:1");
    let e = full(&t, r"C:\Users\bob\AppData\Local\zzqx", &ctx, Some(&bad));
    assert_eq!(e.source, Source::Unknown);
    assert!(e.notes.iter().any(|n| n.contains("问 AI 没成功")), "{:?}", e.notes);
    // AI 回复不是 JSON：同样退回未知
    let junk = MockOracle::ok("我也不知道");
    let e2 = full(&t, r"C:\Users\bob\AppData\Local\zzqx", &ctx, Some(&junk));
    assert_eq!(e2.source, Source::Unknown);
}

#[test]
fn b2_enforce_guards_on_every_source() {
    let ctx = embedded_ctx();
    let plain = r"C:\Users\bob\AppData\Local\x";
    let docs = r"C:\Users\bob\Documents\x";
    let base = || {
        let mut e = llm::to_explanation(llm::LlmAnswer { what: "w".into(), why: "y".into(), lose: "l".into() }, "m", 100);
        e.how = vec![How::Delete];
        e.gain_bytes = 100;
        e
    };
    let with = |src: Source, tier: Tier| {
        let mut e = base();
        e.source = src;
        e.tier = tier;
        e
    };
    let llm_src = || Source::Llm { model: "m".into() };
    let heur = |n: &str| Source::Heuristic { name: n.into() };
    let rule = || Source::Rule { id: "r".into(), group: "g".into(), inherited_from: None };
    // LLM：safe / keep 都改成 review，能腾出清零
    for tier in [Tier::Safe, Tier::Keep] {
        let e = enforce_guards(with(llm_src(), tier), plain, &ctx);
        assert_eq!(e.tier, Tier::Review);
        assert_eq!(e.gain_bytes, 0);
    }
    // 未知：永远 review
    assert_eq!(enforce_guards(with(Source::Unknown, Tier::Safe), plain, &ctx).tier, Tier::Review);
    // 启发式：只有 chromium-cache 能 safe，而且不能在个人文件夹下
    assert_eq!(enforce_guards(with(heur("model-weights"), Tier::Safe), plain, &ctx).tier, Tier::Review);
    assert_eq!(enforce_guards(with(heur("temp-files"), Tier::Safe), plain, &ctx).tier, Tier::Review);
    assert_eq!(enforce_guards(with(heur("chromium-cache"), Tier::Safe), plain, &ctx).tier, Tier::Safe);
    assert_eq!(enforce_guards(with(heur("chromium-cache"), Tier::Safe), docs, &ctx).tier, Tier::Review);
    assert_eq!(enforce_guards(with(heur("virtual-disk"), Tier::Keep), plain, &ctx).tier, Tier::Keep, "keep 不动");
    // 规则：safe 落在个人文件夹下降级，其余不动
    assert_eq!(enforce_guards(with(rule(), Tier::Safe), docs, &ctx).tier, Tier::Review);
    assert_eq!(enforce_guards(with(rule(), Tier::Safe), plain, &ctx).tier, Tier::Safe);
    assert_eq!(enforce_guards(with(rule(), Tier::Keep), docs, &ctx).tier, Tier::Keep);
}
