//! 给 Agent 用的命令行：`scan` / `clean` / `explain`。
//!
//! 和 GUI（`src-tauri/src/lib.rs`）走**同一条 core 路径**：
//! - scan：`scan::scan` → `tier_summary` + `cleanup_candidates`（保护路径用 `Protector::from_system`）；
//!   结果存成快照 `<数据目录>\cli\scan-<盘>.dwsnap`，`clean` 默认用 60 分钟内的快照，不重扫（GUI 也是拿内存里的扫描结果做计划）。
//! - clean：候选组 → `PlanRequest { path, expect_rule = rule_id }`（和清理页勾选一样）→ `build_plan_quick`
//!   （后端重新解释、过保护路径 / 节点 / 档位 / 规则一致性闸）→ `enumerate_plan` → `preflight_with`
//!   （占用 / 正在运行 / 30 分钟内改过 / 云文件 / 权限 …）。没有 `--yes` 就到此为止（预演）；
//!   有 `--yes` 时 `action::execute`，选项和 GUI 的 `execute_plan` 一样（跳过被占用的、动作日志、perf.json、last-run.json），
//!   暂存区成功后 `remember_root`，再 `refresh_dirs` 更新快照。
//! - explain：快照里有这个路径就用快照，否则只扫这个路径（上层文件夹补成空壳，规则继承照样能算）。
//!
//! CLI 自己多加的护栏：review 档只能用 `--ids` 点名、永远不直接删除（只进回收站 / 暂存区）；
//! `--safe` 里预检时变成 review 的项拦下；keep / 未知 / 受保护的一律拒绝（core 的闸本来就会拦）。
//!
//! stdout：`--json` 时只有一个 JSON 文档（UTF-8）；进度和提示都在 stderr。

use diskwhy_core::action::log::ActionLog;
use diskwhy_core::action::preflight::{node_gate, tier_gate, ItemReport, PreflightReport};
use diskwhy_core::action::protect::Protector;
use diskwhy_core::action::quarantine::{self, DestStatus};
use diskwhy_core::action::runstate::RunState;
use diskwhy_core::action::{self, ByDest, CountBytes, Dest, ExecOptions, ExecReport, PerfModel, Plan, PlanRequest, PreflightOptions, Progress, Strategy};
use diskwhy_core::explain::candidates::{cleanup_candidates, CandidateGroup};
use diskwhy_core::explain::{llm, tier_summary, Category, ExplainCtx, Explanation, How, LoseKind, Source, Tier, TierSummary};
use diskwhy_core::scan::{is_volume_root, normalize_root, scan, volume_root_of};
use diskwhy_core::view::{find_node, human_bytes};
use diskwhy_core::win::list_fixed_drives;
use diskwhy_core::{snapshot, Node, NodeKind, ScanOptions, ScanResult};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 1;
pub const EXIT_REFUSED: i32 = 2;
pub const EXIT_PARTIAL: i32 = 3;

/// clean 默认接受多旧的快照（分钟）。
pub const DEFAULT_MAX_AGE_MIN: u64 = 60;

pub const HELP: &str = r#"diskwhy-cli - scan a Windows drive and clean it safely (DiskWhy command line, for agents and humans)
DiskWhy 命令行：扫描 C 盘、解释每一项、按和桌面版相同的护栏安全清理。

USAGE
  diskwhy-cli scan    [--drive C: | --path DIR] [--json] [--quiet]
  diskwhy-cli clean   (--safe | --ids ID[,ID...]) [--to delete|recycle|stash] [--stash-dir DIR]
                      [--yes] [--json] [--rescan] [--max-age MIN] [--drive C: | --path DIR] [--quiet]
  diskwhy-cli explain PATH [--json] [--quiet]
  diskwhy-cli dev ...           developer tools (dev scan / dev explain / dev preflight)

COMMANDS
  scan     Scan a drive (default: the system drive) and list cleanup candidates: stable id,
           tier (safe / review), size, paths, what it is, what is lost, how it comes back.
           The scan is cached under <data dir>\cli\ so `clean` does not have to rescan.
  clean    Without --yes: DRY RUN. Builds the plan and runs the preflight, then prints exactly
           what would be deleted and what would be skipped and why (in use, program running,
           modified in the last 30 min, protected, ...). Nothing is touched.
           With --yes: executes through the same guarded core path as the DiskWhy app and
           reports the bytes actually freed and everything that was skipped.
  explain  What the rules say about one path: tier, reason, and whether the CLI may clean it.

CLEAN OPTIONS
  --safe            all safe-tier candidates (what the app pre-selects)
  --ids a,b,c       candidates by id (from `scan`); review-tier items are only cleaned this way
  --to MODE         default: safe -> deleted directly, review -> recycle bin
                      delete   direct delete, no recycle bin (refused for review-tier items)
                      recycle  everything to the recycle bin
                      stash    review items to the stash folder (DiskWhy暂存区, kept 7 days, restorable
                               in the app) on another drive; safe items are still deleted directly
  --stash-dir DIR   stash location for --to stash (default: pick another fixed drive automatically)
  --yes             really do it (without it: dry run only)
  --rescan          ignore the cached scan and scan again
  --max-age MIN     use the cached scan only if it is younger than MIN minutes (default 60)

COMMON OPTIONS
  --drive C:        drive to scan / clean (default: system drive)
  --path DIR        scan / clean only inside a folder instead of a whole drive
  --json            exactly one UTF-8 JSON document on stdout, nothing else; progress -> stderr
  --quiet           no progress output on stderr

EXIT CODES
  0 ok   1 usage error   2 nothing done / refused   3 partial (some items skipped, failed or refused)

SAFETY
  Keep-tier, unknown and protected paths are always refused. Review-tier items need explicit
  --ids and are never deleted directly. Files in use, folders a program is running from and
  files modified in the last 30 minutes are skipped - no program is ever closed. Admin rights
  are not needed; anything that needs elevation is reported as skipped. Every action is logged
  to <data dir>\actions.log. Data dir: %LOCALAPPDATA%\DiskWhy (override with DISKWHY_DATA_DIR).
"#;

// ───────────────────────── 参数 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum To {
    Delete,
    Recycle,
    Stash,
}

#[derive(Debug, Default, Clone)]
pub struct Opts {
    pub json: bool,
    pub quiet: bool,
    pub drive: Option<String>,
    pub path: Option<String>,
    pub safe: bool,
    pub ids: Vec<String>,
    pub to: Option<To>,
    pub stash_dir: Option<String>,
    pub yes: bool,
    pub rescan: bool,
    pub max_age_min: Option<u64>,
    pub positional: Vec<String>,
}

/// 解析某个子命令的参数（不含子命令本身）。只接受这个子命令认得的开关。
pub fn parse_opts(cmd: &str, args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    let clean = cmd == "clean";
    let has_root = cmd == "scan" || cmd == "clean";
    while i < args.len() {
        let raw = &args[i];
        i += 1;
        let (flag, inline) = match raw.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (raw.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            let v = args.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))?;
            i += 1;
            Ok(v)
        };
        match flag.as_str() {
            "--json" => o.json = true,
            "--quiet" | "-q" => o.quiet = true,
            "--drive" if has_root => o.drive = Some(value("--drive")?),
            "--path" if has_root => o.path = Some(value("--path")?),
            "--safe" if clean => o.safe = true,
            "--ids" | "--id" if clean => {
                let v = value("--ids")?;
                o.ids.extend(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
            }
            "--to" if clean => {
                o.to = Some(match value("--to")?.to_ascii_lowercase().as_str() {
                    "delete" | "permanent" => To::Delete,
                    "recycle" | "recycle-bin" | "recyclebin" => To::Recycle,
                    "stash" | "quarantine" => To::Stash,
                    other => return Err(format!("--to must be delete, recycle or stash (got '{other}')")),
                })
            }
            "--stash-dir" if clean => o.stash_dir = Some(value("--stash-dir")?),
            "--yes" | "-y" if clean => o.yes = true,
            "--rescan" if clean => o.rescan = true,
            "--max-age" if clean => {
                o.max_age_min = Some(value("--max-age")?.parse().map_err(|_| "--max-age needs a number of minutes".to_string())?)
            }
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option for `{cmd}`: {s}")),
            _ => o.positional.push(raw.clone()),
        }
    }
    if o.drive.is_some() && o.path.is_some() {
        return Err("use either --drive or --path, not both".into());
    }
    match cmd {
        "explain" => {
            if o.positional.len() != 1 {
                return Err("explain needs exactly one PATH".into());
            }
        }
        _ if !o.positional.is_empty() => return Err(format!("unexpected argument: {}", o.positional[0])),
        _ => {}
    }
    if clean {
        if !o.safe && o.ids.is_empty() {
            return Err("clean needs --safe and/or --ids ID[,ID...] (see `diskwhy-cli scan` for ids)".into());
        }
        if o.stash_dir.is_some() && o.to != Some(To::Stash) {
            return Err("--stash-dir only makes sense with --to stash".into());
        }
    }
    Ok(o)
}

/// 入口：返回退出码。
pub fn run(argv: Vec<String>) -> i32 {
    let Some(cmd) = argv.first().cloned() else {
        print!("{HELP}");
        return EXIT_USAGE;
    };
    match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print!("{HELP}");
            return EXIT_OK;
        }
        "-V" | "--version" | "version" => {
            println!("diskwhy-cli {}", env!("CARGO_PKG_VERSION"));
            return EXIT_OK;
        }
        _ => {}
    }
    let rest = &argv[1..];
    let wants_json = rest.iter().any(|a| a == "--json");
    if rest.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return EXIT_OK;
    }
    if !matches!(cmd.as_str(), "scan" | "clean" | "explain") {
        return fail(wants_json, EXIT_USAGE, &format!("unknown command '{cmd}' (expected scan, clean, explain; see --help)"));
    }
    let o = match parse_opts(&cmd, rest) {
        Ok(o) => o,
        Err(e) => return fail(wants_json, EXIT_USAGE, &e),
    };
    let r = match cmd.as_str() {
        "scan" => cmd_scan(&o),
        "clean" => cmd_clean(&o),
        _ => cmd_explain(&o),
    };
    match r {
        Ok(code) => code,
        Err((code, msg)) => fail(o.json, code, &msg),
    }
}

type CmdResult = Result<i32, (i32, String)>;

fn usage(msg: impl Into<String>) -> (i32, String) {
    (EXIT_USAGE, msg.into())
}
fn refused(msg: impl Into<String>) -> (i32, String) {
    (EXIT_REFUSED, msg.into())
}

fn fail(json: bool, code: i32, msg: &str) -> i32 {
    if json {
        emit_json(&serde_json::json!({ "ok": false, "exit_code": code, "error": msg }));
    } else {
        eprintln!("diskwhy-cli: {msg}");
        if code == EXIT_USAGE {
            eprintln!("Try `diskwhy-cli --help`.");
        }
    }
    code
}

fn emit_json<T: Serialize>(v: &T) {
    let mut out = std::io::stdout().lock();
    let _ = serde_json::to_writer_pretty(&mut out, v);
    let _ = writeln!(out);
    let _ = out.flush();
}

// ───────────────────────── 扫描根 / 快照缓存 ─────────────────────────

fn system_drive() -> String {
    std::env::var("SystemDrive").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "C:".into())
}

/// `--drive` / `--path` → 规范化的扫描根（`C:\` 或 `D:\some\dir`）。
pub fn resolve_root(o: &Opts) -> Result<String, String> {
    if let Some(p) = &o.path {
        let abs = absolute(p);
        let n = normalize_root(&abs);
        if volume_root_of(&n).is_none() {
            return Err(format!("--path must be a local path with a drive letter (got '{p}')"));
        }
        if !Path::new(&n).is_dir() {
            return Err(format!("--path '{n}' is not an existing folder"));
        }
        return Ok(n);
    }
    let d = o.drive.clone().unwrap_or_else(system_drive);
    let t = d.trim().trim_end_matches(['\\', '/']);
    let t = if t.len() == 1 { format!("{t}:") } else { t.to_string() };
    let b = t.as_bytes();
    if b.len() != 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return Err(format!("--drive must look like C: (got '{d}')"));
    }
    let root = format!("{}:\\", (b[0] as char).to_ascii_uppercase());
    if !Path::new(&root).is_dir() {
        return Err(format!("drive {root} does not exist"));
    }
    Ok(root)
}

fn absolute(p: &str) -> String {
    let pb = PathBuf::from(p.trim());
    if pb.is_absolute() {
        return p.trim().to_string();
    }
    std::path::absolute(&pb).map(|x| x.display().to_string()).unwrap_or_else(|_| p.to_string())
}

/// 快照缓存位置：`<数据目录>\cli\scan-<扫描根>.dwsnap`（数据目录 = `DISKWHY_DATA_DIR` 或 `%LOCALAPPDATA%\DiskWhy`）。
pub fn cache_path(root: &str) -> Option<PathBuf> {
    let mut key: String = root.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    key = key.trim_matches('_').to_string();
    if key.len() > 60 || root.chars().any(|c| !c.is_ascii()) {
        // 长路径 / 非 ASCII：前缀 + 哈希，免得文件名太长或撞名
        let mut h: u64 = 0xcbf29ce484222325;
        for b in root.to_lowercase().bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
        key = format!("{}_{h:016x}", &key[..key.len().min(40)]);
    }
    llm::app_data_dir().map(|d| d.join("cli").join(format!("scan-{key}.dwsnap")))
}

fn save_cache(r: &ScanResult) -> Option<String> {
    let p = cache_path(&r.root)?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).ok()?;
    }
    let tmp = p.with_extension("dwsnap.tmp");
    match snapshot::save(r, &tmp, env!("CARGO_PKG_VERSION")).and_then(|_| std::fs::rename(&tmp, &p).map_err(|e| e.to_string())) {
        Ok(()) => Some(p.display().to_string()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            eprintln!("diskwhy-cli: could not save the scan cache ({e}); `clean` will rescan");
            None
        }
    }
}

/// 读快照缓存：扫描根对得上、不比 `max_age_secs` 旧才用。返回 (结果, 快照路径, 多少秒前)。
fn load_cache(root: &str, max_age_secs: u64) -> Option<(ScanResult, String, u64)> {
    let p = cache_path(root)?;
    if !p.is_file() {
        return None;
    }
    let (r, meta) = snapshot::load(&p).ok()?;
    if !r.root.eq_ignore_ascii_case(root) {
        return None;
    }
    let age = (diskwhy_core::explain::now_unix() - meta.saved_at).max(0) as u64;
    if age > max_age_secs {
        return None;
    }
    Some((r, p.display().to_string(), age))
}

// ───────────────────────── 进度（stderr） ─────────────────────────

struct Ticker {
    quiet: bool,
    tty: bool,
    last: Mutex<Instant>,
}

impl Ticker {
    fn new(quiet: bool) -> Self {
        Ticker { quiet, tty: std::io::stderr().is_terminal(), last: Mutex::new(Instant::now()) }
    }
    /// 终端里每 0.5 秒原地刷新一行；重定向时每 5 秒一行。
    fn tick(&self, line: impl FnOnce() -> String) {
        if self.quiet {
            return;
        }
        let every = if self.tty { 500 } else { 5000 };
        let mut last = self.last.lock().unwrap();
        if last.elapsed().as_millis() < every {
            return;
        }
        *last = Instant::now();
        let s = line();
        if self.tty {
            let s: String = s.chars().take(110).collect();
            eprint!("\r{s:<110}");
        } else {
            eprintln!("{s}");
        }
        let _ = std::io::stderr().flush();
    }
    fn note(&self, s: &str) {
        if self.quiet {
            return;
        }
        self.done();
        eprintln!("{s}");
    }
    fn done(&self) {
        if !self.quiet && self.tty {
            eprint!("\r{:<110}\r", "");
            let _ = std::io::stderr().flush();
        }
    }
}

fn short_path(p: &str, n: usize) -> String {
    let c = p.chars().count();
    if c <= n {
        p.to_string()
    } else {
        format!("...{}", p.chars().skip(c - n + 3).collect::<String>())
    }
}

fn run_scan(root: &str, t: &Ticker) -> Result<ScanResult, String> {
    t.note(&format!("Scanning {root} ... (read-only)"));
    let cancel = AtomicBool::new(false);
    let r = scan(root, &ScanOptions::default(), &cancel, |p| {
        t.tick(|| {
            format!(
                "[{:>5.1}s] {} files, {} dirs, {}  {}",
                p.elapsed_ms as f64 / 1000.0,
                p.files,
                p.dirs,
                human_bytes(p.bytes),
                short_path(&p.current_path, 50)
            )
        })
    })
    .map_err(|e| format!("scan failed: {e}"));
    t.done();
    r
}

// ───────────────────────── 分析（和 GUI 的 analyze 一样） ─────────────────────────

struct Analysis {
    ctx: ExplainCtx,
    rule_warnings: Vec<String>,
    protector: Protector,
    tiers: TierSummary,
    cands: Vec<CandidateGroup>,
}

fn analyze(r: &ScanResult) -> Analysis {
    let (ctx, rule_warnings) = ExplainCtx::load_default();
    let protector = Protector::from_system();
    let tiers = tier_summary(&r.tree, &ctx);
    let mut cands = cleanup_candidates(&r.tree, &ctx, &|p: &str| protector.check(p).is_some());
    // safe 在前，各档按能清的大小降序（id 稳定，顺序只为好看）
    cands.sort_by(|a, b| tier_rank(a.tier).cmp(&tier_rank(b.tier)).then(b.deletable_bytes.cmp(&a.deletable_bytes)).then(b.bytes.cmp(&a.bytes)));
    Analysis { ctx, rule_warnings, protector, tiers, cands }
}

fn tier_rank(t: Tier) -> u8 {
    match t {
        Tier::Safe => 0,
        Tier::Review => 1,
        Tier::Keep => 2,
    }
}

pub fn tier_str(t: Tier) -> &'static str {
    match t {
        Tier::Safe => "safe",
        Tier::Review => "review",
        Tier::Keep => "keep",
    }
}

fn dest_str(d: Dest) -> &'static str {
    match d {
        Dest::Permanent => "delete",
        Dest::Recycle => "recycle",
        Dest::Quarantine => "stash",
    }
}

/// 和界面同一套通用后果标签（`src/labels.ts` 的 LOSE_LABEL）。
pub fn lose_label(k: Option<LoseKind>) -> Option<&'static str> {
    Some(match k? {
        LoseKind::Irreversible => "找不回来",
        LoseKind::Repair => "以后修不了",
        LoseKind::Unsure => "可能有你的东西",
        LoseKind::Reinstall => "删了要重装",
        LoseKind::Relogin => "要重新登录",
        LoseKind::Feature => "少一个功能",
        LoseKind::Redownload => "要重新下载",
        LoseKind::Slower => "下次会慢一点",
    })
}

/// 删了之后怎么回来（按 lose_kind；没写时不下结论，指向 lose）。
pub fn comes_back(tier: Tier, k: Option<LoseKind>) -> &'static str {
    match k {
        Some(LoseKind::Slower) => "会自动重建，下次用到时慢一点",
        Some(LoseKind::Redownload) => "用到时会重新下载（可能很大、很慢）",
        Some(LoseKind::Relogin) => "重新登录后从云端回来，本地设置会丢",
        Some(LoseKind::Reinstall) => "回不来：要重新安装这个工具 / 扩展",
        Some(LoseKind::Feature) => "相关功能会暂时不能用，重新开启 / 重新生成后回来",
        Some(LoseKind::Repair) => "回不来：以后修复 / 卸载 / 回滚可能要重新下载安装包",
        Some(LoseKind::Unsure) => "说不准：里面可能有你自己的东西",
        Some(LoseKind::Irreversible) => "回不来：删了就没了",
        // 规则没写后果类型：不替它下结论（旧版本之类删了不会回来，但也用不着）
        None if tier == Tier::Safe => "可放心删（规则没写怎么回来，见 lose）",
        None => "见 lose 的说明",
    }
}

// ───────────────────────── scan ─────────────────────────

#[derive(Serialize)]
struct PathOut<'a> {
    path: &'a str,
    bytes: u64,
    deletable: bool,
}

#[derive(Serialize)]
struct CandOut<'a> {
    id: &'a str,
    label: &'a str,
    tier: &'static str,
    owner: &'a str,
    category: Category,
    /// 所有路径合计。
    bytes: u64,
    /// 其中能直接清的（clean 实际会处理的部分，预检后以实际为准）。
    cleanable_bytes: u64,
    cleanable: bool,
    /// 桌面版清理页默认勾选的（= `clean --safe` 会选的）。
    default_selected: bool,
    /// clean 允许的去向。
    allowed_to: Vec<&'static str>,
    what: &'a str,
    lose: &'a str,
    lose_kind: Option<LoseKind>,
    lose_label: Option<String>,
    comes_back: &'static str,
    how: &'a [How],
    how_detail: &'a str,
    rule_id: Option<&'a str>,
    heuristic: Option<&'a str>,
    paths: Vec<PathOut<'a>>,
}

fn cand_out(g: &CandidateGroup) -> CandOut<'_> {
    let cleanable = g.deletable_bytes > 0 && g.paths.iter().any(|p| p.deletable);
    CandOut {
        id: &g.key,
        label: &g.name,
        tier: tier_str(g.tier),
        owner: &g.owner,
        category: g.category,
        bytes: g.bytes,
        cleanable_bytes: g.deletable_bytes,
        cleanable,
        default_selected: cleanable && g.tier == Tier::Safe,
        allowed_to: match (cleanable, g.tier) {
            (false, _) | (_, Tier::Keep) => vec![],
            (true, Tier::Safe) => vec!["delete", "recycle", "stash"],
            (true, Tier::Review) => vec!["recycle", "stash"],
        },
        what: &g.what,
        lose: &g.lose,
        lose_kind: g.lose_kind,
        lose_label: g.lose_short.clone().filter(|s| !s.trim().is_empty()).or_else(|| lose_label(g.lose_kind).map(String::from)),
        comes_back: comes_back(g.tier, g.lose_kind),
        how: &g.how,
        how_detail: &g.how_detail,
        rule_id: g.rule_id.as_deref(),
        heuristic: g.heuristic.as_deref(),
        paths: g.paths.iter().map(|p| PathOut { path: &p.path, bytes: p.bytes, deletable: p.deletable }).collect(),
    }
}

#[derive(Serialize, Default)]
struct TierTotal {
    groups: usize,
    bytes: u64,
    cleanable_bytes: u64,
}

#[derive(Serialize)]
struct ScanOut<'a> {
    schema: &'static str,
    ok: bool,
    exit_code: i32,
    root: &'a str,
    drive: DriveOut,
    scanned_at: i64,
    duration_ms: u64,
    files: u64,
    dirs: u64,
    elevated: bool,
    inaccessible_dirs: u64,
    totals: TotalsOut,
    candidates: Vec<CandOut<'a>>,
    snapshot: Option<String>,
    rule_warnings: &'a [String],
}

#[derive(Serialize)]
struct DriveOut {
    root: String,
    total_bytes: u64,
    free_bytes: u64,
    used_bytes: u64,
}

#[derive(Serialize)]
struct TotalsOut {
    safe: TierTotal,
    review: TierTotal,
    /// 按规则库归到“建议保留”的字节（不会出现在候选里，clean 永远拒绝）。
    keep_bytes: u64,
    /// 规则没覆盖到的字节。
    uncovered_bytes: u64,
}

fn totals(cands: &[CandidateGroup], tiers: &TierSummary) -> TotalsOut {
    let mut safe = TierTotal::default();
    let mut review = TierTotal::default();
    for g in cands {
        let t = match g.tier {
            Tier::Safe => &mut safe,
            Tier::Review => &mut review,
            Tier::Keep => continue,
        };
        t.groups += 1;
        t.bytes += g.bytes;
        t.cleanable_bytes += g.deletable_bytes;
    }
    TotalsOut { safe, review, keep_bytes: tiers.keep_bytes, uncovered_bytes: tiers.uncovered_bytes }
}

fn cmd_scan(o: &Opts) -> CmdResult {
    let root = resolve_root(o).map_err(usage)?;
    let t = Ticker::new(o.quiet);
    let r = run_scan(&root, &t).map_err(refused)?;
    t.note("Analyzing (rules, candidates) ...");
    let a = analyze(&r);
    let snap = save_cache(&r);
    let out = ScanOut {
        schema: "diskwhy.scan/1",
        ok: true,
        exit_code: EXIT_OK,
        root: &r.root,
        drive: DriveOut {
            root: volume_root_of(&r.root).unwrap_or_default(),
            total_bytes: r.total_bytes,
            free_bytes: r.free_bytes,
            used_bytes: r.used_bytes,
        },
        scanned_at: diskwhy_core::explain::now_unix(),
        duration_ms: r.duration_ms,
        files: r.files,
        dirs: r.dirs,
        elevated: r.elevated,
        inaccessible_dirs: r.inaccessible_total,
        totals: totals(&a.cands, &a.tiers),
        candidates: a.cands.iter().map(cand_out).collect(),
        snapshot: snap,
        rule_warnings: &a.rule_warnings,
    };
    if o.json {
        emit_json(&out);
    } else {
        print_scan(&out);
    }
    Ok(EXIT_OK)
}

fn print_scan(s: &ScanOut) {
    println!("DiskWhy scan: {}", s.root);
    println!(
        "Drive {}  free {} of {}  |  {} files, {} dirs, {:.1}s{}",
        s.drive.root,
        human_bytes(s.drive.free_bytes),
        human_bytes(s.drive.total_bytes),
        s.files,
        s.dirs,
        s.duration_ms as f64 / 1000.0,
        if s.elevated { "  (admin)" } else { "" }
    );
    println!(
        "Cleanable:  safe {} ({} groups)  |  review {} ({} groups, needs your judgement)  |  keep {} (never cleaned)",
        human_bytes(s.totals.safe.cleanable_bytes),
        s.totals.safe.groups,
        human_bytes(s.totals.review.cleanable_bytes),
        s.totals.review.groups,
        human_bytes(s.totals.keep_bytes)
    );
    for tier in ["safe", "review"] {
        let list: Vec<&CandOut> = s.candidates.iter().filter(|c| c.tier == tier).collect();
        if list.is_empty() {
            continue;
        }
        println!();
        println!(
            "{}",
            if tier == "safe" { "SAFE (可放心清理, `clean --safe` selects the cleanable ones)" } else { "REVIEW (需你判断, only with `clean --ids ...`, never deleted directly)" }
        );
        for c in list {
            let tag = c.lose_label.clone().unwrap_or_default();
            let size = if c.cleanable { human_bytes(c.cleanable_bytes) } else { format!("({})", human_bytes(c.bytes)) };
            println!("  {:>10}  {:<28} {}{}", size, c.id, c.label, if tag.is_empty() { String::new() } else { format!("  [{tag}]") });
            if !c.cleanable {
                println!("              not directly deletable: {}", if c.how_detail.is_empty() { "see app" } else { c.how_detail });
            }
            println!("              {}  -> {}", c.what, c.comes_back);
            for p in c.paths.iter().take(3) {
                println!("              {}{}", p.path, if p.deletable { "" } else { "  (not deletable)" });
            }
            if c.paths.len() > 3 {
                println!("              ... and {} more paths", c.paths.len() - 3);
            }
        }
    }
    for w in s.rule_warnings {
        eprintln!("rule warning: {w}");
    }
    println!();
    if let Some(p) = &s.snapshot {
        println!("Scan cached at {p}");
    }
    println!("Next: `diskwhy-cli clean --safe` (dry run) shows exactly what would be deleted.");
}

// ───────────────────────── clean ─────────────────────────

#[derive(Serialize, Debug, Clone)]
pub struct Refusal {
    pub id: String,
    pub reason: String,
}

/// 选中的候选组（下标 + 是否用 `--ids` 点名的）和拒绝的 id。
pub struct Selection {
    pub groups: Vec<(usize, bool)>,
    pub refused: Vec<Refusal>,
}

/// `--safe` / `--ids` → 候选组。review 档必须点名；`--to delete` 时 review 档拒绝；keep / 未知 / 不能直接删的拒绝。
pub fn select(cands: &[CandidateGroup], ctx: &ExplainCtx, safe: bool, ids: &[String], to: Option<To>) -> Selection {
    let mut groups: Vec<(usize, bool)> = Vec::new();
    let mut refused = Vec::new();
    if safe {
        for (i, g) in cands.iter().enumerate() {
            if g.tier == Tier::Safe && g.deletable_bytes > 0 && g.paths.iter().any(|p| p.deletable) {
                groups.push((i, false));
            }
        }
    }
    for id in ids {
        let Some(i) = cands.iter().position(|g| g.key.eq_ignore_ascii_case(id)) else {
            refused.push(Refusal { id: id.clone(), reason: missing_reason(ctx, id) });
            continue;
        };
        let g = &cands[i];
        if g.deletable_bytes == 0 || !g.paths.iter().any(|p| p.deletable) {
            let how = if g.how_detail.trim().is_empty() { "see the DiskWhy app".to_string() } else { g.how_detail.clone() };
            refused.push(Refusal { id: id.clone(), reason: format!("'{}' cannot be deleted directly; how to free it: {how}", g.name) });
            continue;
        }
        match g.tier {
            Tier::Keep => {
                refused.push(Refusal { id: id.clone(), reason: format!("keep-tier (建议保留): {}", g.what) });
                continue;
            }
            Tier::Review if to == Some(To::Delete) => {
                refused.push(Refusal {
                    id: id.clone(),
                    reason: format!("'{}' is review-tier: never deleted directly; use --to recycle or --to stash", g.name),
                });
                continue;
            }
            _ => {}
        }
        match groups.iter_mut().find(|(j, _)| *j == i) {
            Some(e) => e.1 = true,
            None => groups.push((i, true)),
        }
    }
    Selection { groups, refused }
}

fn missing_reason(ctx: &ExplainCtx, id: &str) -> String {
    match ctx.rules.rules.iter().find(|r| r.id.eq_ignore_ascii_case(id)) {
        Some(r) if r.tier == Tier::Keep => format!("'{id}' is keep-tier (建议保留: {}); DiskWhy never deletes it", r.what),
        Some(r) if !r.how.contains(&How::Delete) => {
            format!("'{id}' cannot be deleted directly; how to free it: {}", if r.how_detail.is_empty() { "see the DiskWhy app" } else { &r.how_detail })
        }
        Some(_) => format!("'{id}' has nothing cleanable in this scan (not on disk, protected, or covered by a stricter rule)"),
        None => format!("unknown id '{id}' (not in this scan's candidate list; run `diskwhy-cli scan` to see ids)"),
    }
}

/// `--to stash` 没给 `--stash-dir` 时自动挑暂存区：源盘、系统盘以外的固定盘，剩余空间够，暂存区能用（不提权）。
fn pick_stash(root: &str, need: u64, create: bool) -> Result<String, String> {
    let src = volume_root_of(root).unwrap_or_default();
    let sys = format!("{}\\", system_drive().trim_end_matches('\\'));
    let mut drives: Vec<_> = list_fixed_drives()
        .into_iter()
        .filter(|d| !d.root.eq_ignore_ascii_case(&src) && !d.root.eq_ignore_ascii_case(&sys) && d.free_bytes > need.saturating_add(64 << 20))
        .collect();
    drives.sort_by(|a, b| b.free_bytes.cmp(&a.free_bytes));
    let mut why = Vec::new();
    for d in &drives {
        match quarantine::prepare_dest(&d.root, create) {
            DestStatus::Ok { dest_root } => return Ok(dest_root),
            DestStatus::NeedsElevation => why.push(format!("{} needs admin to create the stash folder", d.root)),
            DestStatus::Failed { message } => why.push(message),
        }
    }
    Err(format!(
        "no usable stash location (needs a fixed drive other than {src} and the system drive, with enough free space and a writable root{}). Pass --stash-dir DIR or use --to recycle.",
        if why.is_empty() { String::new() } else { format!(": {}", why.join("; ")) }
    ))
}

#[derive(Serialize)]
struct SkipOut {
    path: String,
    code: String,
    reason: String,
    bytes: u64,
    processes: Vec<String>,
}

#[derive(Serialize)]
struct ItemOut {
    /// 所属候选组 id（合并 / 找不到时为 null）。
    id: Option<String>,
    path: String,
    tier: &'static str,
    unknown: bool,
    /// delete / recycle / stash；被拦下时为 null。
    dest: Option<&'static str>,
    /// 被拦下（整项不动）的原因。
    blocked: Option<String>,
    what: String,
    lose: String,
    will_delete_count: usize,
    will_delete_bytes: u64,
    skipped_count: usize,
    skipped_bytes: u64,
    skipped_by_reason: BTreeMap<String, usize>,
    /// 跳过的前 20 条（原因、占用它的进程）。
    skipped_sample: Vec<SkipOut>,
    enum_errors: Vec<String>,
}

#[derive(Serialize)]
struct WarnOut {
    code: String,
    message: String,
    blocking: bool,
}

#[derive(Serialize)]
struct StrategyOut {
    to: &'static str,
    /// safe 档去哪 / review 档去哪。
    safe: &'static str,
    review: &'static str,
    stash_dir: Option<String>,
}

#[derive(Serialize)]
struct ScanSource {
    /// snapshot（用了缓存的扫描结果）/ fresh_scan（这次重新扫的）
    from: &'static str,
    snapshot: Option<String>,
    age_secs: u64,
}

#[derive(Serialize)]
struct PlanTotals {
    will_delete_count: usize,
    will_delete_bytes: u64,
    skipped_count: usize,
    skipped_bytes: u64,
    skipped_by_reason: BTreeMap<String, CountBytes>,
    by_dest: ByDest,
}

#[derive(Serialize)]
struct ExecItemOut {
    path: String,
    blocked: Option<String>,
    ok: usize,
    ok_bytes: u64,
    skipped: usize,
    skipped_bytes: u64,
    failed: usize,
    failed_bytes: u64,
    /// 没成功的前 20 条（跳过 / 失败的原因）。
    problems: Vec<ProblemOut>,
}

#[derive(Serialize)]
struct ProblemOut {
    path: String,
    result: String,
    reason: String,
}

#[derive(Serialize)]
struct ExecOut {
    batch_id: String,
    /// 源盘实际腾出的空间（剩余空间实测差值，按这次该腾出的量封顶；回收站 / 同盘暂存不腾空间）。
    freed_bytes: u64,
    freed_expected_bytes: u64,
    ok: usize,
    ok_bytes: u64,
    skipped: usize,
    skipped_bytes: u64,
    failed: usize,
    failed_bytes: u64,
    not_processed: CountBytes,
    by_dest: ByDest,
    skipped_by_reason: BTreeMap<String, CountBytes>,
    cancelled: bool,
    log_path: String,
    manifest_path: Option<String>,
    notes: Vec<String>,
    items: Vec<ExecItemOut>,
}

#[derive(Serialize)]
struct CleanOut {
    schema: &'static str,
    ok: bool,
    exit_code: i32,
    /// dry_run（只预演）/ executed（真的做了）
    mode: &'static str,
    root: String,
    scan: ScanSource,
    strategy: StrategyOut,
    selected: Vec<String>,
    refused: Vec<Refusal>,
    items: Vec<ItemOut>,
    plan_notes: Vec<String>,
    warnings: Vec<WarnOut>,
    totals: PlanTotals,
    locked_processes: Vec<String>,
    can_execute: bool,
    est_secs: f64,
    result: Option<ExecOut>,
    next_step: Option<String>,
}

fn group_of(item_path: &str, map: &[(String, String, bool)]) -> Option<(String, bool)> {
    let l = item_path.to_lowercase();
    if let Some((_, k, e)) = map.iter().find(|(p, _, _)| *p == l) {
        return Some((k.clone(), *e));
    }
    // 合并：清单项包含了别的请求路径
    let pre = format!("{}\\", l.trim_end_matches('\\'));
    map.iter().find(|(p, _, _)| p.starts_with(&pre)).map(|(_, k, e)| (k.clone(), *e))
}

fn proc_names(ps: &[diskwhy_core::action::winops::LockingProcess]) -> Vec<String> {
    ps.iter().map(|p| format!("{} (PID {})", p.name, p.pid)).collect()
}

fn item_out(it: &ItemReport, plan: &Plan, map: &[(String, String, bool)]) -> ItemOut {
    let pi = plan.items.iter().find(|p| p.path.eq_ignore_ascii_case(&it.path));
    let blocked = it.blocked.clone();
    ItemOut {
        id: group_of(&it.path, map).map(|x| x.0),
        path: it.path.clone(),
        tier: tier_str(it.tier),
        unknown: it.unknown,
        dest: if blocked.is_some() { None } else { pi.map(|p| dest_str(plan.dest_for(p))) },
        blocked,
        what: it.what.clone(),
        lose: it.lose.clone(),
        will_delete_count: it.will_delete_count,
        will_delete_bytes: it.will_delete_bytes,
        skipped_count: it.skipped_count,
        skipped_bytes: it.skipped_bytes,
        skipped_by_reason: it.skipped_by_reason.clone(),
        skipped_sample: it
            .skipped
            .iter()
            .take(20)
            .map(|s| SkipOut { path: s.path.clone(), code: s.code.clone(), reason: s.reason.clone(), bytes: s.bytes, processes: proc_names(&s.processes) })
            .collect(),
        enum_errors: it.enum_errors.clone(),
    }
}

fn exec_out(r: &ExecReport) -> ExecOut {
    ExecOut {
        batch_id: r.batch_id.clone(),
        freed_bytes: r.freed_bytes,
        freed_expected_bytes: r.freed_expected_bytes,
        ok: r.ok,
        ok_bytes: r.ok_bytes,
        skipped: r.skipped,
        skipped_bytes: r.skipped_bytes,
        failed: r.failed,
        failed_bytes: r.failed_bytes,
        not_processed: r.not_processed,
        by_dest: r.by_dest,
        skipped_by_reason: r.skipped_by_reason.clone(),
        cancelled: r.cancelled,
        log_path: r.log_path.clone(),
        manifest_path: r.manifest_path.clone(),
        notes: r.notes.clone(),
        items: r
            .items
            .iter()
            .map(|i| ExecItemOut {
                path: i.path.clone(),
                blocked: i.blocked.clone(),
                ok: i.ok,
                ok_bytes: i.ok_bytes,
                skipped: i.skipped,
                skipped_bytes: i.skipped_bytes,
                failed: i.failed,
                failed_bytes: i.failed_bytes,
                problems: i
                    .details
                    .iter()
                    .filter(|d| d.result != "ok")
                    .take(20)
                    .map(|d| ProblemOut { path: d.path.clone(), result: d.result.clone(), reason: d.reason.clone() })
                    .collect(),
            })
            .collect(),
    }
}

fn cmd_clean(o: &Opts) -> CmdResult {
    let root = resolve_root(o).map_err(usage)?;
    let t = Ticker::new(o.quiet);
    let max_age = o.max_age_min.unwrap_or(DEFAULT_MAX_AGE_MIN).saturating_mul(60);
    let cached = if o.rescan { None } else { load_cache(&root, max_age) };
    let (mut r, source) = match cached {
        Some((r, p, age)) => {
            t.note(&format!("Using cached scan of {root} from {} min ago ({p}); --rescan to scan again", age / 60));
            (r, ScanSource { from: "snapshot", snapshot: Some(p), age_secs: age })
        }
        None => {
            let r = run_scan(&root, &t).map_err(refused)?;
            let p = save_cache(&r);
            (r, ScanSource { from: "fresh_scan", snapshot: p, age_secs: 0 })
        }
    };
    t.note("Analyzing (rules, candidates) ...");
    let a = analyze(&r);
    let sel = select(&a.cands, &a.ctx, o.safe, &o.ids, o.to);
    if sel.groups.is_empty() {
        let mut msg = String::from("nothing selected");
        if o.safe && o.ids.is_empty() {
            msg.push_str(": no safe-tier candidates with cleanable bytes in this scan");
        }
        for f in &sel.refused {
            msg.push_str(&format!("\n  refused {}: {}", f.id, f.reason));
        }
        if o.json {
            emit_json(&serde_json::json!({
                "schema": "diskwhy.clean/1", "ok": false, "exit_code": EXIT_REFUSED, "mode": if o.yes { "executed" } else { "dry_run" },
                "root": root, "error": msg, "refused": sel.refused, "selected": Vec::<String>::new(),
            }));
            return Ok(EXIT_REFUSED);
        }
        return Err(refused(msg));
    }
    let has_review = sel.groups.iter().any(|(i, _)| a.cands[*i].tier == Tier::Review);
    let review_need: u64 = sel.groups.iter().filter(|(i, _)| a.cands[*i].tier == Tier::Review).map(|(i, _)| a.cands[*i].deletable_bytes).sum();

    // 去向：和 GUI 的 currentPlanArgs 一样（safe 默认直接删；明确选回收站时 safe 也进回收站）
    let (strategy, safe_direct, to_label) = match o.to {
        None => (Strategy::RecycleBin, true, "default"),
        Some(To::Recycle) => (Strategy::RecycleBin, false, "recycle"),
        Some(To::Delete) => (Strategy::Permanent, true, "delete"),
        Some(To::Stash) => {
            if let Some(d) = &o.stash_dir {
                (Strategy::Quarantine { dest_root: normalize_root(&absolute(d)) }, true, "stash")
            } else if !has_review {
                // 和 GUI 一样：清单里没有 review 档时不去解析暂存区，safe 照样直接删
                (Strategy::RecycleBin, true, "stash")
            } else {
                let d = pick_stash(&root, review_need, o.yes).map_err(refused)?;
                (Strategy::Quarantine { dest_root: d }, true, "stash")
            }
        }
    };
    let stash_dir = match &strategy {
        Strategy::Quarantine { dest_root } => Some(dest_root.clone()),
        _ => None,
    };
    let review_dest = match &strategy {
        Strategy::RecycleBin => "recycle",
        Strategy::Quarantine { .. } => "stash",
        Strategy::Permanent => "refused",
    };

    // 清单：每组能直接删的路径，expect_rule = 规则 id（和清理页一样）
    let mut reqs: Vec<PlanRequest> = Vec::new();
    let mut map: Vec<(String, String, bool)> = Vec::new();
    for (i, explicit) in &sel.groups {
        let g = &a.cands[*i];
        for p in g.paths.iter().filter(|p| p.deletable) {
            reqs.push(PlanRequest { path: p.path.clone(), expect_rule: g.rule_id.clone() });
            map.push((normalize_root(&p.path).to_lowercase(), g.key.clone(), *explicit));
        }
    }
    let mut plan = action::build_plan_quick(&r, &a.ctx, &a.protector, &reqs, strategy.clone());
    plan.safe_direct = safe_direct;
    // CLI 的额外护栏：review 档只认点名的，且不直接删
    let dests: Vec<Dest> = plan.items.iter().map(|i| plan.dest_for(i)).collect();
    for (k, item) in plan.items.iter_mut().enumerate() {
        if item.blocked.is_some() {
            continue;
        }
        let explicit = group_of(&item.path, &map).map(|x| x.1).unwrap_or(false);
        match item.tier {
            Tier::Safe => {}
            Tier::Review if !explicit => {
                item.blocked = Some("now judged review-tier (需你判断): name its id explicitly with --ids to include it".into())
            }
            Tier::Review if dests[k] == Dest::Permanent => {
                item.blocked = Some("review-tier: the CLI never deletes these directly; use --to recycle or --to stash".into())
            }
            Tier::Review => {}
            Tier::Keep => item.blocked = Some(format!("keep-tier (建议保留): {}", item.what)),
        }
    }

    // 枚举 + 预检（和 GUI 的 preview_plan 一样），进度到 stderr
    t.note("Planning and preflight (read-only checks: in use, running programs, recent changes) ...");
    let prog = Arc::new(Progress::default());
    let done = AtomicBool::new(false);
    let rep: PreflightReport = std::thread::scope(|s| {
        let (pr, tk, dn) = (&prog, &t, &done);
        s.spawn(move || {
            while !dn.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(200));
                let v = pr.view();
                tk.tick(|| format!("[{}] {} dirs, {} entries, checked {}/{}, in use {}", v.phase, v.dirs, v.entries, v.checked, v.to_check, v.locked));
            }
        });
        action::enumerate_plan(&mut plan, &prog);
        let rep = action::preflight_with(&mut plan, &a.protector, &PreflightOptions::default(), &prog);
        done.store(true, Ordering::Relaxed);
        rep
    });
    t.done();

    let mut out = CleanOut {
        schema: "diskwhy.clean/1",
        ok: true,
        exit_code: EXIT_OK,
        mode: "dry_run",
        root: root.clone(),
        scan: source,
        strategy: StrategyOut { to: to_label, safe: if safe_direct { "delete" } else { review_dest }, review: review_dest, stash_dir },
        selected: sel.groups.iter().map(|(i, _)| a.cands[*i].key.clone()).collect(),
        refused: sel.refused.clone(),
        items: rep.items.iter().map(|it| item_out(it, &plan, &map)).collect(),
        plan_notes: plan.warnings.clone(),
        warnings: rep.warnings.iter().map(|w| WarnOut { code: w.code.clone(), message: w.message.clone(), blocking: w.blocking }).collect(),
        totals: PlanTotals {
            will_delete_count: rep.total_entries,
            will_delete_bytes: rep.total_bytes,
            skipped_count: rep.total_skipped,
            skipped_bytes: rep.skipped_bytes,
            skipped_by_reason: rep.skipped_by_reason.clone(),
            by_dest: rep.by_dest,
        },
        locked_processes: proc_names(&rep.locked_processes),
        can_execute: rep.can_execute,
        est_secs: rep.est_secs,
        result: None,
        next_step: None,
    };
    let any_blocked = out.items.iter().any(|i| i.blocked.is_some());

    if !o.yes {
        out.exit_code = if !rep.can_execute || rep.total_entries == 0 {
            EXIT_REFUSED
        } else if !out.refused.is_empty() {
            EXIT_PARTIAL
        } else {
            EXIT_OK
        };
        out.ok = out.exit_code != EXIT_REFUSED;
        out.next_step = Some(if rep.can_execute && rep.total_entries > 0 {
            "dry run only, nothing was touched. Re-run the same command with --yes to execute (after the user agrees).".into()
        } else {
            "nothing can be cleaned with this selection; see blocked items / warnings.".into()
        });
        finish_clean(o, &out);
        return Ok(out.exit_code);
    }

    if !rep.can_execute {
        out.ok = false;
        out.exit_code = EXIT_REFUSED;
        out.next_step = Some("not executed: nothing to delete or a blocking warning (see warnings).".into());
        finish_clean(o, &out);
        return Ok(out.exit_code);
    }

    // 执行：和 GUI 的 execute_plan 一样的选项
    let log_path = ActionLog::default_path().ok_or_else(|| refused("cannot locate the DiskWhy data folder (%LOCALAPPDATA%) for actions.log"))?;
    let log = ActionLog::open(&log_path);
    let mut opts = ExecOptions::new(&log);
    opts.skip_locked = true;
    opts.perf = PerfModel::load_default();
    opts.perf_path = PerfModel::default_path();
    opts.run_state_path = RunState::default_path();
    t.note(&format!("Executing ({}) ...", plan.method_label()));
    let res = action::execute(&plan, &a.protector, &opts, |p| {
        t.tick(|| format!("{}/{} entries, {} / {}  {}", p.done, p.total, human_bytes(p.bytes_done), human_bytes(p.total_bytes), short_path(&p.current, 50)))
    });
    t.done();
    let report = match res {
        Ok(rep) => rep,
        Err(e) => return Err(refused(format!("not executed: {e}"))),
    };
    if let Strategy::Quarantine { dest_root } = &plan.strategy {
        if report.by_dest.quarantine.count > 0 {
            quarantine::remember_root(dest_root);
        }
    }
    // 更新快照缓存（和 GUI 的 refresh_after 一样只重扫受影响的子树）
    if !report.affected_dirs.is_empty() {
        let _ = action::refresh::refresh_dirs(&mut r, &report.affected_dirs);
        let _ = save_cache(&r);
    }
    out.mode = "executed";
    out.exit_code = if report.ok == 0 {
        EXIT_REFUSED
    } else if report.failed > 0 || report.skipped > 0 || report.cancelled || report.not_processed.count > 0 || !out.refused.is_empty() || any_blocked {
        EXIT_PARTIAL
    } else {
        EXIT_OK
    };
    out.ok = report.ok > 0;
    out.next_step = None;
    out.result = Some(exec_out(&report));
    finish_clean(o, &out);
    Ok(out.exit_code)
}

fn finish_clean(o: &Opts, out: &CleanOut) {
    if o.json {
        emit_json(out);
    } else {
        print_clean(out);
    }
}

fn reasons(m: &BTreeMap<String, usize>) -> String {
    m.iter().map(|(k, v)| format!("{} {v}", reason_word(k))).collect::<Vec<_>>().join(", ")
}

fn reason_word(code: &str) -> &str {
    match code {
        "hot" => "modified <30min",
        "locked" => "in use",
        "running" => "program running",
        "no_permission" => "needs admin/no permission",
        "protected" => "protected",
        "changed" => "changed since plan",
        "missing" => "gone",
        "cloud" => "cloud file",
        "recent" => "too recent",
        "hardlink" => "hardlink",
        "recycle_unavailable" => "recycle bin can't take it",
        "unreadable" => "unreadable",
        other => other,
    }
}

fn print_clean(c: &CleanOut) {
    let executed = c.mode == "executed";
    println!("{}", if executed { "DiskWhy clean: EXECUTED" } else { "DiskWhy clean: DRY RUN - nothing was deleted (预演，未删除任何文件)" });
    println!(
        "Root {}  |  scan: {}{}",
        c.root,
        c.scan.from,
        if c.scan.from == "snapshot" { format!(" ({} min old)", c.scan.age_secs / 60) } else { String::new() }
    );
    println!(
        "Where: safe -> {}, review -> {}{}",
        c.strategy.safe,
        c.strategy.review,
        c.strategy.stash_dir.as_ref().map(|d| format!("  (stash at {d})")).unwrap_or_default()
    );
    for f in &c.refused {
        println!("REFUSED {}: {}", f.id, f.reason);
    }
    println!();
    for it in &c.items {
        let id = it.id.clone().unwrap_or_else(|| "-".into());
        match &it.blocked {
            Some(b) => {
                println!("  [blocked] {id}  {}", it.path);
                println!("            {b}");
            }
            None => {
                println!(
                    "  {:>10}  [{}] {id} -> {}  ({} files)",
                    human_bytes(it.will_delete_bytes),
                    it.tier,
                    it.dest.unwrap_or("?"),
                    it.will_delete_count
                );
                println!("              {}", it.path);
                if it.skipped_count > 0 {
                    println!("              skipped {} files / {}: {}", it.skipped_count, human_bytes(it.skipped_bytes), reasons(&it.skipped_by_reason));
                    for s in it.skipped_sample.iter().filter(|s| !s.processes.is_empty()).take(2) {
                        println!("                e.g. {} - {}", short_path(&s.path, 70), s.reason);
                    }
                }
            }
        }
        for e in it.enum_errors.iter().take(2) {
            println!("              note: {e}");
        }
    }
    println!();
    for n in &c.plan_notes {
        println!("note: {n}");
    }
    for w in &c.warnings {
        println!("{}: {}", if w.blocking { "BLOCKING" } else { "warning" }, w.message);
    }
    let tt = &c.totals;
    match &c.result {
        None => {
            println!(
                "Would process {} files, {} (delete {}, recycle {}, stash {}); skip {} files, {}.",
                tt.will_delete_count,
                human_bytes(tt.will_delete_bytes),
                human_bytes(tt.by_dest.permanent.bytes),
                human_bytes(tt.by_dest.recycle.bytes),
                human_bytes(tt.by_dest.quarantine.bytes),
                tt.skipped_count,
                human_bytes(tt.skipped_bytes)
            );
            if c.est_secs > 0.0 {
                println!("Estimated time: {:.0} s", c.est_secs.ceil());
            }
        }
        Some(r) => {
            println!(
                "Freed {} on the drive. Done {} files / {} (deleted {}, recycled {}, stashed {}); skipped {} / {}; failed {} / {}.",
                human_bytes(r.freed_bytes),
                r.ok,
                human_bytes(r.ok_bytes),
                human_bytes(r.by_dest.permanent.bytes),
                human_bytes(r.by_dest.recycle.bytes),
                human_bytes(r.by_dest.quarantine.bytes),
                r.skipped,
                human_bytes(r.skipped_bytes),
                r.failed,
                human_bytes(r.failed_bytes)
            );
            if r.cancelled {
                println!("Stopped before finishing.");
            }
            for n in &r.notes {
                println!("note: {n}");
            }
            println!("Log: {}", r.log_path);
            if let Some(m) = &r.manifest_path {
                println!("Stash manifest: {m} (restore from the DiskWhy app, kept 7 days)");
            }
        }
    }
    if let Some(n) = &c.next_step {
        println!("{n}");
    }
}

// ───────────────────────── explain ─────────────────────────

fn dir_shell(path: &str, name: &str, child: Node) -> Node {
    Node {
        name: name.to_string(),
        path: path.to_string(),
        kind: NodeKind::Dir,
        logical: child.logical,
        allocated: child.allocated,
        mtime: 0,
        attrs: 0x10,
        reparse_tag: 0,
        inaccessible: false,
        error_code: 0,
        reparse_skipped: false,
        cloud_placeholder: false,
        system_reserved: false,
        hardlink_dup: false,
        mapped: false,
        files: child.files,
        dirs: child.dirs + 1,
        children: vec![child],
    }
}

/// 只扫了某个文件夹时，把它的上层文件夹补成只有这一个孩子的空壳，挂到盘根下（规则继承要看上层路径）。
pub fn wrap_with_ancestors(mut tree: Node) -> Node {
    let path = tree.path.clone();
    if is_volume_root(&path) {
        return tree;
    }
    let segs: Vec<&str> = path.split('\\').filter(|s| !s.is_empty()).collect();
    if let Some(last) = segs.last() {
        tree.name = last.to_string();
    }
    let mut cur = tree;
    for k in (1..segs.len()).rev() {
        let p = if k == 1 { format!("{}\\", segs[0]) } else { segs[..k].join("\\") };
        let name = if k == 1 { p.clone() } else { segs[k - 1].to_string() };
        cur = dir_shell(&p, &name, cur);
    }
    cur
}

#[derive(Serialize)]
struct ExplainOut<'a> {
    schema: &'static str,
    ok: bool,
    exit_code: i32,
    path: String,
    kind: &'static str,
    bytes: u64,
    /// safe / review / keep / unknown
    tier: &'static str,
    source: &'a Source,
    owner: Option<&'a str>,
    category: Category,
    what: &'a str,
    why: &'a str,
    lose: &'a str,
    lose_kind: Option<LoseKind>,
    lose_label: Option<String>,
    comes_back: &'static str,
    how: &'a [How],
    how_detail: &'a str,
    confidence: f32,
    notes: &'a [String],
    clues: &'a [String],
    /// 受保护路径（DiskWhy 永远不碰）的原因。
    protected: Option<String>,
    /// `clean` 能不能处理它（按候选组：用 `candidate_id` 选，会处理整组）。
    cli_cleanable: bool,
    cli_refusal: Option<String>,
    candidate_id: Option<String>,
    /// 解释用的扫描树：snapshot（缓存的扫描结果）/ scan（这次只扫了这个路径）
    tree_from: &'static str,
}

fn cmd_explain(o: &Opts) -> CmdResult {
    let raw = &o.positional[0];
    let path = normalize_root(&absolute(raw));
    let Some(vol) = volume_root_of(&path) else {
        return Err(usage(format!("PATH must be a local path with a drive letter (got '{raw}')")));
    };
    let meta = std::fs::symlink_metadata(&path).map_err(|_| refused(format!("'{path}' does not exist")))?;
    let t = Ticker::new(o.quiet);
    // 1) 快照里有（任何年龄都行，只用来解释）
    let mut tree: Option<(Node, &'static str)> = None;
    // 整盘的快照，或扫过某个上层文件夹（--path）留下的快照：从近到远找
    let mut roots: Vec<String> = Vec::new();
    let mut cur = path.clone();
    loop {
        roots.push(cur.clone());
        match cur.rsplit_once('\\') {
            Some((p, _)) if !is_volume_root(&cur) => cur = normalize_root(if p.len() == 2 { &cur[..3] } else { p }),
            _ => break,
        }
    }
    for root in roots {
        if !cache_path(&root).is_some_and(|p| p.is_file()) {
            continue;
        }
        if let Some((r, _, _)) = load_cache(&root, u64::MAX) {
            if find_node(&r.tree, &path).is_some() {
                tree = Some((r.tree, "snapshot"));
                break;
            }
        }
    }
    // 2) 只扫这个路径（文件就扫它所在的文件夹）
    if tree.is_none() {
        let target = if meta.is_dir() || is_volume_root(&path) { path.clone() } else { path.rsplit_once('\\').map(|(p, _)| normalize_root(p)).unwrap_or(vol.clone()) };
        let r = run_scan(&target, &t).map_err(refused)?;
        let r_root = r.tree.path.clone();
        let wrapped = wrap_with_ancestors(r.tree);
        debug_assert!(diskwhy_core::view::find_dir(&wrapped, &r_root).is_some());
        tree = Some((wrapped, "scan"));
    }
    let (tree, tree_from) = tree.unwrap();
    let (ctx, _warnings) = ExplainCtx::load_default();
    let protector = Protector::from_system();
    let Some((chain, node)) = find_node(&tree, &path) else {
        return Err(refused(format!("'{path}' is not in the scan (inaccessible?)")));
    };
    let full = if node.kind == NodeKind::Dir { node.path.clone() } else { diskwhy_core::fs::join(&chain.last().map(|p| p.path.clone()).unwrap_or_default(), &node.name) };
    let target = diskwhy_core::explain::Target { node, path: &full, parent: chain.last().copied(), ancestors: &chain };
    let e: Explanation = diskwhy_core::explain::explain(&target, &ctx);
    let protected = protector.check(&full);
    let refusal = protected.clone().or_else(|| node_gate(node)).or_else(|| tier_gate(&e));
    let candidate_id = if refusal.is_none() {
        match &e.source {
            Source::Rule { id, .. } => Some(id.clone()),
            Source::Heuristic { name } => Some(format!("h:{name}")),
            _ => None,
        }
    } else {
        None
    };
    let out = ExplainOut {
        schema: "diskwhy.explain/1",
        ok: true,
        exit_code: EXIT_OK,
        path: full.clone(),
        kind: if node.kind == NodeKind::Dir { "dir" } else { "file" },
        bytes: node.allocated,
        tier: if e.is_unknown() { "unknown" } else { tier_str(e.tier) },
        source: &e.source,
        owner: e.owner.as_deref(),
        category: e.category,
        what: &e.what,
        why: &e.why,
        lose: &e.lose,
        lose_kind: e.lose_kind,
        lose_label: e.lose_short.clone().filter(|s| !s.trim().is_empty()).or_else(|| lose_label(e.lose_kind).map(String::from)),
        comes_back: comes_back(e.tier, e.lose_kind),
        how: &e.how,
        how_detail: &e.how_detail,
        confidence: e.confidence,
        notes: &e.notes,
        clues: &e.clues,
        protected,
        cli_cleanable: refusal.is_none(),
        cli_refusal: refusal,
        candidate_id,
        tree_from,
    };
    if o.json {
        emit_json(&out);
    } else {
        println!("{}  ({}, {})", out.path, out.kind, human_bytes(out.bytes));
        println!("Tier: {}   Source: {}", out.tier, action::plan::source_label(out.source));
        if let Some(ow) = out.owner {
            println!("Owner: {ow}");
        }
        if !out.what.is_empty() {
            println!("What: {}", out.what);
        }
        if !out.why.is_empty() {
            println!("Why: {}", out.why);
        }
        if !out.lose.is_empty() {
            println!("If deleted: {}  -> {}", out.lose, out.comes_back);
        }
        if !out.how_detail.is_empty() {
            println!("How to free: {}", out.how_detail);
        }
        for n in out.notes {
            println!("Note: {n}");
        }
        for c in out.clues.iter().take(5) {
            println!("Clue: {c}");
        }
        match (&out.cli_refusal, &out.candidate_id) {
            (Some(r), _) => println!("CLI: will NOT clean this - {r}"),
            (None, Some(id)) => println!("CLI: cleanable as part of candidate '{id}' (`diskwhy-cli clean --ids {id}`, dry run first)"),
            (None, None) => println!("CLI: cleanable"),
        }
    }
    Ok(EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_clean_flags() {
        let o = parse_opts("clean", &a(&["--ids", "a,b", "--id=c", "--to", "stash", "--stash-dir", "D:\\x", "--yes", "--json"])).unwrap();
        assert_eq!(o.ids, vec!["a", "b", "c"]);
        assert_eq!(o.to, Some(To::Stash));
        assert!(o.yes && o.json && !o.safe);
        assert!(parse_opts("clean", &a(&["--json"])).unwrap_err().contains("--safe"));
        assert!(parse_opts("clean", &a(&["--safe", "--to", "shred"])).is_err());
        assert!(parse_opts("clean", &a(&["--safe", "--stash-dir", "D:\\x"])).is_err());
        assert!(parse_opts("scan", &a(&["--safe"])).is_err(), "scan must not accept clean flags");
        assert!(parse_opts("scan", &a(&["--drive", "C:", "--path", "D:\\"])).is_err());
        assert!(parse_opts("explain", &a(&[])).is_err());
        assert_eq!(parse_opts("explain", &a(&["C:\\x", "--json"])).unwrap().positional, vec!["C:\\x"]);
    }

    #[test]
    fn drive_forms() {
        for d in ["C", "c:", "C:\\", "c:/"] {
            let o = Opts { drive: Some(d.into()), ..Default::default() };
            assert_eq!(resolve_root(&o).unwrap(), "C:\\");
        }
        let o = Opts { drive: Some("CC:".into()), ..Default::default() };
        assert!(resolve_root(&o).is_err());
    }

    #[test]
    fn ancestors_shell() {
        let leaf = dir_shell("D:\\a\\b", "whatever", dir_shell("D:\\a\\b\\c", "c", dummy_leaf()));
        let w = wrap_with_ancestors(leaf);
        assert_eq!(w.path, "D:\\");
        let chain = diskwhy_core::view::find_dir(&w, "D:\\a\\b\\c").expect("chain");
        assert_eq!(chain.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), vec!["D:\\", "D:\\a", "D:\\a\\b", "D:\\a\\b\\c"]);
        assert_eq!(chain[2].name, "b", "the scanned folder gets its own name");
    }

    fn dummy_leaf() -> Node {
        Node {
            name: "f".into(),
            path: String::new(),
            kind: NodeKind::File,
            logical: 1,
            allocated: 4096,
            mtime: 0,
            attrs: 0,
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

    #[test]
    fn cache_key_is_stable_and_short() {
        std::env::set_var(llm::DATA_DIR_ENV, std::env::temp_dir().join("dw-cli-unit"));
        let p = cache_path("C:\\").unwrap();
        assert!(p.ends_with("cli\\scan-C.dwsnap"), "{}", p.display());
        let long = format!("D:\\{}", "很长的文件夹".repeat(20));
        let q = cache_path(&long).unwrap();
        assert!(q.file_name().unwrap().to_string_lossy().len() < 90);
        assert_eq!(q, cache_path(&long).unwrap());
    }

    #[test]
    fn comes_back_defaults() {
        assert!(comes_back(Tier::Safe, None).contains("见 lose"));
        assert!(comes_back(Tier::Review, Some(LoseKind::Irreversible)).contains("回不来"));
        assert_eq!(lose_label(Some(LoseKind::Redownload)), Some("要重新下载"));
    }
}
