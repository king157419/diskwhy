//! 阶段 B-3：带解释的 JSON 导出、扫描根以上的祖先、子树完整性、商店应用沙箱重定向、独立的 cargo 编译目录。
//! CLI 的端到端测试在 `crates/diskwhy-cli/tests/explain_cli.rs`。

use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::LlmSettings;
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::RuleSet;
use diskwhy_core::explain::{msix_redirect, ExplainCtx, Explanation, Source, Tier, tier_summary};
use diskwhy_core::view;
use diskwhy_core::view::{explain_path, export_tree, outer_ancestors, subtree_complete, truncate_tree};
use diskwhy_core::*;
use std::collections::BTreeMap;

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

fn leaf(name: &str, kind: NodeKind, size: u64) -> Node {
    Node {
        name: name.to_string(),
        path: String::new(),
        kind,
        logical: size,
        allocated: size,
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

/// 内存里的树：文件路径 → 大小。根固定为 `C:\`。
fn tree(entries: &[(&str, u64)]) -> Node {
    #[derive(Default)]
    struct D {
        dirs: BTreeMap<String, D>,
        files: Vec<(String, u64)>,
    }
    let mut root = D::default();
    for (p, sz) in entries {
        let rel = p.strip_prefix(r"C:\").unwrap();
        let parts: Vec<&str> = rel.split('\\').filter(|s| !s.is_empty()).collect();
        let mut cur = &mut root;
        for part in &parts[..parts.len() - 1] {
            cur = cur.dirs.entry(part.to_string()).or_default();
        }
        cur.files.push((parts[parts.len() - 1].to_string(), *sz));
    }
    fn build(name: &str, path: &str, d: &D) -> Node {
        let mut children: Vec<Node> = Vec::new();
        for (n, sub) in &d.dirs {
            let p = if path.ends_with('\\') { format!("{path}{n}") } else { format!("{path}\\{n}") };
            children.push(build(n, &p, sub));
        }
        for (n, sz) in &d.files {
            children.push(leaf(n, NodeKind::File, *sz));
        }
        children.sort_by(|a, b| b.allocated.cmp(&a.allocated));
        let mut me = leaf(name, NodeKind::Dir, 0);
        me.path = path.to_string();
        me.attrs = 0x10;
        me.logical = children.iter().map(|c| c.logical).sum();
        me.allocated = children.iter().map(|c| c.allocated).sum();
        me.files = children.iter().map(|c| c.files).sum();
        me.dirs = children.iter().map(|c| if c.kind == NodeKind::Dir { 1 + c.dirs } else { 0 }).sum();
        me.children = children;
        me
    }
    build(r"C:\", r"C:\", &root)
}

fn find<'a>(n: &'a view::ExplainedNode, path: &str) -> Option<&'a view::ExplainedNode> {
    if n.node.path.eq_ignore_ascii_case(path) {
        return Some(n);
    }
    n.children.iter().find_map(|c| find(c, path))
}

fn count_dirs(n: &view::ExplainedNode) -> usize {
    (n.node.kind == NodeKind::Dir) as usize + n.children.iter().map(count_dirs).sum::<usize>()
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

fn ex(t: &Node, ctx: &ExplainCtx, p: &str) -> Explanation {
    explain_path(t, p, ctx).unwrap_or_else(|| panic!("树里没有 {p}")).1
}

#[test]
fn b3_subtree_complete_and_export_tree_keeps_spine() {
    let ctx = embedded_ctx();
    let t = tree(&[
        (r"C:\Users\bob\AppData\Local\pip\cache\http-v2\a\b\blob.bin", 9_000_000),
        (r"C:\Users\bob\AppData\Local\pip\cache\selfcheck\x.json", 100),
        (r"C:\Users\bob\AppData\Local\Other\deep\deeper\x.dat", 10),
        (r"C:\Windows\System32\a.dll", 5_000_000),
        (r"C:\Windows\Fonts\b.ttf", 1_000),
    ]);
    // 1. 完整性：扫出来的树是完整的；截断后不完整；没有子目录的叶子目录截断后也不完整（文件没了）。
    assert!(subtree_complete(&t));
    let cut = truncate_tree(&t, 2);
    assert!(!subtree_complete(&cut));
    let win = cut.children.iter().find(|c| c.name == "Windows").unwrap();
    assert!(!subtree_complete(win), "Windows 的子目录在截断后没有子项");

    // 2. 导出：只留 1 层，但保留一条深路径（前 N 大目录）的整条链和链上每层的直接子项。
    let keep = r"C:\Users\bob\AppData\Local\pip\cache\http-v2";
    let ex_tree = export_tree(&t, 1, &[keep], &[], Some(&ctx));
    let k = find(&ex_tree, keep).expect("前 N 大目录应在导出的树里");
    assert!(k.children.iter().any(|c| c.node.name == "a"), "前 N 大目录的直接子项要保留");
    assert!(find(&ex_tree, r"C:\Users\bob\AppData\Local\pip\cache\http-v2\a\b").is_none(), "再往下就不保留了");
    assert!(find(&ex_tree, r"C:\Users\bob\AppData\Local\pip\cache\selfcheck").is_some(), "链上每层的兄弟也保留（旁边有什么）");
    assert!(find(&ex_tree, r"C:\Users\bob\AppData\Local\Other").is_some());
    assert!(find(&ex_tree, r"C:\Users\bob\AppData\Local\Other\deep").is_none(), "不在链上的只到那一层");
    assert!(find(&ex_tree, r"C:\Windows").is_some(), "depth=1：根的直接子项都在");
    assert!(find(&ex_tree, r"C:\Windows").unwrap().children.is_empty(), "不在链上的第 1 层不再往下");

    // 3. 摘要：每个目录都有，在完整树上算（http-v2 继承 pip-cache）；文件没有。
    fn walk(n: &view::ExplainedNode) {
        match n.node.kind {
            NodeKind::Dir => assert!(n.explanation.is_some(), "{} 没有摘要", n.node.path),
            _ => assert!(n.explanation.is_none()),
        }
        n.children.iter().for_each(walk);
    }
    walk(&ex_tree);
    let s = k.explanation.as_ref().unwrap();
    assert_eq!(s.tier, Tier::Safe);
    assert!(matches!(&s.source, Source::Rule { id, inherited_from: Some(_), .. } if id == "pip-cache"));
    assert_eq!(s.owner.as_deref(), Some("pip"));

    // 4. 序列化：explanation 和 children 平铺在节点上；读回成 Node 时 explanation 被忽略，结构不变。
    let v = serde_json::to_value(&ex_tree).unwrap();
    assert!(v["explanation"]["tier"].is_string());
    assert!(v.get("node").is_none(), "节点字段要平铺，不能多套一层");
    let back: Node = serde_json::from_value(v).unwrap();
    assert_eq!(back.children.len(), t.children.len());
    let dirs_back = {
        fn c(n: &Node) -> usize {
            (n.kind == NodeKind::Dir) as usize + n.children.iter().map(c).sum::<usize>()
        }
        c(&back)
    };
    assert_eq!(dirs_back, count_dirs(&ex_tree));

    // 5. 不给 ctx：没有摘要。
    let plain = export_tree(&t, 1, &[], &[], None);
    assert!(plain.explanation.is_none());
}

#[test]
fn b3_outer_ancestors_from_real_dirs() {
    let base = std::env::temp_dir().join(format!("diskwhy-b3-outer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join(r"a\b\c")).unwrap();
    std::fs::write(base.join(r"a\b\package.json"), b"{}").unwrap();
    let c = base.join(r"a\b\c").to_string_lossy().to_string();
    let outer = outer_ancestors(&c);
    assert!(outer[0].path.len() == 3 && outer[0].path.ends_with(":\\"), "第一层是盘符根：{}", outer[0].path);
    let last = outer.last().unwrap();
    assert!(last.path.eq_ignore_ascii_case(&base.join(r"a\b").to_string_lossy()));
    assert!(last.children.iter().any(|x| x.name == "c" && x.kind == NodeKind::Dir));
    assert!(last.children.iter().any(|x| x.name == "package.json" && x.kind == NodeKind::File), "上层的文件名也要有（旁边有没有 package.json）");
    for w in outer.windows(2) {
        assert!(w[1].path.to_lowercase().starts_with(&w[0].path.to_lowercase()), "从浅到深");
    }
    assert!(outer_ancestors(r"C:\").is_empty());
    assert_eq!(outer_ancestors(r"C:\Windows").len(), 1);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn b3_msix_sandbox_redirect_and_standalone_cargo_target() {
    let ctx = embedded_ctx();
    let pkg = r"C:\Users\bob\AppData\Local\Packages\Claude_pzs8sxrjxfjjc";
    let t = tree(&[
        (&format!(r"{pkg}\LocalCache\Local\pip\cache\http-v2\a.bin"), 5_000_000),
        (&format!(r"{pkg}\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle\rootfs.vhdx"), 8_000_000),
        (&format!(r"{pkg}\LocalCache\Local\qzx-unknown\blob.qzx"), 3_000_000),
        (&format!(r"{pkg}\LocalCache\Local\DiskWhy-build\target\CACHEDIR.TAG"), 177),
        (&format!(r"{pkg}\LocalCache\Local\DiskWhy-build\target\.rustc_info.json"), 1_111),
        (&format!(r"{pkg}\LocalCache\Local\DiskWhy-build\target\debug\deps\libx.rlib"), 7_000_000),
        (&format!(r"{pkg}\Settings\settings.dat"), 8_000),
        (r"C:\Users\bob\AppData\Local\MyBuild\target\CACHEDIR.TAG", 177),
        (r"C:\Users\bob\AppData\Local\MyBuild\target\.rustc_info.json", 1_111),
        (r"C:\Users\bob\AppData\Local\MyBuild\target\release\app.exe", 2_000_000),
        (r"C:\Users\bob\AppData\Local\JustTag\CACHEDIR.TAG", 177),
    ]);

    // 识别：只认 Packages\<包>\LocalCache\<种类>\<至少一层>。
    let r = msix_redirect(&format!(r"{pkg}\LocalCache\Local\pip\cache"), &ctx).unwrap();
    assert_eq!(r.package, "Claude_pzs8sxrjxfjjc");
    assert_eq!(r.virtual_path, r"C:\Users\bob\AppData\Local\pip\cache");
    assert_eq!(msix_redirect(&format!(r"{pkg}\LocalCache\Roaming\Claude"), &ctx).unwrap().virtual_path, r"C:\Users\bob\AppData\Roaming\Claude");
    assert!(msix_redirect(&format!(r"{pkg}\LocalCache\Local"), &ctx).is_none());
    assert!(msix_redirect(&format!(r"{pkg}\Settings\x"), &ctx).is_none());
    assert!(msix_redirect(r"C:\Users\bob\AppData\Local\pip\cache", &ctx).is_none());

    // 沙箱里的 pip 缓存：按原位置的规则说明（精确 + 继承），并写明重定向。
    let e = ex(&t, &ctx, &format!(r"{pkg}\LocalCache\Local\pip\cache"));
    assert_eq!(rule_id(&e), Some("pip-cache"), "{:?}", e.source);
    assert_eq!(e.tier, Tier::Safe);
    assert!(e.notes[0].contains("沙箱") && e.notes[0].contains(r"AppData\Local\pip\cache"), "{:?}", e.notes);
    let e = ex(&t, &ctx, &format!(r"{pkg}\LocalCache\Local\pip\cache\http-v2"));
    assert_eq!(rule_id(&e), Some("pip-cache"));
    assert!(matches!(&e.source, Source::Rule { inherited_from: Some(_), .. }));

    // 真实路径上有更具体的规则时照旧（Claude 虚拟机镜像）。
    let e = ex(&t, &ctx, &format!(r"{pkg}\LocalCache\Roaming\Claude\vm_bundles"));
    assert_eq!(rule_id(&e), Some("claude-desktop-vm"));

    // 沙箱里的 cargo 编译目录：不再说成“Claude 的登录信息和设置”，而是 Rust 编译产物。
    for p in [r"LocalCache\Local\DiskWhy-build\target", r"LocalCache\Local\DiskWhy-build\target\debug", r"LocalCache\Local\DiskWhy-build\target\debug\deps"] {
        let e = ex(&t, &ctx, &format!(r"{pkg}\{p}"));
        assert_eq!(heur_name(&e), Some("rust-target"), "{p}: {:?} {}", e.source, e.what);
        assert_eq!(e.tier, Tier::Review);
        assert!(e.notes.iter().any(|n| n.contains("沙箱")), "{p}: {:?}", e.notes);
    }

    // 沙箱里谁也认不出的：第 1 轮评审后不再沿用包规则的说法（“Claude 的登录信息……”），
    // 而是“沙箱里的东西，还没认出是谁的”，线索第一条写明重定向。
    let e = ex(&t, &ctx, &format!(r"{pkg}\LocalCache\Local\qzx-unknown"));
    assert!(e.is_unknown(), "{:?}", e.source);
    assert!(e.what.contains("沙箱"), "{}", e.what);
    assert!(e.clues[0].contains("Claude_pzs8sxrjxfjjc") && e.clues[0].contains(r"AppData\Local\qzx-unknown"), "{:?}", e.clues);
    // LocalCache\Local 这一层本身：沙箱说明（self 规则），不再是“Claude 的登录信息”。
    let e = ex(&t, &ctx, &format!(r"{pkg}\LocalCache\Local"));
    assert_eq!(rule_id(&e), Some("store-app-sandbox-appdata"));
    assert!(e.what.contains("挪到了这里"));
    // 汇总：沙箱里的 pip 缓存按 pip-cache 记，认不出的记“规则没覆盖”，不再记到 claude-desktop-data。
    let s = tier_summary(&t, &ctx);
    let find = |id: &str| s.top_safe.iter().chain(&s.top_review).chain(&s.top_keep).find(|r| r.id == id).map(|r| r.bytes);
    assert_eq!(find("pip-cache"), Some(5_000_000), "{s:?}");
    assert!(s.uncovered_bytes >= 3_000_000, "{s:?}");
    assert_eq!(find("claude-desktop-data"), Some(8_000), "只剩 Settings 里的：{s:?}");
    // 沙箱外的包数据照旧，不加提醒。
    let e = ex(&t, &ctx, &format!(r"{pkg}\Settings"));
    assert_eq!(rule_id(&e), Some("claude-desktop-data"));
    assert!(!e.notes.iter().any(|n| n.contains("沙箱")));

    // 不在沙箱里、旁边也没有 Cargo.toml 的 cargo 编译目录（CARGO_TARGET_DIR）。
    let e = ex(&t, &ctx, r"C:\Users\bob\AppData\Local\MyBuild\target");
    assert_eq!(heur_name(&e), Some("rust-target"), "{:?} {}", e.source, e.what);
    assert!(e.what.contains("单独放在这里"));
    let e = ex(&t, &ctx, r"C:\Users\bob\AppData\Local\MyBuild\target\release");
    assert_eq!(heur_name(&e), Some("rust-target"));
    // 只有 CACHEDIR.TAG（别的软件也会放）不算。
    let e = ex(&t, &ctx, r"C:\Users\bob\AppData\Local\JustTag");
    assert_ne!(heur_name(&e), Some("rust-target"));
}
