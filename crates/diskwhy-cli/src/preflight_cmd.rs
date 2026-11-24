//! `diskwhy-cli preflight <root> [--from snap] [--profile]`：按清理页的**默认勾选**（可放心清理、能直接删的全部组）
//! 生成计划并预检，打印每一步用时。只读：不删、不移动任何文件（和 GUI 点「清理」后、确认之前做的事完全一样）。
//!
//! `--profile` 另外把预检的各个环节单独重跑一遍计时（枚举 / 取指纹 / 保护路径 / 占用探测 / Restart Manager），
//! 用来找慢在哪里。单独重跑时文件系统缓存已经是热的，数字只用来比较各环节的相对大小。

use crate::legacy::Args;
use diskwhy_core::action::protect::Protector;
use diskwhy_core::action::{self, winops, Decision, EntryKind, PlanRequest, PreflightOptions, Progress, SkipReason, Strategy};
use diskwhy_core::explain::candidates::cleanup_candidates;
use diskwhy_core::explain::{ExplainCtx, Tier};
use diskwhy_core::view::human_bytes;
use diskwhy_core::ScanResult;
use std::collections::BTreeMap;
use std::time::Instant;

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

pub fn run(r: &ScanResult, args: &Args) {
    let t = Instant::now();
    let (ctx, _) = ExplainCtx::load_default();
    let protector = Protector::from_system();
    println!("规则 + 保护路径加载 {:.2} 秒", secs(t));

    let t = Instant::now();
    let cands = cleanup_candidates(&r.tree, &ctx, &|p: &str| protector.check(p).is_some());
    let groups: Vec<_> = cands.iter().filter(|g| g.tier == Tier::Safe && g.deletable_bytes > 0).collect();
    let reqs: Vec<PlanRequest> = groups
        .iter()
        .flat_map(|g| g.paths.iter().filter(|p| p.deletable).map(|p| PlanRequest { path: p.path.clone(), expect_rule: g.rule_id.clone() }))
        .collect();
    let sel_bytes: u64 = groups.iter().map(|g| g.deletable_bytes).sum();
    println!(
        "清理候选 {:.2} 秒：默认勾选 {} 组、{} 个路径、扫描时 {}",
        secs(t),
        groups.len(),
        reqs.len(),
        human_bytes(sel_bytes)
    );

    // ── 和 GUI 一样：快速计划（弹层立刻出现）→ 枚举 → 逐个检查 ──
    let t_all = Instant::now();
    let t = Instant::now();
    let mut plan = action::build_plan_quick(r, &ctx, &protector, &reqs, Strategy::RecycleBin);
    let q = action::quote(&plan);
    println!(
        "快速计划 + 估算 {:.3} 秒（GUI 此时弹出确认框）：{} 项，被拦下 {}，估算 {}",
        secs(t),
        plan.items.len(),
        plan.items.iter().filter(|i| i.blocked.is_some()).count(),
        human_bytes(q.total_bytes)
    );
    let prog = std::sync::Arc::new(Progress::default());
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ticker = {
        let (prog, done) = (prog.clone(), done.clone());
        let t0 = Instant::now();
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1000));
                let v = prog.view();
                eprintln!(
                    "  [{:>5.1}s] {} 文件夹 {} 条目 {} 已检查 {}/{} 占用 {} RM {}/{}",
                    t0.elapsed().as_secs_f64(), v.phase, v.dirs, v.entries, v.checked, v.to_check, v.locked, v.rm_done, v.rm_total
                );
            }
        })
    };
    let t = Instant::now();
    action::enumerate_plan(&mut plan, &prog);
    let entries: usize = plan.items.iter().map(|i| i.entries.len()).sum();
    let dirs: usize = plan.items.iter().map(|i| i.dirs.len()).sum();
    println!("枚举 {:.2} 秒：{} 个条目、{} 个子文件夹", secs(t), entries, dirs);
    let t = Instant::now();
    let rep = action::preflight_with(&mut plan, &protector, &PreflightOptions::default(), &prog);
    println!("逐个检查 {:.2} 秒，分段（毫秒）：{:?}", secs(t), rep.timing_ms);
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = ticker.join();
    println!("合计（点清理 → 检查完成）{:.2} 秒", secs(t_all));
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    for it in &rep.items {
        for (k, n) in &it.skipped_by_reason {
            *by.entry(k.clone()).or_default() += n;
        }
    }
    println!(
        "结果：会删 {} 个、{}；跳过 {} 个 {:?}；占用进程 {:?}；can_execute = {}",
        rep.total_entries,
        human_bytes(rep.total_bytes),
        rep.total_skipped,
        by,
        rep.locked_processes.iter().map(|p| format!("{}#{}", p.name, p.pid)).collect::<Vec<_>>(),
        rep.can_execute
    );
    let mut big: Vec<_> = plan.items.iter().map(|i| (i.entries.len(), i.path.clone(), i.blocked.clone())).collect();
    big.sort_by(|a, b| b.0.cmp(&a.0));
    println!("条目最多的项：");
    for (n, p, b) in big.iter().take(8) {
        println!("  {:>8}  {}{}", n, p, b.as_ref().map(|x| format!("  [拦下：{x}]")).unwrap_or_default());
    }

    if !args.profile {
        return;
    }
    // ── 分环节单独计时（热缓存） ──
    println!("\n── 分环节单独重跑（热缓存）──");
    let files: Vec<&action::Entry> = plan.items.iter().filter(|i| i.blocked.is_none()).flat_map(|i| i.entries.iter()).filter(|e| e.kind == EntryKind::File).collect();
    let n = files.len().max(1) as f64;

    let t = Instant::now();
    let mut listed = 0usize;
    for it in plan.items.iter().filter(|i| i.blocked.is_none() && i.is_dir) {
        let mut stack = vec![it.path.clone()];
        while let Some(d) = stack.pop() {
            if let Ok(l) = winops::list_dir(&d) {
                for e in l {
                    listed += 1;
                    if e.attrs & 0x10 != 0 && e.attrs & 0x400 == 0 {
                        stack.push(diskwhy_core::fs::join(&d, &e.name));
                    }
                }
            }
        }
    }
    let s = secs(t);
    println!("只列目录（FindFirstFileExW）  {:>7.2} 秒  {} 条", s, listed);

    // 并行打开的吞吐（16 线程按目录顺序分块）：三种打开方式各跑一遍
    let paths: Vec<&str> = files.iter().map(|e| e.path.as_str()).collect();
    let par = |f: &(dyn Fn(&str) + Sync)| {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 16);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let t = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| loop {
                    let b = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if b * 64 >= paths.len() {
                        break;
                    }
                    for p in &paths[b * 64..((b + 1) * 64).min(paths.len())] {
                        f(p);
                    }
                });
            }
        });
        secs(t)
    };
    let s = par(&|p| {
        let _ = winops::fingerprint(p, true);
    });
    println!("并行：只读属性打开 + 取指纹         {:>7.2} 秒  {:.1} µs/个", s, s * 1e6 / n);
    let s = par(&|p| {
        let _ = winops::delete_probe(p, false);
    });
    println!("并行：只 DELETE 打开（占用探测）    {:>7.2} 秒  {:.1} µs/个", s, s * 1e6 / n);
    let s = par(&|p| {
        let _ = winops::probe_and_fingerprint(p, false);
    });
    println!("并行：一次打开探测 + 指纹（新）     {:>7.2} 秒  {:.1} µs/个", s, s * 1e6 / n);

    let t = Instant::now();
    for e in &files {
        let _ = winops::fingerprint(&e.path, true);
    }
    let s = secs(t);
    println!("取指纹 fingerprint            {:>7.2} 秒  {:.1} µs/个", s, s * 1e6 / n);

    let t = Instant::now();
    for e in &files {
        let _ = protector.entry_blocked(&e.path);
    }
    let s = secs(t);
    println!("保护路径 entry_blocked（旧）  {:>7.2} 秒  {:.1} µs/个", s, s * 1e6 / n);

    let t = Instant::now();
    for e in &files {
        let _ = diskwhy_core::action::protect::normalize(&e.path).map(|nn| protector.check_segs(&nn.segs));
    }
    let s = secs(t);
    println!("  其中纯字符串 check_segs      {:>7.2} 秒  {:.1} µs/个", s, s * 1e6 / n);

    let t = Instant::now();
    for e in &files {
        if let Some((parent, _)) = e.path.rsplit_once('\\') {
            let _ = winops::final_path(parent);
        }
    }
    let s = secs(t);
    println!("  其中父目录 final_path        {:>7.2} 秒  {:.1} µs/个", s, s * 1e6 / n);

    let t = Instant::now();
    let mut locked = Vec::new();
    for e in &files {
        if let Err(c) = winops::delete_probe(&e.path, false) {
            if winops::is_sharing_violation(c) {
                locked.push(e.path.clone());
            }
        }
    }
    let s = secs(t);
    println!("占用探测 delete_probe          {:>7.2} 秒  {:.1} µs/个，被占用 {} 个", s, s * 1e6 / n, locked.len());

    if !locked.is_empty() {
        let t = Instant::now();
        let procs = winops::rm_locking_processes(&locked.iter().take(1000).cloned().collect::<Vec<_>>());
        println!("Restart Manager（被占用的 {} 个一组）  {:.2} 秒 → {:?}", locked.len().min(1000), secs(t), procs.map(|v| v.len()));
        // 按清单项分组
        let mut by_item: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for p in &locked {
            let it = plan.items.iter().find(|i| p.to_lowercase().starts_with(&i.path.to_lowercase())).map(|i| i.path.clone()).unwrap_or_default();
            by_item.entry(it).or_default().push(p.clone());
        }
        println!("被占用文件分布在 {} 个清单项：", by_item.len());
        let mut sizes: Vec<_> = by_item.iter().map(|(k, v)| (v.len(), k.clone())).collect();
        sizes.sort_by(|a, b| b.0.cmp(&a.0));
        for (n, k) in sizes.iter().take(10) {
            println!("  {n:>5}  {k}");
        }
        let t = Instant::now();
        let mut tot = 0;
        for v in by_item.values() {
            tot += winops::rm_locking_processes(v).map(|x| x.len()).unwrap_or(0);
        }
        println!("RM 按清单项分组、依次问  {:.2} 秒 → 进程合计 {tot}", secs(t));
        let t = Instant::now();
        let mut tot = 0;
        let mut calls = 0;
        for v in by_item.values() {
            for f in v.iter().take(1) {
                calls += 1;
                tot += winops::rm_locking_processes(std::slice::from_ref(f)).map(|x| x.len()).unwrap_or(0);
            }
        }
        println!("RM 每个清单项只问 1 个文件（{calls} 次）  {:.2} 秒 → 进程合计 {tot}", secs(t));
        let t = Instant::now();
        let mut per = Vec::new();
        for f in locked.iter().take(20) {
            let t1 = Instant::now();
            let n = winops::rm_locking_processes(std::slice::from_ref(f)).map(|x| x.len()).unwrap_or(0);
            per.push(format!("{:.0}ms/{n}", t1.elapsed().as_secs_f64() * 1000.0));
        }
        println!("RM 逐个问前 20 个  {:.2} 秒：{}", secs(t), per.join(" "));
    }
    for k in [1usize, 100, 1000] {
        let sample: Vec<String> = files.iter().take(k).map(|e| e.path.clone()).collect();
        let t = Instant::now();
        let res = winops::rm_locking_processes(&sample);
        println!("Restart Manager（{} 个普通文件一组）  {:.3} 秒 → {:?}", sample.len(), secs(t), res.map(|v| v.len()));
    }
    let hot = plan
        .items
        .iter()
        .flat_map(|i| i.decisions.iter())
        .filter(|d| matches!(d, Decision::Skip { reason: SkipReason::Hot { .. } }))
        .count();
    println!("（热文件 {hot} 个：只比较修改时间，不另外开文件）");
}
