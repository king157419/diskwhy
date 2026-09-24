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
    check("wechat-render-cache", Tier::Safe, &[]);
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
    check("chromium-generic-sw-cache", Tier::Safe, &[]);
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
    // ** 通配 + 具体规则优先于上层继承：飞书里的 CacheStorage 是 safe，其余 review
    let sw = ex(&t, r"C:\Users\bob\AppData\Roaming\LarkShell\aha\users\51af\profile_explorer\Service Worker\CacheStorage", &ctx);
    assert_eq!((rule_id(&sw), sw.tier), (Some("chromium-generic-sw-cache"), Tier::Safe));
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
    assert_heur(&fx.explain("py"), "python-env", Tier::Review);
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
    let body = llm::build_request_body("test-model", &input).to_string();

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

// ───────────────────────── S22 · 阶段 B-1：schema 补测 + 重复模式 + 每条规则的样例 ─────────────────────────

/// 拼一条合法规则的 TOML（`extra` 是额外字段行）。
fn rule_toml(id: &str, matches: &str, extra: &str) -> String {
    format!(
        "[[rule]]\nid = \"{id}\"\nowner = \"o\"\ncategory = \"cache\"\ntier = \"safe\"\nmatch = [{matches}]\n\
         what = \"w\"\nwhy = \"y\"\nlose = \"l\"\nhow = \"delete\"\nhow_detail = \"\"\nrefs = []\nverified_on = \"2026-09-23\"\n{extra}\n"
    )
}

#[test]
fn validation_accepts_legal_variants() {
    // 合法写法：how 数组、scope/kind/when_child/owner_from_path、* 与 ** 通配、各个允许的根变量、refs 列表
    let variants: &[(&str, &str)] = &[
        ("how 数组", r#"
[[rule]]
id = "a-1"
owner = "o"
category = "model"
tier = "review"
match = ['%USERPROFILE%\.cache\foo']
what = "w"
why = "y"
lose = "l"
how = ["migrate", "delete"]
how_detail = "先迁移"
refs = ["https://example.com/a", "http://example.com/b"]
verified_on = "2026-09-23"
"#),
        ("容器 + 目录 + when_child + owner_from_path", r#"
[[rule]]
id = "b-2"
owner = "o"
category = "cache"
tier = "safe"
scope = "self"
kind = "dir"
when_child = ["Cache_Data", "index"]
owner_from_path = true
match = ['%APPDATA%\**\Cache', '%LOCALAPPDATA%\*\Cache']
what = "w"
why = "y"
lose = "l"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-09-23"
"#),
        ("keep + builtin_tool + 各种根变量", r#"
[[rule]]
id = "c-3"
owner = "o"
category = "system"
tier = "keep"
match = ['%SystemRoot%\x', '%WINDIR%\y', '%SystemDrive%\z', '%ProgramFiles%\p', '%ProgramFiles(x86)%\q', '%ProgramData%\r', '%TEMP%\t', '%PUBLIC%\Libraries']
what = "w"
why = "y"
lose = "l"
how = "builtin_tool"
how_detail = "用系统工具"
refs = []
verified_on = "2026-09-23"
"#),
        ("safe 可以在用户目录下的非个人文件夹（.cache）", r#"
[[rule]]
id = "d-4"
owner = "o"
category = "cache"
tier = "safe"
match = ['%USERPROFILE%\.cache\pip', '%userprofile%\AppData\Local\x']
what = "w"
why = "y"
lose = "l"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-09-23"
"#),
    ];
    for (name, text) in variants {
        let (rs, errs) = RuleSet::from_sources(&[("t", text)], &fake_env());
        assert!(errs.is_empty(), "「{name}」应合法：{errs:?}");
        assert_eq!(rs.rules.len(), 1, "「{name}」应加载 1 条");
    }
    // 非法 tier / category / how 取值：反序列化就失败
    for (name, from, to) in [
        ("tier 取值非法", "tier = \"safe\"", "tier = \"maybe\""),
        ("category 取值非法", "category = \"cache\"", "category = \"stuff\""),
        ("how 取值非法", "how = \"delete\"", "how = \"nuke\""),
        ("verified_on 不是日期", "verified_on = \"2026-09-23\"", "verified_on = \"2026-13-40\""),
    ] {
        let t = GOOD.replace(from, to);
        assert_ne!(t, GOOD);
        let (rs, errs) = RuleSet::from_sources(&[("t", &t)], &fake_env());
        assert!(rs.rules.is_empty() && !errs.is_empty(), "「{name}」应加载失败");
    }
    // safe 护栏：公共文件夹、OneDrive 变体、通配顶在个人文件夹那一层
    for m in [r"'%PUBLIC%\Documents\x'", r"'%USERPROFILE%\Pictures'", r"'%USERPROFILE%\OneDrive'", r"'%USERPROFILE%\*\cache'", r"'%USERPROFILE%'"] {
        let (rs, errs) = RuleSet::from_sources(&[("t", &rule_toml("s-x", m, ""))], &fake_env());
        assert!(rs.rules.is_empty() && errs.iter().any(|e| e.contains("safe")), "safe 规则 {m} 应被拦：{errs:?}");
        // 同样的位置写成 review 就可以
        let (rs, errs) = RuleSet::from_sources(&[("t", &rule_toml("s-x", m, "").replace("tier = \"safe\"", "tier = \"review\""))], &fake_env());
        assert!(errs.is_empty() && rs.rules.len() == 1, "review 规则 {m} 应放行：{errs:?}");
    }
}

#[test]
fn duplicate_match_is_rejected() {
    let env = fake_env();
    // 1. 两条规则写了同一个模式（大小写、斜杠不同）：后来的不加载
    let a = rule_toml("dup-a", r"'%LOCALAPPDATA%\Foo\Cache'", "");
    let b = rule_toml("dup-b", r"'%localappdata%/foo/CACHE'", "");
    let (rs, errs) = RuleSet::from_sources(&[("g1", &a), ("g2", &b)], &env);
    assert!(rs.by_id("dup-a").is_some() && rs.by_id("dup-b").is_none());
    assert!(errs.iter().any(|e| e.contains("dup-b") && e.contains("dup-a") && e.contains("重复")), "{errs:?}");
    // 2. 同一条规则里写两遍
    let (rs, errs) = RuleSet::from_sources(&[("g", &rule_toml("dup-c", r"'%APPDATA%\x', '%APPDATA%\X'", ""))], &env);
    assert!(rs.rules.is_empty() && errs.iter().any(|e| e.contains("写了两遍")), "{errs:?}");
    // 3. %WINDIR% 和 %SystemRoot% 视为同一个根
    let w = rule_toml("dup-w", r"'%WINDIR%\Foo'", "").replace("tier = \"safe\"", "tier = \"review\"");
    let s = rule_toml("dup-s", r"'%SystemRoot%\foo'", "").replace("tier = \"safe\"", "tier = \"review\"");
    let (rs, errs) = RuleSet::from_sources(&[("g", &format!("{w}\n{s}"))], &env);
    assert!(rs.by_id("dup-s").is_none() && !errs.is_empty(), "{errs:?}");
    // 4. 同名模式但 when_child 不同：认的是不同东西，不算重复
    let x = rule_toml("wc-a", r"'%APPDATA%\**\Store'", "when_child = [\"a\"]");
    let y = rule_toml("wc-b", r"'%APPDATA%\**\Store'", "when_child = [\"b\"]");
    let (rs, errs) = RuleSet::from_sources(&[("g", &format!("{x}\n{y}"))], &env);
    assert!(errs.is_empty() && rs.rules.len() == 2, "{errs:?}");
    // 5. 展开后撞车：只跳过撞车的那个模式，规则照常加载，给警告
    let p = rule_toml("exp-a", r"'%LOCALAPPDATA%\Bar'", "");
    let q = rule_toml("exp-b", r"'%USERPROFILE%\AppData\Local\bar', '%LOCALAPPDATA%\Baz'", "");
    let (rs, errs) = RuleSet::from_sources(&[("g", &format!("{p}\n{q}"))], &env);
    assert!(rs.by_id("exp-b").is_some(), "展开撞车不应扔掉整条规则");
    assert!(errs.iter().any(|e| e.contains("exp-b") && e.contains("exp-a") && e.contains("已跳过")), "{errs:?}");
    assert_eq!(rs.patterns.len(), 2, "exp-a 一个 + exp-b 没撞车的一个");
    let ctx = ctx_with(env.clone(), rs, vec![]);
    let t = tree(&[(r"C:\Users\bob\AppData\Local\Bar\x", 1), (r"C:\Users\bob\AppData\Local\Baz\x", 1)]);
    assert_eq!(rule_id(&ex(&t, r"C:\Users\bob\AppData\Local\Bar", &ctx)), Some("exp-a"));
    assert_eq!(rule_id(&ex(&t, r"C:\Users\bob\AppData\Local\Baz", &ctx)), Some("exp-b"));
    // 6. 同一条规则里两种写法展开后相同（互为兜底）：允许，没有警告
    let (rs, errs) = RuleSet::from_sources(&[("g", &rule_toml("tmp", r"'%TEMP%', '%LOCALAPPDATA%\Temp'", ""))], &env);
    assert!(errs.is_empty() && rs.rules.len() == 1, "{errs:?}");
    // 7. 用户规则和内置规则重复：用户规则不加载，给警告
    let dir = temp_root("userdup");
    std::fs::write(dir.join("dup.toml"), rule_toml("my-npm", r"'%LOCALAPPDATA%\npm-cache'", "")).unwrap();
    let (mut rs, _) = RuleSet::embedded(&env);
    let warns = rs.load_user_dir(&dir, &env);
    assert!(rs.by_id("my-npm").is_none());
    assert!(warns.iter().any(|w| w.contains("my-npm") && w.contains("npm-cache")), "{warns:?}");
    let _ = std::fs::remove_dir_all(&dir);
    // 8. 某台机器 %TEMP% 被改到 C:\Windows\Temp：user-temp 的这个模式跳过，但规则还在、%LOCALAPPDATA%\Temp 照常生效
    let mut odd = fake_env();
    odd.set("TEMP", r"C:\Windows\Temp");
    let (rs, errs) = RuleSet::embedded(&odd);
    assert!(rs.by_id("user-temp").is_some(), "{errs:?}");
    assert!(errs.iter().all(|e| e.contains("已跳过")), "只应有跳过警告：{errs:?}");
    let ctx = ctx_with(odd, rs, vec![]);
    let t = tree(&[(r"C:\Windows\Temp\a", 1), (r"C:\Users\bob\AppData\Local\Temp\b", 1)]);
    assert_eq!(rule_id(&ex(&t, r"C:\Windows\Temp", &ctx)), Some("windows-temp"));
    assert_eq!(rule_id(&ex(&t, r"C:\Users\bob\AppData\Local\Temp", &ctx)), Some("user-temp"));
}

/// 按模式造一个具体路径：`**` 换成 `deep`（为空时去掉），`*` 换成 `x1`，`?` 换成 `q`。
fn sample_paths(expanded: &str) -> Vec<String> {
    let mut out = Vec::new();
    for deep in [None, Some(r"sample\deep")] {
        let segs: Vec<String> = expanded
            .split('\\')
            .filter(|s| !s.is_empty())
            .filter_map(|s| {
                if s == "**" {
                    deep.map(str::to_string)
                } else {
                    Some(s.replace('*', "x1").replace('?', "q"))
                }
            })
            .collect();
        let p = segs.join("\\");
        let p = if p.ends_with(':') { format!("{p}\\") } else { p };
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// 每条内置规则至少有一个具体路径能让它“胜出”（被选为这条路径的说明）。
/// 既证明每个模式都能匹配，也能抓出被更具体的规则完全盖住、永远不会生效的死规则。
#[test]
fn every_rule_has_a_matching_sample() {
    use diskwhy_core::explain::rules::KindFilter;
    let ctx = embedded_ctx();
    let file_like = |last: &str| {
        let l = last.to_lowercase();
        [".sys", ".db", ".dmp", ".ost", ".pst", ".vhdx", ".elog"].iter().any(|x| l.ends_with(x))
    };
    let mut dead = Vec::new();
    let mut samples = 0usize;
    for (ri, rule) in ctx.rules.rules.iter().enumerate() {
        let mut won = false;
        let mut tried = Vec::new();
        for (_, p) in ctx.rules.patterns.iter().filter(|(i, _)| *i == ri) {
            for path in sample_paths(&p.expanded) {
                let last = path.rsplit('\\').next().unwrap_or("").to_string();
                let is_file = rule.kind == KindFilter::File || (rule.kind == KindFilter::Any && rule.when_child.is_empty() && file_like(&last));
                let entries: Vec<(String, u64)> = if path == r"C:\" {
                    vec![(r"C:\x.bin".to_string(), 1)]
                } else if is_file {
                    vec![(path.clone(), 1)]
                } else if let Some(c) = rule.when_child.first() {
                    vec![(format!("{path}\\{c}\\f.bin"), 1)]
                } else {
                    vec![(format!("{path}\\f.bin"), 1)]
                };
                let refs: Vec<(&str, u64)> = entries.iter().map(|(a, b)| (a.as_str(), *b)).collect();
                let t = tree(&refs);
                samples += 1;
                let e = ex(&t, &path, &ctx);
                if rule_id(&e) == Some(rule.id.as_str()) {
                    won = true;
                    // 规则胜出时，解释里的字段都来自这条规则
                    assert_eq!(e.what, rule.what);
                    assert_eq!(e.category, rule.category);
                    assert!((e.confidence - 1.0).abs() < f32::EPSILON);
                } else {
                    tried.push(format!("{path} → {:?}", e.source));
                }
            }
        }
        if !won {
            dead.push(format!("{}：{tried:?}", rule.id));
        }
    }
    assert!(dead.is_empty(), "这些规则没有任何样例能命中（死规则或模式写错）：\n{}", dead.join("\n"));
    println!("{} 条规则、{samples} 个样例路径全部通过", ctx.rules.rules.len());
}

/// 任务 B-1 点名的条目：用真实长相的路径逐条核对档位、办法和说明来源。
#[test]
fn b1_listed_items_real_paths() {
    let ctx = embedded_ctx();
    let cases: &[(&str, &str, Tier)] = &[
        // Windows 系统项
        (r"C:\Windows\WinSxS", "winsxs", Tier::Keep),
        (r"C:\Windows.old", "windows-old", Tier::Review),
        (r"C:\Windows\Installer", "windows-installer-cache", Tier::Keep),
        (r"C:\Windows\ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization", "delivery-optimization", Tier::Safe),
        (r"C:\hiberfil.sys", "hiberfil", Tier::Review),
        (r"C:\pagefile.sys", "pagefile", Tier::Keep),
        (r"C:\Windows\Temp", "windows-temp", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Temp", "user-temp", Tier::Safe),
        (r"C:\Windows\Prefetch", "prefetch", Tier::Keep),
        (r"C:\Users\bob\AppData\Local\Microsoft\Windows\Explorer\thumbcache_256.db", "thumbnail-cache", Tier::Safe),
        (r"C:\Windows\SoftwareDistribution\Download", "windows-update-download", Tier::Safe),
        (r"C:\Windows\Logs", "windows-logs", Tier::Review),
        (r"C:\ProgramData\Microsoft\Windows Defender\Scans", "defender-scans", Tier::Keep),
        (r"C:\ProgramData\Microsoft\Windows Defender\Quarantine", "defender-quarantine", Tier::Review),
        // 浏览器缓存
        (r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Default\Cache", "chrome-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Profile 1\Code Cache", "chrome-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Default\GPUCache", "chrome-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Google\Chrome\User Data\Default\Service Worker\CacheStorage", "chrome-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Microsoft\Edge\User Data\Default\Cache", "edge-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Microsoft\Edge\User Data\Default\Service Worker\ScriptCache", "edge-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Mozilla\Firefox\Profiles\ab12cd.default-release\cache2", "firefox-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\BraveSoftware\Brave-Browser\User Data\Default\Cache", "chromium-browsers-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Roaming\SomeApp\GPUCache", "chromium-generic-gpu-cache", Tier::Safe),
        // 开发缓存
        (r"C:\Users\bob\AppData\Local\npm-cache", "npm-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\pnpm\store", "pnpm-store", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Yarn\Cache", "yarn-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\pip\cache", "pip-cache", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\uv\cache", "uv-cache", Tier::Safe),
        (r"C:\Users\bob\.cargo\registry", "cargo-registry", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\go-build", "go-cache", Tier::Safe),
        (r"C:\Users\bob\.gradle\caches", "gradle-cache", Tier::Safe),
        (r"C:\Users\bob\.m2\repository", "maven-repo", Tier::Review),
        (r"C:\Users\bob\.nuget\packages", "nuget-cache", Tier::Safe),
        (r"C:\Users\bob\miniconda3\pkgs", "conda-pkgs", Tier::Safe),
        (r"C:\Users\bob\AppData\Local\Docker\wsl\disk\docker_data.vhdx", "docker-data-vhdx", Tier::Keep),
        (r"C:\Users\bob\AppData\Local\Packages\CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc\LocalState\ext4.vhdx", "wsl-distro-vhdx", Tier::Keep),
        (r"C:\Users\bob\AppData\Local\Microsoft\VisualStudio", "visual-studio-cache", Tier::Review),
        (r"C:\Users\bob\AppData\Local\JetBrains\PyCharm2025.2\caches", "jetbrains-caches", Tier::Safe),
        (r"C:\Users\bob\AppData\Roaming\Code\CachedData", "vscode-family-cached", Tier::Safe),
        (r"C:\Users\bob\AppData\Roaming\Cursor\CachedData", "vscode-family-cached", Tier::Safe),
        (r"C:\Users\bob\AppData\Roaming\Cursor\User", "vscode-family-user", Tier::Keep),
    ];
    // 文件就放文件本身，文件夹里放一个 1 字节的文件
    let is_file = |p: &str| [".sys", ".db", ".vhdx"].iter().any(|x| p.ends_with(x));
    // 通用 Chromium 缓存规则要求里面有 index / data_0（when_child），按真实长相放 data_0
    let inner = |p: &str| if p.ends_with("GPUCache") { "data_0" } else { "f.bin" };
    let entries: Vec<String> =
        cases.iter().map(|(p, _, _)| if is_file(p) { p.to_string() } else { format!("{p}\\{}", inner(p)) }).collect();
    let t = tree(&entries.iter().map(|p| (p.as_str(), 1u64)).collect::<Vec<_>>());
    for (p, id, tier) in cases {
        let e = ex(&t, p, &ctx);
        assert_eq!(rule_id(&e), Some(*id), "{p}：{:?}", e.source);
        assert_eq!(e.tier, *tier, "{p}");
        // 说明文案不能空；keep 档不能给删除；safe 档能腾出的就是它的大小
        assert!(!e.what.is_empty() && !e.why.is_empty() && !e.lose.is_empty(), "{p}");
        if *tier == Tier::Keep {
            assert!(!e.how.contains(&How::Delete) && e.gain_bytes == 0, "{p}");
        }
        if *tier == Tier::Safe {
            assert_eq!(e.gain_bytes, 1, "{p}");
        }
    }
    let q = ctx.rules.by_id("defender-quarantine").unwrap();
    assert!(q.how.contains(&How::BuiltinTool) && q.how_detail.contains("QuarantinePurgeItemsAfterDelay"));
    let s = ctx.rules.by_id("defender-scans").unwrap();
    assert!(s.how_detail.contains("ScanPurgeItemsAfterDelay"));
}
