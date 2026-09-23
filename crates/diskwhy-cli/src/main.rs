//! diskwhy-cli：命令行扫描工具。
//!
//! 用法：
//!   diskwhy-cli scan <root> [--top 30] [--json out.json] [--json-depth 3]
//!                           [--mode exact|fast] [--threads N] [--keep-files 50] [--quiet]
//!   diskwhy-cli explain <root> [--top 50] [--json out.json] [--md coverage.md] [--mode exact|fast] [--quiet]
//!     扫描后对前 N 大目录逐个解释（档位 / 来源 / 这是什么），统计规则 / 启发式 / 未知各多少。

use diskwhy_core::scan::scan;
use diskwhy_core::view::{human_bytes, node_view, top_dirs, truncate_tree, NodeView};
use diskwhy_core::*;
use serde::Serialize;
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

mod explain_cmd;

const USAGE: &str = "用法：
  diskwhy-cli scan <root> [--top 30] [--json out.json] [--json-depth 3] [--mode exact|fast] [--threads N] [--keep-files 50] [--quiet]
  diskwhy-cli explain <root> [--top 50] [--json out.json] [--md coverage.md] [--mode exact|fast] [--quiet]";

#[derive(PartialEq)]
enum Cmd {
    Scan,
    Explain,
}

struct Args {
    cmd: Cmd,
    md: Option<String>,
    root: String,
    top: usize,
    json: Option<String>,
    json_depth: usize,
    mode: SizeMode,
    threads: usize,
    keep_files: usize,
    quiet: bool,
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
        root: String::new(),
        top: default_top,
        json: None,
        json_depth: 3,
        mode: SizeMode::Exact,
        threads: 0,
        keep_files: 50,
        quiet: false,
    };
    while let Some(x) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} 缺少参数值"));
        match x.as_str() {
            "--top" => a.top = val("--top")?.parse().map_err(|_| "--top 需要数字")?,
            "--json" => a.json = Some(val("--json")?),
            "--md" => a.md = Some(val("--md")?),
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
    if a.root.is_empty() {
        return Err(format!("缺少扫描根目录\n{USAGE}"));
    }
    Ok(a)
}

#[derive(Serialize)]
struct JsonOut<'a> {
    #[serde(flatten)]
    result: &'a ScanResult,
    top_dirs: Vec<NodeView>,
    json_tree_depth: usize,
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
    let opts = ScanOptions {
        top_files_per_dir: args.keep_files,
        size_mode: args.mode,
        threads: args.threads,
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let quiet = args.quiet;
    let mut last_print = Instant::now();
    let res = scan(&args.root, &opts, &cancel, |p| {
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

    if args.cmd == Cmd::Explain {
        explain_cmd::run(&r, &args);
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
        println!("  {:<14} -{:>10}  硬链接等重复计数（阶段 C 去重）", "重复计数", human_bytes(b.overcount));
    }
    let total = b.total();
    println!(
        "  合计 {} B {} 卷已用 {} B（差额原始值 gap = {} B）",
        total,
        if total == used as i128 { "==" } else { "!=" },
        used,
        b.gap
    );

    if let Some(path) = &args.json {
        let mut trimmed = r.clone_shallow_tree(args.json_depth);
        trimmed.inaccessible_dirs.truncate(200);
        let out = JsonOut {
            result: &trimmed,
            top_dirs: tops.iter().map(|d| node_view(d, None, used)).collect(),
            json_tree_depth: args.json_depth,
        };
        match std::fs::File::create(path)
            .map_err(|e| e.to_string())
            .and_then(|f| serde_json::to_writer_pretty(std::io::BufWriter::new(f), &out).map_err(|e| e.to_string()))
        {
            Ok(()) => println!("\nJSON 已写入 {path}（目录树截到 {} 层）", args.json_depth),
            Err(e) => {
                eprintln!("写 JSON 失败：{e}");
                std::process::exit(1);
            }
        }
    }
}

trait ShallowTree {
    fn clone_shallow_tree(&self, depth: usize) -> ScanResult;
}

impl ShallowTree for ScanResult {
    fn clone_shallow_tree(&self, depth: usize) -> ScanResult {
        ScanResult {
            root: self.root.clone(),
            total_bytes: self.total_bytes,
            used_bytes: self.used_bytes,
            free_bytes: self.free_bytes,
            cluster_size: self.cluster_size,
            tree: truncate_tree(&self.tree, depth),
            buckets: self.buckets.clone(),
            duration_ms: self.duration_ms,
            files: self.files,
            dirs: self.dirs,
            elevated: self.elevated,
            inaccessible_dirs: self.inaccessible_dirs.clone(),
            inaccessible_total: self.inaccessible_total,
            size_mode: self.size_mode,
            top_files_per_dir: self.top_files_per_dir,
        }
    }
}
