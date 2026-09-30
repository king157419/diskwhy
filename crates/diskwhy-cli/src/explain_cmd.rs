//! `diskwhy-cli explain <root> --top 50`：扫描后对前 N 大目录逐个解释，并统计覆盖率。

use super::Args;
use diskwhy_core::action::protect::Protector;
use diskwhy_core::explain::candidates::{cleanup_candidates, CandidateGroup};
use diskwhy_core::explain::{tier_summary, ExplainCtx, Explanation, Source, Tier, TierSummary};
use diskwhy_core::view::{explain_path, human_bytes, top_dirs};
use diskwhy_core::ScanResult;
use serde::Serialize;
use std::fmt::Write as _;
use std::time::Instant;

#[derive(Serialize)]
struct Row {
    rank: usize,
    path: String,
    allocated: u64,
    /// safe / review / keep / unknown
    tier: String,
    source_kind: String,
    source: String,
    what: String,
    owner: Option<String>,
    clues: Vec<String>,
    explanation: Explanation,
}

#[derive(Serialize)]
struct Report {
    root: String,
    scan_seconds: f64,
    used_bytes: u64,
    rules_loaded: usize,
    rule_warnings: Vec<String>,
    registry_apps: usize,
    registry_ms: u64,
    explain_ms: u64,
    counts: Counts,
    rows: Vec<Row>,
    tiers: TierSummary,
    /// 清理页的候选（阶段 F）。
    candidates_ms: u64,
    candidates: Vec<CandidateGroup>,
}

#[derive(Serialize, Default)]
struct Counts {
    total: usize,
    rule_exact: usize,
    rule_inherited: usize,
    heuristic: usize,
    llm: usize,
    unknown: usize,
    /// (规则 + 启发式) / 总数
    coverage_pct: f64,
}

fn tier_str(e: &Explanation) -> &'static str {
    if e.is_unknown() {
        return "unknown";
    }
    match e.tier {
        Tier::Safe => "safe",
        Tier::Review => "review",
        Tier::Keep => "keep",
    }
}

fn tier_cn(t: &str) -> &'static str {
    match t {
        "safe" => "可清理",
        "review" => "需判断",
        "keep" => "保留",
        _ => "未知",
    }
}

fn source_str(e: &Explanation) -> (String, String) {
    match &e.source {
        Source::Rule { id, inherited_from: None, .. } => ("rule".into(), format!("规则 {id}")),
        Source::Rule { id, inherited_from: Some(_), .. } => ("rule-inherited".into(), format!("规则 {id}（继承上层）")),
        Source::Heuristic { name } => ("heuristic".into(), format!("启发式 {name}")),
        Source::Llm { model } => ("llm".into(), format!("AI 推测 {model}")),
        Source::Unknown => ("unknown".into(), "未知".into()),
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

pub fn run(r: &ScanResult, args: &Args) {
    let t0 = Instant::now();
    let (ctx, warnings) = ExplainCtx::load_default();
    let load_ms = t0.elapsed().as_millis();

    let t1 = Instant::now();
    let tops = top_dirs(&r.tree, args.top);
    let mut rows = Vec::new();
    let mut c = Counts { total: tops.len(), ..Default::default() };
    for (i, d) in tops.iter().enumerate() {
        let Some((path, e)) = explain_path(&r.tree, &d.path, &ctx) else { continue };
        let (kind, src) = source_str(&e);
        match kind.as_str() {
            "rule" => c.rule_exact += 1,
            "rule-inherited" => c.rule_inherited += 1,
            "heuristic" => c.heuristic += 1,
            "llm" => c.llm += 1,
            _ => c.unknown += 1,
        }
        rows.push(Row {
            rank: i + 1,
            path,
            allocated: d.allocated,
            tier: tier_str(&e).into(),
            source_kind: kind,
            source: src,
            what: e.what.clone(),
            owner: e.owner.clone(),
            clues: e.clues.clone(),
            explanation: e,
        });
    }
    let explain_ms = t1.elapsed().as_millis() as u64;
    let covered = c.rule_exact + c.rule_inherited + c.heuristic;
    c.coverage_pct = if c.total > 0 { covered as f64 * 100.0 / c.total as f64 } else { 0.0 };
    let tiers = tier_summary(&r.tree, &ctx);
    let t2 = Instant::now();
    let protector = Protector::from_system();
    let candidates = cleanup_candidates(&r.tree, &ctx, &|p: &str| protector.check(p).is_some());
    let candidates_ms = t2.elapsed().as_millis() as u64;

    // ── 终端输出 ──
    println!("DiskWhy 解释：{}", r.root);
    println!(
        "扫描 {:.1} 秒 | 卷已用 {} | 规则 {} 条（含用户规则，加载+注册表共 {} ms）| 注册表已安装软件 {} 个（读取 {} ms）| 解释前 {} 大目录用时 {} ms",
        r.duration_ms as f64 / 1000.0,
        human_bytes(r.used_bytes),
        ctx.rules.rules.len(),
        load_ms,
        ctx.registry.apps.len(),
        ctx.registry.load_ms,
        tops.len(),
        explain_ms
    );
    for w in &warnings {
        println!("规则警告：{w}");
    }
    println!();
    println!("{:>3}  {:>10}  {:<6}  {:<34}  路径", "#", "占用", "档位", "来源");
    for row in &rows {
        println!(
            "{:>3}  {:>10}  {:<6}  {:<34}  {}",
            row.rank,
            human_bytes(row.allocated),
            tier_cn(&row.tier),
            clip(&row.source, 34),
            row.path
        );
        println!("{:>17}→ {}{}", "", row.what, row.owner.as_ref().map(|o| format!("（属于：{o}）")).unwrap_or_default());
    }
    println!();
    println!(
        "统计（前 {} 大目录）：规则命中 {}（精确 {} + 继承上层 {}）| 启发式 {} | 未知 {} | 规则+启发式覆盖率 {:.1}%",
        c.total,
        c.rule_exact + c.rule_inherited,
        c.rule_exact,
        c.rule_inherited,
        c.heuristic,
        c.unknown,
        c.coverage_pct
    );
    println!(
        "全盘按规则库归档（{} ms）：可清理(safe) {} | 需判断(review) {} | 保留(keep) {} | 规则未覆盖 {} | 命中规则 {}/{}",
        tiers.duration_ms,
        human_bytes(tiers.safe_bytes),
        human_bytes(tiers.review_bytes),
        human_bytes(tiers.keep_bytes),
        human_bytes(tiers.uncovered_bytes),
        tiers.rules_hit,
        tiers.rules_total
    );
    let sum = |t: Tier, del: bool| -> u64 {
        candidates.iter().filter(|g| g.tier == t).map(|g| if del { g.deletable_bytes } else { g.bytes }).sum()
    };
    println!(
        "清理候选（{} ms）：{} 组 | 可放心清理 {}（能直接删 {}）| 需你判断 {}（能直接删 {}）",
        candidates_ms,
        candidates.len(),
        human_bytes(sum(Tier::Safe, false)),
        human_bytes(sum(Tier::Safe, true)),
        human_bytes(sum(Tier::Review, false)),
        human_bytes(sum(Tier::Review, true)),
    );
    for g in candidates.iter().take(args.top) {
        println!(
            "  {:<6} {:>10} {:>10}  {:<12} {:<10} {:<24} {} 个位置  {}",
            if g.tier == Tier::Safe { "可清理" } else { "需判断" },
            human_bytes(g.bytes),
            human_bytes(g.deletable_bytes),
            g.lose_kind.map(|k| k.as_str()).unwrap_or("-"),
            g.lose_short.as_deref().unwrap_or("-"),
            clip(&g.name, 24),
            g.paths.len(),
            g.paths.first().map(|p| p.path.as_str()).unwrap_or("")
        );
    }
    let unknowns: Vec<&Row> = rows.iter().filter(|r| r.source_kind == "unknown").collect();
    if !unknowns.is_empty() {
        println!();
        println!("未知项（下一批规则的待办）：");
        for u in &unknowns {
            println!("  #{} {} {}", u.rank, human_bytes(u.allocated), u.path);
            for cl in &u.clues {
                println!("      · {cl}");
            }
        }
    }

    let report = Report {
        root: r.root.clone(),
        scan_seconds: r.duration_ms as f64 / 1000.0,
        used_bytes: r.used_bytes,
        rules_loaded: ctx.rules.rules.len(),
        rule_warnings: warnings.clone(),
        registry_apps: ctx.registry.apps.len(),
        registry_ms: ctx.registry.load_ms,
        explain_ms,
        counts: c,
        rows,
        tiers,
        candidates_ms,
        candidates,
    };

    if let Some(p) = &args.json {
        match std::fs::File::create(p)
            .map_err(|e| e.to_string())
            .and_then(|f| serde_json::to_writer_pretty(std::io::BufWriter::new(f), &report).map_err(|e| e.to_string()))
        {
            Ok(()) => println!("\nJSON 已写入 {p}"),
            Err(e) => {
                eprintln!("写 JSON 失败：{e}");
                std::process::exit(1);
            }
        }
    }
    if let Some(p) = &args.md {
        match std::fs::write(p, markdown(&report)) {
            Ok(()) => println!("Markdown 已写入 {p}"),
            Err(e) => {
                eprintln!("写 Markdown 失败：{e}");
                std::process::exit(1);
            }
        }
    }
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn markdown(r: &Report) -> String {
    let c = &r.counts;
    let mut s = String::new();
    let _ = writeln!(s, "## 前 {} 大目录逐项解释（`diskwhy-cli explain {} --top {}` 原始输出）\n", c.total, r.root, c.total);
    let _ = writeln!(
        s,
        "扫描 {:.1} 秒，卷已用 {}；规则 {} 条，注册表已安装软件 {} 个（读取 {} ms），解释 {} 个目录用时 {} ms。\n",
        r.scan_seconds,
        human_bytes(r.used_bytes),
        r.rules_loaded,
        r.registry_apps,
        r.registry_ms,
        c.total,
        r.explain_ms
    );
    let _ = writeln!(s, "| # | 占用 | 档位 | 来源 | 路径 | 这是什么 |");
    let _ = writeln!(s, "|---:|---:|---|---|---|---|");
    for row in &r.rows {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | `{}` | {} |",
            row.rank,
            human_bytes(row.allocated),
            tier_cn(&row.tier),
            md_escape(&row.source),
            row.path,
            md_escape(&row.what)
        );
    }
    let _ = writeln!(s, "\n### 统计\n");
    let _ = writeln!(s, "| 来源 | 个数 |\n|---|---:|");
    let _ = writeln!(s, "| 规则精确命中 | {} |", c.rule_exact);
    let _ = writeln!(s, "| 规则继承上层 | {} |", c.rule_inherited);
    let _ = writeln!(s, "| 启发式 | {} |", c.heuristic);
    let _ = writeln!(s, "| 未知 | {} |", c.unknown);
    let _ = writeln!(s, "| **规则 + 启发式覆盖率** | **{:.1}%**（{}/{}） |", c.coverage_pct, c.rule_exact + c.rule_inherited + c.heuristic, c.total);
    let t = &r.tiers;
    let _ = writeln!(s, "\n### 全盘按规则库归档（不含启发式）\n");
    let _ = writeln!(s, "| 档位 | 字节 |\n|---|---:|");
    let _ = writeln!(s, "| 可清理 safe | {} |", human_bytes(t.safe_bytes));
    let _ = writeln!(s, "| 需判断 review | {} |", human_bytes(t.review_bytes));
    let _ = writeln!(s, "| 保留 keep | {} |", human_bytes(t.keep_bytes));
    let _ = writeln!(s, "| 规则未覆盖 | {} |", human_bytes(t.uncovered_bytes));
    let _ = writeln!(s, "\n命中过的规则 {}/{}。safe 档最大的几项：", t.rules_hit, t.rules_total);
    for it in &t.top_safe {
        let _ = writeln!(s, "- `{}` {}：{}", it.id, human_bytes(it.bytes), it.what);
    }
    let unknowns: Vec<&Row> = r.rows.iter().filter(|x| x.source_kind == "unknown").collect();
    let _ = writeln!(s, "\n### 未知项（下一批规则的待办）\n");
    if unknowns.is_empty() {
        let _ = writeln!(s, "（无）");
    }
    for u in unknowns {
        let _ = writeln!(s, "- **#{} `{}`**（{}）", u.rank, u.path, human_bytes(u.allocated));
        for cl in &u.clues {
            let _ = writeln!(s, "  - {}", md_escape(cl));
        }
    }
    s
}
