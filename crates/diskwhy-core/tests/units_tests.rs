//! 按年龄挑（`min_age_days`，临时文件夹）+ “正在运行”的整项 / 整个子项跳过。
//!
//! - 校验：min_age_days 只能用在 tree、能直接删的文件夹规则上；
//! - 清理页和解释的“能腾出”只算 N 天以上没改过的直接子项（子文件夹看整棵子树里最新的文件）；
//! - 预检（真实文件、真实进程）：%TEMP% 下每个直接子项单独看——有一个文件是最近改的 → 整个子项“最近改过”；
//!   有程序从里面运行（真的跑一个复制过去的 exe）→ 整个子项“正在运行”；有文件被占用 → 整个子项跳过；
//!   别的规则的清单项里有程序在运行 → 整项“正在运行”；执行开始时再查一遍也能发现。
//!   **所有真实文件都只在本测试自己的临时目录里；只结束本测试自己启动的进程。**
#![cfg(windows)]

use diskwhy_core::action::protect::Protector;
use diskwhy_core::action::{self, Decision, PlanRequest, PreflightOptions, SkipReason, Strategy};
use diskwhy_core::explain::candidates::cleanup_candidates;
use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::LlmSettings;
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::{validate, RuleDef, RuleSet};
use diskwhy_core::explain::{ExplainCtx, How, Tier};
use diskwhy_core::scan::{scan_with, volume_root_of};
use diskwhy_core::view::explain_path;
use diskwhy_core::win::{volume_space, Win32Fs};
use diskwhy_core::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

const DAY: u64 = 86_400;

fn def(text: &str) -> RuleDef {
    #[derive(serde::Deserialize)]
    struct F {
        rule: Vec<RuleDef>,
    }
    toml::from_str::<F>(text).unwrap().rule.remove(0)
}

const AGE_RULE: &str = r#"[[rule]]
id = "x"
owner = "o"
category = "cache"
tier = "safe"
min_age_days = 7
match = ['%TEMP%']
what = "w"
why = "y"
lose = "l"
how = ["delete", "builtin_tool"]
how_detail = "用系统工具"
refs = []
verified_on = "2026-11-02"
"#;

#[test]
fn min_age_days_validation() {
    let ok = validate(&def(AGE_RULE), &mut HashSet::new());
    assert!(ok.is_empty(), "{ok:?}");
    let errs = |t: &str| validate(&def(t), &mut HashSet::new()).join("；");
    assert!(errs(&AGE_RULE.replace("min_age_days = 7", "min_age_days = 0")).contains("至少是 1"));
    assert!(errs(&AGE_RULE.replace("min_age_days = 7", "min_age_days = 7\nscope = \"self\"")).contains("scope = tree"));
    assert!(errs(&AGE_RULE.replace(r#"how = ["delete", "builtin_tool"]"#, r#"how = "builtin_tool""#)).contains("how 要含 delete"));
    // 内置的两条临时文件规则都用上了
    let env = EnvMap::from_pairs(&[("TEMP", r"C:\T"), ("SystemRoot", r"C:\Windows"), ("LOCALAPPDATA", r"C:\L")]);
    let (rs, errs) = RuleSet::embedded(&env);
    assert!(errs.is_empty(), "{errs:?}");
    for id in ["user-temp", "windows-temp"] {
        let r = rs.by_id(id).unwrap();
        assert_eq!(r.min_age_days, Some(7), "{id}");
        assert!(r.how.contains(&How::Delete), "{id}");
    }
}

// ───────────────────────── 真实文件 ─────────────────────────

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_aged(p: &Path, data: &[u8], age_secs: u64) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(age_secs)).unwrap();
}

const FOO_RULE: &str = r#"[[rule]]
id = "foo-cache"
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
"#;

/// 复制一个系统自带的小 exe 进来并运行（`ping -n 60 127.0.0.1`，跑一分钟左右），测试结束时结束它。
struct Running(std::process::Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn run_copy(exe: &Path) -> Running {
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(r"C:\Windows\System32\PING.EXE", exe).unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(exe).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(10 * DAY)).unwrap();
    drop(f);
    let c = std::process::Command::new(exe)
        .args(["-n", "60", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    Running(c)
}

fn all_skip(item: &action::PlanItem, f: impl Fn(&SkipReason) -> bool, under: &str) -> bool {
    let pre = format!("{}\\", under.to_lowercase());
    let mut n = 0;
    for (e, d) in item.entries.iter().zip(&item.decisions) {
        let p = e.path.to_lowercase();
        if p == under.to_lowercase() || p.starts_with(&pre) {
            n += 1;
            if !matches!(d, Decision::Skip { reason } if f(reason)) {
                return false;
            }
        }
    }
    n > 0
}

fn all_delete(item: &action::PlanItem, under: &str) -> bool {
    let pre = format!("{}\\", under.to_lowercase());
    let sel: Vec<&Decision> =
        item.entries.iter().zip(&item.decisions).filter(|(e, _)| e.path.to_lowercase() == under.to_lowercase() || e.path.to_lowercase().starts_with(&pre)).map(|(_, d)| d).collect();
    !sel.is_empty() && sel.iter().all(|d| matches!(d, Decision::Delete))
}

#[test]
fn temp_children_judged_by_age_and_running_or_busy_children_skipped_whole() {
    let base = std::env::temp_dir().join(format!("diskwhy-units-{}", std::process::id()));
    let _tmp = Temp(base.clone());
    let t = base.join("Temp");
    let old = 10 * DAY;
    write_aged(&t.join(r"oldA\a.bin"), &[1u8; 3000], old);
    write_aged(&t.join(r"oldA\sub\b.bin"), &[2u8; 3000], old);
    write_aged(&t.join(r"mixed\old.bin"), &[3u8; 3000], old);
    write_aged(&t.join(r"mixed\deep\fresh.bin"), &[4u8; 3000], 2 * 3600); // 2 小时前：不算“30 分钟内”，但在 7 天内
    write_aged(&t.join("old.txt"), b"old", old);
    write_aged(&t.join("new.txt"), b"new", 3 * DAY);
    write_aged(&t.join(r"held\data.bin"), &[5u8; 4000], old);
    write_aged(&t.join(r"held\other.bin"), &[6u8; 4000], old);
    write_aged(&t.join(r"runner\data.bin"), &[7u8; 4000], old);
    // 别的规则（Foo 缓存，safe）的文件夹里有程序在运行
    let foo = base.join("Foo");
    write_aged(&foo.join(r"x\cache.bin"), &[8u8; 4000], old);

    let root = base.to_string_lossy().to_string();
    let tdir = t.to_string_lossy().to_string();
    let env = EnvMap::from_pairs(&[("TEMP", tdir.as_str()), ("LOCALAPPDATA", root.as_str())]);
    let (mut rules, errs) = RuleSet::embedded(&env);
    assert!(errs.is_empty(), "{errs:?}");
    let (extra, e2) = RuleSet::from_sources(&[("t", FOO_RULE)], &env);
    assert!(e2.is_empty());
    let off = rules.rules.len();
    for (ri, p) in extra.patterns {
        rules.patterns.push((ri + off, p));
    }
    rules.rules.extend(extra.rules);
    let ctx = ExplainCtx::new(env, rules, RegistrySnapshot::default(), LlmSettings::default());

    // 运行中的 exe（复制进来、改成 10 天前）
    let _p1 = run_copy(&t.join(r"runner\bin\pingcopy.exe"));
    let _p2 = run_copy(&foo.join(r"bin\pingfoo.exe"));
    let held = std::fs::File::open(t.join(r"held\data.bin")).unwrap();

    let vol = volume_space(&volume_root_of(&root).unwrap()).unwrap();
    let r = scan_with(&Win32Fs, &root, vol, false, &ScanOptions::default(), &AtomicBool::new(false), |_| {}).unwrap();

    // 清理页 / 解释：只算 7 天以上没改过的直接子项（按扫描树里的修改时间）
    let groups = cleanup_candidates(&r.tree, &ctx, &|_| false);
    let g = groups.iter().find(|g| g.key == "user-temp").expect("临时文件候选");
    assert_eq!(g.tier, Tier::Safe);
    assert_eq!(g.paths.len(), 1);
    let new_bytes: u64 = ["mixed", "new.txt"].iter().map(|n| explain_path(&r.tree, &t.join(n).to_string_lossy(), &ctx).unwrap().1.gain_bytes).sum();
    assert_eq!(new_bytes, 0, "最近 7 天改过的不算能腾出");
    let e_root = explain_path(&r.tree, &tdir, &ctx).unwrap().1;
    assert_eq!(g.deletable_bytes, e_root.gain_bytes);
    let mixed_alloc = view::find_node(&r.tree, &t.join("mixed").to_string_lossy()).unwrap().1.allocated;
    let temp_alloc = view::find_node(&r.tree, &tdir).unwrap().1.allocated;
    assert!(g.deletable_bytes < temp_alloc - mixed_alloc + 1, "{} vs {}", g.deletable_bytes, temp_alloc);

    // 预检
    let prot = Protector::from_system();
    let reqs = vec![
        PlanRequest { path: tdir.clone(), expect_rule: Some("user-temp".into()) },
        PlanRequest { path: foo.to_string_lossy().to_string(), expect_rule: Some("foo-cache".into()) },
    ];
    let mut plan = action::build_plan(&r, &ctx, &prot, &reqs, Strategy::RecycleBin);
    assert!(plan.items.iter().all(|i| i.blocked.is_none()), "{:?}", plan.items.iter().map(|i| &i.blocked).collect::<Vec<_>>());
    assert_eq!(plan.items[0].age_root.as_deref(), Some(tdir.as_str()));
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let it = &plan.items[0];
    let s = |n: &str| t.join(n).to_string_lossy().to_string();
    assert!(all_delete(it, &s("oldA")), "整个子文件夹都旧：删");
    assert!(all_delete(it, &s("old.txt")));
    assert!(all_skip(it, |r| matches!(r, SkipReason::Recent { days: 7 }), &s("mixed")), "子文件夹里有一个新文件：整个不动 {:?}", it.decisions);
    assert!(all_skip(it, |r| matches!(r, SkipReason::Recent { .. }), &s("new.txt")));
    assert!(
        all_skip(it, |r| matches!(r, SkipReason::Running { processes } if processes.iter().any(|p| p.name.eq_ignore_ascii_case("pingcopy.exe"))), &s("runner")),
        "有程序从里面运行：整个子文件夹不动（连 data.bin 也不删）"
    );
    assert!(all_skip(it, |r| matches!(r, SkipReason::Locked { .. }), &s("held")), "有文件被占用：整个子文件夹不动");
    let fi = &plan.items[1];
    assert!(
        fi.decisions.iter().all(|d| matches!(d, Decision::Skip { reason: SkipReason::Running { .. } })),
        "别的规则的清单项里有程序在运行：整项不动 {:?}",
        fi.decisions
    );

    // 执行开始时再查一遍：还在运行 → 拦下 / 跳过
    let (items, prefixes) = action::preflight::busy_at_start(&plan, diskwhy_core::explain::now_unix());
    assert!(items.iter().any(|(ii, why)| *ii == 1 && why.contains("正在运行")), "{items:?}");
    assert!(prefixes.iter().any(|p| p == &format!("{}\\", s("runner").to_lowercase())), "{prefixes:?}");
    assert!(!prefixes.iter().any(|p| p.starts_with(&s("olda").to_lowercase())), "没人用的旧子项不拦：{prefixes:?}");
    drop(held);
}
