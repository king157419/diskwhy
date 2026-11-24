//! 阶段 B：解释引擎测试。
//!
//! - 规则库校验（内置规则全部通过 + 各种违规写法会被拦住）；
//! - 规则匹配（环境变量展开、`*` / `**` 通配、大小写、继承、when_child）；
//! - 每个启发式：在 %TEMP% 下真实建目录、用真实扫描器扫一遍，再解释；
//! - LLM：请求体不含文件内容、路径已脱敏（含一个本地假服务器抓真实发出的字节）。

use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::{self, LlmSettings};
use diskwhy_core::explain::registry::{InstalledApp, RegistrySnapshot};
use diskwhy_core::explain::rules::{validate, RuleDef, RuleSet, EMBEDDED};
use diskwhy_core::explain::{tier_summary, ExplainCtx, Explanation, How, Source, Target, Tier};
use diskwhy_core::scan::scan;
use diskwhy_core::view::{self, explain_path, listing};
use diskwhy_core::*;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
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

fn ctx_with(env: EnvMap, rules: RuleSet, apps: Vec<InstalledApp>) -> ExplainCtx {
    ExplainCtx::new(env, rules, RegistrySnapshot::from_apps(apps), LlmSettings::default())
}

fn embedded_ctx() -> ExplainCtx {
    let env = fake_env();
    let (rules, errs) = RuleSet::embedded(&env);
    assert!(errs.is_empty(), "内置规则有错误：{errs:#?}");
    ctx_with(env, rules, Vec::new())
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
            children.push(Node {
                name: n.clone(),
                path: String::new(),
                kind: NodeKind::File,
                logical: *sz,
                allocated: *sz,
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
            });
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

// ───────────────────────── 1. 规则库校验 ─────────────────────────

#[test]
fn embedded_rules_all_valid_and_enough() {
    let env = fake_env();
    let (rs, errs) = RuleSet::embedded(&env);
    assert!(errs.is_empty(), "内置规则校验失败：{errs:#?}");
    assert!(rs.rules.len() >= 80, "规则只有 {} 条，要求 ≥ 80", rs.rules.len());
    let groups: HashSet<&str> = rs.rules.iter().map(|r| r.group.as_str()).collect();
    for (g, _) in EMBEDDED {
        assert!(groups.contains(g), "分组 {g} 里没有规则");
    }
    // 每条规则的每个模式都能在假环境里展开
    let per_rule: usize = rs.rules.iter().map(|r| r.raw_patterns.len()).sum();
    assert_eq!(per_rule, rs.patterns.len(), "有模式没能展开");
    println!("内置规则 {} 条，模式 {} 个，分组 {:?}", rs.rules.len(), rs.patterns.len(), groups);
}

#[test]
fn every_rule_has_all_fields_in_raw_toml() {
    // 直接按 TOML 表检查字段齐全（不依赖 serde 默认值）。
    const REQUIRED: &[&str] = &["id", "owner", "category", "tier", "match", "what", "why", "lose", "how", "how_detail", "refs", "verified_on"];
    for (group, text) in EMBEDDED {
        let v: toml::Value = toml::from_str(text).unwrap_or_else(|e| panic!("{group} 解析失败：{e}"));
        let rules = v.get("rule").and_then(|r| r.as_array()).unwrap_or_else(|| panic!("{group} 没有 [[rule]]"));
        for r in rules {
            let t = r.as_table().unwrap();
            for f in REQUIRED {
                assert!(t.contains_key(*f), "{group} 的规则 {:?} 缺字段 {f}", t.get("id"));
            }
        }
    }
}

#[test]
fn safe_rules_never_under_personal_folders_and_keep_never_delete() {
    let ctx = embedded_ctx();
    for (ri, p) in &ctx.rules.patterns {
        let r = &ctx.rules.rules[*ri];
        if r.tier == Tier::Safe {
            let segs = diskwhy_core::explain::pattern::split_lower(&p.expanded);
            assert!(!ctx.is_protected(&segs), "safe 规则 {} 的模式 {} 落在个人文件夹下", r.id, p.expanded);
        }
        if r.tier == Tier::Keep {
            assert!(!r.how.contains(&How::Delete), "keep 规则 {} 的 how 含 delete", r.id);
        }
    }
}

fn def(toml_text: &str) -> RuleDef {
    #[derive(serde::Deserialize)]
    struct F {
        rule: Vec<RuleDef>,
    }
    let f: F = toml::from_str(toml_text).expect("测试用 TOML 应能解析");
    f.rule.into_iter().next().unwrap()
}

const GOOD: &str = r#"
[[rule]]
id = "x"
owner = "o"
category = "cache"
tier = "safe"
match = ['%LOCALAPPDATA%\x']
what = "w"
why = "y"
lose = "l"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-09-23"
"#;

#[test]
fn validation_rejects_bad_rules() {
    assert!(validate(&def(GOOD), &mut HashSet::new()).is_empty());

    let cases: &[(&str, &str, &str)] = &[
        ("safe 落在文档下", r"match = ['%LOCALAPPDATA%\x']", r"match = ['%USERPROFILE%\Documents\x']"),
        ("safe 落在桌面下（大小写不同）", r"match = ['%LOCALAPPDATA%\x']", r"match = ['%USERPROFILE%\desktop\x']"),
        ("safe 落在 OneDrive 下", r"match = ['%LOCALAPPDATA%\x']", r"match = ['%USERPROFILE%\OneDrive - Contoso\x']"),
        ("safe 用通配覆盖整个用户目录", r"match = ['%LOCALAPPDATA%\x']", r"match = ['%USERPROFILE%\**\Cache']"),
        ("safe 落在下载下", r"match = ['%LOCALAPPDATA%\x']", r"match = ['%USERPROFILE%\Downloads']"),
        ("keep 却能 delete", "tier = \"safe\"", "tier = \"keep\""),
        ("未知变量", r"match = ['%LOCALAPPDATA%\x']", r"match = ['%FOO%\x']"),
        ("没有变量开头", r"match = ['%LOCALAPPDATA%\x']", r"match = ['C:\x']"),
        ("路径含 ..", r"match = ['%LOCALAPPDATA%\x']", r"match = ['%LOCALAPPDATA%\..\x']"),
        ("migrate 没写 how_detail", "how = \"delete\"", "how = \"migrate\""),
        ("日期格式错", "verified_on = \"2026-09-23\"", "verified_on = \"2026/9/23\""),
        ("what 为空", "what = \"w\"", "what = \"  \""),
        ("id 非法", "id = \"x\"", "id = \"X Y\""),
    ];
    for (name, from, to) in cases {
        let t = GOOD.replace(from, to);
        assert_ne!(t, GOOD, "用例 {name} 没替换成功");
        let errs = validate(&def(&t), &mut HashSet::new());
        assert!(!errs.is_empty(), "用例「{name}」应该校验失败，但通过了");
    }
    // 重复 id
    let mut seen = HashSet::new();
    assert!(validate(&def(GOOD), &mut seen).is_empty());
    assert!(!validate(&def(GOOD), &mut seen).is_empty(), "重复 id 应失败");
    // 缺字段：反序列化直接失败
    let missing = GOOD.replace("lose = \"l\"\n", "");
    assert!(toml::from_str::<toml::Value>(&missing).is_ok());
    let (rs, errs) = RuleSet::from_sources(&[("t", &missing)], &fake_env());
    assert!(rs.rules.is_empty() && !errs.is_empty(), "缺 lose 字段应加载失败");
    // 未知字段
    let extra = GOOD.replace("refs = []", "refs = []\nfoo = 1");
    let (rs, errs) = RuleSet::from_sources(&[("t", &extra)], &fake_env());
    assert!(rs.rules.is_empty() && !errs.is_empty(), "未知字段应加载失败");
}

#[test]
fn required_items_from_plan_are_covered() {
    let ctx = embedded_ctx();
    let r = |id: &str| ctx.rules.by_id(id).unwrap_or_else(|| panic!("缺规则 {id}"));
    let check = |id: &str, tier: Tier, how: &[How]| {
        let x = r(id);
        assert_eq!(x.tier, tier, "{id} 档位不对");
        for h in how {
            assert!(x.how.contains(h), "{id} 的 how 应含 {h:?}，实际 {:?}", x.how);
        }
    };
    check("winsxs", Tier::Keep, &[How::BuiltinTool]);
    assert!(r("winsxs").how_detail.contains("DISM /Online /Cleanup-Image /StartComponentCleanup"));
    check("windows-old", Tier::Review, &[How::BuiltinTool]);
    check("windows-installer-cache", Tier::Keep, &[]);
    check("prefetch", Tier::Keep, &[]);
    check("hiberfil", Tier::Review, &[How::BuiltinTool]);
    assert!(r("hiberfil").how_detail.contains("powercfg /h off"));
    check("pagefile", Tier::Keep, &[]);
    check("user-temp", Tier::Safe, &[]);
    check("windows-temp", Tier::Safe, &[]);
    check("delivery-optimization", Tier::Safe, &[How::BuiltinTool]);
    check("package-cache", Tier::Review, &[]);
    check("thumbnail-cache", Tier::Safe, &[]);
    check("docker-data-vhdx", Tier::Keep, &[How::Migrate]);
    assert!(r("docker-data-vhdx").how_detail.contains("Disk image location"));
    check("wsl-distro-vhdx", Tier::Keep, &[How::Migrate]);
    assert!(r("wsl-distro-vhdx").how_detail.contains("wsl --export") && r("wsl-distro-vhdx").how_detail.contains("wsl --import"));
    check("wechat-files", Tier::Keep, &[How::Migrate]);
    check("qq-files", Tier::Keep, &[How::Migrate]);
    // 缓存复查（2026-11）：radium 里有内置浏览器资料和小程序本地存储，不是缓存 → review，只能用微信自带清理
    check("wechat-radium", Tier::Review, &[How::BuiltinTool]);
    check("wechat-logs", Tier::Safe, &[]);
    check("feishu-update", Tier::Safe, &[]);
    check("feishu-larkshell", Tier::Review, &[]);
    check("netease-music-cache", Tier::Safe, &[]);
    check("netease-music-downloads", Tier::Keep, &[]);
    for id in ["huggingface-hub", "ollama-models", "torch-hub", "modelscope-cache", "insightface-models", "paddle-models", "lm-studio-models"] {
        check(id, Tier::Review, &[How::Migrate, How::Delete]);
        assert!(r(id).lose.contains("重新下载"), "{id} 的 lose 应说明可重新下载");
    }
    check("conda-pkgs", Tier::Safe, &[How::BuiltinTool]);
    assert!(r("conda-pkgs").how_detail.contains("conda clean --all"));
    check("conda-envs", Tier::Keep, &[How::Migrate]);
    check("chrome-cache", Tier::Safe, &[]);
    check("edge-cache", Tier::Safe, &[]);
    check("firefox-cache", Tier::Safe, &[]);
    check("chrome-profile", Tier::Keep, &[]);
    check("edge-profile", Tier::Keep, &[]);
    check("chromium-generic-cache", Tier::Safe, &[]);
    // 缓存复查：Service Worker 的 CacheStorage / ScriptCache 是网站数据（和 IndexedDB 同类），不是缓存 → review
    check("chromium-generic-sw-cache", Tier::Review, &[]);
    // 开发缓存全家
    for id in ["npm-cache", "pnpm-store", "yarn-cache", "pip-cache", "uv-cache", "cargo-registry", "go-cache", "gradle-cache", "nuget-cache"] {
        assert_eq!(r(id).tier, Tier::Safe, "{id} 应为 safe");
    }
    check("maven-repo", Tier::Review, &[]);
    for id in ["visual-studio-cache", "jetbrains-caches", "vscode-family-cached"] {
        r(id);
    }
    // 中国软件分组覆盖面
    for id in ["wecom-files", "dingtalk-data", "baidu-netdisk", "thunder-data", "qqmusic-data", "kugou-data", "bilibili-data", "wps-program", "sogou-pinyin-userdata", "ms-pinyin-userdata"] {
        r(id);
    }
}

// ───────────────────────── 2. 匹配 ─────────────────────────

#[test]
fn env_expansion_case_and_wildcards() {
    let ctx = embedded_ctx();
    let t = tree(&[
        (r"C:\Users\bob\AppData\Local\npm-cache\_cacache\index-v5\aa", 10),
        (r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Profile 3\Cache\Cache_Data\index", 10),
        (r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Profile 3\Bookmarks", 10),
        (r"C:\Users\bob\AppData\Local\Microsoft\Windows\Explorer\thumbcache_1024.db", 10),
        (r"C:\Users\bob\AppData\Local\Docker\wsl\disk\docker_data.vhdx", 1000),
        (r"C:\Users\bob\AppData\Local\wsl\{d1f9e0f8-2451-4607-a4db-2cfc50dc641c}\ext4.vhdx", 1000),
        (r"C:\Users\bob\AppData\Roaming\LarkShell\aha\users\51af\profile_explorer\Service Worker\CacheStorage\ab\x", 10),
        (r"C:\Users\bob\AppData\Roaming\LarkShell\sdk_storage\x", 10),
        (r"C:\Users\bob\AppData\Roaming\SomeElectronApp\Cache\Cache_Data\index", 10),
        (r"C:\Users\bob\AppData\Roaming\SomeTool\Cache\settings.json", 10),
        (r"C:\Windows\WinSxS\amd64_x\a.dll", 10),
        (r"C:\Windows\Temp\x.tmp", 10),
        (r"C:\hiberfil.sys", 10),
    ]);
    // 大小写不敏感：路径用全小写 / 全大写查
    assert_eq!(rule_id(&ex(&t, r"c:\users\BOB\appdata\local\NPM-CACHE", &ctx)), Some("npm-cache"));
    // 继承：npm-cache 下的子目录也按 npm-cache 说明
    let e = ex(&t, r"C:\Users\bob\AppData\Local\npm-cache\_cacache", &ctx);
    assert_eq!(rule_id(&e), Some("npm-cache"));
    assert!(matches!(&e.source, Source::Rule { inherited_from: Some(_), .. }));
    // * 通配 Profile 名；缓存 safe，Profile 本身 keep（继承自 User Data）
    let cache = ex(&t, r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Profile 3\Cache", &ctx);
    assert_eq!((rule_id(&cache), cache.tier), (Some("chrome-cache"), Tier::Safe));
    let prof = ex(&t, r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Profile 3", &ctx);
    assert_eq!((rule_id(&prof), prof.tier), (Some("chrome-profile"), Tier::Keep));
    let bm = ex(&t, r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Profile 3\Bookmarks", &ctx);
    assert_eq!(bm.tier, Tier::Keep, "书签文件继承 Profile 的 keep");
    // 文件名通配
    assert_eq!(rule_id(&ex(&t, r"C:\Users\bob\AppData\Local\Microsoft\Windows\Explorer\thumbcache_1024.db", &ctx)), Some("thumbnail-cache"));
    // keep + migrate 的虚拟磁盘
    let d = ex(&t, r"C:\Users\bob\AppData\Local\Docker\wsl\disk\docker_data.vhdx", &ctx);
    assert_eq!((rule_id(&d), d.tier), (Some("docker-data-vhdx"), Tier::Keep));
    assert!(d.how.contains(&How::Migrate) && d.gain_bytes == 0);
    let w = ex(&t, r"C:\Users\bob\AppData\Local\wsl\{d1f9e0f8-2451-4607-a4db-2cfc50dc641c}\ext4.vhdx", &ctx);
    assert_eq!((rule_id(&w), w.tier), (Some("wsl-distro-vhdx"), Tier::Keep));
    // ** 通配 + 具体规则优先于上层继承：飞书里的 CacheStorage 由 Service Worker 规则说明（缓存复查后是 review）
    let sw = ex(&t, r"C:\Users\bob\AppData\Roaming\LarkShell\aha\users\51af\profile_explorer\Service Worker\CacheStorage", &ctx);
    assert_eq!((rule_id(&sw), sw.tier), (Some("chromium-generic-sw-cache"), Tier::Review));
    assert!(sw.owner.as_deref().unwrap_or("").contains("LarkShell"), "owner_from_path 应推断出 LarkShell：{:?}", sw.owner);
    let sdk = ex(&t, r"C:\Users\bob\AppData\Roaming\LarkShell\sdk_storage", &ctx);
    assert_eq!((rule_id(&sdk), sdk.tier), (Some("feishu-larkshell"), Tier::Review));
    // when_child：有 Cache_Data 才算 Chromium 缓存
    assert_eq!(rule_id(&ex(&t, r"C:\Users\bob\AppData\Roaming\SomeElectronApp\Cache", &ctx)), Some("chromium-generic-cache"));
    assert_ne!(rule_id(&ex(&t, r"C:\Users\bob\AppData\Roaming\SomeTool\Cache", &ctx)), Some("chromium-generic-cache"));
    // 系统项
    assert_eq!(ex(&t, r"C:\Windows\WinSxS", &ctx).tier, Tier::Keep);
    assert_eq!(rule_id(&ex(&t, r"C:\Windows\WinSxS\amd64_x", &ctx)), Some("winsxs"));
    assert_eq!(ex(&t, r"C:\Windows\Temp", &ctx).tier, Tier::Safe);
    let h = ex(&t, r"C:\hiberfil.sys", &ctx);
    assert_eq!((rule_id(&h), h.tier), (Some("hiberfil"), Tier::Review));
    // scope = self 的容器：AppData 自己有说明，但不会传给没有规则的子项
    let unknown_child = ex(&t, r"C:\Users\bob\AppData\Roaming\SomeTool", &ctx);
    assert_ne!(rule_id(&unknown_child), Some("appdata-roaming"));
}

#[test]
fn missing_env_var_skips_pattern_but_keeps_rule() {
    // 模拟 32 位系统 / 精简环境：没有 ProgramFiles(x86)、TEMP、PUBLIC
    let env = EnvMap::from_pairs(&[
        ("SystemDrive", "C:"),
        ("SystemRoot", r"C:\Windows"),
        ("USERPROFILE", r"C:\Users\bob"),
        ("LOCALAPPDATA", r"C:\Users\bob\AppData\Local"),
        ("APPDATA", r"C:\Users\bob\AppData\Roaming"),
        ("ProgramFiles", r"C:\Program Files"),
        ("ProgramData", r"C:\ProgramData"),
    ]);
    let (rs, errs) = RuleSet::embedded(&env);
    assert!(errs.is_empty(), "缺变量不是规则错误：{errs:?}");
    assert!(rs.patterns.len() < rs.rules.iter().map(|r| r.raw_patterns.len()).sum::<usize>());
    // 规则本身还在（只是用到缺失变量的模式被跳过）
    assert!(rs.by_id("edge-program").is_some());
    let t = tree(&[(r"C:\Users\bob\AppData\Local\Temp\a", 1)]);
    let ctx = ctx_with(env, rs, vec![]);
    assert_eq!(rule_id(&ex(&t, r"C:\Users\bob\AppData\Local\Temp", &ctx)), Some("user-temp"), "%LOCALAPPDATA%\\Temp 兜底");
}

#[test]
fn user_rules_dir_is_loaded_and_validated() {
    let dir = temp_root("userrules");
    std::fs::write(dir.join("mine.toml"), GOOD.replace("id = \"x\"", "id = \"my-rule\"")).unwrap();
    std::fs::write(dir.join("bad.toml"), GOOD.replace("tier = \"safe\"", "tier = \"keep\"").replace("id = \"x\"", "id = \"bad-rule\"")).unwrap();
    let env = fake_env();
    let (mut rs, _) = RuleSet::embedded(&env);
    let warns = rs.load_user_dir(&dir, &env);
    assert!(rs.by_id("my-rule").is_some());
    assert!(rs.by_id("bad-rule").is_none());
    assert!(warns.iter().any(|w| w.contains("bad-rule")), "{warns:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tier_summary_splits_bytes_by_rule() {
    let ctx = embedded_ctx();
    let t = tree(&[
        (r"C:\Users\bob\AppData\Local\npm-cache\x", 100),
        (r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Default\Cache\Cache_Data\index", 50),
        (r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Default\Bookmarks", 7),
        (r"C:\Windows\WinSxS\a", 1000),
        (r"C:\Windows.old\x", 300),
        (r"C:\mystery\x", 5),
        // 用户目录是 scope = self 的容器规则：里面的散文件不归它
        (r"C:\Users\bob\loose.txt", 9),
    ]);
    let s = tier_summary(&t, &ctx);
    assert_eq!(s.safe_bytes, 150);
    assert_eq!(s.keep_bytes, 1007);
    assert_eq!(s.review_bytes, 300);
    assert_eq!(s.uncovered_bytes, 14);
    assert_eq!(s.safe_bytes + s.review_bytes + s.keep_bytes + s.uncovered_bytes, t.allocated);
}

// ───────────────────────── 3. 启发式（真实临时目录） ─────────────────────────

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn temp_root(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("diskwhy-b-{}-{}-{}", tag, std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// 用 set_len 造一个大文件（NTFS 上会真实分配簇，但不用真的写数据）。
fn put_big(root: &Path, rel: &str, len: u64) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let f = std::fs::File::create(p).unwrap();
    f.set_len(len).unwrap();
}

/// 真实扫描临时目录，然后解释其中某个相对路径。规则库为空，逼出启发式。
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

fn empty_rules_ctx(root: &Path, apps: Vec<InstalledApp>) -> ExplainCtx {
    let r = root.to_str().unwrap();
    let env = EnvMap::from_pairs(&[
        ("SystemDrive", "C:"),
        ("USERPROFILE", r),
        ("LOCALAPPDATA", &format!(r"{r}\Local")),
        ("APPDATA", &format!(r"{r}\Roaming")),
        ("ProgramFiles", &format!(r"{r}\PF")),
        ("ProgramData", &format!(r"{r}\PD")),
    ]);
    ctx_with(env, RuleSet::default(), apps)
}

fn assert_heur(e: &Explanation, name: &str, tier: Tier) {
    assert_eq!(heur_name(e), Some(name), "期望启发式 {name}，实际 {:?}（what = {}）", e.source, e.what);
    assert_eq!(e.tier, tier, "{name} 档位");
    assert_ne!(e.tier, Tier::Safe, "启发式永远不能是 safe");
}

#[test]
fn heuristic_signatures_on_real_dirs() {
    let root = temp_root("heur");
    // 1 Chromium 缓存
    put(&root, r"appx\Cache\Cache_Data\index", b"i");
    put(&root, r"appx\Cache\Cache_Data\data_0", &[0u8; 5000]);
    // 2 Python 环境
    put(&root, r"py\Lib\site-packages\numpy\__init__.py", b"#");
    put(&root, r"py\python.exe", &[0u8; 3000]);
    // 3 node_modules（有 package.json）与 没有 package.json 的对照组
    put(&root, r"webapp\package.json", b"{}");
    put(&root, r"webapp\node_modules\left-pad\index.js", b"x");
    put(&root, r"orphan\node_modules\left-pad\index.js", b"x");
    // 4 Rust target
    put(&root, r"rs\Cargo.toml", b"[package]");
    put(&root, r"rs\target\debug\app.exe", &[0u8; 9000]);
    // 5 虚拟环境
    put(&root, r"proj\.venv\pyvenv.cfg", b"home = C:\\Python");
    put(&root, r"proj\.venv\Lib\site-packages\a.py", b"#");
    // 6 .NET
    put(&root, r"net\App.csproj", b"<Project/>");
    put(&root, r"net\bin\Debug\App.dll", &[0u8; 4000]);
    put(&root, r"net\obj\project.assets.json", b"{}");
    // 7 模型权重（单个 > 100MB）
    put_big(&root, r"weights\model.safetensors", 150 * 1024 * 1024);
    put(&root, r"weights\config.json", b"{}");
    // 8 虚拟磁盘
    put_big(&root, r"vm\disk.vhdx", 8 * 1024 * 1024);
    put(&root, r"vm\notes.txt", b"n");
    // 9 日志
    put(&root, r"logs\app.log", &[b'l'; 200_000]);
    put(&root, r"logs\crash.dmp", &[0u8; 100_000]);
    put(&root, r"logs\readme.txt", &[b'r'; 1000]);
    // 10 下载目录里的安装包（USERPROFILE = root）
    put(&root, r"Downloads\setups\tool.msi", &[0u8; 300_000]);
    put(&root, r"Downloads\setups\other.exe", &[0u8; 200_000]);
    put(&root, r"Downloads\setups\readme.txt", &[0u8; 100]);
    // 11 临时文件
    for i in 0..6 {
        put(&root, &format!(r"tmpdir\x{i}.tmp"), &[0u8; 1000]);
    }
    put(&root, r"tmpdir\keep.dat", &[0u8; 500]);
    // 12 媒体
    put_big(&root, r"clips\a.mp4", 20 * 1024 * 1024);
    put(&root, r"clips\cover.jpeg", &[0u8; 50_000]);
    // 未知
    put(&root, r"Local\weirdthing\blob.xyz", &[0u8; 70_000]);
    put(&root, r"Local\weirdthing\sub\q.abc", &[0u8; 30_000]);

    let ctx = empty_rules_ctx(&root, Vec::new());
    let fx = Fixture::scan(root, ctx);

    assert_heur(&fx.explain(r"appx\Cache"), "chromium-cache", Tier::Review);
    // 有 python.exe + Lib\site-packages：2026-09-24 起由更具体的 python-runtime 认（同为 review）。
    assert_heur(&fx.explain("py"), "python-runtime", Tier::Review);
    assert_heur(&fx.explain(r"webapp\node_modules"), "node-modules", Tier::Review);
    assert_ne!(heur_name(&fx.explain(r"orphan\node_modules")), Some("node-modules"), "没有 package.json 不应判成 Node 依赖");
    assert_heur(&fx.explain(r"rs\target"), "rust-target", Tier::Review);
    assert_heur(&fx.explain(r"proj\.venv"), "python-venv", Tier::Review);
    assert_heur(&fx.explain(r"net\bin"), "dotnet-build", Tier::Review);
    assert_heur(&fx.explain(r"net\obj"), "dotnet-build", Tier::Review);
    let m = fx.explain("weights");
    assert_heur(&m, "model-weights", Tier::Review);
    assert!(m.how.contains(&How::Migrate) && m.how.contains(&How::Delete));
    assert_heur(&fx.explain(r"weights\model.safetensors"), "model-weights", Tier::Review);
    let v = fx.explain("vm");
    assert_heur(&v, "virtual-disk", Tier::Keep);
    assert!(v.how.contains(&How::Migrate) && !v.how.contains(&How::Delete));
    assert_heur(&fx.explain("logs"), "logs-dumps", Tier::Review);
    assert_heur(&fx.explain(r"Downloads\setups"), "downloaded-installer", Tier::Review);
    assert_heur(&fx.explain(r"Downloads\setups\tool.msi"), "downloaded-installer", Tier::Review);
    assert_heur(&fx.explain("tmpdir"), "temp-files", Tier::Review);
    assert_heur(&fx.explain("clips"), "media-files", Tier::Review);

    // 未知：线索齐全
    let u = fx.explain(r"Local\weirdthing");
    assert_eq!(u.source, Source::Unknown);
    assert_eq!(u.what, "我也不确定这是什么");
    assert_eq!(u.tier, Tier::Review);
    let all = u.clues.join("\n");
    assert!(all.contains("最大的几个子项") && all.contains("blob.xyz"), "{all}");
    assert!(all.contains("主要文件类型") && all.contains(".xyz"), "{all}");
    assert!(all.contains("最近一次修改"), "{all}");
    assert!(all.contains("不在 AppData 下") || all.contains("AppData"), "{all}");
}

#[test]
fn heuristic_sibling_marker_found_even_when_parent_files_aggregated() {
    // 父目录文件多于 top_files_per_dir（50），package.json 会被合并进 Rest，要退回去问文件系统。
    let root = temp_root("agg");
    put(&root, r"big\package.json", b"{}");
    for i in 0..80 {
        put(&root, &format!(r"big\f{i:03}.txt"), &vec![0u8; 5000 + i * 10]);
    }
    put(&root, r"big\node_modules\x\index.js", b"x");
    let ctx = empty_rules_ctx(&root, Vec::new());
    let fx = Fixture::scan(root, ctx);
    let big = view::find_dir(&fx.result.tree, fx.root.join("big").to_str().unwrap()).unwrap();
    let big = big.last().unwrap();
    assert!(big.children.iter().any(|c| c.kind == NodeKind::Rest), "前提：父目录应有 Rest 聚合");
    assert!(!big.children.iter().any(|c| c.name == "package.json"), "前提：package.json 被合并掉了");
    assert_heur(&fx.explain(r"big\node_modules"), "node-modules", Tier::Review);
}

#[test]
fn heuristic_registry_owner_and_age() {
    let root = temp_root("reg");
    put(&root, r"PF\FooApp\foo.exe", &[0u8; 50_000]);
    put(&root, r"Local\Fooware\state\s.bin2", &[0u8; 20_000]);
    put(&root, r"Roaming\Contoso\thing.q", &[0u8; 20_000]);
    put(&root, r"PD\NVIDIA Corporation\NVIDIA App\UpdateFramework\x.q", &[0u8; 20_000]);
    put(&root, r"PD\NVIDIA Corporation\Other Thing\y.q", &[0u8; 20_000]);
    let rs = root.to_str().unwrap();
    let apps = vec![
        InstalledApp::new("Foo App 2.1 (64-bit)", "Foo Inc.", &format!(r"{rs}\PF\FooApp\"), "", "HKLM64"),
        InstalledApp::new("Fooware Studio", "Fooware Ltd", "", "", "HKCU"),
        InstalledApp::new("Widget Maker", "Contoso Corporation", "", "", "HKLM32"),
        // 同一厂商多个软件：厂商文件夹名不能随便前缀匹配到其中一个
        InstalledApp::new("NVIDIA Nsight Visual Studio Edition 2025.4.0.25287", "NVIDIA Corporation", "", "", "HKLM64"),
        InstalledApp::new("NVIDIA App 11.0.8.299", "NVIDIA Corporation", "", "", "HKLM64"),
        InstalledApp::new("NVIDIA Graphics Driver 581.29", "NVIDIA Corporation", "", "", "HKLM64"),
    ];
    let mut ctx = empty_rules_ctx(&root, apps);
    ctx.now = diskwhy_core::explain::now_unix() + 400 * 86_400; // 让它“一年多没动”
    let fx = Fixture::scan(root, ctx);

    let a = fx.explain(r"PF\FooApp");
    assert_heur(&a, "registry-install-dir", Tier::Review);
    assert!(a.owner.as_deref().unwrap().contains("Foo App") && a.owner.as_deref().unwrap().contains("Foo Inc."), "{:?}", a.owner);
    assert!(a.notes.iter().any(|n| n.contains("个月没动过")), "应有年龄提示：{:?}", a.notes);

    let b = fx.explain(r"Local\Fooware");
    assert_heur(&b, "registry-appdata", Tier::Review);
    assert!(b.what.contains("Fooware Studio"), "{}", b.what);

    let c = fx.explain(r"Roaming\Contoso");
    assert_heur(&c, "registry-appdata", Tier::Review);
    assert!(c.owner.as_deref().unwrap().contains("Contoso"), "发布者匹配：{:?}", c.owner);

    // 厂商文件夹下的具体软件：精确名字优先
    let n = fx.explain(r"PD\NVIDIA Corporation\NVIDIA App\UpdateFramework");
    assert!(n.what.contains("NVIDIA App 11.0.8.299"), "应归到 NVIDIA App 而不是随便一个 NVIDIA 软件：{}", n.what);
    // 对不上具体软件时，只说到厂商，不瞎猜
    let o = fx.explain(r"PD\NVIDIA Corporation\Other Thing");
    assert!(!o.what.contains("Nsight") && o.what.contains("「NVIDIA Corporation」家"), "应只说到厂商：{}", o.what);
    assert!(o.owner.as_deref().unwrap_or("").contains("NVIDIA Corporation"), "{:?}", o.owner);
    assert!(o.clues.iter().any(|c| c.contains("NVIDIA App 11.0.8.299")), "线索里应列出这家装了哪些：{:?}", o.clues);
}

#[test]
fn name_normalization_is_simple_and_symmetric() {
    use diskwhy_core::explain::heuristics::names_match;
    use diskwhy_core::explain::registry::normalize_name;
    assert!(names_match(&normalize_name("Docker"), &normalize_name("Docker Desktop")));
    assert!(names_match(&normalize_name("Google"), &normalize_name("Google LLC")));
    assert!(!names_match(&normalize_name("Go"), &normalize_name("Google Chrome")), "太短的名字不做前缀匹配");
    assert!(!names_match(&normalize_name("Zoom"), &normalize_name("Docker Desktop")));
}

#[test]
fn listing_attaches_explanation_to_every_child() {
    let root = temp_root("list");
    put(&root, r"a\Cache\Cache_Data\index", b"i");
    put(&root, r"b\x.log", &[0u8; 10_000]);
    put(&root, r"c\y.unknownext", &[0u8; 10_000]);
    put(&root, "loose.bin2", &[0u8; 10_000]);
    let ctx = empty_rules_ctx(&root, Vec::new());
    let fx = Fixture::scan(root, ctx);
    let l = listing(&fx.result, fx.root.to_str().unwrap(), 1000, Some(&fx.ctx)).unwrap();
    assert!(l.node.explanation.is_some());
    assert_eq!(l.children.len(), 4);
    assert!(l.children.iter().all(|c| c.explanation.is_some()));
    assert!(l.children.iter().all(|c| c.explanation.as_ref().unwrap().tier != Tier::Safe), "空规则库下不应出现 safe");
    let l2 = listing(&fx.result, fx.root.to_str().unwrap(), 1000, None).unwrap();
    assert!(l2.children.iter().all(|c| c.explanation.is_none()));
}

// ───────────────────────── 4. LLM ─────────────────────────

const SECRET: &str = "TOP-SECRET-FILE-CONTENT-7f3a9c";

#[test]
fn llm_request_body_has_no_file_content_and_redacted_path() {
    // 放在真实用户目录下（%TEMP% 在 C:\Users\<名字>\ 下），文件里写上机密内容。
    let root = temp_root("llm");
    put(&root, r"mystery\notes.txt", SECRET.as_bytes());
    put(&root, r"mystery\data\blob.bin2", format!("{SECRET}{SECRET}").as_bytes());
    put(&root, r"mystery\readme.md", SECRET.as_bytes());
    let ctx = empty_rules_ctx(&root, Vec::new());
    let fx = Fixture::scan(root.clone(), ctx);
    let (chain, node) = view::find_node(&fx.result.tree, root.join("mystery").to_str().unwrap()).unwrap();
    let path = root.join("mystery").to_string_lossy().to_string();
    let t = Target { node, path: &path, parent: chain.last().copied(), ancestors: &chain };
    let input = llm::build_input(&t);
    let body = llm::build_request_body("http://127.0.0.1:11434/v1", "test-model", &input).to_string();

    assert!(!body.contains(SECRET), "请求体里出现了文件内容！");
    assert!(body.contains("notes.txt") && body.contains("data\\\\"), "应包含子项名：{body}");
    let user = std::env::var("USERNAME").unwrap_or_default();
    if !user.is_empty() && root.to_string_lossy().to_lowercase().contains(&format!(r"\users\{}", user.to_lowercase())) {
        assert!(!body.to_lowercase().contains(&format!(r"users\\\\{}", user.to_lowercase())), "用户目录没有脱敏：{body}");
        assert!(input.parent.starts_with('~'), "父路径应以 ~ 开头：{}", input.parent);
    }
    // 只发了这些字段
    let v: serde_json::Value = serde_json::to_value(&input).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 9, "{keys:?}");
}

#[test]
fn redact_path_rules() {
    assert_eq!(llm::redact_path(r"C:\Users\king\AppData\Local"), r"~\AppData\Local");
    assert_eq!(llm::redact_path(r"c:\USERS\Bob"), "~");
    assert_eq!(llm::redact_path(r"D:\Users\x\y"), r"~\y");
    assert_eq!(llm::redact_path(r"C:\Windows\System32"), r"C:\Windows\System32");
    assert_eq!(llm::redact_path(r"C:\"), r"C:\");
}

#[test]
fn llm_parse_and_always_review() {
    let a = llm::parse_answer("好的：\n```json\n{\"what\":\"某游戏的录像\",\"owner\":\"某游戏\",\"why\":\"自动录制\",\"lose\":\"录像没了\",\"confidence\":1.7}\n```").unwrap();
    assert_eq!(a.what, "某游戏的录像");
    assert!(a.confidence <= 1.0);
    let e = llm::to_explanation(a, "m", 123);
    assert_eq!(e.tier, Tier::Review);
    assert_eq!(e.source, Source::Llm { model: "m".into() });
    assert!(e.confidence <= 0.9);
    assert!(llm::parse_answer("我不知道").is_err());
    assert!(llm::parse_answer("{\"what\":\"\"}").is_err());
    assert_eq!(llm::chat_url("http://localhost:11434/v1/"), "http://localhost:11434/v1/chat/completions");
    assert_eq!(llm::chat_url("https://x/v1/chat/completions"), "https://x/v1/chat/completions");
}

#[test]
fn llm_settings_roundtrip_and_default_off() {
    let dir = temp_root("settings");
    let p = dir.join("settings.json");
    assert_eq!(LlmSettings::load_from(&p), LlmSettings::default());
    assert!(!LlmSettings::default().enabled, "默认必须关闭");
    let s = LlmSettings { enabled: true, base_url: "http://localhost:11434/v1".into(), model: "qwen2.5".into(), api_key: String::new() };
    s.save_to(&p).unwrap();
    assert_eq!(LlmSettings::load_from(&p), s);
    std::fs::write(&p, "not json").unwrap();
    assert_eq!(LlmSettings::load_from(&p), LlmSettings::default(), "坏文件退回默认");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn llm_network_failure_is_graceful() {
    // 端口 9（discard）通常没人听：应返回 Err 而不是 panic。
    let s = LlmSettings { enabled: true, base_url: "http://127.0.0.1:9/v1".into(), model: "m".into(), api_key: String::new() };
    let r = llm::test_connection(&s);
    assert!(r.is_err(), "{r:?}");
    let off = LlmSettings::default();
    let ctx = ctx_with(fake_env(), RuleSet::default(), vec![]);
    let t = tree(&[(r"C:\x\y", 1)]);
    let (chain, node) = view::find_node(&t, r"C:\x").unwrap();
    let tg = Target { node, path: r"C:\x", parent: chain.last().copied(), ancestors: &chain };
    let mut ctx = ctx;
    ctx.settings = off;
    let e = llm::ask(&tg, &ctx).unwrap_err();
    assert!(e.contains("设置"), "{e}");
}

/// 本地假 OpenAI 服务：收一个请求，记下原始字节，回一个固定答案。
fn fake_server(reply_json: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
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
        let body = format!("{{\"choices\":[{{\"message\":{{\"role\":\"assistant\",\"content\":{}}}}}]}}", serde_json::to_string(reply_json).unwrap());
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        sock.write_all(resp.as_bytes()).unwrap();
        String::from_utf8_lossy(&buf).to_string()
    });
    (format!("http://127.0.0.1:{port}/v1"), h)
}

#[test]
fn llm_end_to_end_against_local_fake_server() {
    let root = temp_root("llm-e2e");
    put(&root, r"Local\ACLOS-like\wonderfulVideos\a.mp4", SECRET.as_bytes());
    put(&root, r"Local\ACLOS-like\wonderfulVideos\b.jpeg", SECRET.as_bytes());
    let (url, h) = fake_server(r#"{"what":"像是某个游戏平台自动录下的精彩时刻视频","owner":"某游戏平台","why":"自动录制","lose":"删了这些录像就没了","confidence":0.4}"#);
    let mut ctx = empty_rules_ctx(&root, Vec::new());
    ctx.settings = LlmSettings { enabled: true, base_url: url, model: "fake-model".into(), api_key: "sk-test".into() };
    let fx = Fixture::scan(root.clone(), ctx);
    let p = root.join(r"Local\ACLOS-like").to_string_lossy().to_string();
    let (chain, node) = view::find_node(&fx.result.tree, &p).unwrap();
    let t = Target { node, path: &p, parent: chain.last().copied(), ancestors: &chain };
    let e = llm::ask(&t, &fx.ctx).expect("假服务器应返回成功");
    let raw = h.join().unwrap();

    assert_eq!(e.tier, Tier::Review);
    assert_eq!(e.source, Source::Llm { model: "fake-model".into() });
    assert!(e.what.contains("精彩时刻"));
    assert!(raw.starts_with("POST /v1/chat/completions "), "{}", &raw[..raw.len().min(200)]);
    assert!(raw.contains("Bearer sk-test"));
    assert!(!raw.contains(SECRET), "真实发出的请求里出现了文件内容！");
    assert!(raw.contains("wonderfulVideos"), "应发送子项名");
    let user = std::env::var("USERNAME").unwrap_or_default().to_lowercase();
    if !user.is_empty() && p.to_lowercase().contains(&format!(r"\users\{user}\")) {
        assert!(!raw.to_lowercase().contains(&format!(r"users\\\\{user}")), "真实请求里用户名没脱敏");
    }
    // Kimi K2.6+ 固定 temperature/top_p，传了就 400：真实请求里不能有任何采样参数。
    for k in ["temperature", "top_p", "max_tokens"] {
        assert!(!raw.contains(&format!("\"{k}\"")), "请求里不该带 {k}：{raw}");
    }
}

// ───────────────────────── 阶段 J：多家服务商预设（DeepSeek / 通义 / Kimi / OpenAI / Ollama） ─────────────────────────

#[test]
fn llm_request_body_sends_no_sampling_params() {
    let input = llm::LlmInput {
        name: "x".into(),
        parent: "~".into(),
        kind: "文件夹".into(),
        total_size: "1 GB".into(),
        files: 1,
        dirs: 0,
        top_children: vec![],
        extensions: vec![],
        modified: "2026-01-01".into(),
    };
    let b = llm::build_request_body("https://api.moonshot.cn/v1", "kimi-k2.6", &input);
    let o = b.as_object().unwrap();
    for k in ["temperature", "top_p", "max_tokens", "n", "presence_penalty", "frequency_penalty"] {
        assert!(!o.contains_key(k), "不该发 {k}：{b}");
    }
    assert_eq!(o["model"], "kimi-k2.6");
    assert_eq!(o["stream"], false);
}

#[test]
fn llm_preset_urls_build_official_endpoints() {
    // 各家官方文档里的 chat/completions 完整地址
    assert_eq!(llm::chat_url("https://api.deepseek.com"), "https://api.deepseek.com/chat/completions");
    assert_eq!(llm::chat_url("https://dashscope.aliyuncs.com/compatible-mode/v1"), "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions");
    assert_eq!(llm::chat_url("https://api.moonshot.cn/v1"), "https://api.moonshot.cn/v1/chat/completions");
    assert_eq!(llm::chat_url("https://api.openai.com/v1"), "https://api.openai.com/v1/chat/completions");
    assert_eq!(llm::chat_url(" http://localhost:11434/v1/ "), "http://localhost:11434/v1/chat/completions");
}

#[test]
fn llm_parse_tolerates_fences_reasoning_and_think() {
    let ok = |s: &str| llm::parse_answer(s).unwrap_or_else(|e| panic!("应能解析：{e}\n{s}"));
    // 1) 代码块，没写 json 标签
    let a = ok("```\n{\"what\":\"缓存\",\"owner\":\"\",\"why\":\"w\",\"lose\":\"l\",\"confidence\":0.5}\n```");
    assert_eq!(a.what, "缓存");
    // 2) 先推理（推理里带花括号），再给 JSON，后面还补一句（也带花括号）
    let a = ok("我来分析一下。名字里有 {cache} 字样，结构像 {a:b}。\n结论：\n```json\n{\"what\":\"浏览器缓存\",\"owner\":\"Edge\",\"why\":\"上网自动存\",\"lose\":\"网页第一次打开慢一点\",\"confidence\":0.8}\n```\n以上仅供参考 {完}");
    assert_eq!(a.what, "浏览器缓存");
    assert_eq!(a.owner, "Edge");
    assert!((a.confidence - 0.8).abs() < 1e-6);
    // 3) <think> 思考段（Ollama 的 qwen3 / deepseek-r1），思考里有一个“示例” JSON
    let a = ok("<think>格式应该是 {\"what\":\"示例\"}，嗯……</think>\n{\"what\":\"游戏录像\",\"why\":\"自动录\",\"lose\":\"录像没了\",\"confidence\":\"0.4\"}");
    assert_eq!(a.what, "游戏录像");
    assert!((a.confidence - 0.4).abs() < 1e-6, "字符串 confidence 也认");
    // 4) 推理里先举了个例子 JSON，正式答案在后：取最后一个
    let a = ok("比如 {\"what\":\"举例\"} 这种。我的回答：{\"what\":\"正式答案\",\"confidence\":70}");
    assert_eq!(a.what, "正式答案");
    assert!((a.confidence - 0.7).abs() < 1e-6, "0–100 的百分数换成 0–1");
    // 5) 字符串里带花括号、owner 是 null、confidence 是 "60%"
    let a = ok("{\"what\":\"模板 {x} 的输出\",\"owner\":null,\"why\":\"}\",\"lose\":\"不确定\",\"confidence\":\"60%\"}");
    assert_eq!(a.what, "模板 {x} 的输出");
    assert_eq!(a.owner, "");
    assert_eq!(a.why, "}");
    assert!((a.confidence - 0.6).abs() < 1e-6);
    // 6) 坏的
    assert!(llm::parse_answer("我不知道").is_err());
    assert!(llm::parse_answer("{\"what\":\"\"}").is_err());
    assert!(llm::parse_answer("<think>{\"what\":\"只在思考里\"}").is_err(), "没闭合的思考段整段丢掉");
    assert!(llm::parse_answer("{\"what\": \"截断了").is_err());
    let a = ok("{\"what\":\"x\",\"confidence\":-3}");
    assert_eq!(a.confidence, 0.0);
}

#[test]
fn llm_retry_drops_only_params_the_server_named() {
    let b = serde_json::json!({"model":"m","temperature":0.2,"top_p":0.9,"messages":[]});
    // Kimi 的真实报错文案（见 MoonshotAI/kimi-cli#1924）
    let kimi = r#"{"error":{"message":"invalid temperature: only 0.6 is allowed for this model","type":"invalid_request_error"}}"#;
    assert_eq!(llm::params_to_drop(&b, 400, kimi), vec!["temperature"]);
    // OpenAI 推理模型的报错
    let oai = r#"{"error":{"message":"Unsupported value: 'temperature' does not support 0.2 with this model. Only the default (1) value is supported.","param":"temperature"}}"#;
    assert_eq!(llm::params_to_drop(&b, 400, oai), vec!["temperature"]);
    // 不是 400、没点名、或请求里本来就没有这个参数：不重试
    assert!(llm::params_to_drop(&b, 401, kimi).is_empty());
    assert!(llm::params_to_drop(&b, 400, "model not found").is_empty());
    let no_t = serde_json::json!({"model":"m","messages":[]});
    assert!(llm::params_to_drop(&no_t, 400, kimi).is_empty());
}

#[test]
fn llm_test_connection_sends_no_max_tokens_and_strips_think() {
    let (url, h) = fake_server("<think>用户要我说你好</think>你好");
    let s = LlmSettings { enabled: true, base_url: url, model: "gpt-5.4-mini".into(), api_key: "sk-test".into() };
    let r = llm::test_connection(&s).expect("假服务器应返回成功");
    let raw = h.join().unwrap();
    assert!(r.contains("你好") && !r.contains("think"), "{r}");
    for k in ["temperature", "max_tokens"] {
        assert!(!raw.contains(&format!("\"{k}\"")), "测试连接不该带 {k}：{raw}");
    }
}

// ───────────────────────── 阶段 C：管理员才看得到的目录 + 阶段 B 遗留未知项 ─────────────────────────

/// 以管理员身份扫描时会多出来的系统目录，全部必须是 keep（没有删除按钮）。
#[test]
fn admin_only_system_dirs_are_all_keep() {
    let ctx = embedded_ctx();
    let t = tree(&[
        (r"C:\System Volume Information\{3808876b-c176-4e48-b7ae-04046e6cc752}", 4_000_000_000),
        (r"C:\System Volume Information\Syscache.hve", 10_000_000),
        (r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.0_x64__8wekyb3d8bbwe\wt.exe", 5_000_000),
        (r"C:\Windows\WinSxS\amd64_microsoft-windows-foo_31bf3856ad364e35_10.0.1_none_x\foo.dll", 1_000_000),
        (r"C:\Windows\System32\config\SOFTWARE", 100_000_000),
        (r"C:\Windows\System32\sru\SRUDB.dat", 50_000_000),
        (r"C:\Windows\ServiceProfiles\NetworkService\AppData\Local\x.dat", 1_000),
        (r"C:\Windows\CSC\v2.0.6\x", 1_000),
        (r"C:\Windows\Prefetch\X.pf", 1_000),
        (r"C:\ProgramData\Microsoft\Windows\AppRepository\StateRepository-Machine.srd", 30_000_000),
        (r"C:\ProgramData\Microsoft\Windows\Hyper-V\data.vmcx", 1_000),
        (r"C:\ProgramData\Packages\Microsoft.Foo_8wekyb3d8bbwe\x", 1_000),
        (r"C:\ProgramData\Microsoft\Windows Defender Advanced Threat Protection\Cache\x", 1_000),
        (r"C:\ProgramData\Microsoft\Crypto\SystemKeys\k", 1_000),
        (r"C:\ProgramData\Microsoft\Diagnosis\ETLLogs\x.etl", 1_000),
        (r"C:\ProgramData\Microsoft\Search\Data\Applications\Windows\Windows.edb", 1_000),
        (r"C:\ProgramData\Microsoft\Windows Defender\Scans\x", 1_000),
    ]);
    for p in [
        r"C:\System Volume Information",
        r"C:\System Volume Information\{3808876b-c176-4e48-b7ae-04046e6cc752}",
        r"C:\Program Files\WindowsApps",
        r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.0_x64__8wekyb3d8bbwe",
        r"C:\Windows\WinSxS",
        r"C:\Windows\WinSxS\amd64_microsoft-windows-foo_31bf3856ad364e35_10.0.1_none_x",
        r"C:\Windows\System32\config",
        r"C:\Windows\System32\config\SOFTWARE",
        r"C:\Windows\System32\sru",
        r"C:\Windows\ServiceProfiles",
        r"C:\Windows\ServiceProfiles\NetworkService",
        r"C:\Windows\CSC",
        r"C:\Windows\Prefetch",
        r"C:\ProgramData\Microsoft\Windows\AppRepository",
        r"C:\ProgramData\Microsoft\Windows\Hyper-V",
        r"C:\ProgramData\Packages",
        r"C:\ProgramData\Packages\Microsoft.Foo_8wekyb3d8bbwe",
        r"C:\ProgramData\Microsoft\Windows Defender Advanced Threat Protection",
        r"C:\ProgramData\Microsoft\Crypto",
        r"C:\ProgramData\Microsoft\Diagnosis",
        r"C:\ProgramData\Microsoft\Search\Data",
        r"C:\ProgramData\Microsoft\Windows Defender",
    ] {
        let e = ex(&t, p, &ctx);
        assert_eq!(e.tier, Tier::Keep, "{p} 应该是 keep，实际 {:?}（{:?}）", e.tier, e.source);
        assert!(!e.how.contains(&How::Delete), "{p} keep 档不能给删除");
        assert!(rule_id(&e).is_some(), "{p} 应由规则库说明：{:?}", e.source);
    }
    assert_eq!(rule_id(&ex(&t, r"C:\Windows\System32\config", &ctx)), Some("registry-hives"));
    assert_eq!(rule_id(&ex(&t, r"C:\ProgramData\Microsoft\Windows\AppRepository", &ctx)), Some("appx-repository"));
    assert_eq!(rule_id(&ex(&t, r"C:\System Volume Information", &ctx)), Some("system-volume-information"));
}

/// 阶段 B 的 7 个未知项现在都有规则；查不到官方说明的都是 review 而且写了“不确定 / 线索”。
#[test]
fn former_unknowns_now_have_rules() {
    let ctx = embedded_ctx();
    let t = tree(&[
        (r"C:\Users\bob\AppData\Local\OpenAI\Codex\bin\codex.exe", 200_000_000),
        (r"C:\Users\bob\.cache\codex-runtimes\codex-primary-runtime\runtime.json", 1_000),
        (r"C:\Users\bob\.vscode\argv.json", 1_000),
        (r"C:\Users\bob\.vscode\extensions\ms-python.python-1\x.js", 1_000_000),
        (r"C:\Users\bob\.antigravity\argv.json", 1_000),
        (r"C:\Users\bob\AppData\Roaming\ACLOS\TcVersions\a.zip", 1_000_000),
        (r"C:\Users\bob\AppData\ACLOS\aclos-highlight\v.mp4", 1_000_000),
        (r"C:\Users\bob\AppData\Local\kimi-desktop-updater\installer.exe", 400_000_000),
        (r"C:\Users\bob\AppData\Local\someapp-updater\notes.txt", 1_000),
        (r"C:\Users\bob\AppData\Local\Pandoc\pandoc.exe", 200_000_000),
    ]);
    let cases = [
        (r"C:\Users\bob\AppData\Local\OpenAI\Codex", "codex-desktop-local", Tier::Review),
        (r"C:\Users\bob\.cache\codex-runtimes", "codex-runtimes-cache", Tier::Review),
        (r"C:\Users\bob\.vscode", "vscode-home-dir", Tier::Review),
        (r"C:\Users\bob\.vscode\extensions", "vscode-family-extensions", Tier::Review),
        (r"C:\Users\bob\.antigravity", "antigravity-home-dir", Tier::Review),
        (r"C:\Users\bob\AppData\Roaming\ACLOS", "aclos-game-recorder", Tier::Review),
        (r"C:\Users\bob\AppData\ACLOS", "aclos-game-recorder", Tier::Review),
        (r"C:\Users\bob\AppData\Local\kimi-desktop-updater", "electron-updater-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Pandoc", "pandoc-local", Tier::Review),
    ];
    for (p, id, tier) in cases {
        let e = ex(&t, p, &ctx);
        assert_eq!(rule_id(&e), Some(id), "{p}：{:?}", e.source);
        assert_eq!(e.tier, tier, "{p}");
    }
    // electron-updater 规则靠 when_child 认：没有 installer.exe / pending 的 *-updater 不命中
    let other = ex(&t, r"C:\Users\bob\AppData\Local\someapp-updater", &ctx);
    assert_ne!(rule_id(&other), Some("electron-updater-cache"));
    // 名字推断出具体软件
    let k = ex(&t, r"C:\Users\bob\AppData\Local\kimi-desktop-updater", &ctx);
    assert!(k.owner.as_deref().unwrap_or("").contains("electron-builder"), "{:?}", k.owner);
    // 查不到官方说明的，说明里要写明
    for p in [r"C:\Users\bob\.cache\codex-runtimes", r"C:\Users\bob\AppData\Roaming\ACLOS"] {
        let e = ex(&t, p, &ctx);
        assert!(e.why.contains("没有找到") || e.lose.contains("不确定"), "{p} 应写明没核实：{}", e.why);
    }
}

// ───────────────────────── 2026-09-24 审计：商店应用重定向 + 用户自己放的大东西 ─────────────────────────

/// 商店版应用的 LocalCache\Local|Roaming 按真实 AppData 路径说明，不再笼统说成“Claude 的数据”。
#[test]
fn msix_localcache_is_explained_by_real_appdata_path() {
    let ctx = embedded_ctx();
    let pkg = r"C:\Users\bob\AppData\Local\Packages\Claude_abc123";
    let t = tree(&[
        (&format!(r"{pkg}\LocalCache\Local\mybuild\target\.rustc_info.json"), 1_000),
        (&format!(r"{pkg}\LocalCache\Local\mybuild\target\CACHEDIR.TAG"), 1_000),
        (&format!(r"{pkg}\LocalCache\Local\mybuild\target\debug\deps\a.rlib"), 5_000_000_000),
        (&format!(r"{pkg}\LocalCache\Local\pip\cache\http\x"), 300_000_000),
        (&format!(r"{pkg}\LocalCache\Local\unknownthing\a.dat"), 10_000_000),
        (&format!(r"{pkg}\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle\rootfs.vhdx"), 8_000_000_000),
        (&format!(r"{pkg}\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle\sessiondata.vhdx"), 500_000_000),
        (&format!(r"{pkg}\LocalState\x"), 1_000),
    ]);

    let m = ctx.msix_redirect(&format!(r"{pkg}\LocalCache\Local\pip\cache")).expect("应识别为重定向");
    assert_eq!(m.mapped, r"C:\Users\bob\AppData\Local\pip\cache");
    assert_eq!(m.app_name(), "Claude");
    assert!(ctx.msix_redirect(&format!(r"{pkg}\LocalCache\Local")).is_none(), "LocalCache\\Local 本身不算");
    assert_eq!(
        ctx.msix_redirect(&format!(r"{pkg}\LocalCache\Roaming\Claude")).unwrap().mapped,
        r"C:\Users\bob\AppData\Roaming\Claude"
    );

    assert_eq!(rule_id(&ex(&t, pkg, &ctx)), Some("claude-desktop-package"));
    assert_eq!(rule_id(&ex(&t, &format!(r"{pkg}\LocalCache\Local"), &ctx)), Some("msix-localcache"));
    // 真实路径 %LOCALAPPDATA%\pip\cache 是 safe 规则
    let pip = ex(&t, &format!(r"{pkg}\LocalCache\Local\pip\cache"), &ctx);
    assert_eq!((rule_id(&pip), pip.tier), (Some("pip-cache"), Tier::Safe));
    assert!(pip.notes.iter().any(|n| n.contains("同一份") && n.contains(r"AppData\Local\pip\cache")), "{:?}", pip.notes);
    // 编译产物：自己认出来 + 子文件夹沿用
    assert_eq!(heur_name(&ex(&t, &format!(r"{pkg}\LocalCache\Local\mybuild\target"), &ctx)), Some("rust-target"));
    assert_eq!(heur_name(&ex(&t, &format!(r"{pkg}\LocalCache\Local\mybuild\target\debug\deps"), &ctx)), Some("rust-target"));
    assert_eq!(heur_name(&ex(&t, &format!(r"{pkg}\LocalCache\Local\mybuild"), &ctx)), Some("dominant-child"));
    // 认不出来的不再冒充“Claude 桌面版的数据”，线索里写明真实位置
    let unk = ex(&t, &format!(r"{pkg}\LocalCache\Local\unknownthing"), &ctx);
    assert!(unk.is_unknown(), "{:?}", unk.source);
    assert!(unk.clues.iter().any(|c| c.contains(r"AppData\Local\unknownthing")), "{:?}", unk.clues);
    // Claude 自己的数据照旧
    assert_eq!(rule_id(&ex(&t, &format!(r"{pkg}\LocalCache\Roaming\Claude"), &ctx)), Some("claude-desktop-data"));
    assert_eq!(rule_id(&ex(&t, &format!(r"{pkg}\LocalCache\Roaming\Claude\vm_bundles"), &ctx)), Some("claude-desktop-vm"));
    let sd = ex(&t, &format!(r"{pkg}\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle\sessiondata.vhdx"), &ctx);
    assert_eq!((rule_id(&sd), sd.tier), (Some("claude-cowork-sessiondata"), Tier::Keep));
    // 其他子文件夹仍按商店应用数据说明
    assert_eq!(rule_id(&ex(&t, &format!(r"{pkg}\LocalState"), &ctx)), Some("store-app-package"));

    // 汇总也按真实路径归档：pip 缓存算 safe，不再算进 Claude 的 review
    let s = tier_summary(&t, &ctx);
    assert!(s.top_safe.iter().any(|r| r.id == "pip-cache" && r.bytes == 300_000_000), "{:?}", s.top_safe);
    assert!(!s.top_review.iter().any(|r| r.id == "claude-desktop-package" || r.id == "store-app-package" && r.bytes > 1_000_000_000));
}

/// 任何盘上用户自己放的大东西：聊天记录、游戏、虚拟机、conda、SDK、编译产物。启发式结果都不是 safe。
#[test]
fn user_big_items_on_any_drive() {
    let ctx = embedded_ctx();
    let t = tree(&[
        // 微信聊天记录（改到别的文件夹）
        (r"C:\wx\xwechat_files\wxid_a_1\msg\file\2026-06\a.pdf", 3_000_000_000),
        (r"C:\wx\xwechat_files\wxid_a_1\cache\img\x", 100_000_000),
        (r"C:\old\Tencent Files\12345\FileRecv\a.zip", 100_000_000),
        // 腾讯游戏：游戏本体 + 自动录像
        (r"C:\vg\Tencent Games\VALORANT\live\ShooterGame\Content\Paks\a.pak", 3_000_000_000),
        (r"C:\vg\Tencent Games\wonderfulVideos123\v1\a.mp4", 9_000_000_000),
        // Steam 库
        (r"C:\steam\steamapps\common\Some Game\game.exe", 2_000_000_000),
        (r"C:\steam\steamapps\appmanifest_1.acf", 1_000),
        (r"C:\steam\steamapps\shadercache\1\x", 10_000_000),
        (r"C:\steam\steamui\x", 1_000_000),
        // 虚拟机
        (r"C:\vms\win11\win11.vmx", 1_000),
        (r"C:\vms\win11\win11.vmdk", 40_000_000_000),
        (r"C:\vms\ubuntu\ubuntu.vbox", 1_000),
        (r"C:\vms\ubuntu\ubuntu.vdi", 20_000_000_000),
        // conda
        (r"C:\conda\envs\e1\conda-meta\a.json", 1_000),
        (r"C:\conda\envs\e1\Lib\site-packages\torch\lib\x.dll", 4_000_000_000),
        (r"C:\conda\envs\e2\conda-meta\b.json", 1_000),
        (r"C:\conda\envs\e2\python.exe", 100_000),
        (r"C:\conda\pkgs\urls.txt", 1_000),
        (r"C:\conda\pkgs\numpy-2.0-py312_0\info\index.json", 1_000),
        (r"C:\conda\pkgs\numpy-2.0-py312_0\Lib\x.pyd", 50_000_000),
        // 安卓 / Gradle / Flutter / pub
        (r"C:\sdk\android\platform-tools\adb.exe", 10_000_000),
        (r"C:\sdk\android\build-tools\35.0.0\aapt2.exe", 10_000_000),
        (r"C:\sdk\android\platforms\android-35\android.jar", 50_000_000),
        (r"C:\sdk\android\system-images\android-35\google_apis\x86_64\system.img", 3_000_000_000),
        (r"C:\sdk\gradle\caches\9.3.1\transforms\x", 3_000_000_000),
        (r"C:\sdk\gradle\wrapper\dists\gradle-9\x.zip", 700_000_000),
        (r"C:\sdk\flutter\bin\flutter.bat", 1_000),
        (r"C:\sdk\flutter\bin\cache\dart-sdk\bin\dart.exe", 2_000_000_000),
        (r"C:\sdk\flutter\packages\flutter\pubspec.yaml", 1_000),
        (r"C:\sdk\pub_cache\hosted\pub.dev\x", 200_000_000),
        (r"C:\sdk\pub_cache\hosted-hashes\pub.dev\x", 1_000),
        // Rust 编译产物（CARGO_TARGET_DIR 改了名字）
        (r"C:\build\out\.rustc_info.json", 1_000),
        (r"C:\build\out\CACHEDIR.TAG", 1_000),
        (r"C:\build\out\debug\deps\a.rlib", 2_000_000_000),
        // Python 库里的 pyproject.toml 不是“你的项目”
        (r"C:\py\Lib\site-packages\pkga\pyproject.toml", 1_000),
        (r"C:\py\Lib\site-packages\pkgb\pyproject.toml", 1_000),
        (r"C:\py\python.exe", 100_000),
        // 真的代码项目
        (r"C:\code\app1\package.json", 1_000),
        (r"C:\code\app2\Cargo.toml", 1_000),
        (r"C:\code\app2\.git\HEAD", 1_000),
    ]);
    let cases: &[(&str, &str, Tier)] = &[
        (r"C:\wx", "chat-files-container", Tier::Keep),
        (r"C:\wx\xwechat_files", "chat-files", Tier::Keep),
        (r"C:\wx\xwechat_files\wxid_a_1\msg\file\2026-06", "chat-files", Tier::Keep),
        (r"C:\wx\xwechat_files\wxid_a_1\cache", "chat-files-cache", Tier::Review),
        (r"C:\old\Tencent Files\12345", "chat-files", Tier::Keep),
        (r"C:\vg\Tencent Games", "game-tencent-root", Tier::Keep),
        (r"C:\vg", "game-tencent-root", Tier::Keep),
        (r"C:\vg\Tencent Games\VALORANT\live\ShooterGame\Content\Paks", "game-tencent", Tier::Keep),
        (r"C:\vg\Tencent Games\wonderfulVideos123", "game-highlights", Tier::Review),
        (r"C:\steam", "steam-root", Tier::Keep),
        (r"C:\steam\steamapps", "steam-library", Tier::Keep),
        (r"C:\steam\steamapps\common\Some Game", "steam-game", Tier::Keep),
        (r"C:\steam\steamapps\shadercache", "steam-cache", Tier::Review),
        (r"C:\vms\win11", "vm-vmware", Tier::Keep),
        (r"C:\vms\ubuntu", "vm-virtualbox", Tier::Keep),
        (r"C:\conda", "conda-root", Tier::Keep),
        (r"C:\conda\envs", "conda-envs", Tier::Keep),
        (r"C:\conda\envs\e1", "conda-env", Tier::Keep),
        (r"C:\conda\envs\e1\Lib\site-packages\torch", "conda-env", Tier::Keep),
        (r"C:\conda\pkgs", "conda-pkgs", Tier::Review),
        (r"C:\sdk", "dev-sdk-container", Tier::Review),
        (r"C:\sdk\android", "android-sdk", Tier::Review),
        (r"C:\sdk\android\system-images", "android-sdk-part", Tier::Review),
        (r"C:\sdk\gradle", "gradle-home", Tier::Review),
        (r"C:\sdk\gradle\caches", "gradle-cache", Tier::Review),
        (r"C:\sdk\flutter", "flutter-sdk", Tier::Review),
        (r"C:\sdk\flutter\bin\cache", "flutter-cache", Tier::Review),
        (r"C:\sdk\pub_cache", "pub-cache", Tier::Review),
        (r"C:\build\out", "rust-target", Tier::Review),
        (r"C:\build\out\debug\deps", "rust-target", Tier::Review),
        (r"C:\py", "python-runtime", Tier::Review),
        (r"C:\code", "user-projects", Tier::Keep),
        (r"C:\code\app2", "user-projects", Tier::Keep),
    ];
    for (p, name, tier) in cases {
        let e = ex(&t, p, &ctx);
        assert_eq!(heur_name(&e), Some(*name), "{p}：{:?} / {}", e.source, e.what);
        assert_eq!(e.tier, *tier, "{p}");
        assert!(e.tier != Tier::Safe);
        if e.tier == Tier::Keep {
            assert!(!e.how.contains(&How::Delete), "{p} keep 不能给删除");
        }
    }
    // conda 环境集合列出各环境大小
    let envs = ex(&t, r"C:\conda\envs", &ctx);
    assert!(envs.clues.iter().any(|c| c.contains("e1") && c.contains("GB")), "{:?}", envs.clues);
    // 虚拟机给迁移
    assert!(ex(&t, r"C:\vms\win11", &ctx).how.contains(&How::Migrate));
    // 腾讯游戏文件夹要点出录像占了多少
    assert!(ex(&t, r"C:\vg\Tencent Games", &ctx).clues.iter().any(|c| c.contains("wonderfulVideos")));
    // site-packages 里的包不是用户项目
    let sp = ex(&t, r"C:\py\Lib\site-packages", &ctx);
    assert_ne!(heur_name(&sp), Some("user-projects"), "{:?}", sp.what);
    // 启发式的参考链接
    assert!(ex(&t, r"C:\conda\pkgs", &ctx).refs.iter().any(|r| r.contains("conda")));
}

// ───────────────────────── 阶段 F：后果类型 lose_kind + 清理候选 ─────────────────────────

use diskwhy_core::explain::candidates::{cleanup_candidates, short_name};
use diskwhy_core::explain::heuristics::heuristic_lose_kind;
use diskwhy_core::explain::{Category, LoseKind};

#[test]
fn every_review_rule_has_lose_kind() {
    let ctx = embedded_ctx();
    let mut n = 0;
    for r in &ctx.rules.rules {
        if r.tier == Tier::Review {
            n += 1;
            assert!(r.lose_kind.is_some(), "review 规则 {} 没有 lose_kind", r.id);
        }
    }
    // 72 条原有 review 规则 + 阶段 F 新增的 C:\tmp / C:\Temp / C:\AMD 三条；
    // “旧版本”规则（keep_latest）把 codex-runtimes-previous 从 review 改成了 safe（正在用的版本另外当 keep）
    // 缓存复查（2026-11）：wechat-radium、yarn-berry-cache 新增，chromium-generic-sw-cache 从 safe 降成 review
    assert_eq!(n, 77, "review 规则条数变了：{n}");
    // 抽查几条 README 里的例子
    let k = |id: &str| ctx.rules.by_id(id).and_then(|r| r.lose_kind);
    assert_eq!(k("recycle-bin"), Some(LoseKind::Irreversible));
    assert_eq!(k("maven-repo"), Some(LoseKind::Irreversible));
    assert_eq!(k("package-cache"), Some(LoseKind::Repair));
    assert_eq!(k("dot-cache-root"), Some(LoseKind::Unsure));
    assert_eq!(k("vscode-family-extensions"), Some(LoseKind::Reinstall));
    assert_eq!(k("wechat-appdata"), Some(LoseKind::Relogin));
    assert_eq!(k("hiberfil"), Some(LoseKind::Feature));
    assert_eq!(k("huggingface-hub"), Some(LoseKind::Redownload));
    assert_eq!(k("visual-studio-cache"), Some(LoseKind::Slower));
    assert_eq!(k("root-tmp-dir"), Some(LoseKind::Unsure));
    assert_eq!(k("root-temp-dir"), Some(LoseKind::Unsure));
    assert_eq!(k("amd-installer-files"), Some(LoseKind::Repair));
}

#[test]
fn lose_kind_validation() {
    let review = GOOD.replace("tier = \"safe\"", "tier = \"review\"");
    let errs = validate(&def(&review), &mut HashSet::new());
    assert!(errs.iter().any(|e| e.contains("lose_kind")), "review 没写 lose_kind 应该报错：{errs:?}");
    let ok = review.replace("refs = []", "refs = []\nlose_kind = \"slower\"\nlose_short = \"下次会慢一点\"");
    assert!(validate(&def(&ok), &mut HashSet::new()).is_empty());
    let bad = review.replace("refs = []", "refs = []\nlose_kind = \"maybe\"");
    let errs = validate(&def(&bad), &mut HashSet::new());
    assert!(errs.iter().any(|e| e.contains("maybe")), "不认识的 lose_kind 应该报错：{errs:?}");
    let bad_safe = GOOD.replace("refs = []", "refs = []\nlose_kind = \"Slower\"");
    assert!(!validate(&def(&bad_safe), &mut HashSet::new()).is_empty(), "safe 写了不认识的值也要报错");
    // safe / keep 可以不写
    assert!(validate(&def(GOOD), &mut HashSet::new()).is_empty());
    // 整个文件里有一条不认识的值：这条规则加载失败，错误信息带出规则 id
    let (rs, errs) = RuleSet::from_sources(&[("t", &bad)], &fake_env());
    assert!(rs.rules.is_empty() && errs.iter().any(|e| e.contains("`x`")));
    // 严重程度顺序：Ord 越小越坏
    assert!(LoseKind::Irreversible < LoseKind::Repair && LoseKind::Redownload < LoseKind::Slower);
    assert_eq!(LoseKind::ALL.iter().min(), Some(&LoseKind::Irreversible));
    for k in LoseKind::ALL {
        assert_eq!(LoseKind::parse(k.as_str()), Some(k));
    }
}

#[test]
fn explanations_carry_lose_kind() {
    let ctx = embedded_ctx();
    let t = tree(&[
        (r"C:\Users\bob\.m2\repository\org\a.jar", 100),
        (r"C:\Users\bob\AppData\Local\npm-cache\x", 10),
        (r"C:\mystery\x", 5),
        (r"C:\Windows\WinSxS\a", 7),
    ]);
    let maven = ex(&t, r"C:\Users\bob\.m2\repository", &ctx);
    assert_eq!(maven.lose_kind, Some(LoseKind::Irreversible));
    let json = serde_json::to_string(&maven).unwrap();
    assert!(json.contains("\"lose_kind\":\"irreversible\""), "应序列化成 snake_case：{json}");
    // 缓存复查：要联网才能回来的 safe 缓存标“要重下载”
    assert_eq!(ex(&t, r"C:\Users\bob\AppData\Local\npm-cache", &ctx).lose_kind, Some(LoseKind::Redownload));
    let unk = ex(&t, r"C:\mystery", &ctx);
    assert!(unk.is_unknown());
    assert_eq!(unk.lose_kind, Some(LoseKind::Unsure));
    // LLM 兜底永远是“需你确认”
    let a = llm::parse_answer(r#"{"what":"w","owner":"","why":"y","lose":"l","confidence":0.5}"#).unwrap();
    assert_eq!(llm::to_explanation(a, "m", 1).lose_kind, Some(LoseKind::Unsure));
}

#[test]
fn heuristic_review_results_always_have_lose_kind() {
    for name in [
        "chromium-cache", "python-venv", "python-env", "node-modules", "rust-target", "dotnet-build", "model-weights", "logs-dumps",
        "downloaded-installer", "temp-files", "media-files", "registry-install-dir", "registry-program-dir", "registry-appdata",
        "chat-files-cache", "game-highlights", "steam-cache", "steam-workshop", "python-runtime", "conda-pkgs", "android-sdk-part",
        "android-user-home", "android-avd", "gradle-home", "gradle-cache", "flutter-sdk", "flutter-cache", "pub-cache", "dev-sdk-container",
        "pnpm-store", "ollama-models-dir", "visual-studio-install", "app-install-elsewhere", "driver-packages", "dominant-child", "whatever-new",
    ] {
        assert!(heuristic_lose_kind(name, Tier::Review).is_some(), "{name} 没有后果类型");
    }
    assert_eq!(heuristic_lose_kind("rust-target", Tier::Review), Some(LoseKind::Slower));
    assert_eq!(heuristic_lose_kind("node-modules", Tier::Review), Some(LoseKind::Redownload));
    assert_eq!(heuristic_lose_kind("model-weights", Tier::Review), Some(LoseKind::Redownload));
    assert_eq!(heuristic_lose_kind("virtual-disk", Tier::Keep), Some(LoseKind::Unsure));
    assert_eq!(heuristic_lose_kind("whatever-new", Tier::Review), Some(LoseKind::Unsure));
}

#[test]
fn cleanup_candidates_group_by_rule_and_respect_guards() {
    let ctx = embedded_ctx();
    let t = tree(&[
        (r"C:\Users\bob\AppData\Local\npm-cache\_cacache\a", 100),
        (r"C:\Users\bob\AppData\Roaming\npm-cache\b", 20),
        (r"C:\Users\bob\.m2\repository\org\a.jar", 300),
        (r"C:\Windows.old\x", 500),
        (r"C:\Windows\Temp\t.tmp", 40),
        (r"C:\Windows\WinSxS\a", 1000),
        (r"C:\Users\bob\proj\Cargo.toml", 1),
        (r"C:\Users\bob\proj\target\debug\app.exe", 700),
        (r"C:\Users\bob\proj\target\.rustc_info.json", 1),
        (r"C:\Users\bob\web\package.json", 1),
        (r"C:\Users\bob\web\node_modules\left-pad\index.js", 60),
        (r"C:\tmp\mine.parquet", 90),
        (r"C:\mystery\x", 5),
    ]);
    // 模拟保护路径：C:\Windows 下的一律拦下
    let blocked = |p: &str| p.to_lowercase().starts_with(r"c:\windows\");
    let g = cleanup_candidates(&t, &ctx, &blocked);
    let by = |k: &str| g.iter().find(|x| x.key == k).unwrap_or_else(|| panic!("没有候选 {k}：{:?}", g.iter().map(|x| &x.key).collect::<Vec<_>>()));

    let npm = by("npm-cache");
    assert_eq!(npm.tier, Tier::Safe);
    assert_eq!(npm.paths.len(), 2, "同一条规则的两个位置合成一组");
    assert_eq!((npm.bytes, npm.deletable_bytes), (120, 120));
    assert_eq!(npm.rule_id.as_deref(), Some("npm-cache"));
    assert_eq!(npm.name, "npm 缓存");

    let maven = by("maven-repo");
    assert_eq!((maven.tier, maven.lose_kind, maven.deletable_bytes), (Tier::Review, Some(LoseKind::Irreversible), 300));

    // Windows.old 只能用系统工具：列出来，但不能直接删
    let old = by("windows-old");
    assert_eq!((old.bytes, old.deletable_bytes), (500, 0));
    assert!(old.paths.iter().all(|p| !p.deletable));

    // 保护路径拦下的 safe 规则：列出来但不能删
    let wt = by("windows-temp");
    assert_eq!((wt.bytes, wt.deletable_bytes), (40, 0));
    // 硬拦（保护路径）的带短原因；只是“不建议直接删”的（Windows.old 只能用系统工具）没有，界面上可以二次确认后强制勾
    assert!(wt.paths.iter().all(|p| p.hard.as_deref() == Some("系统保护")), "{:?}", wt.paths);
    assert!(old.paths.iter().all(|p| p.hard.is_none()));
    assert!(npm.paths.iter().all(|p| p.hard.is_none()));
    // cleanup_candidates_with：原因原样带出来
    let g2 = diskwhy_core::explain::candidates::cleanup_candidates_with(&t, &ctx, &|p: &str| blocked(p).then(|| "系统目录".to_string()));
    let wt2 = g2.iter().find(|x| x.key == "windows-temp").unwrap();
    assert_eq!(wt2.paths[0].hard.as_deref(), Some("系统目录"));

    // keep 规则（WinSxS）不出现
    assert!(g.iter().all(|x| x.tier != Tier::Keep));

    // 启发式：Rust 编译产物 / node_modules（review，可删）
    let rt = by("h:rust-target");
    assert_eq!((rt.tier, rt.lose_kind, rt.bytes), (Tier::Review, Some(LoseKind::Slower), 701));
    assert_eq!(rt.paths[0].path, r"C:\Users\bob\proj\target");
    assert_eq!(rt.name, "Rust 编译产物");
    let nm = by("h:node-modules");
    assert_eq!((nm.lose_kind, nm.deletable_bytes), (Some(LoseKind::Redownload), 60));

    // C:\tmp：需你确认，只能手动
    let tmp = by("root-tmp-dir");
    assert_eq!((tmp.lose_kind, tmp.deletable_bytes), (Some(LoseKind::Unsure), 0));

    // 未知的东西不出现
    assert!(g.iter().all(|x| !x.paths.iter().any(|p| p.path.contains("mystery"))));
    // 候选路径两两不重叠（不会重复计数）
    let all: Vec<String> = g.iter().flat_map(|x| x.paths.iter().map(|p| p.path.to_lowercase())).collect();
    for a in &all {
        for b in &all {
            assert!(a == b || !a.starts_with(&format!("{b}\\")), "{a} 在 {b} 里面，重复计数");
        }
    }
}

#[test]
fn cleanup_candidate_with_stricter_rule_inside_is_not_deleted_whole() {
    // 外层 safe 可删的规则里面套着一条 keep 规则：外层不能整个删，只记它自己那部分；里面的 keep 不出现
    const RULES: &str = r#"
[[rule]]
id = "outer"
owner = "o"
category = "cache"
tier = "safe"
match = ['%LOCALAPPDATA%\outer']
what = "w"
why = "y"
lose = "l"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-09-30"

[[rule]]
id = "inner-keep"
owner = "o"
category = "userdata"
tier = "keep"
match = ['%LOCALAPPDATA%\outer\keepme']
what = "w"
why = "y"
lose = "l"
how = "none"
how_detail = ""
refs = []
verified_on = "2026-09-30"

[[rule]]
id = "inner-safe"
owner = "o"
category = "cache"
tier = "safe"
match = ['%LOCALAPPDATA%\outer2\cache']
what = "w"
why = "y"
lose = "l"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-09-30"

[[rule]]
id = "outer2"
owner = "o"
category = "cache"
tier = "safe"
match = ['%LOCALAPPDATA%\outer2']
what = "w"
why = "y"
lose = "l"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-09-30"
"#;
    let env = fake_env();
    let (rules, errs) = RuleSet::from_sources(&[("t", RULES)], &env);
    assert!(errs.is_empty(), "{errs:?}");
    let ctx = ctx_with(env, rules, Vec::new());
    let t = tree(&[
        (r"C:\Users\bob\AppData\Local\outer\junk.tmp", 50),
        (r"C:\Users\bob\AppData\Local\outer\keepme\x", 30),
        (r"C:\Users\bob\AppData\Local\outer2\a", 10),
        (r"C:\Users\bob\AppData\Local\outer2\cache\b", 5),
    ]);
    let g = cleanup_candidates(&t, &ctx, &|_| false);
    let outer = g.iter().find(|x| x.key == "outer").expect("应有 outer 候选");
    assert_eq!((outer.bytes, outer.deletable_bytes), (50, 0), "里面有 keep 的东西，不能整个删，只记自己那部分");
    assert!(g.iter().all(|x| x.key != "inner-keep"));
    // 外层 safe 里面套着同样可删的 safe：整个外层就是一个候选，里面的字节算进来，不再单独列（不会重复计数）
    let o2 = g.iter().find(|x| x.key == "outer2").unwrap();
    assert_eq!((o2.bytes, o2.deletable_bytes, o2.paths.len()), (15, 15, 1));
    assert!(g.iter().all(|x| x.key != "inner-safe"));

    // 云端占位 / 读不到的东西不影响“能不能勾”：预检时会逐个文件跳过
    let ctx = embedded_ctx();
    let mut t2 = tree(&[(r"C:\Users\bob\AppData\Local\Temp\junk.tmp", 50), (r"C:\Users\bob\AppData\Local\Temp\sub\x", 30)]);
    fn find_mut<'a>(n: &'a mut Node, path: &str) -> Option<&'a mut Node> {
        if n.path.eq_ignore_ascii_case(path) {
            return Some(n);
        }
        n.children.iter_mut().find_map(|c| find_mut(c, path))
    }
    find_mut(&mut t2, r"C:\Users\bob\AppData\Local\Temp\sub").unwrap().cloud_placeholder = true;
    let g2 = cleanup_candidates(&t2, &ctx, &|_| false);
    let temp2 = g2.iter().find(|x| x.key == "user-temp").unwrap();
    assert_eq!((temp2.bytes, temp2.deletable_bytes, temp2.paths.len()), (80, 80, 1));
    assert_eq!(short_name("Google Chrome（谷歌浏览器）", Category::Cache), "Google Chrome 缓存");
    assert_eq!(short_name("微信", Category::Log), "微信日志");
    assert_eq!(short_name("回收站", Category::Trash), "回收站");
}

// ───────────────────────── 阶段 H：具体后果短语 lose_short ─────────────────────────

use diskwhy_core::explain::heuristics::{heuristic_lose_short, UNKNOWN_SHORT};
use diskwhy_core::explain::rules::LOSE_SHORT_MAX;

#[test]
fn every_review_rule_has_short_concrete_tag() {
    let ctx = embedded_ctx();
    let mut n = 0;
    for r in &ctx.rules.rules {
        if let Some(s) = &r.lose_short {
            assert!(s.chars().count() <= LOSE_SHORT_MAX, "规则 {} 的 lose_short `{s}` 超过 {LOSE_SHORT_MAX} 个字", r.id);
        }
        if r.tier == Tier::Review {
            n += 1;
            assert!(r.lose_short.as_deref().is_some_and(|s| !s.trim().is_empty()), "review 规则 {} 没有 lose_short", r.id);
        }
    }
    assert_eq!(n, 77); // 旧版本规则 -1；缓存复查 +3（wechat-radium、yarn-berry-cache 新增，chromium-generic-sw-cache 降档）
    let s = |id: &str| ctx.rules.by_id(id).and_then(|r| r.lose_short.clone()).unwrap_or_default();
    assert_eq!(s("hiberfil"), "不能休眠");
    assert_eq!(s("windows-old"), "不能退回旧系统");
    assert_eq!(s("package-cache"), "卸载软件会出错");
    assert_eq!(s("nvidia-driver-installers"), "重装驱动要重下");
    assert_eq!(s("wechat-appdata"), "微信要重新登录");
    assert_eq!(s("huggingface-hub"), "模型要重新下载");
    assert_eq!(s("ollama-models"), "模型要重新下载");
    assert_eq!(s("rustup-toolchains"), "Rust 要重装");
    assert_eq!(s("vscode-family-extensions"), "编辑器插件要重装");
    assert_eq!(s("root-tmp-dir"), "可能有你放的文件");
    assert_eq!(s("root-temp-dir"), "可能有你放的文件");
    assert_eq!(s("maven-repo"), "要重新下载依赖");
    assert_eq!(s("dot-cache-root"), "里面混着别的东西");
}

#[test]
fn lose_short_validation() {
    let review = GOOD.replace("tier = \"safe\"", "tier = \"review\"").replace("refs = []", "refs = []\nlose_kind = \"slower\"");
    let errs = validate(&def(&review), &mut HashSet::new());
    assert!(errs.iter().any(|e| e.contains("lose_short")), "review 没写 lose_short 应该报错：{errs:?}");
    let empty = review.replace("refs = []", "refs = []\nlose_short = \"  \"");
    assert!(!validate(&def(&empty), &mut HashSet::new()).is_empty(), "空的 lose_short 也算没写");
    let long = review.replace("refs = []", "refs = []\nlose_short = \"回收站里的东西没了\"");
    let errs = validate(&def(&long), &mut HashSet::new());
    assert!(errs.iter().any(|e| e.contains("9 个字")), "超过 8 个字应该报错：{errs:?}");
    let ok = review.replace("refs = []", "refs = []\nlose_short = \"Rust 要重装\"");
    assert!(validate(&def(&ok), &mut HashSet::new()).is_empty(), "8 个字（含空格和字母）正好可以");
    // safe 可以不写，写了也不能超长
    assert!(validate(&def(GOOD), &mut HashSet::new()).is_empty());
    let long_safe = GOOD.replace("refs = []", "refs = []\nlose_short = \"一二三四五六七八九\"");
    assert!(!validate(&def(&long_safe), &mut HashSet::new()).is_empty());
}

#[test]
fn explanations_and_candidates_carry_lose_short() {
    let ctx = embedded_ctx();
    let t = tree(&[
        (r"C:\Users\bob\.m2\repository\org\a.jar", 100),
        (r"C:\Users\bob\AppData\Local\npm-cache\x", 10),
        (r"C:\mystery\x", 5),
        (r"C:\proj\node_modules\left-pad\index.js", 60),
        (r"C:\proj\package.json", 1),
    ]);
    let maven = ex(&t, r"C:\Users\bob\.m2\repository", &ctx);
    assert_eq!(maven.lose_short.as_deref(), Some("要重新下载依赖"));
    assert!(serde_json::to_string(&maven).unwrap().contains("\"lose_short\":\"要重新下载依赖\""));
    assert_eq!(ex(&t, r"C:\Users\bob\AppData\Local\npm-cache", &ctx).lose_short, None);
    assert_eq!(ex(&t, r"C:\mystery", &ctx).lose_short.as_deref(), Some(UNKNOWN_SHORT));
    let a = llm::parse_answer(r#"{"what":"w","owner":"","why":"y","lose":"l","confidence":0.5}"#).unwrap();
    assert!(llm::to_explanation(a, "m", 1).lose_short.is_some());
    let nm = ex(&t, r"C:\proj\node_modules", &ctx);
    assert_eq!(nm.lose_short.as_deref(), Some("项目要重装依赖"), "{:?}", nm.source);
    let groups = cleanup_candidates(&t, &ctx, &|_: &str| false);
    let g = groups.iter().find(|g| g.rule_id.as_deref() == Some("maven-repo")).unwrap();
    assert_eq!(g.lose_short.as_deref(), Some("要重新下载依赖"));
}

#[test]
fn heuristic_review_results_have_short_tags() {
    for name in [
        "chromium-cache", "python-venv", "python-env", "node-modules", "rust-target", "dotnet-build", "model-weights", "logs-dumps",
        "downloaded-installer", "temp-files", "media-files", "registry-install-dir", "registry-program-dir", "registry-appdata",
        "chat-files-cache", "game-highlights", "steam-cache", "steam-workshop", "python-runtime", "conda-pkgs", "android-sdk", "android-sdk-part",
        "android-user-home", "android-avd", "gradle-home", "gradle-cache", "flutter-sdk", "flutter-cache", "pub-cache", "dev-sdk-container",
        "pnpm-store", "ollama-models-dir", "visual-studio-install", "app-install-elsewhere", "driver-packages", "virtual-disk", "vm-vmware",
        "vm-virtualbox", "vm-hyperv", "user-projects", "chat-files", "conda-envs", "conda-env", "conda-base", "conda-root",
    ] {
        let s = heuristic_lose_short(name).unwrap_or_else(|| panic!("{name} 没有 lose_short"));
        assert!(s.chars().count() <= LOSE_SHORT_MAX, "{name}: {s}");
    }
    assert!(UNKNOWN_SHORT.chars().count() <= LOSE_SHORT_MAX && llm::LLM_SHORT.chars().count() <= LOSE_SHORT_MAX);
    assert_eq!(heuristic_lose_short("model-weights"), Some("模型要重新下载"));
    assert_eq!(heuristic_lose_short("vm-vmware"), Some("虚拟机会没了"));
    assert_eq!(heuristic_lose_short("whatever-new"), None);
}
