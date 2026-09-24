//! 阶段 B-4（GUI 接入）用到的 core 行为：`Explanation.tool`（BuiltinTool 的工具名）。

use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::LlmSettings;
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::RuleSet;
use diskwhy_core::explain::tools::tool_name;
use diskwhy_core::explain::{enforce_guards, ExplainCtx, How, Source};
use diskwhy_core::view::explain_path;
use diskwhy_core::*;

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

fn ctx() -> ExplainCtx {
    let env = fake_env();
    let (rules, errs) = RuleSet::embedded(&env);
    assert!(errs.is_empty(), "内置规则有错误：{errs:#?}");
    ExplainCtx::new(env, rules, RegistrySnapshot::from_apps(Vec::new()), LlmSettings::default())
}

fn node(name: &str, path: &str, kind: NodeKind, size: u64, children: Vec<Node>) -> Node {
    Node {
        name: name.to_string(),
        path: path.to_string(),
        kind,
        logical: size,
        allocated: size,
        mtime: 1_700_000_000,
        attrs: if kind == NodeKind::Dir { 0x10 } else { 0x20 },
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
        children,
    }
}

/// 一条链 C:\a\b\c，末端放一个文件。
fn chain(path: &str, size: u64) -> Node {
    let parts: Vec<&str> = path.trim_start_matches(r"C:\").split('\\').collect();
    let mut cur = node("f.bin", "", NodeKind::File, size, vec![]);
    for i in (0..parts.len()).rev() {
        let p = format!(r"C:\{}", parts[..=i].join("\\"));
        cur = node(parts[i], &p, NodeKind::Dir, size, vec![cur]);
    }
    node(r"C:\", r"C:\", NodeKind::Dir, size, vec![cur])
}

#[test]
fn b4_every_builtin_tool_rule_names_its_tool() {
    let c = ctx();
    let mut missing = Vec::new();
    let mut n = 0;
    for r in &c.rules.rules {
        if !r.how.contains(&How::BuiltinTool) {
            continue;
        }
        n += 1;
        match tool_name(&r.how_detail, Some(&r.owner)) {
            Some(t) => {
                println!("{} -> {t}", r.id);
                assert!(!t.trim().is_empty() && t.chars().count() <= 40, "{}: 工具名太长或为空：{t}", r.id);
                if r.id == "baidu-netdisk" {
                    assert_eq!(t, "百度网盘 自己的设置", "先写的是软件自己的设置，卸载是备选");
                }
            }
            None => missing.push(r.id.clone()),
        }
    }
    assert!(n >= 50, "builtin_tool 规则只有 {n} 条？");
    assert!(missing.is_empty(), "这些 builtin_tool 规则认不出工具名：{missing:?}");
}

#[test]
fn b4_explanation_carries_tool_only_for_builtin_tool() {
    let c = ctx();
    let t = chain(r"C:\Windows\WinSxS", 5_000_000_000);
    let e = explain_path(&t, r"C:\Windows\WinSxS", &c).unwrap().1;
    assert!(matches!(&e.source, Source::Rule { id, .. } if id == "winsxs"));
    assert_eq!(e.tool.as_deref(), Some("DISM 命令（管理员终端）"));
    // JSON 里有 tool 字段，前端据此显示工具名
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["tool"], "DISM 命令（管理员终端）");

    assert_eq!(
        tool_name("以管理员身份运行 powercfg /h off 关闭休眠", None).as_deref(),
        Some("powercfg 命令（管理员终端）")
    );

    // how 里没有 BuiltinTool 时不带 tool，JSON 里也不出现这个键（护栏出口统一重算）
    let mut m = e.clone();
    m.how = vec![How::Migrate];
    let m = enforce_guards(m, r"C:\Windows\WinSxS", &c);
    assert!(m.tool.is_none());
    assert!(serde_json::to_value(&m).unwrap().get("tool").is_none());

    // 未知项：没有工具名
    let t = chain(r"C:\zzz-unknown-thing", 10);
    let e = explain_path(&t, r"C:\zzz-unknown-thing", &c).unwrap().1;
    assert!(e.is_unknown());
    assert!(e.tool.is_none());
    assert_eq!(e.what, "还没认出来，先别动");
    assert!(!e.clues.is_empty(), "未知项要列出线索");
}
