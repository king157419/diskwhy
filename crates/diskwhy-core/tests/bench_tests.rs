//! 阶段 J 执行引擎基准（`#[ignore]`，手动跑）：
//!
//! ```text
//! set DISKWHY_BENCH_DIR=D:\desk\claudex\13\dw-test-bench   （里面 <case>\cache\… 由 mkbench.py 事先建好）
//! set DISKWHY_BENCH_CASE=perm | quar | recycle_dir | recycle_file | recycle_file1 | perm1
//! cargo test --release -p diskwhy-core --test bench_tests -- --ignored --nocapture
//! ```
//!
//! 只动 `DISKWHY_BENCH_DIR\<case>\cache` 里由脚本建的文件；回收站模式测完把自己的东西从回收站挪回来再删掉。
#![cfg(windows)]

use diskwhy_core::action::log::ActionLog;
use diskwhy_core::action::protect::Protector;
use diskwhy_core::action::{self, quarantine, ExecOptions, PlanRequest, PreflightOptions, Strategy};
use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::LlmSettings;
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::RuleSet;
use diskwhy_core::explain::ExplainCtx;
use diskwhy_core::scan::{scan_with, volume_root_of};
use diskwhy_core::win::{volume_space, Win32Fs};
use diskwhy_core::ScanOptions;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

const RULES: &str = r#"
[[rule]]
id = "bench-cache"
owner = "基准"
category = "cache"
tier = "safe"
match = ['%TEMP%\cache']
what = "基准缓存"
why = "基准"
lose = "不丢数据"
how = "delete"
how_detail = ""
refs = ["https://example.com/"]
verified_on = "2026-10-06"
"#;

/// 回收站里原路径在 `under` 下面的项挪回原处（只动我们自己的），返回个数。
fn restore_under(vol: &str, under: &str) -> usize {
    let rb = Path::new(vol).join("$Recycle.Bin");
    let under_l = under.to_lowercase();
    let mut n = 0;
    for sid in std::fs::read_dir(&rb).into_iter().flatten().flatten() {
        let Ok(rd) = std::fs::read_dir(sid.path()) else { continue };
        for f in rd.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            if !fname.starts_with("$I") {
                continue;
            }
            let Ok(b) = std::fs::read(f.path()) else { continue };
            if b.len() < 28 || u64::from_le_bytes(b[0..8].try_into().unwrap()) != 2 {
                continue;
            }
            let chars = u32::from_le_bytes(b[24..28].try_into().unwrap()) as usize;
            let u: Vec<u16> = b[28..(28 + chars * 2).min(b.len())].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            let orig = String::from_utf16_lossy(&u);
            if orig.to_lowercase().starts_with(&under_l) {
                let r = sid.path().join(format!("$R{}", &fname[2..]));
                if let Some(p) = Path::new(&orig).parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                if std::fs::rename(&r, &orig).is_ok() {
                    let _ = std::fs::remove_file(f.path());
                    n += 1;
                }
            }
        }
    }
    n
}

#[test]
#[ignore]
fn bench_execute() {
    let base = std::env::var("DISKWHY_BENCH_DIR").expect("DISKWHY_BENCH_DIR");
    let case = std::env::var("DISKWHY_BENCH_CASE").expect("DISKWHY_BENCH_CASE");
    // xvol：源在 %TEMP%（C 盘，只放少量小文件），暂存到 DISKWHY_BENCH_DIR 所在的盘
    let root = if case == "xvol" { format!(r"{}\dw-bench\xvol", std::env::temp_dir().display().to_string().trim_end_matches('\\')) } else { format!(r"{base}\{case}") };
    let cache = format!(r"{root}\cache");
    assert!(Path::new(&cache).is_dir(), "先用 mkbench.py 建 {cache}");
    let env = EnvMap::from_pairs(&[("TEMP", root.as_str())]);
    let (rules, errs) = RuleSet::from_sources(&[("bench", RULES)], &env);
    assert!(errs.is_empty(), "{errs:?}");
    let ctx = ExplainCtx::new(env, rules, RegistrySnapshot::default(), LlmSettings::default());
    let t = Instant::now();
    let vol = volume_root_of(&root).unwrap();
    let r = scan_with(&Win32Fs, &root, volume_space(&vol).unwrap(), false, &ScanOptions::default(), &AtomicBool::new(false), |_| {}).unwrap();
    let scan_s = t.elapsed().as_secs_f64();
    let prot = Protector::from_system();
    let (strategy, safe_direct, dir_recycle, workers) = match case.as_str() {
        "perm" => (Strategy::Permanent, true, true, 0),
        "perm1" => (Strategy::Permanent, true, true, 1),
        "quar" => (Strategy::Quarantine { dest_root: format!(r"{base}\q") }, false, true, 0),
        "xvol" => (Strategy::Quarantine { dest_root: format!(r"{base}\q") }, false, true, 0),
        "recycle_dir" => (Strategy::RecycleBin, false, true, 0),
        "recycle_file" => (Strategy::RecycleBin, false, false, 0),
        "recycle_file1" => (Strategy::RecycleBin, false, false, 1),
        c => panic!("不认识的 case {c}"),
    };
    let t = Instant::now();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[PlanRequest { path: cache.clone(), expect_rule: None }], strategy);
    plan.safe_direct = safe_direct;
    let plan_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let pre_s = t.elapsed().as_secs_f64();
    assert!(rep.can_execute, "{:?}", rep.warnings);
    let log = ActionLog::open(Path::new(&format!(r"{base}\{case}-actions.log")));
    let mut o = ExecOptions::new(&log);
    o.dir_recycle = dir_recycle;
    if workers > 0 {
        o.workers = workers;
    }
    let t = Instant::now();
    let ex = action::execute(&plan, &prot, &o, |_| {}).unwrap();
    let exec_s = t.elapsed().as_secs_f64();
    let n = rep.total_entries;
    println!(
        "BENCH case={case} entries={n} skipped={} est_secs={:.1} | scan {scan_s:.1}s plan {plan_s:.1}s preflight {pre_s:.1}s | execute {exec_s:.2}s = {:.0} files/s (workers {}) ok={} failed={} skipped={} dirs_removed={} by_dest={:?}",
        rep.total_skipped,
        rep.est_secs,
        n as f64 / exec_s,
        ex.workers,
        ex.ok,
        ex.failed,
        ex.skipped,
        ex.items.iter().map(|i| i.dirs_removed).sum::<usize>(),
        ex.by_dest
    );
    let m = &ex.measured;
    let per = |c: (usize, f64)| if c.0 > 0 { c.1 / c.0 as f64 } else { 0.0 };
    println!(
        "MEASURED permanent {:.3} ms/个 ({}) | recycle {:.3} ms/个 ({}) | rename {:.3} ms/个 ({}) | rmdir {:.3} ms/个 ({}) | busy {:.0} ms / wall {:.0} ms × {} 线程 → 并行效率 {:.2}",
        per(m.permanent),
        m.permanent.0,
        per(m.recycle),
        m.recycle.0,
        per(m.rename),
        m.rename.0,
        per(m.rmdir),
        m.rmdir.0,
        m.busy_ms(),
        ex.duration_ms,
        ex.workers,
        m.busy_ms() / (ex.duration_ms.max(1) as f64 * ex.workers as f64)
    );
    if case.starts_with("recycle") {
        let t = Instant::now();
        let back = restore_under(&vol, &cache);
        println!("从回收站挪回 {back} 项（{:.1}s）", t.elapsed().as_secs_f64());
    }
    if let Some(mp) = &ex.manifest_path {
        let t = Instant::now();
        let pr = quarantine::purge(Path::new(mp), &log).unwrap();
        println!("清空暂存批次 {} 个（{:.1}s，removed={}）", pr.deleted_entries, t.elapsed().as_secs_f64(), pr.removed);
    }
}
