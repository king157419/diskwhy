//! `diskwhy-cli explain`：解释一个路径本身（第 0 行）和它下面前 N 大目录，并统计覆盖率。
//!
//! 输入可以是现扫的路径，也可以是 `scan --json` 的输出 / 快照（`--from`）。
//! JSON 里的目录树是截断过的：规则只看路径，截断不影响；启发式要看子树内容，
//! 所以规则没命中、而子树又不完整时，回磁盘把这个文件夹重新读一遍再算（只读；`--no-reread` 关掉）。

use super::Args;
use diskwhy_core::explain::llm::{replace_ci, HttpOracle, LlmOracle};
use diskwhy_core::explain::rules::Scope;
use diskwhy_core::explain::{tier_summary, ExplainCtx, Explanation, Source, Tier, TierSummary};
use diskwhy_core::scan::{normalize_root, scan};
use diskwhy_core::view::{explain_at, find_node, human_bytes, outer_ancestors, subtree_complete, top_dirs};
use diskwhy_core::{Node, NodeKind, ScanResult};
use serde::Serialize;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// `--json` 数组里的一项：一条完整的 Explanation，外加是哪个路径、多大、排第几。
#[derive(Serialize)]
struct Row {
    /// 0 = 目标本身；1.. = 前 N 大目录的名次。
    rank: usize,
    path: String,
    allocated: u64,
    /// 这一行是从磁盘重新读了子树后算的（JSON 里被截断了）。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    reread: bool,
    #[serde(flatten)]
    explanation: Explanation,
}

#[derive(Serialize)]
struct Report<'a> {
    root: String,
    target: String,
    input: &'a str,
    tree_complete: bool,
    scan_seconds: f64,
    used_bytes: u64,
    rules_loaded: usize,
    rule_warnings: Vec<String>,
    registry_apps: usize,
    registry_ms: u64,
    explain_ms: u64,
    reread_dirs: usize,
    reread_ms: u64,
    llm_requested: bool,
    llm_calls: usize,
    counts: Counts,
    rows: &'a [Row],
    tiers: Option<TierSummary>,
}

#[derive(Serialize, Default, Clone)]
struct Counts {
    /// 参与统计的行数（前 N 大目录；`--top 0` 时就是目标本身）。
    total: usize,
    rule_exact: usize,
    /// rule_exact 里命中的是“容器”规则（scope = self，只说明文件夹本身，如 C:\Users）。
    rule_container: usize,
    rule_inherited: usize,
    heuristic: usize,
    llm: usize,
    unknown: usize,
    rule_pct: f64,
    heuristic_pct: f64,
    llm_pct: f64,
    unknown_pct: f64,
    /// (规则 + 启发式) / 总数
    coverage_pct: f64,
}

fn tier_key(e: &Explanation) -> &'static str {
    if e.is_unknown() {
        return "unknown";
    }
    match e.tier {
        Tier::Safe => "safe",
        Tier::Review => "review",
        Tier::Keep => "keep",
    }
}

fn tier_cn(e: &Explanation) -> &'static str {
    match tier_key(e) {
        "safe" => "可清理",
        "review" => "需判断",
        "keep" => "保留",
        _ => "未知",
    }
}

fn source_label(e: &Explanation, ctx: &ExplainCtx) -> String {
    match &e.source {
        Source::Rule { id, inherited_from: None, .. } => {
            if is_container(e, ctx) {
                format!("规则 {id}（容器）")
            } else {
                format!("规则 {id}")
            }
        }
        Source::Rule { id, inherited_from: Some(_), .. } => format!("规则 {id}（继承上层）"),
        Source::Heuristic { name } => format!("启发式 {name}"),
        Source::Llm { model } => format!("AI 推测 {model}"),
        Source::Unknown => "未知".into(),
    }
}

fn is_container(e: &Explanation, ctx: &ExplainCtx) -> bool {
    match &e.source {
        Source::Rule { id, inherited_from: None, .. } => ctx.rules.by_id(id).is_some_and(|r| r.scope == Scope::SelfOnly),
        _ => false,
    }
}

/// 按显示宽度（中文算 2）截断。
fn clip(s: &str, width: usize) -> String {
    let mut w = 0;
    let mut out = String::new();
    for ch in s.chars() {
        let cw = if ch.is_ascii() { 1 } else { 2 };
        if w + cw > width {
            out.push('…');
            return out;
        }
        w += cw;
        out.push(ch);
    }
    out
}

/// 输出到哪：`--json -` 时标准输出留给 JSON，人看的部分改走标准错误。
struct Out {
    to_stderr: bool,
}

impl Out {
    fn line(&self, s: impl AsRef<str>) {
        if self.to_stderr {
            eprintln!("{}", s.as_ref());
        } else {
            println!("{}", s.as_ref());
        }
    }
}

pub fn run(r: &ScanResult, args: &Args, target: Option<&str>) {
    let out = Out { to_stderr: args.json.as_deref() == Some("-") };
    let t0 = Instant::now();
    let (ctx, warnings) = ExplainCtx::load_default();
    let load_ms = t0.elapsed().as_millis();
    let outer = outer_ancestors(&r.root);
    let tree_complete = subtree_complete(&r.tree);
    let input = if args.from.is_some() {
        if tree_complete {
            "快照 / 完整的 JSON"
        } else {
            "scan --json 输出（目录树截断过）"
        }
    } else {
        "现扫"
    };

    let target_path = target.map(normalize_root).unwrap_or_else(|| r.tree.path.clone());
    let Some((_, target_node)) = find_node(&r.tree, &target_path) else {
        eprintln!("扫描结果里找不到「{target_path}」（JSON 里只保留了前几层和前 N 大目录；可以直接 explain 这个路径让它现扫）");
        std::process::exit(1);
    };

    // --llm：开关打开并且设置里启用了才真的会问。
    let oracle = HttpOracle { settings: &ctx.settings };
    let llm: Option<&dyn LlmOracle> = if args.llm && ctx.settings.usable() { Some(&oracle) } else { None };
    if args.llm && llm.is_none() {
        out.line("提示：用了 --llm，但设置里 AI 推测还没启用（%LOCALAPPDATA%\\DiskWhy\\settings.json 的 enabled / 地址 / 模型），本次不会联网。");
    }

    let t1 = Instant::now();
    let mut paths: Vec<(usize, String, u64)> = vec![(0, target_path.clone(), target_node.allocated)];
    if target_node.kind == NodeKind::Dir {
        for (i, d) in top_dirs(target_node, args.top).iter().enumerate() {
            paths.push((i + 1, d.path.clone(), d.allocated));
        }
    }
    let mut rows = Vec::new();
    let mut reread_dirs = 0usize;
    let mut reread_ms = 0u64;
    let mut llm_calls = 0usize;
    for (rank, path, allocated) in paths {
        let Some((chain, node)) = find_node(&r.tree, &path) else { continue };
        let full = if node.kind == NodeKind::Dir { node.path.clone() } else { path.clone() };
        let anc: Vec<&Node> = outer.iter().chain(chain.iter().copied()).collect();
        let mut e = explain_at(node, &full, &anc, &ctx, None);
        let mut fresh: Option<ScanResult> = None;
        if !e.is_rule() && node.kind == NodeKind::Dir && !subtree_complete(node) && !args.no_reread {
            let t = Instant::now();
            if let Ok(f) = scan(&full, &args.scan_options(), &AtomicBool::new(false), |_| {}) {
                e = explain_at(&f.tree, &full, &anc, &ctx, None);
                e.notes.push("扫描结果 JSON 里这个文件夹的子目录被截断了，这条说明是重新读了一遍磁盘（只读）后算的。".into());
                fresh = Some(f);
                reread_dirs += 1;
            }
            reread_ms += t.elapsed().as_millis() as u64;
        }
        let reread = fresh.is_some();
        if e.is_unknown() && llm.is_some() {
            let n = fresh.as_ref().map(|f| &f.tree).unwrap_or(node);
            let before = e.notes.clone();
            e = explain_at(n, &full, &anc, &ctx, llm);
            if reread && !e.is_unknown() {
                e.notes.extend(before);
            }
            llm_calls += 1;
        }
        rows.push(Row { rank, path: full, allocated, reread, explanation: e });
    }
    let explain_ms = t1.elapsed().as_millis() as u64;

    // 统计：前 N 大目录（--top 0 时就是目标本身）。
    let stat_rows: Vec<&Row> = if rows.len() > 1 { rows.iter().filter(|x| x.rank > 0).collect() } else { rows.iter().collect() };
    let mut c = Counts { total: stat_rows.len(), ..Default::default() };
    for row in &stat_rows {
        let e = &row.explanation;
        match &e.source {
            Source::Rule { inherited_from: None, .. } => {
                c.rule_exact += 1;
                if is_container(e, &ctx) {
                    c.rule_container += 1;
                }
            }
            Source::Rule { .. } => c.rule_inherited += 1,
            Source::Heuristic { .. } => c.heuristic += 1,
            Source::Llm { .. } => c.llm += 1,
            Source::Unknown => c.unknown += 1,
        }
    }
    let p = |n: usize| if c.total > 0 { n as f64 * 100.0 / c.total as f64 } else { 0.0 };
    c.rule_pct = p(c.rule_exact + c.rule_inherited);
    c.heuristic_pct = p(c.heuristic);
    c.llm_pct = p(c.llm);
    c.unknown_pct = p(c.unknown);
    c.coverage_pct = p(c.rule_exact + c.rule_inherited + c.heuristic);
    let tiers = (target_node.kind == NodeKind::Dir).then(|| tier_summary(target_node, &ctx));

    // ── 人看的输出 ──
    out.line(format!("DiskWhy 解释：{target_path}（输入：{input}）"));
    out.line(format!(
        "扫描 {:.1} 秒 | 卷已用 {} | 规则 {} 条（含用户规则，加载+注册表共 {} ms）| 注册表已安装软件 {} 个 | 解释 {} 行用时 {} ms{}",
        r.duration_ms as f64 / 1000.0,
        human_bytes(r.used_bytes),
        ctx.rules.rules.len(),
        load_ms,
        ctx.registry.apps.len(),
        rows.len(),
        explain_ms,
        if reread_dirs > 0 { format!("（其中回磁盘补读 {reread_dirs} 个文件夹 {reread_ms} ms）") } else { String::new() }
    ));
    for w in &warnings {
        out.line(format!("规则警告：{w}"));
    }
    out.line("");
    out.line(format!("{:>3}  {:>10}  {:<6}  {:<34}  路径", "#", "占用", "档位", "来源"));
    for row in &rows {
        let e = &row.explanation;
        out.line(format!(
            "{:>3}  {:>10}  {:<6}  {:<34}  {}",
            if row.rank == 0 { "本身".to_string() } else { row.rank.to_string() },
            human_bytes(row.allocated),
            tier_cn(e),
            clip(&source_label(e, &ctx), 34),
            row.path
        ));
        out.line(format!("{:>17}→ {}{}", "", e.what, e.owner.as_ref().map(|o| format!("（属于：{o}）")).unwrap_or_default()));
    }
    out.line("");
    out.line(format!(
        "统计（{} 个目录）：规则 {}（精确 {}，其中容器 {}；继承上层 {}）= {:.1}% | 启发式 {} = {:.1}% | AI {} | 未知 {} = {:.1}% | 规则+启发式 {:.1}%",
        c.total,
        c.rule_exact + c.rule_inherited,
        c.rule_exact,
        c.rule_container,
        c.rule_inherited,
        c.rule_pct,
        c.heuristic,
        c.heuristic_pct,
        c.llm,
        c.unknown,
        c.unknown_pct,
        c.coverage_pct
    ));
    if args.llm {
        out.line(format!("--llm：问了 AI {llm_calls} 次（只问规则和启发式都认不出的目录）"));
    }
    if let Some(t) = &tiers {
        out.line(format!(
            "按规则库归档（{} ms{}）：可清理(safe) {} | 需判断(review) {} | 保留(keep) {} | 规则未覆盖 {} | 命中规则 {}/{}",
            t.duration_ms,
            if tree_complete { "" } else { "，JSON 树截断过，只按保留下来的层算，偏粗" },
            human_bytes(t.safe_bytes),
            human_bytes(t.review_bytes),
            human_bytes(t.keep_bytes),
            human_bytes(t.uncovered_bytes),
            t.rules_hit,
            t.rules_total
        ));
    }
    let unknowns: Vec<&&Row> = stat_rows.iter().filter(|r| r.explanation.is_unknown()).collect();
    if !unknowns.is_empty() {
        out.line("");
        out.line("未知项（下一批规则的待办）：");
        for u in &unknowns {
            out.line(format!("  #{} {} {}", u.rank, human_bytes(u.allocated), u.path));
            for cl in &u.explanation.clues {
                out.line(format!("      · {cl}"));
            }
        }
    }

    // ── 文件输出 ──
    if let Some(p) = &args.json {
        let res = if p == "-" {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            serde_json::to_writer_pretty(&mut lock, &rows).map_err(|e| e.to_string()).and_then(|_| writeln!(lock).map_err(|e| e.to_string()))
        } else {
            std::fs::File::create(p)
                .map_err(|e| e.to_string())
                .and_then(|f| serde_json::to_writer_pretty(std::io::BufWriter::new(f), &rows).map_err(|e| e.to_string()))
        };
        match res {
            Ok(()) if p != "-" => out.line(format!("\nExplanation 数组（{} 项）已写入 {p}", rows.len())),
            Ok(()) => {}
            Err(e) => {
                eprintln!("写 JSON 失败：{e}");
                std::process::exit(1);
            }
        }
    }
    let report = Report {
        root: r.root.clone(),
        target: target_path.clone(),
        input,
        tree_complete,
        scan_seconds: r.duration_ms as f64 / 1000.0,
        used_bytes: r.used_bytes,
        rules_loaded: ctx.rules.rules.len(),
        rule_warnings: warnings.clone(),
        registry_apps: ctx.registry.apps.len(),
        registry_ms: ctx.registry.load_ms,
        explain_ms,
        reread_dirs,
        reread_ms,
        llm_requested: args.llm,
        llm_calls,
        counts: c,
        rows: &rows,
        tiers,
    };
    if let Some(p) = &args.report {
        match std::fs::File::create(p)
            .map_err(|e| e.to_string())
            .and_then(|f| serde_json::to_writer_pretty(std::io::BufWriter::new(f), &report).map_err(|e| e.to_string()))
        {
            Ok(()) => out.line(format!("报告已写入 {p}")),
            Err(e) => {
                eprintln!("写报告失败：{e}");
                std::process::exit(1);
            }
        }
    }
    if let Some(p) = &args.md {
        match std::fs::write(p, markdown(&report, &ctx)) {
            Ok(()) => out.line(format!("Markdown 已写入 {p}")),
            Err(e) => {
                eprintln!("写 Markdown 失败：{e}");
                std::process::exit(1);
            }
        }
    }
}

// ───────────────────────── Markdown（docs/coverage.md 的表格部分） ─────────────────────────

/// 把路径和文字里的 `\Users\<用户名>` 换成 `\Users\<用户>`。
fn redact(s: &str, user: &str) -> String {
    if user.len() < 2 {
        return s.to_string();
    }
    replace_ci(s, &format!("\\Users\\{user}"), "\\Users\\<用户>")
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn current_user(ctx: &ExplainCtx) -> String {
    ctx.env
        .get("USERPROFILE")
        .and_then(|p| p.trim_end_matches('\\').rsplit('\\').next().map(str::to_string))
        .or_else(|| ctx.env.get("USERNAME").map(str::to_string))
        .unwrap_or_default()
}

fn markdown(r: &Report, ctx: &ExplainCtx) -> String {
    let user = current_user(ctx);
    let red = |s: &str| md_escape(&redact(s, &user));
    let c = &r.counts;
    let mut s = String::new();
    let _ = writeln!(s, "## 前 {} 大目录逐项解释\n", c.total);
    let _ = writeln!(
        s,
        "目标 `{}`（输入：{}）。扫描 {:.1} 秒，卷已用 {}；规则 {} 条，注册表已安装软件 {} 个；解释 {} 行用时 {} ms{}。路径里的用户名已换成 `<用户>`。\n",
        red(&r.target),
        r.input,
        r.scan_seconds,
        human_bytes(r.used_bytes),
        r.rules_loaded,
        r.registry_apps,
        r.rows.len(),
        r.explain_ms,
        if r.reread_dirs > 0 { format!("（其中回磁盘补读 {} 个被截断的文件夹，{} ms）", r.reread_dirs, r.reread_ms) } else { String::new() }
    );
    let _ = writeln!(s, "| # | 目录 | 大小 | 档位 | 来源 | 一句话说明 |");
    let _ = writeln!(s, "|---:|---|---:|---|---|---|");
    for row in r.rows {
        let e = &row.explanation;
        let owner = e.owner.as_ref().map(|o| format!("（属于：{}）", red(o))).unwrap_or_default();
        let _ = writeln!(
            s,
            "| {} | `{}` | {} | {} | {} | {}{} |",
            if row.rank == 0 { "本身".to_string() } else { row.rank.to_string() },
            redact(&row.path, &user),
            human_bytes(row.allocated),
            tier_cn(e),
            md_escape(&source_label(e, ctx)),
            red(&e.what),
            owner
        );
    }
    let _ = writeln!(s, "\n### 命中率（{} 个目录）\n", c.total);
    let _ = writeln!(s, "| 来源 | 个数 | 占比 |\n|---|---:|---:|");
    let _ = writeln!(s, "| 规则精确命中 | {} | {:.1}% |", c.rule_exact, pct(c.rule_exact, c.total));
    let _ = writeln!(s, "| 　其中“容器”规则（只说明文件夹本身，如 `C:\\Users`） | {} | {:.1}% |", c.rule_container, pct(c.rule_container, c.total));
    let _ = writeln!(s, "| 规则继承上层 | {} | {:.1}% |", c.rule_inherited, pct(c.rule_inherited, c.total));
    let _ = writeln!(s, "| **规则合计** | **{}** | **{:.1}%** |", c.rule_exact + c.rule_inherited, c.rule_pct);
    let _ = writeln!(s, "| 启发式 | {} | {:.1}% |", c.heuristic, c.heuristic_pct);
    if c.llm > 0 {
        let _ = writeln!(s, "| AI 推测 | {} | {:.1}% |", c.llm, c.llm_pct);
    }
    let _ = writeln!(s, "| 未知 | {} | {:.1}% |", c.unknown, c.unknown_pct);
    let _ = writeln!(s, "| **规则 + 启发式** | **{}** | **{:.1}%** |", c.rule_exact + c.rule_inherited + c.heuristic, c.coverage_pct);
    if let Some(t) = &r.tiers {
        let _ = writeln!(s, "\n### 按规则库归档（不含启发式{}）\n", if r.tree_complete { "" } else { "；JSON 树截断过，数字偏粗" });
        let _ = writeln!(s, "| 档位 | 字节 |\n|---|---:|");
        let _ = writeln!(s, "| 可清理 safe | {} |", human_bytes(t.safe_bytes));
        let _ = writeln!(s, "| 需判断 review | {} |", human_bytes(t.review_bytes));
        let _ = writeln!(s, "| 保留 keep | {} |", human_bytes(t.keep_bytes));
        let _ = writeln!(s, "| 规则未覆盖 | {} |", human_bytes(t.uncovered_bytes));
        let _ = writeln!(s, "\n命中过的规则 {}/{}。", t.rules_hit, t.rules_total);
    }
    // 没被规则说明的（启发式 + AI + 未知），按大小取前 10。
    let mut missed: Vec<&Row> = r.rows.iter().filter(|x| x.rank > 0 && !x.explanation.is_rule()).collect();
    missed.sort_by(|a, b| b.allocated.cmp(&a.allocated));
    let _ = writeln!(s, "\n### 规则没命中的前 10 个目录（下一批规则的待办）\n");
    if missed.is_empty() {
        let _ = writeln!(s, "（无）");
    }
    for m in missed.iter().take(10) {
        let e = &m.explanation;
        let _ = writeln!(s, "- **#{} `{}`**（{}）：{} — {}", m.rank, redact(&m.path, &user), human_bytes(m.allocated), md_escape(&source_label(e, ctx)), red(&e.what));
        for cl in e.clues.iter().take(6) {
            let _ = writeln!(s, "  - {}", red(cl));
        }
    }
    s
}

fn pct(n: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        n as f64 * 100.0 / total as f64
    }
}
