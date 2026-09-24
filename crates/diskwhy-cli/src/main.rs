//! diskwhy-cli：命令行扫描工具。
//!
//! 用法：
//!   diskwhy-cli scan <root> [--top 30] [--json out.json] [--json-depth 3] [--no-explain]
//!                           [--mode exact|fast] [--threads N] [--keep-files 50] [--quiet]
//!     `--json` 输出的目录树里，每个目录节点带解释摘要（explanation：tier / owner / source / what），
//!     前 N 大目录（top_dirs）带完整解释；前 N 大目录连同它们的上层文件夹都保留在树里，
//!     所以之后可以 `explain --from out.json` 接着解释，不用重扫。
//!
//!   diskwhy-cli explain <路径> [--top 50] [--json out.json|-] [--report r.json] [--md c.md] [--llm] [--quiet]
//!     扫描这个路径，解释它本身（第 0 行），再对它下面前 N 大目录逐个解释；`--top 0` 只解释它本身。
//!   diskwhy-cli explain --from scan.json|x.dwsnap [<路径>] [--top 50] [...]
//!     不扫描，直接对 scan 的 JSON（或快照）解释；给了路径就从那个文件夹开始。
//!     `--json` 输出 Explanation 数组（每项多带 rank / path / allocated），`-` 表示写到标准输出。
//!     `--llm`：对规则和启发式都认不出的目录问 AI（还需要设置里已启用；默认不开，不联网）。
//!     `--no-reread`：JSON 里被截断的文件夹不回磁盘补读（默认会补读，只读不写）。
//!
//! 阶段 C：
//!   --no-dedup          关掉硬链接 / 目录映射去重（用于对比耗时和结果）
//!   --snapshot <file>   扫描后把完整结果存成快照（gzip JSON，.dwsnap）
//!   --from <file>       不扫描，直接读快照（或 --json 输出）再出报告

use diskwhy_core::explain::ExplainCtx;
use diskwhy_core::scan::scan;
use diskwhy_core::view::{explain_at, export_tree, find_node, human_bytes, node_view, outer_ancestors, top_dirs, NodeView};
use diskwhy_core::*;
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

mod explain_cmd;

const USAGE: &str = "用法：
  diskwhy-cli scan <root> [--top 30] [--json out.json] [--json-depth 3] [--no-explain] [--mode exact|fast] [--threads N] [--keep-files 50] [--no-dedup] [--snapshot out.dwsnap] [--quiet]
  diskwhy-cli explain <路径> [--top 50] [--json out.json|-] [--report report.json] [--md coverage.md] [--llm] [--mode exact|fast] [--no-dedup] [--quiet]
  diskwhy-cli explain --from scan.json|snapshot.dwsnap [<路径>] [--top 50] [--json out.json|-] [--report report.json] [--md coverage.md] [--llm] [--no-reread]
  diskwhy-cli scan --from snapshot.dwsnap [--top N]   （读快照，不扫描）

  explain：第 0 行是 <路径> 本身，后面是它下面前 N 大目录；--top 0 只解释 <路径> 本身。
  --json 写 Explanation 数组（- 表示标准输出）；--report 写含统计的完整报告；
  --llm 只对认不出的目录问 AI，而且设置里要先启用（默认都不开，不联网）。";

#[derive(PartialEq)]
pub enum Cmd {
    Scan,
    Explain,
}

pub struct Args {
    pub cmd: Cmd,
    pub md: Option<String>,
    pub report: Option<String>,
    pub root: String,
    pub top: usize,
    pub json: Option<String>,
    pub json_depth: usize,
    pub mode: SizeMode,
    pub threads: usize,
    pub keep_files: usize,
    pub quiet: bool,
    pub no_dedup: bool,
    pub no_explain: bool,
    pub llm: bool,
    pub no_reread: bool,
    pub snapshot: Option<String>,
    pub from: Option<String>,
}

impl Args {
    pub fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            top_files_per_dir: self.keep_files,
            size_mode: self.mode,
            threads: self.threads,
            dedup: !self.no_dedup,
            ..Default::default()
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = match it.next().as_deref() {
        Some("scan") => Cmd::Scan,
        Some("explain") => Cmd::Explain,
        Some("-h") | Some("--help") | None => return Err(USAGE.into()),
        Some(other) => return Err(format!("未知子命令 {other}\n{USAGE}")),
    };
    let default_top = if cmd == Cmd::Explain { 50 } else { 30 };
    let mut a = Args {
        cmd,
        md: None,
        report: None,
        root: String::new(),
        top: default_top,
        json: None,
        json_depth: 3,
        mode: SizeMode::Exact,
        threads: 0,
        keep_files: 50,
        quiet: false,
        no_dedup: false,
        no_explain: false,
        llm: false,
        no_reread: false,
        snapshot: None,
        from: None,
    };
    while let Some(x) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} 缺少参数值"));
        match x.as_str() {
            "--top" => a.top = val("--top")?.parse().map_err(|_| "--top 需要数字")?,
            "--json" => a.json = Some(val("--json")?),
            "--md" => a.md = Some(val("--md")?),
            "--report" => a.report = Some(val("--report")?),
            "--json-depth" => a.json_depth = val("--json-depth")?.parse().map_err(|_| "--json-depth 需要数字")?,
            "--threads" => a.threads = val("--threads")?.parse().map_err(|_| "--threads 需要数字")?,
            "--keep-files" => a.keep_files = val("--keep-files")?.parse().map_err(|_| "--keep-files 需要数字")?,
            "--mode" => {
                a.mode = match val("--mode")?.as_str() {
                    "exact" => SizeMode::Exact,
                    "fast" => SizeMode::Fast,
                    m => return Err(format!("未知模式 {m}（exact|fast）")),
                }
            }
            "--quiet" | "-q" => a.quiet = true,
            "--no-dedup" => a.no_dedup = true,
            "--no-explain" => a.no_explain = true,
            "--llm" => a.llm = true,
            "--no-reread" => a.no_reread = true,
            "--snapshot" => a.snapshot = Some(val("--snapshot")?),
            "--from" => a.from = Some(val("--from")?),
            s if s.starts_with("--") => return Err(format!("未知参数 {s}\n{USAGE}")),
            s => {
                if a.root.is_empty() {
                    a.root = s.to_string()
                } else {
                    return Err(format!("多余参数 {s}"));
                }
            }
        }
    }
    if a.root.is_empty() && a.from.is_none() {
        return Err(format!("缺少路径\n{USAGE}"));
    }
    if a.cmd == Cmd::Scan && (a.llm || a.report.is_some()) {
        return Err("--llm / --report 只能用在 explain 上".into());
    }
    Ok(a)
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let opts = args.scan_options();
    let cancel = AtomicBool::new(false);
    let quiet = args.quiet;
    let mut last_print = Instant::now();
    if let Some(from) = args.from.clone() {
        let t = Instant::now();
        let loaded = snapshot::with_big_stack(move || snapshot::load(std::path::Path::new(&from)));
        match loaded {
            Ok((r, meta)) => {
                eprintln!(
                    "已读取{}：{}（保存于 Unix {}，程序版本 {}），读取用时 {:.2} 秒",
                    if meta.kind == "snapshot" { "完整快照" } else { " scan --json 输出（目录树截断过）" },
                    args.from.as_deref().unwrap_or(""),
                    meta.saved_at,
                    if meta.app_version.is_empty() { "未知" } else { &meta.app_version },
                    t.elapsed().as_secs_f64()
                );
                let target = if args.root.is_empty() { None } else { Some(args.root.clone()) };
                save_then_report(r, &args, target);
            }
            Err(e) => {
                eprintln!("读取快照失败：{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    // explain 一个文件：扫它所在的文件夹，再解释这个文件。
    let mut scan_root = args.root.clone();
    let mut target = None;
    if args.cmd == Cmd::Explain && std::path::Path::new(&args.root).is_file() {
        let norm = scan::normalize_root(&args.root);
        if let Some((parent, _)) = norm.rsplit_once('\\') {
            scan_root = if parent.len() == 2 { format!("{parent}\\") } else { parent.to_string() };
            target = Some(norm.clone());
        }
    }
    let res = scan(&scan_root, &opts, &cancel, |p| {
        if quiet || last_print.elapsed().as_millis() < 500 {
            return;
        }
        last_print = Instant::now();
        let mut cur = p.current_path.clone();
        if cur.chars().count() > 70 {
            cur = format!("…{}", cur.chars().rev().take(69).collect::<Vec<_>>().into_iter().rev().collect::<String>());
        }
        eprint!(
            "\r[{:>5.1}s] 文件 {:>9} 目录 {:>8} 已扫 {:>10}  {:<72}",
            p.elapsed_ms as f64 / 1000.0,
            p.files,
            p.dirs,
            human_bytes(p.bytes),
            cur
        );
        let _ = std::io::stderr().flush();
    });
    if !quiet {
        eprintln!();
    }
    let r = match res {
        Ok(r) => r,
        Err(e) => {
            eprintln!("扫描失败：{e}");
            std::process::exit(1);
        }
    };
    save_then_report(r, &args, target);
}

/// 有 `--snapshot` 就先存快照（扫描结果或 `--from` 读进来的都行），再出报告。
fn save_then_report(r: ScanResult, args: &Args, target: Option<String>) {
    if let Some(p) = args.snapshot.clone() {
        let t = Instant::now();
        let r2 = std::sync::Arc::new(r);
        let r3 = r2.clone();
        let saved = snapshot::with_big_stack(move || snapshot::save(&r3, std::path::Path::new(&p), env!("CARGO_PKG_VERSION")));
        match saved {
            Ok(n) => eprintln!("快照已写入 {}（{}，用时 {:.2} 秒）", args.snapshot.as_deref().unwrap_or(""), human_bytes(n), t.elapsed().as_secs_f64()),
            Err(e) => {
                eprintln!("写快照失败：{e}");
                std::process::exit(1);
            }
        }
        let r = std::sync::Arc::try_unwrap(r2).unwrap_or_else(|a| (*a).clone());
        report(r, args, target);
    } else {
        report(r, args, target);
    }
}

fn report(r: ScanResult, args: &Args, target: Option<String>) {
    if args.cmd == Cmd::Explain {
        explain_cmd::run(&r, args, target.as_deref());
        return;
    }
    let used = r.used_bytes;
    println!("DiskWhy 扫描结果：{}", r.root);
    println!(
        "耗时 {:.1} 秒 | 文件 {} | 目录 {} | 无权限目录 {} | 管理员 {} | 大小模式 {:?} | 簇 {} B",
        r.duration_ms as f64 / 1000.0,
        r.files,
        r.dirs,
        r.inaccessible_total,
        if r.elevated { "是" } else { "否" },
        r.size_mode,
        r.cluster_size
    );
    println!(
        "卷：总容量 {} | 已用 {} ({} B) | 可用 {}",
        human_bytes(r.total_bytes),
        human_bytes(used),
        used,
        human_bytes(r.free_bytes)
    );
    println!();
    println!("前 {} 大目录（按实际占用 allocated，占比 = 占卷已用空间）：", args.top);
    println!("{:>4}  {:>11}  {:>6}  路径", "#", "占用", "占比");
    let tops = top_dirs(&r.tree, args.top);
    for (i, d) in tops.iter().enumerate() {
        let mut flag = String::new();
        if d.system_reserved {
            flag.push_str(" [系统保留]");
        }
        println!("{:>4}  {:>11}  {:>5.1}%  {}{}", i + 1, human_bytes(d.allocated), pct(d.allocated, used), d.path, flag);
    }
    println!();
    let b = &r.buckets;
    println!("已用空间归属（卷已用 {}）：", human_bytes(used));
    let row = |name: &str, v: u64, note: &str| {
        println!("  {:<14} {:>11}  {:>5.1}%  {}", name, human_bytes(v), pct(v, used), note);
    };
    row("已扫描", b.scanned, "");
    row(
        "无权限",
        b.inaccessible,
        &format!(
            "差额推算；无权限目录 {} 个（系统保留下 {} 个），含 NTFS 元数据",
            b.inaccessible_dirs_other + b.inaccessible_dirs_system,
            b.inaccessible_dirs_system
        ),
    );
    row(
        "系统保留",
        b.system_reserved,
        &format!("按名字量到 {}，差额兜底 {}", human_bytes(b.system_reserved - b.system_reserved_estimated), human_bytes(b.system_reserved_estimated)),
    );
    row(
        "云占位",
        b.cloud_placeholder,
        &format!("{} 个文件，逻辑大小 {}，未打开、本地按 0 计", b.cloud_placeholder_files, human_bytes(b.cloud_placeholder_logical)),
    );
    row("未归属", b.unattributed, if b.scope_is_volume_root { "" } else { "（扫描根不是卷根，含扫描范围外的内容）" });
    if b.overcount > 0 {
        let note = if b.dedup_enabled {
            "去重后仍多出来的部分（没识别出的重复计数，或扫描期间卷上数据在变）"
        } else {
            "没有去重：硬链接 / 目录映射被重复计数"
        };
        println!("  {:<14} -{:>10}  {note}", "重复计数", human_bytes(b.overcount));
    }
    if b.dedup_enabled {
        println!(
            "  去重：硬链接 {} 个多余名字、{}；商店应用目录映射 {} 处、{}（已从上面各项里扣除）",
            b.hardlink_dedup_files,
            human_bytes(b.hardlink_dedup),
            b.mapped_dirs,
            human_bytes(b.mapped_dedup)
        );
        for m in r.mapped_dirs.iter().take(10) {
            println!("    {} = {}（{}）", m.path, m.same_as, human_bytes(m.bytes));
        }
    } else {
        println!("  去重：未开启（--no-dedup 或旧快照）");
    }
    let total = b.total();
    println!(
        "  合计 {} B {} 卷已用 {} B（差额原始值 gap = {} B = {:+.2} GB）",
        total,
        if total == used as i128 { "==" } else { "!=" },
        used,
        b.gap,
        b.gap as f64 / (1u64 << 30) as f64
    );

    if let Some(path) = &args.json {
        match write_scan_json(&r, &tops, args, path) {
            Ok(msg) => println!("\n{msg}"),
            Err(e) => {
                eprintln!("写 JSON 失败：{e}");
                std::process::exit(1);
            }
        }
    }
}

/// scan --json：扫描结果 + 截断的目录树（每个目录节点带解释摘要）+ 前 N 大目录（带完整解释）。
fn write_scan_json(r: &ScanResult, tops: &[&Node], args: &Args, path: &str) -> Result<String, String> {
    let t0 = Instant::now();
    let loaded = if args.no_explain { None } else { Some(ExplainCtx::load_default()) };
    let ctx = loaded.as_ref().map(|(c, _)| c);
    let outer = outer_ancestors(&r.root);
    let keep: Vec<&str> = tops.iter().map(|d| d.path.as_str()).collect();
    let tree = export_tree(&r.tree, args.json_depth, &keep, &outer, ctx);
    let used = r.used_bytes;
    let top_views: Vec<NodeView> = tops
        .iter()
        .map(|d| {
            let mut v = node_view(d, None, used);
            if let (Some(ctx), Some((chain, node))) = (ctx, find_node(&r.tree, &d.path)) {
                let anc: Vec<&Node> = outer.iter().chain(chain.iter().copied()).collect();
                v.explanation = Some(explain_at(node, &d.path, &anc, ctx, None));
            }
            v
        })
        .collect();
    let explain_ms = t0.elapsed().as_millis() as u64;

    let mut head = r.clone_without_tree();
    head.inaccessible_dirs.truncate(200);
    let mut v = serde_json::to_value(&head).map_err(|e| e.to_string())?;
    v["tree"] = serde_json::to_value(&tree).map_err(|e| e.to_string())?;
    v["top_dirs"] = serde_json::to_value(&top_views).map_err(|e| e.to_string())?;
    v["json_tree_depth"] = args.json_depth.into();
    v["explain"] = match &loaded {
        Some((c, warnings)) => serde_json::json!({
            "rules_loaded": c.rules.rules.len(),
            "rule_warnings": warnings,
            "registry_apps": c.registry.apps.len(),
            "explain_ms": explain_ms,
            "note": "tree 里每个目录节点的 explanation 是摘要（tier / owner / source / what），不调 AI；top_dirs 里是完整解释。"
        }),
        None => serde_json::json!({ "disabled": true }),
    };
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(f), &v).map_err(|e| e.to_string())?;
    Ok(format!(
        "JSON 已写入 {path}（目录树保留 {} 层 + 前 {} 大目录及其上层；{}）",
        args.json_depth,
        tops.len(),
        if ctx.is_some() { format!("每个目录附解释摘要，解释用时 {explain_ms} ms") } else { "未附解释（--no-explain）".into() }
    ))
}

trait WithoutTree {
    fn clone_without_tree(&self) -> ScanResult;
}

impl WithoutTree for ScanResult {
    fn clone_without_tree(&self) -> ScanResult {
        ScanResult {
            root: self.root.clone(),
            total_bytes: self.total_bytes,
            used_bytes: self.used_bytes,
            free_bytes: self.free_bytes,
            cluster_size: self.cluster_size,
            tree: self.tree.clone_shallow(),
            buckets: self.buckets.clone(),
            duration_ms: self.duration_ms,
            files: self.files,
            dirs: self.dirs,
            elevated: self.elevated,
            inaccessible_dirs: self.inaccessible_dirs.clone(),
            inaccessible_total: self.inaccessible_total,
            size_mode: self.size_mode,
            top_files_per_dir: self.top_files_per_dir,
            mapped_dirs: self.mapped_dirs.clone(),
        }
    }
}
