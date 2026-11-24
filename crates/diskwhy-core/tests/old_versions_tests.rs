//! “旧版本”规则（`keep_latest`）：同一个父文件夹里一堆版本，只有正在用的版本当 keep，其余按规则（safe）。
//!
//! - 校验：keep_latest 只能用在最后一段带通配的模式上；
//! - 解释：版本号排序（1.10 > 1.9）、改按修改时间、修改时间最新的一直留着、正在用的版本里面的东西也 keep、
//!   商店版 Claude 的 LocalCache 路径；
//! - 清理页：正在用的版本不出候选，外层文件夹里有它就不能整个删；LocalCache 里的旧版本能列出来；
//! - 汇总：正在用的版本算 keep；
//! - 预检：旧版本文件夹里有一个文件被占用（真的运行一个 exe），整个文件夹跳过；没人用的旧版本照常删。
//!   **所有真实文件都只在本测试自己的临时目录里。**
#![cfg(windows)]

use diskwhy_core::action::protect::Protector;
use diskwhy_core::action::{self, Decision, PlanRequest, PreflightOptions, SkipReason, Strategy};
use diskwhy_core::explain::candidates::cleanup_candidates;
use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::LlmSettings;
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::{validate, RuleDef, RuleSet};
use diskwhy_core::explain::{tier_summary, ExplainCtx, Explanation, How, Source, Tier, CURRENT_VERSION_INSIDE_WHAT, CURRENT_VERSION_WHAT};
use diskwhy_core::scan::{scan_with, volume_root_of};
use diskwhy_core::view::explain_path;
use diskwhy_core::win::{volume_space, Win32Fs};
use diskwhy_core::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

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

fn embedded_plus(extra: &str) -> ExplainCtx {
    let env = fake_env();
    let (mut rules, errs) = RuleSet::embedded(&env);
    assert!(errs.is_empty(), "内置规则有错误：{errs:#?}");
    if !extra.is_empty() {
        let dir = std::env::temp_dir().join(format!("diskwhy-ov-rules-{}-{}", std::process::id(), extra.len()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("extra.toml"), extra).unwrap();
        let w = rules.load_user_dir(&dir, &env);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(w.is_empty(), "{w:?}");
    }
    ExplainCtx::new(env, rules, RegistrySnapshot::default(), LlmSettings::default())
}

/// 内存里的树：(路径, 大小, 修改时间)；以 `\` 结尾的是目录（可以只给目录来设它的修改时间）。根固定为 `C:\`。
fn tree(entries: &[(&str, u64, i64)]) -> Node {
    #[derive(Default)]
    struct D {
        mtime: Option<i64>,
        dirs: BTreeMap<String, D>,
        files: Vec<(String, u64, i64)>,
    }
    let mut root = D::default();
    for (p, sz, mt) in entries {
        let rel = p.strip_prefix(r"C:\").expect("路径必须以 C:\\ 开头");
        let is_dir = rel.ends_with('\\');
        let parts: Vec<&str> = rel.split('\\').filter(|s| !s.is_empty()).collect();
        let mut cur = &mut root;
        let n = if is_dir { parts.len() } else { parts.len() - 1 };
        for part in &parts[..n] {
            cur = cur.dirs.entry(part.to_string()).or_default();
        }
        if is_dir {
            cur.mtime = Some(*mt);
        } else {
            cur.files.push((parts[n].to_string(), *sz, *mt));
        }
    }
    fn node(name: &str, path: &str, kind: NodeKind, size: u64, mtime: i64, children: Vec<Node>) -> Node {
        Node {
            name: name.to_string(),
            path: path.to_string(),
            kind,
            logical: size,
            allocated: size,
            mtime,
            attrs: if kind == NodeKind::Dir { 0x10 } else { 0x20 },
            reparse_tag: 0,
            inaccessible: false,
            error_code: 0,
            reparse_skipped: false,
            cloud_placeholder: false,
            system_reserved: false,
            hardlink_dup: false,
            mapped: false,
            files: if kind == NodeKind::File { 1 } else { children.iter().map(|c| c.files).sum() },
            dirs: children.iter().map(|c| if c.kind == NodeKind::Dir { 1 + c.dirs } else { 0 }).sum(),
            children,
        }
    }
    fn build(name: &str, path: &str, d: &D) -> Node {
        let mut children: Vec<Node> = Vec::new();
        for (n, sub) in &d.dirs {
            let p = if path.ends_with('\\') { format!("{path}{n}") } else { format!("{path}\\{n}") };
            children.push(build(n, &p, sub));
        }
        for (n, sz, mt) in &d.files {
            children.push(node(n, "", NodeKind::File, *sz, *mt, vec![]));
        }
        children.sort_by(|a, b| b.allocated.cmp(&a.allocated));
        let size = children.iter().map(|c| c.allocated).sum();
        node(name, path, NodeKind::Dir, size, d.mtime.unwrap_or(1_700_000_000), children)
    }
    build(r"C:\", r"C:\", &root)
}

fn ex(t: &Node, path: &str, ctx: &ExplainCtx) -> Explanation {
    explain_path(t, path, ctx).unwrap_or_else(|| panic!("树里找不到 {path}")).1
}

fn rule_id(e: &Explanation) -> Option<&str> {
    match &e.source {
        Source::Rule { id, .. } => Some(id.as_str()),
        _ => None,
    }
}

const CC: &str = r"C:\Users\bob\AppData\Roaming\Claude\claude-code";
const T0: i64 = 1_700_000_000;

// ───────────────────────── 校验 ─────────────────────────

fn def(text: &str) -> RuleDef {
    #[derive(serde::Deserialize)]
    struct F {
        rule: Vec<RuleDef>,
    }
    toml::from_str::<F>(text).unwrap().rule.remove(0)
}

const BASE: &str = r#"[[rule]]
id = "x"
owner = "o"
category = "app"
tier = "safe"
keep_latest = 1
match = ['%LOCALAPPDATA%\Foo\app-*']
what = "w"
why = "y"
lose = "l"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-11-02"
"#;

#[test]
fn keep_latest_validation() {
    assert!(validate(&def(BASE), &mut HashSet::new()).is_empty());
    let errs = |t: &str| validate(&def(t), &mut HashSet::new()).join("；");
    assert!(errs(&BASE.replace(r"\app-*", r"\app")).contains("最后一段必须带通配"));
    assert!(errs(&BASE.replace(r"\app-*", r"\**")).contains("最后一段必须带通配"));
    assert!(errs(&BASE.replace(r"\Foo\app-*", r"\*\bin")).contains("最后一段必须带通配"));
    assert!(errs(&BASE.replace("keep_latest = 1", "keep_latest = 0")).contains("至少是 1"));
    assert!(errs(&BASE.replace(r#"tier = "safe""#, r#"tier = "keep""#).replace(r#"how = "delete""#, r#"how = "none""#)).contains("keep 档"));
}

// ───────────────────────── 解释 ─────────────────────────

#[test]
fn version_order_marks_highest_current_and_children_inherit() {
    let ctx = embedded_plus("");
    let t = tree(&[
        (&format!(r"{CC}\2.1.9\aaa\claude.exe"), 280, T0),
        (&format!(r"{CC}\2.1.10\bbb\claude.exe"), 285, T0),
        (&format!(r"{CC}\2.1.10\bbb\.verified"), 1, T0),
    ]);
    let old = ex(&t, &format!(r"{CC}\2.1.9"), &ctx);
    assert_eq!(rule_id(&old), Some("claude-code-old-versions"));
    assert_eq!(old.tier, Tier::Safe, "{old:?}");
    assert_eq!(old.gain_bytes, 280);
    let cur = ex(&t, &format!(r"{CC}\2.1.10"), &ctx);
    assert_eq!(rule_id(&cur), Some("claude-code-old-versions"));
    assert_eq!(cur.tier, Tier::Keep);
    assert_eq!(cur.what, CURRENT_VERSION_WHAT);
    assert_eq!(cur.gain_bytes, 0);
    assert!(!cur.how.contains(&How::Delete));
    // 里面的东西跟着：正在用的版本里 keep，旧版本里 safe
    let inner_cur = ex(&t, &format!(r"{CC}\2.1.10\bbb\claude.exe"), &ctx);
    assert_eq!(inner_cur.tier, Tier::Keep, "{inner_cur:?}");
    assert_eq!(inner_cur.what, CURRENT_VERSION_INSIDE_WHAT);
    let inner_dir = ex(&t, &format!(r"{CC}\2.1.10\bbb"), &ctx);
    assert_eq!(inner_dir.tier, Tier::Keep);
    let inner_old = ex(&t, &format!(r"{CC}\2.1.9\aaa\claude.exe"), &ctx);
    assert_eq!(inner_old.tier, Tier::Safe);
    // 上层 claude-code 文件夹本身还是桌面版数据（review），不受影响
    assert_eq!(rule_id(&ex(&t, CC, &ctx)), Some("claude-desktop-data"));
}

#[test]
fn store_app_localcache_versions_are_explained_and_listed() {
    let ctx = embedded_plus("");
    let lc = r"C:\Users\bob\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude-code";
    let t = tree(&[(&format!(r"{lc}\2.1.308\h1\claude.exe"), 280, T0), (&format!(r"{lc}\2.1.309\h2\claude.exe"), 285, T0)]);
    let old = ex(&t, &format!(r"{lc}\2.1.308"), &ctx);
    assert_eq!((rule_id(&old), old.tier), (Some("claude-code-old-versions"), Tier::Safe), "{old:?}");
    let cur = ex(&t, &format!(r"{lc}\2.1.309"), &ctx);
    assert_eq!((rule_id(&cur), cur.tier), (Some("claude-code-old-versions"), Tier::Keep));
    assert_eq!(ex(&t, &format!(r"{lc}\2.1.309\h2\claude.exe"), &ctx).tier, Tier::Keep);
    // 清理页：只列旧版本（LocalCache 里写明了路径的旧版本规则），正在用的不列
    let groups = cleanup_candidates(&t, &ctx, &|_| false);
    let g = groups.iter().find(|g| g.rule_id.as_deref() == Some("claude-code-old-versions")).expect("应有 Claude Code 旧版本候选");
    assert_eq!(g.tier, Tier::Safe);
    let paths: Vec<&str> = g.paths.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(paths, vec![format!(r"{lc}\2.1.308").as_str()]);
    assert!(g.paths[0].deletable);
    // LocalCache 里别的东西照旧不列
    assert!(groups.iter().all(|g| g.paths.iter().all(|p| !p.path.contains("2.1.309"))));
}

#[test]
fn non_version_names_use_mtime_and_newest_mtime_is_always_kept() {
    let ctx = embedded_plus("");
    let cr = r"C:\Users\bob\.cache\codex-runtimes";
    let t = tree(&[
        (&format!(r"{cr}\codex-primary-runtime\runtime.json"), 10, T0),
        (&format!(r"{cr}\codex-primary-runtime.previous-31450cd2-c150-4854-b46f-06c64e827b34\"), 0, T0 + 10),
        (&format!(r"{cr}\codex-primary-runtime.previous-31450cd2-c150-4854-b46f-06c64e827b34\a.bin"), 100, T0),
        (&format!(r"{cr}\codex-primary-runtime.previous-0492752e-1608-4b51-8e88-080d5fdd9ba9\"), 0, T0 + 30),
        (&format!(r"{cr}\codex-primary-runtime.previous-0492752e-1608-4b51-8e88-080d5fdd9ba9\a.bin"), 100, T0),
        (&format!(r"{cr}\codex-primary-runtime.previous-fefff732-19c9-456d-a679-0bc2ad09c96a\"), 0, T0 + 20),
        (&format!(r"{cr}\codex-primary-runtime.previous-fefff732-19c9-456d-a679-0bc2ad09c96a\a.bin"), 100, T0),
    ]);
    let tier = |name: &str| ex(&t, &format!(r"{cr}\{name}"), &ctx).tier;
    assert_eq!(tier("codex-primary-runtime.previous-0492752e-1608-4b51-8e88-080d5fdd9ba9"), Tier::Keep); // 最新
    assert_eq!(tier("codex-primary-runtime.previous-fefff732-19c9-456d-a679-0bc2ad09c96a"), Tier::Safe);
    assert_eq!(tier("codex-primary-runtime.previous-31450cd2-c150-4854-b46f-06c64e827b34"), Tier::Safe);
    // 正在用的 codex-primary-runtime 不归这条规则
    assert_eq!(rule_id(&ex(&t, &format!(r"{cr}\codex-primary-runtime"), &ctx)), Some("codex-runtimes-cache"));

    // 版本号最高的是 12.1.0.29162，但 12.1.0.28485 修改时间最新：两个都留
    let w = r"C:\Users\bob\AppData\Local\Kingsoft\WPS Office";
    let t2 = tree(&[
        (&format!(r"{w}\12.1.0.28485\"), 0, T0 + 99),
        (&format!(r"{w}\12.1.0.28485\office6\wps.exe"), 100, T0),
        (&format!(r"{w}\12.1.0.29162\"), 0, T0 + 50),
        (&format!(r"{w}\12.1.0.29162\office6\wps.exe"), 100, T0),
        (&format!(r"{w}\12.1.0.20000\"), 0, T0),
        (&format!(r"{w}\12.1.0.20000\office6\wps.exe"), 100, T0),
        (&format!(r"{w}\ttttt\x.txt"), 5, T0 + 200),
    ]);
    let tier2 = |name: &str| ex(&t2, &format!(r"{w}\{name}"), &ctx);
    assert_eq!(tier2("12.1.0.29162").tier, Tier::Keep);
    assert_eq!(tier2("12.1.0.28485").tier, Tier::Keep);
    assert_eq!(tier2("12.1.0.20000").tier, Tier::Safe);
    assert_eq!(rule_id(&tier2("12.1.0.20000")), Some("wps-office-old-versions"));
    // 不像版本号的文件夹不归旧版本规则（还是 WPS 程序本体，keep）
    assert_eq!(rule_id(&tier2("ttttt")), Some("wps-program"));
}

#[test]
fn file_versions_keep_latest_two() {
    let ctx = embedded_plus("");
    let sb = r"C:\Users\bob\.codex\.sandbox-bin";
    let t = tree(&[
        (&format!(r"{sb}\codex-command-runner-0.125.0-alpha.3.exe"), 8, T0),
        (&format!(r"{sb}\codex-command-runner-0.160.1.exe"), 8, T0),
        (&format!(r"{sb}\codex-command-runner-0.166.0.exe"), 8, T0),
        (&format!(r"{sb}\codex-command-runner-0.166.0-alpha.5.1.exe"), 8, T0 + 5),
        (&format!(r"{sb}\codex-command-runner.exe"), 8, T0),
    ]);
    let tier = |n: &str| ex(&t, &format!(r"{sb}\{n}"), &ctx).tier;
    assert_eq!(tier("codex-command-runner-0.166.0.exe"), Tier::Keep);
    assert_eq!(tier("codex-command-runner-0.166.0-alpha.5.1.exe"), Tier::Keep);
    assert_eq!(tier("codex-command-runner-0.160.1.exe"), Tier::Safe);
    assert_eq!(tier("codex-command-runner-0.125.0-alpha.3.exe"), Tier::Safe);
    assert_ne!(rule_id(&ex(&t, &format!(r"{sb}\codex-command-runner.exe"), &ctx)), Some("codex-command-runner-old"));
}

// ───────────────────────── 清理页 + 汇总 ─────────────────────────

const FOO_RULES: &str = r#"[[rule]]
id = "foo-root"
owner = "Foo"
category = "cache"
tier = "safe"
match = ['%LOCALAPPDATA%\Foo']
what = "Foo 的缓存"
why = "测试"
lose = "不丢"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-11-02"

[[rule]]
id = "foo-old"
owner = "Foo"
category = "app"
tier = "safe"
keep_latest = 1
kind = "dir"
match = ['%LOCALAPPDATA%\Foo\app-*']
what = "Foo 的旧版本"
why = "测试"
lose = "不影响正在用的版本"
how = "delete"
how_detail = ""
refs = []
verified_on = "2026-11-02"
"#;

#[test]
fn candidates_exclude_current_and_parent_is_not_whole_deletable() {
    let ctx = embedded_plus(FOO_RULES);
    let f = r"C:\Users\bob\AppData\Local\Foo";
    let t = tree(&[
        (&format!(r"{f}\app-2.0.0\foo.exe"), 200, T0),
        (&format!(r"{f}\app-10.0.0\foo.exe"), 1000, T0),
        (&format!(r"{f}\app-9.0.0\foo.exe"), 300, T0),
        (&format!(r"{f}\cache.bin"), 7, T0),
    ]);
    assert_eq!(ex(&t, &format!(r"{f}\app-10.0.0"), &ctx).tier, Tier::Keep);
    let groups = cleanup_candidates(&t, &ctx, &|_| false);
    let old = groups.iter().find(|g| g.key == "foo-old").expect("旧版本候选");
    let mut paths: Vec<(&str, bool)> = old.paths.iter().map(|p| (p.path.as_str(), p.deletable)).collect();
    paths.sort();
    assert_eq!(paths, vec![(format!(r"{f}\app-2.0.0").as_str(), true), (format!(r"{f}\app-9.0.0").as_str(), true)]);
    assert_eq!(old.deletable_bytes, 500);
    // 外层 Foo 里有正在用的版本：不能整个删，只算它自己的那部分
    let root = groups.iter().find(|g| g.key == "foo-root").expect("外层候选");
    assert_eq!(root.paths.len(), 1);
    assert!(!root.paths[0].deletable, "里面有正在用的版本，外层不能整个删");
    assert!(groups.iter().all(|g| g.paths.iter().all(|p| !p.path.ends_with("app-10.0.0"))));

    // 汇总：正在用的版本算 keep，旧版本算 safe
    let s = tier_summary(&t, &ctx);
    assert_eq!(s.keep_bytes, 1000, "{s:?}");
    assert_eq!(s.safe_bytes, 507);
    assert!(s.top_keep.iter().any(|r| r.id == "foo-old" && r.bytes == 1000 && r.what == CURRENT_VERSION_WHAT));
    assert!(s.top_safe.iter().any(|r| r.id == "foo-old" && r.bytes == 500));
}

// ───────────────────────── 预检：在用的旧版本整个跳过 ─────────────────────────

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_old(p: &Path, data: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7200)).unwrap();
}

#[test]
fn preflight_skips_whole_old_version_folder_when_any_file_is_in_use() {
    let base = std::env::temp_dir().join(format!("diskwhy-ov-pre-{}", std::process::id()));
    let tmp = Temp(base.clone());
    let foo = base.join("Foo");
    // 三个版本：app-1.0.0 有个文件被本进程占着（不给 FILE_SHARE_DELETE 以外的…这里用 Restart Manager 能看到的普通打开），
    // app-2.0.0 没人用，app-3.0.0 是正在用的版本。
    for v in ["app-1.0.0", "app-2.0.0", "app-3.0.0"] {
        write_old(&foo.join(v).join("foo.exe"), b"MZ fake");
        write_old(&foo.join(v).join("res").join("data.bin"), &vec![7u8; 5000]);
    }
    let root = base.to_string_lossy().to_string();
    let env = EnvMap::from_pairs(&[("LOCALAPPDATA", root.as_str())]);
    let (rules, errs) = RuleSet::from_sources(&[("t", FOO_RULES)], &env);
    assert!(errs.is_empty(), "{errs:?}");
    let ctx = ExplainCtx::new(env, rules, RegistrySnapshot::default(), LlmSettings::default());
    let vol = volume_space(&volume_root_of(&root).unwrap()).unwrap();
    let r = scan_with(&Win32Fs, &root, vol, false, &ScanOptions::default(), &AtomicBool::new(false), |_| {}).unwrap();
    // 占住 app-1.0.0 里的一个数据文件（全共享打开：逐个文件的探测看不出“被占用”，只有 Restart Manager 能看出来）
    let held = std::fs::File::open(foo.join("app-1.0.0").join("res").join("data.bin")).unwrap();
    let s = |v: &str| foo.join(v).to_string_lossy().to_string();
    let prot = Protector::from_system();
    let reqs: Vec<PlanRequest> = ["app-1.0.0", "app-2.0.0", "app-3.0.0"].iter().map(|v| PlanRequest { path: s(v), expect_rule: Some("foo-old".into()) }).collect();
    let mut plan = action::build_plan(&r, &ctx, &prot, &reqs, Strategy::RecycleBin);
    assert_eq!(plan.items[0].version_root.as_deref(), Some(s("app-1.0.0").as_str()));
    assert!(plan.items[2].blocked.is_some(), "正在用的版本不能加入清单");
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let busy = &plan.items[0];
    assert!(busy.blocked.is_none());
    assert!(
        busy.decisions.iter().all(|d| matches!(d, Decision::Skip { reason: SkipReason::Locked { .. } })),
        "在用的旧版本应整个跳过：{:?}",
        busy.decisions
    );
    assert!(
        busy.decisions.iter().any(|d| matches!(d, Decision::Skip { reason: SkipReason::Locked { processes } } if processes.iter().any(|p| p.pid == std::process::id()))),
        "应报告是本进程在用"
    );
    let idle = &plan.items[1];
    assert!(idle.decisions.iter().all(|d| matches!(d, Decision::Delete)), "没人用的旧版本照常删：{:?}", idle.decisions);
    assert!(rep.locked_count >= 2, "{rep:?}");
    drop(held);
    drop(tmp);
}
