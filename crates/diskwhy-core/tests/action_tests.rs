//! 阶段 D：带护栏的动作。**所有真实删除 / 移动 / 回收都只针对本测试自己在临时目录里创建的文件。**
//!
//! - 保护路径：每一类一个用例（含祖先、大小写变体、`\\?\` 前缀、结尾的点、`/`、链接绕路、短文件名）；
//! - 档位门槛：keep / 未知 / AI 推测 / 只能用工具 的项被拒绝，safe / review 放行；
//! - 规则一致性：规则目录的父目录、加入时和现在规则不一致 → 拒绝；
//! - 热文件跳过、云占位跳过（attrib +O）、链接只删自身（mklink /J，删后目标完好）；
//! - 重新校验：生成清单后替换 / 修改文件 → 跳过；
//! - 隔离区 round-trip：同卷（改名）+ 跨卷（复制→校验→删除），manifest 字段、SHA-256 一致、还原回原位；
//! - 回收站：真的进了 `$Recycle.Bin\<SID>`（解析 $I 记录找到原路径），再把**这一条**还原回来；
//! - 占用检测：自己开句柄占住文件，Restart Manager 报告当前进程 PID，进程没被杀；
//! - 日志：每个动作一行、字段齐全；
//! - 执行后刷新子树。
#![cfg(windows)]

use diskwhy_core::action::log::{ActionLog, FIELDS};
use diskwhy_core::action::protect::Protector;
use diskwhy_core::action::quarantine::{self, Manifest};
use diskwhy_core::action::{self, Decision, EntryKind, ExecOptions, PlanRequest, PreflightOptions, SkipReason, Strategy};
use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::LlmSettings;
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::RuleSet;
use diskwhy_core::explain::{ExplainCtx, Source};
use diskwhy_core::scan::{scan_with, volume_root_of};
use diskwhy_core::win::{volume_space, Win32Fs};
use diskwhy_core::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

// ───────────────────────── 工具 ─────────────────────────

struct TempTree(PathBuf);

fn uniq() -> String {
    format!("{}-{}", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos())
}

impl TempTree {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("diskwhy-act-{tag}-{}", uniq()));
        fs::create_dir_all(&p).unwrap();
        TempTree(p)
    }
    fn in_dir(base: &Path, tag: &str) -> Self {
        let p = base.join(format!("diskwhy-act-{tag}-{}", uniq()));
        fs::create_dir_all(&p).unwrap();
        TempTree(p)
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
    fn s(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.0.to_string_lossy().to_string()
        } else {
            self.p(rel).to_string_lossy().to_string()
        }
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        // 清掉 attrib +O 等属性再删（只删本测试自己建的临时目录）
        let _ = Command::new("attrib").args(["-O", "-R", "/S", "/D"]).arg(format!("{}\\*", self.0.display())).output();
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 写文件并把修改时间调到 2 小时前（避开“30 分钟内修改过”）。
fn write_old(p: &Path, data: &[u8]) {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(p, data).unwrap();
    backdate(p, 7200);
}

fn backdate(p: &Path, secs: u64) {
    let f = fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(secs)).unwrap();
}

fn sha(p: &Path) -> String {
    let data = fs::read(p).unwrap();
    let h = Sha256::digest(&data);
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn content(tag: &str, n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    let seed = tag.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let mut x = seed;
    for _ in 0..n {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        v.push((x >> 16) as u8);
    }
    v
}

const TEST_RULES: &str = r#"
[[rule]]
id = "t-cache"
owner = "测试"
category = "cache"
tier = "safe"
match = ['%TEMP%\cache']
what = "测试用缓存"
why = "测试"
lose = "不丢数据"
how = "delete"
how_detail = ""
refs = ["https://example.com/"]
verified_on = "2026-09-23"

[[rule]]
id = "t-review"
owner = "测试"
category = "model"
tier = "review"
lose_kind = "redownload"
lose_short = "模型要重新下载"
match = ['%TEMP%\review']
what = "测试用模型"
why = "测试"
lose = "删了要重新下载 3 GB 的模型"
how = ["migrate", "delete"]
how_detail = "可以迁走"
refs = ["https://example.com/"]
verified_on = "2026-09-23"

[[rule]]
id = "t-keep"
owner = "测试"
category = "userdata"
tier = "keep"
match = ['%TEMP%\keep']
what = "测试用个人数据"
why = "测试"
lose = "会丢数据"
how = "migrate"
how_detail = "迁到别的盘"
refs = ["https://example.com/"]
verified_on = "2026-09-23"

[[rule]]
id = "t-tool"
owner = "测试"
category = "cache"
tier = "safe"
match = ['%TEMP%\tool']
what = "只能用工具清的缓存"
why = "测试"
lose = "不丢数据"
how = "builtin_tool"
how_detail = "用自带工具清"
refs = ["https://example.com/"]
verified_on = "2026-09-23"

[[rule]]
id = "t-nested"
owner = "测试"
category = "cache"
tier = "safe"
match = ['%TEMP%\parent\cache']
what = "嵌套缓存"
why = "测试"
lose = "不丢数据"
how = "delete"
how_detail = ""
refs = ["https://example.com/"]
verified_on = "2026-09-23"
"#;

fn ctx_for(root: &str) -> ExplainCtx {
    let env = EnvMap::from_pairs(&[("TEMP", root)]);
    let (rules, errs) = RuleSet::from_sources(&[("test", TEST_RULES)], &env);
    assert!(errs.is_empty(), "测试规则校验失败：{errs:?}");
    ExplainCtx::new(env, rules, RegistrySnapshot::default(), LlmSettings::default())
}

fn scan_dir(root: &str) -> ScanResult {
    let vol = volume_space(&volume_root_of(root).unwrap()).unwrap();
    scan_with(&Win32Fs, root, vol, false, &ScanOptions::default(), &AtomicBool::new(false), |_| {}).unwrap()
}

fn req(p: &str) -> PlanRequest {
    PlanRequest { path: p.to_string(), expect_rule: None }
}

fn mklink_j(link: &Path, target: &Path) {
    let out = Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(link).arg(target).output().unwrap();
    assert!(out.status.success(), "mklink /J 失败：{}", String::from_utf8_lossy(&out.stderr));
}

fn test_protector() -> Protector {
    let vars: Vec<(String, String)> = [
        ("SystemRoot", r"C:\Windows"),
        ("ProgramFiles", r"C:\Program Files"),
        ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
        ("ProgramData", r"C:\ProgramData"),
        ("USERPROFILE", r"C:\Users\alice"),
        ("OneDrive", r"C:\Users\alice\OneDrive"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    let known = vec![(r"D:\Docs".to_string(), "文档")];
    Protector::with(&vars, &known, &[r"C:\".into(), r"D:\".into()])
}

fn assert_blocked(p: &Protector, path: &str, must_contain: &str) {
    match p.check(path) {
        Some(why) => assert!(why.contains(must_contain), "「{path}」被拒绝了，但原因「{why}」里没有「{must_contain}」"),
        None => panic!("「{path}」应该被拒绝，却放行了"),
    }
}

fn assert_allowed(p: &Protector, path: &str) {
    if let Some(why) = p.check(path) {
        panic!("「{path}」应该放行，却被拒绝：{why}");
    }
}

// ───────────────────────── 1. 保护路径 ─────────────────────────

#[test]
fn protect_rejects_volume_roots() {
    let p = test_protector();
    for x in [r"C:\", "c:", r"\\?\C:\", "d:/", r"E:\", r"C:\."] {
        assert_blocked(&p, x, "卷根");
    }
}

#[test]
fn protect_rejects_windows_winsxs_installer() {
    let p = test_protector();
    for x in [r"C:\Windows", r"c:\windows\system32\drivers", r"\\?\C:\WINDOWS\Temp", "C:/Windows/", r"C:\Windows.\Fonts", r"D:\Windows\x"] {
        assert_blocked(&p, x, "Windows");
    }
    for x in [r"C:\Windows\WinSxS", r"C:\WINDOWS\winsxs\amd64_foo", r"D:\Backup\WinSxS\x"] {
        assert_blocked(&p, x, "WinSxS");
    }
    assert_blocked(&p, r"C:\Windows\Installer\1a2b.msi", "Installer");
    assert_blocked(&p, r"\\?\c:\windows\INSTALLER", "Installer");
}

#[test]
fn protect_rejects_program_files() {
    let p = test_protector();
    for x in [r"C:\Program Files", r"C:\Program Files\App\cache", r"C:\PROGRAM FILES (X86)\Foo", r"\\?\D:\Program Files\x", r"E:\Program Files (Arm)"] {
        assert_blocked(&p, x, "程序安装目录");
    }
}

#[test]
fn protect_rejects_programdata_microsoft_windows() {
    let p = test_protector();
    assert_blocked(&p, r"C:\ProgramData\Microsoft\Windows", "ProgramData");
    assert_blocked(&p, r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs", "ProgramData");
    assert_blocked(&p, r"c:\programdata\MICROSOFT", "上层");
    assert_blocked(&p, r"C:\ProgramData", "上层");
    assert_allowed(&p, r"C:\ProgramData\SomeVendor\Cache");
    assert_allowed(&p, r"C:\ProgramData\Microsoft\Office\cache");
}

#[test]
fn protect_rejects_personal_folders_and_direct_children() {
    let p = test_protector();
    for (x, w) in [
        (r"C:\Users\alice\Desktop", "桌面"),
        (r"C:\Users\alice\Desktop\todo.txt", "桌面"),
        (r"C:\Users\alice\Documents", "文档"),
        (r"c:\users\ALICE\documents\Projects", "文档"),
        (r"C:\Users\alice\Pictures", "图片"),
        (r"C:\Users\alice\Videos\clip.mp4", "视频"),
        (r"C:\Users\alice\Music", "音乐"),
        (r"C:\Users\bob\Downloads\setup.exe", "下载"),
        (r"\\?\C:\USERS\BOB\DOWNLOADS", "下载"),
        (r"D:\Docs", "文档"),      // 已知文件夹被重定向到 D 盘
        (r"D:\Docs\report", "文档"), // 其直接子项
    ] {
        assert_blocked(&p, x, w);
    }
    // 更深一层按要求放行（规则 / 档位门槛会另外把关）
    assert_allowed(&p, r"C:\Users\alice\Downloads\proj\node_modules");
    assert_allowed(&p, r"D:\Docs\a\b");
}

#[test]
fn protect_rejects_secret_dirs() {
    let p = test_protector();
    for x in [r"C:\Users\alice\.ssh", r"C:\Users\alice\.ssh\id_ed25519", r"C:\Users\alice\.GnuPG", r"C:\Users\alice\AppData\Local\x\.aws\credentials", r"D:\work\.ssh"] {
        assert_blocked(&p, x, "密钥");
    }
}

#[test]
fn protect_rejects_onedrive_root() {
    let p = test_protector();
    assert_blocked(&p, r"C:\Users\alice\OneDrive", "OneDrive");
    assert_blocked(&p, r"C:\Users\carol\OneDrive - Contoso", "OneDrive");
    assert_blocked(&p, r"c:\users\alice\onedrive\", "OneDrive");
}

#[test]
fn protect_rejects_recycle_bin_and_svi() {
    let p = test_protector();
    assert_blocked(&p, r"C:\$Recycle.Bin", "回收站");
    assert_blocked(&p, r"C:\$RECYCLE.BIN\S-1-5-21-1\$R0ABC.bin", "回收站");
    assert_blocked(&p, r"D:\System Volume Information", "System Volume Information");
    assert_blocked(&p, r"\\?\d:\system volume information\x", "System Volume Information");
}

#[test]
fn protect_rejects_ancestors() {
    let p = test_protector();
    // 用户目录及其上层：里面有桌面 / 文档
    assert_blocked(&p, r"C:\Users", "用户目录");
    assert_blocked(&p, r"C:\Users\alice", "用户目录");
    assert_blocked(&p, r"c:\USERS\alice\", "用户目录");
    // 卷根是一切的祖先
    assert_blocked(&p, r"D:\", "卷根");
}

#[test]
fn protect_rejects_bad_path_forms() {
    let p = test_protector();
    assert_blocked(&p, r"C:\Users\alice\AppData\..\Desktop", "..");
    assert_blocked(&p, r"\\server\share\cache", "网络");
    assert_blocked(&p, r"\\?\UNC\server\share", "网络");
    assert_blocked(&p, r"relative\cache", "盘符");
    assert_blocked(&p, r"C:\temp\x:stream", "冒号");
    // 纯字符串模式（不访问文件系统）下，短文件名无法确认 → 拒绝
    assert_blocked(&p, r"C:\PROGRA~1\App", "短文件名");
}

#[test]
fn protect_allows_ordinary_cache_paths() {
    let p = test_protector();
    assert_allowed(&p, r"C:\Users\alice\AppData\Local\Temp\x");
    assert_allowed(&p, r"C:\Users\alice\AppData\Local\npm-cache");
    assert_allowed(&p, r"D:\projects\app\node_modules");
}

/// 真实文件系统：路径里的联接指向 C:\Windows、短文件名 PROGRA~1 —— 解析后仍然拒绝。
#[test]
fn protect_resolves_links_and_short_names_on_real_fs() {
    let p = Protector::from_system();
    let t = TempTree::new("protect-link");
    let win = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    mklink_j(&t.p("win-link"), Path::new(&win));
    let via_link = format!("{}\\System32", t.s("win-link"));
    let why = p.check(&via_link).expect("经过联接指到 Windows 的路径必须被拒绝");
    assert!(why.contains("Windows") && why.contains("实际位置"), "{why}");
    // 联接本身（最后一段）不跟随：删联接只删链接，允许
    assert!(p.check(&t.s("win-link")).is_none());
    // 短文件名：PROGRA~1 展开成 Program Files 后拒绝
    let sd = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let short = format!(r"{sd}\PROGRA~1");
    if Path::new(&short).exists() {
        let why = p.check(&short).expect("PROGRA~1 必须被拒绝");
        assert!(why.contains("程序安装目录"), "{why}");
    }
    // 临时目录本身放行
    assert!(p.check(&t.s("")).is_none());
    fs::remove_dir(t.p("win-link")).unwrap(); // RemoveDirectory 只删联接
    assert!(Path::new(&win).join("System32").exists());
}

// ───────────────────────── 2. 档位门槛 / 3. 规则一致性 ─────────────────────────

#[test]
fn tier_gate_rejects_keep_unknown_llm_and_tool_only() {
    let t = TempTree::new("tier");
    for d in ["cache", "review", "keep", "tool", "mystery"] {
        write_old(&t.p(&format!("{d}\\a.dat")), b"hello");
    }
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let reqs: Vec<PlanRequest> = ["cache", "review", "keep", "tool", "mystery"].iter().map(|d| req(&t.s(d))).collect();
    let plan = action::build_plan(&r, &ctx, &prot, &reqs, Strategy::RecycleBin);
    let by = |d: &str| plan.items.iter().find(|i| i.path.ends_with(&format!("\\{d}"))).unwrap();
    assert!(by("cache").blocked.is_none(), "{:?}", by("cache").blocked);
    assert!(by("review").blocked.is_none(), "{:?}", by("review").blocked);
    let keep = by("keep").blocked.clone().expect("keep 必须被拒绝");
    assert!(keep.contains("建议保留"), "{keep}");
    let unk = by("mystery").blocked.clone().expect("未知必须被拒绝");
    assert!(unk.contains("先搞清楚它是什么再说"), "{unk}");
    assert!(by("mystery").unknown);
    let tool = by("tool").blocked.clone().expect("只能用工具的必须被拒绝");
    assert!(tool.contains("不建议直接删") && tool.contains("工具"), "{tool}");
    // 被拒绝的项不枚举内容
    assert!(by("keep").entries.is_empty() && by("mystery").entries.is_empty());
    // AI 推测：解释来源是 Llm → 拒绝（即使模型说得再肯定）
    let e = diskwhy_core::explain::llm::to_explanation(
        diskwhy_core::explain::llm::LlmAnswer { what: "缓存".into(), owner: "x".into(), why: "".into(), lose: "".into(), confidence: 1.0 },
        "fake-model",
        100,
    );
    assert!(matches!(e.source, Source::Llm { .. }));
    let why = action::preflight::tier_gate(&e).expect("AI 推测必须被拒绝");
    assert!(why.contains("AI"), "{why}");
    println!("keep: {keep}\nunknown: {unk}\ntool: {tool}\nllm: {why}");
}

/// 用户二次确认后强制加入（forced）：keep / 认不出来 / 只能用工具 / 规则对不上的都放行，但只进暂存区 / 回收站
/// （策略是“直接删除”、档位是 safe 时也不直接删）；保护路径照样拦；本来就能过闸的不标 forced。
#[test]
fn forced_items_pass_tier_gate_but_never_go_permanent() {
    let t = TempTree::new("forced");
    for d in ["cache", "keep", "tool", "mystery"] {
        write_old(&t.p(&format!("{d}\\a.dat")), &content(d, 3000));
    }
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let names = ["cache", "keep", "tool", "mystery"];
    let reqs: Vec<PlanRequest> = names.iter().map(|d| req(&t.s(d))).collect();
    // 前端传来的写法大小写不一样也认
    let forced: Vec<String> = names.iter().map(|d| t.s(d).to_uppercase()).chain([r"C:\Windows".to_string()]).collect();
    let mut reqs_all = reqs.clone();
    reqs_all.push(req(r"C:\Windows"));
    let q = TempTree::new("forced-q");
    let mut plan = action::build_plan_forced(&r, &ctx, &prot, &reqs_all, &forced, Strategy::Quarantine { dest_root: q.s("") });
    let idx = |d: &str| plan.items.iter().position(|i| i.path.to_lowercase().ends_with(&format!("\\{d}"))).unwrap();
    // 保护路径：forced 也拦
    let win = plan.items.iter().find(|i| i.path.eq_ignore_ascii_case(r"C:\Windows")).unwrap();
    assert!(win.blocked.is_some() && !win.forced, "保护路径必须照样拦：{:?}", win.blocked);
    // 本来就能过闸的：不标 forced，safe 档照样直接删
    let c = &plan.items[idx("cache")];
    assert!(c.blocked.is_none() && !c.forced);
    assert_eq!(plan.dest_for(c), action::Dest::Permanent);
    // keep / 只能用工具 / 认不出来：放行、标 forced、进暂存区
    for d in ["keep", "tool", "mystery"] {
        let it = &plan.items[idx(d)];
        assert!(it.blocked.is_none(), "{d} 强制加入后不该被拦：{:?}", it.blocked);
        assert!(it.forced, "{d} 应标 forced");
        assert_eq!(plan.dest_for(it), action::Dest::Quarantine, "{d}");
    }
    // 不传 forced：照旧整项拒绝
    let plain = action::build_plan(&r, &ctx, &prot, &reqs, Strategy::RecycleBin);
    assert!(plain.items.iter().filter(|i| !i.path.ends_with("\\cache")).all(|i| i.blocked.is_some() && !i.forced));
    // 策略是“直接删除”：forced 的 safe 档（t-tool）和 keep 都改进回收站
    let mut perm = plan.clone();
    perm.strategy = Strategy::Permanent;
    assert_eq!(perm.dest_for(&perm.items[idx("tool")]), action::Dest::Recycle);
    assert_eq!(perm.dest_for(&perm.items[idx("keep")]), action::Dest::Recycle);
    assert_eq!(perm.dest_for(&perm.items[idx("cache")]), action::Dest::Permanent);
    // 预检 + 执行：forced 的都进了暂存区（找得回来），再整批还原
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert!(rep.can_execute, "{:?}", rep.warnings);
    assert!(rep.items.iter().filter(|i| i.forced).count() == 3);
    assert_eq!(rep.review_items.len(), 3, "forced 的项要在确认区单独列出");
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    for d in ["keep", "tool", "mystery"] {
        assert!(!t.p(&format!("{d}\\a.dat")).exists(), "{d} 应已移走");
    }
    assert!(!t.p("cache\\a.dat").exists());
    assert_eq!(ex.by_dest.quarantine.count, 3, "{:?}", ex.by_dest);
    assert_eq!(ex.by_dest.permanent.count, 1, "{:?}", ex.by_dest);
    let rr = quarantine::restore(Path::new(ex.manifest_path.as_ref().unwrap()), &log).unwrap();
    assert_eq!(rr.restored, 3);
    for d in ["keep", "tool", "mystery"] {
        assert!(t.p(&format!("{d}\\a.dat")).exists(), "{d} 应已还原");
    }
}

#[test]
fn rule_consistency_rejects_parent_of_rule_dir_and_changed_rule() {
    let t = TempTree::new("rulecons");
    write_old(&t.p(r"parent\cache\a.bin"), b"x");
    write_old(&t.p(r"parent\other.bin"), b"y");
    write_old(&t.p(r"cache\b.bin"), b"z");
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let node = |p: &str| diskwhy_core::view::find_node(&r.tree, p).unwrap().1.clone();
    let rule = |id: &str, from: Option<&str>| Source::Rule { id: id.into(), group: "test".into(), inherited_from: from.map(str::to_string) };
    // 规则 t-nested 的 match 是 %TEMP%\parent\cache；拿它去删父目录 %TEMP%\parent → 拒绝
    let parent = t.s("parent");
    let err = action::preflight::rule_consistent(&ctx, &rule("t-nested", None), &parent, &node(&parent), None).unwrap_err();
    assert!(err.contains("对不上"), "{err}");
    // 伪造“继承自 parent\cache”去删 parent → 拒绝
    let from = t.s(r"parent\cache");
    let err2 = action::preflight::rule_consistent(&ctx, &rule("t-nested", Some(&from)), &parent, &node(&parent), None).unwrap_err();
    assert!(err2.contains("父目录"), "{err2}");
    // 正常：规则目录本身 / 里面
    action::preflight::rule_consistent(&ctx, &rule("t-nested", None), &from, &node(&from), None).unwrap();
    // build_plan：加入时是 t-nested，现在路径算出来是 t-cache → 拒绝
    let prot = Protector::from_system();
    let plan = action::build_plan(&r, &ctx, &prot, &[PlanRequest { path: t.s("cache"), expect_rule: Some("t-nested".into()) }], Strategy::RecycleBin);
    let b = plan.items[0].blocked.clone().expect("规则变了必须拒绝");
    assert!(b.contains("变成了"), "{b}");
    // 父目录本身在 build_plan 里：没有规则 → 未知 → 档位门槛拒绝
    let plan2 = action::build_plan(&r, &ctx, &prot, &[req(&parent)], Strategy::RecycleBin);
    assert!(plan2.items[0].blocked.is_some());
    println!("parent: {err}\ninherited: {err2}\nchanged: {b}\nparent via plan: {:?}", plan2.items[0].blocked);
}

// ───────────────────────── 4/5/6. 热文件、云占位、链接 ─────────────────────────

#[test]
fn hot_and_cloud_skipped_and_junction_removed_without_touching_target() {
    let t = TempTree::new("hotcloud");
    // 链接目标（在清单项外面），里面 3 个文件
    for i in 0..3 {
        write_old(&t.p(&format!(r"target\keep{i}.txt")), &content(&format!("k{i}"), 1000));
    }
    let target_hashes: Vec<String> = (0..3).map(|i| sha(&t.p(&format!(r"target\keep{i}.txt")))).collect();
    fs::create_dir_all(t.p("cache")).unwrap();
    mklink_j(&t.p(r"cache\lnk"), &t.p("target"));
    fs::write(t.p(r"cache\hot.bin"), b"just written").unwrap(); // 刚写的 → 热文件
    write_old(&t.p(r"cache\cloud.bin"), b"pretend cloud placeholder");
    let out = Command::new("attrib").arg("+O").arg(t.p(r"cache\cloud.bin")).output().unwrap();
    assert!(out.status.success());
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    assert!(plan.items[0].blocked.is_none(), "{:?}", plan.items[0].blocked);
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let item = &plan.items[0];
    let dec = |name: &str| {
        let i = item.entries.iter().position(|e| e.path.ends_with(name)).unwrap_or_else(|| panic!("清单里没有 {name}"));
        (&item.entries[i], &item.decisions[i])
    };
    assert!(matches!(dec("hot.bin").1, Decision::Skip { reason: SkipReason::Hot { .. } }), "{:?}", dec("hot.bin").1);
    assert!(matches!(dec("cloud.bin").1, Decision::Skip { reason: SkipReason::Cloud }), "{:?}", dec("cloud.bin").1);
    assert!(dec("cloud.bin").0.fp.is_none(), "云占位不应被打开取指纹");
    let (lnk, d) = dec("lnk");
    assert_eq!(lnk.kind, EntryKind::Link);
    assert_eq!(*d, Decision::RemoveLink);
    // 链接目标里的文件不在清单里（没跟进去）
    assert!(!item.entries.iter().any(|e| e.path.contains("keep0.txt")));
    // 预览里明确标出
    let ir = &rep.items[0];
    let v = ir.will_delete.iter().find(|v| v.path.ends_with("lnk")).unwrap();
    assert!(v.note.as_deref().unwrap_or("").contains("只删链接本身"), "{:?}", v.note);
    assert_eq!(rep.hot_count, 1);
    assert_eq!(rep.cloud_count, 1);
    assert_eq!(ir.skipped_by_reason.get("hot"), Some(&1));
    assert_eq!(ir.skipped_by_reason.get("cloud"), Some(&1));
    assert!(rep.warnings.iter().any(|w| w.code == "hot"));
    assert!(rep.warnings.iter().any(|w| w.code == "cloud"));
    // 执行（回收站策略；清单里唯一要处理的是链接，链接直接删自身，不进回收站）
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!(ex.failed, 0, "{:?}", ex.items);
    assert_eq!(ex.items[0].links_removed, 1);
    assert!(fs::symlink_metadata(t.p(r"cache\lnk")).is_err(), "链接应该被删掉");
    for i in 0..3 {
        assert_eq!(sha(&t.p(&format!(r"target\keep{i}.txt"))), target_hashes[i], "链接目标里的文件必须完好");
    }
    assert!(t.p(r"cache\hot.bin").exists() && t.p(r"cache\cloud.bin").exists(), "跳过的文件必须还在");
    println!("预检警告：{:#?}", rep.warnings.iter().map(|w| &w.message).collect::<Vec<_>>());
}

// ───────────────────────── 9. 重新校验 ─────────────────────────

#[test]
fn reverify_skips_files_replaced_or_modified_after_plan() {
    let t = TempTree::new("reverify");
    for n in ["a", "b", "c", "d"] {
        write_old(&t.p(&format!(r"cache\{n}.bin")), &content(n, 4096));
    }
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::new("reverify-q");
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(plan.items[0].entry_count(), 4);
    let a_mtime = fs::metadata(t.p(r"cache\a.bin")).unwrap().modified().unwrap();
    // a：删掉重建，同样大小、同样内容、修改时间也调回原值 → 只有文件 ID 变了
    fs::remove_file(t.p(r"cache\a.bin")).unwrap();
    fs::write(t.p(r"cache\a.bin"), content("a", 4096)).unwrap();
    fs::OpenOptions::new().write(true).open(t.p(r"cache\a.bin")).unwrap().set_modified(a_mtime).unwrap();
    // b：追加内容（大小变了）
    fs::OpenOptions::new().append(true).open(t.p(r"cache\b.bin")).unwrap().write_all(b"more").unwrap();
    backdate(&t.p(r"cache\b.bin"), 7200);
    // c：同样大小改写内容（修改时间变了）
    fs::write(t.p(r"cache\c.bin"), content("c2", 4096)).unwrap();
    backdate(&t.p(r"cache\c.bin"), 5000);
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    let det = &ex.items[0].details;
    let reason = |n: &str| det.iter().find(|d| d.path.ends_with(n)).map(|d| (d.result.clone(), d.reason.clone())).unwrap();
    assert_eq!(reason("a.bin").0, "skipped");
    assert!(reason("a.bin").1.contains("文件 ID 变了"), "{:?}", reason("a.bin"));
    assert_eq!(reason("b.bin").0, "skipped");
    assert!(reason("b.bin").1.contains("大小"), "{:?}", reason("b.bin"));
    assert_eq!(reason("c.bin").0, "skipped");
    assert!(reason("c.bin").1.contains("修改时间"), "{:?}", reason("c.bin"));
    assert_eq!(reason("d.bin").0, "ok");
    assert_eq!(ex.ok, 1);
    for n in ["a", "b", "c"] {
        assert!(t.p(&format!(r"cache\{n}.bin")).exists(), "{n}.bin 被改过，必须留在原处");
    }
    assert!(!t.p(r"cache\d.bin").exists());
    // 把 d 还原回来，保持测试目录干净
    let rr = quarantine::restore(Path::new(ex.manifest_path.as_ref().unwrap()), &log).unwrap();
    assert_eq!(rr.restored, 1);
    println!("a: {:?}\nb: {:?}\nc: {:?}", reason("a.bin"), reason("b.bin"), reason("c.bin"));
}

// ───────────────────────── 隔离区 round-trip ─────────────────────────

fn quarantine_round_trip(dest_base: &Path, expect_method: &str) {
    let t = TempTree::new("qrt");
    let files = [r"cache\one.bin", r"cache\sub\two.bin", r"cache\sub\deep\three.bin", r"cache\中文 名字.txt"];
    let mut hashes = Vec::new();
    for (i, f) in files.iter().enumerate() {
        write_old(&t.p(f), &content(f, 1000 + i * 70_000));
        hashes.push((sha(&t.p(f)), fs::metadata(t.p(f)).unwrap().modified().unwrap()));
    }
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::in_dir(dest_base, "qdest");
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert!(rep.can_execute, "{:?}", rep.warnings);
    assert_eq!(rep.total_entries, 4);
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!((ex.ok, ex.failed), (4, 0), "{:?}", ex.items);
    for f in &files {
        assert!(!t.p(f).exists(), "{f} 应该被移走");
    }
    assert!(t.p("cache").is_dir(), "清单项本身的文件夹保留");
    assert!(!t.p(r"cache\sub").exists(), "变空的子文件夹被删掉");
    // manifest
    let mp = PathBuf::from(ex.manifest_path.clone().unwrap());
    let m = Manifest::load(&mp).unwrap();
    assert_eq!(m.entries.len(), 4);
    for (i, f) in files.iter().enumerate() {
        let e = m.entries.iter().find(|e| e.original == t.s(f)).unwrap_or_else(|| panic!("manifest 里没有 {f}"));
        assert_eq!(e.source, "t-cache");
        assert_eq!(e.method, expect_method);
        assert_eq!(e.status, "stored");
        assert_eq!(e.size, fs::metadata(e.stored.as_ref().unwrap()).unwrap().len());
        assert!(e.quarantined_at > 0 && e.mtime > 0);
        let stored = PathBuf::from(e.stored.as_ref().unwrap());
        assert!(stored.starts_with(q.p(quarantine::DIR_NAME)), "{}", stored.display());
        assert_eq!(sha(&stored), hashes[i].0, "隔离后内容必须一致");
    }
    let batches = quarantine::list_batches(&[q.s("")]);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].stored_entries, 4);
    assert!(!batches[0].expired);
    println!("manifest 示例：{}", serde_json::to_string_pretty(&m.entries[0]).unwrap());
    // 还原
    let rr = quarantine::restore(&mp, &log).unwrap();
    assert_eq!((rr.restored, rr.failed, rr.skipped), (4, 0, 0), "{:?}", rr.problems);
    assert!(rr.batch_removed, "全部还原后批次目录应被清掉");
    for (i, f) in files.iter().enumerate() {
        assert_eq!(sha(&t.p(f)), hashes[i].0, "{f} 还原后哈希必须一致");
        assert_eq!(fs::metadata(t.p(f)).unwrap().modified().unwrap(), hashes[i].1, "{f} 修改时间应保留");
    }
    assert!(quarantine::list_batches(&[q.s("")]).is_empty());
}

#[test]
fn quarantine_round_trip_same_volume() {
    quarantine_round_trip(&std::env::temp_dir(), "rename");
}

/// 跨卷：源在 %TEMP%（C 盘），隔离区在另一个固定盘。优先用环境变量 DISKWHY_TEST_XVOL_DIR。
#[test]
fn quarantine_round_trip_cross_volume() {
    let temp_vol = volume_root_of(&std::env::temp_dir().to_string_lossy()).unwrap();
    let base = match std::env::var("DISKWHY_TEST_XVOL_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => match diskwhy_core::win::list_fixed_drives().into_iter().find(|d| !d.root.eq_ignore_ascii_case(&temp_vol) && d.free_bytes > 100 << 20) {
            Some(d) => PathBuf::from(d.root),
            None => {
                println!("SKIP：本机没有第二个固定盘，跨卷隔离没有测");
                return;
            }
        },
    };
    // 很多机器上普通用户不能在盘的根目录建文件夹（本机 D:\ 就是这样）：建不了就说明原因并跳过，
    // 设置 DISKWHY_TEST_XVOL_DIR 指向另一个盘上可写的文件夹即可测。
    let probe = base.join(format!("diskwhy-xvol-probe-{}", uniq()));
    if let Err(e) = fs::create_dir_all(&probe) {
        println!("SKIP：在 {} 里建不了文件夹（{e}），跨卷隔离没有测；请设置 DISKWHY_TEST_XVOL_DIR", base.display());
        return;
    }
    fs::remove_dir(&probe).unwrap();
    let base_vol = volume_root_of(&base.to_string_lossy()).unwrap();
    assert!(!base_vol.eq_ignore_ascii_case(&temp_vol), "DISKWHY_TEST_XVOL_DIR 必须在另一个盘上");
    println!("跨卷：{temp_vol} → {}", base.display());
    quarantine_round_trip(&base, "copy_verify_delete");
}

// ───────────────────────── 回收站 ─────────────────────────

/// 在 `X:\$Recycle.Bin\<SID>\$I*` 里找原路径**等于** `original` 的记录；返回 ($I, $R, 记录的原路径)。
/// 先按 $I 文件大小筛（第 2 版 $I = 28 字节头 + 原路径 UTF-16 含结尾 0）：本机回收站有几十万项，逐个读太慢。
pub fn find_in_recycle_bin(vol: &str, original: &str) -> Option<(PathBuf, PathBuf, String)> {
    let want_v2 = 28 + 2 * (original.encode_utf16().count() as u64 + 1);
    let name = original;
    let rb = Path::new(vol).join("$Recycle.Bin");
    for sid in fs::read_dir(&rb).ok()?.flatten() {
        let Ok(rd) = fs::read_dir(sid.path()) else { continue };
        for f in rd.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            if !fname.starts_with("$I") {
                continue;
            }
            match f.metadata() {
                Ok(m) if m.len() == want_v2 || m.len() == 544 => {}
                _ => continue,
            }
            let Ok(b) = fs::read(f.path()) else { continue };
            if b.len() < 28 {
                continue;
            }
            let ver = u64::from_le_bytes(b[0..8].try_into().unwrap());
            let (start, chars) = if ver == 2 {
                (28usize, u32::from_le_bytes(b[24..28].try_into().unwrap()) as usize)
            } else {
                (24usize, 260usize)
            };
            let end = (start + chars * 2).min(b.len());
            let u: Vec<u16> = b[start..end].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            let orig = String::from_utf16_lossy(&u);
            if orig.eq_ignore_ascii_case(name) || orig.to_lowercase() == name.to_lowercase() {
                let r = sid.path().join(format!("$R{}", &fname[2..]));
                return Some((f.path(), r, orig));
            }
        }
    }
    None
}

#[test]
fn recycle_bin_really_receives_file_then_restore_only_ours() {
    let t = TempTree::new("recycle");
    let name = format!("diskwhy-recycle-probe-{}.bin", uniq());
    let rel = format!(r"cache\{name}");
    let data = content(&name, 123_457);
    write_old(&t.p(&rel), &data);
    let h0 = sha(&t.p(&rel));
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.total_entries, 1);
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!((ex.ok, ex.failed), (1, 0), "{:?}", ex.items);
    assert!(!t.p(&rel).exists(), "原位置应该没有了");
    let vol = volume_root_of(&t.s("")).unwrap();
    let (i_path, r_path, recorded) = find_in_recycle_bin(&vol, &t.s(&rel)).expect("回收站里必须能找到这个文件（$Recycle.Bin 下的 $I 记录）");
    assert!(r_path.exists(), "对应的 $R 数据文件必须存在：{}", r_path.display());
    assert_eq!(fs::metadata(&r_path).unwrap().len(), data.len() as u64);
    assert_eq!(sha(&r_path), h0, "回收站里的内容必须与原文件一致");
    println!("回收站记录：{} → {}（原路径记录为 {recorded}）", i_path.display(), r_path.display());
    // 只动我们自己这一条：把 $R 移回原位，删掉对应的 $I
    fs::rename(&r_path, t.p(&rel)).unwrap();
    fs::remove_file(&i_path).unwrap();
    assert_eq!(sha(&t.p(&rel)), h0, "从回收站还原后哈希一致");
    assert!(find_in_recycle_bin(&vol, &t.s(&rel)).is_none());
}

// ───────────────────────── 7. 占用检测 ─────────────────────────

#[test]
fn locked_file_reported_by_restart_manager_and_process_not_killed() {
    let t = TempTree::new("locked");
    write_old(&t.p(r"cache\locked.bin"), b"locked content");
    write_old(&t.p(r"cache\free.bin"), b"free content");
    // 自己开一个不共享的句柄占住它
    let mut h = fs::OpenOptions::new().read(true).write(true).share_mode(0).open(t.p(r"cache\locked.bin")).unwrap();
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::new("locked-q");
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let me = std::process::id();
    let item = &plan.items[0];
    let i = item.entries.iter().position(|e| e.path.ends_with("locked.bin")).unwrap();
    match &item.decisions[i] {
        Decision::Skip { reason: SkipReason::Locked { processes } } => {
            assert!(processes.iter().any(|p| p.pid == me), "Restart Manager 应该报告当前进程 PID {me}，实际：{processes:?}");
            println!("Restart Manager 报告：{processes:?}");
        }
        d => panic!("被占用的文件应该跳过，实际 {d:?}"),
    }
    assert!(rep.locked_processes.iter().any(|p| p.pid == me));
    assert!(rep.warnings.iter().any(|w| w.code == "locked" && w.message.contains(&me.to_string())));
    // 用户选“取消”（不跳过被占用的）→ 什么都不做
    let log = ActionLog::open(&t.p("actions.log"));
    let mut o = ExecOptions::new(&log);
    o.skip_locked = false;
    let err = action::execute(&plan, &prot, &o, |_| {}).unwrap_err();
    assert!(err.contains("被占用"), "{err}");
    assert!(t.p(r"cache\free.bin").exists());
    // 用户选“跳过被占用的”→ 其余照常
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!(ex.ok, 1);
    // 当前进程还活着、句柄还能用、被占用的文件还在
    h.write_all(b" still mine").unwrap();
    drop(h);
    assert!(t.p(r"cache\locked.bin").exists());
    let rr = quarantine::restore(Path::new(ex.manifest_path.as_ref().unwrap()), &log).unwrap();
    assert_eq!(rr.restored, 1);
}

// ───────────────────────── 8. 空间提示 ─────────────────────────

#[test]
fn recycle_low_space_warning_matches_volume_state() {
    let t = TempTree::new("space");
    write_old(&t.p(r"cache\x.bin"), b"x");
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let vol = volume_root_of(&root).unwrap();
    let sp = volume_space(&vol).unwrap();
    let low = (sp.free_bytes as f64) < sp.total_bytes as f64 * 0.10;
    let w = rep.warnings.iter().find(|w| w.code == "recycle_low_space");
    assert_eq!(w.is_some(), low, "可用 {} / {}，警告：{:?}", sp.free_bytes, sp.total_bytes, rep.warnings);
    if let Some(w) = w {
        assert!(w.message.contains("回收站") && w.message.contains("清空回收站") && w.message.contains("暂存"));
        println!("{vol} 可用 {:.1}% → 警告：{}", sp.free_bytes as f64 * 100.0 / sp.total_bytes as f64, w.message);
    }
    // 可切换到的隔离区盘不包含源所在的盘
    assert!(rep.quarantine_choices.iter().all(|d| !d.root.eq_ignore_ascii_case(&vol)));
    // 隔离区放在同一个盘上 → 提示不会腾空间
    let q = TempTree::new("space-q");
    let mut plan2 = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan2.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    let rep2 = action::preflight(&mut plan2, &prot, &PreflightOptions::default());
    assert!(rep2.warnings.iter().any(|w| w.code == "quarantine_same_volume"));
}

// ───────────────────────── 日志 ─────────────────────────

#[test]
fn action_log_one_line_per_action_with_all_fields() {
    let t = TempTree::new("log");
    write_old(&t.p(r"cache\a.bin"), b"a");
    write_old(&t.p(r"cache\sub\b.bin"), b"b");
    fs::write(t.p(r"cache\hot.bin"), b"hot").unwrap();
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::new("log-q");
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!((ex.ok, ex.skipped), (2, 1));
    let rr = quarantine::restore(Path::new(ex.manifest_path.as_ref().unwrap()), &log).unwrap();
    assert_eq!(rr.restored, 2);
    drop(log);
    let text = fs::read_to_string(&logp).unwrap();
    let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    for l in &lines {
        let o = l.as_object().unwrap();
        for f in FIELDS {
            assert!(o.contains_key(*f), "日志行缺字段 {f}：{l}");
        }
        assert_eq!(o.len(), FIELDS.len(), "字段数不对：{l}");
    }
    let ops: Vec<(&str, &str)> = lines.iter().map(|l| (l["op"].as_str().unwrap(), l["result"].as_str().unwrap())).collect();
    let count = |op: &str, res: &str| ops.iter().filter(|(o, r)| *o == op && *r == res).count();
    assert_eq!(count("quarantine", "ok"), 2, "{ops:?}");
    assert_eq!(count("skip", "skipped"), 1, "热文件跳过也要记一行：{ops:?}");
    assert_eq!(count("rmdir", "ok"), 1, "{ops:?}");
    assert_eq!(count("restore", "ok"), 2, "{ops:?}");
    assert_eq!(ops.iter().filter(|(o, _)| *o == "summary").count(), 2, "执行和还原各一行汇总：{ops:?}");
    assert_eq!(count("start", "ok"), 1, "阶段 J：执行开始写一条 start：{ops:?}");
    assert_eq!(count("batch_begin", "ok"), 1, "阶段 J 取证：每块开始前一条 batch_begin（这里只有一块）：{ops:?}");
    let st = lines.iter().find(|l| l["op"] == "start").unwrap();
    assert!(st["reason"].as_str().unwrap().contains("父进程"), "start 记录带父进程号和程序路径：{st}");
    assert_eq!(lines.len(), 1 + 1 + 2 + 1 + 1 + 1 + 2 + 1);
    let q0 = lines.iter().find(|l| l["op"] == "quarantine").unwrap();
    assert_eq!(q0["source"], "t-cache");
    assert_eq!(q0["tier"], "safe");
    assert!(q0["dest"].as_str().unwrap().contains(quarantine::DIR_NAME));
    println!("日志 {} 行，示例：{}", lines.len(), text.lines().next().unwrap());
}

// ───────────────────────── 刷新子树 / 指引 ─────────────────────────

#[test]
fn refresh_rescans_only_affected_subtree() {
    let t = TempTree::new("refresh");
    write_old(&t.p(r"cache\sub\big.bin"), &content("big", 300_000));
    write_old(&t.p(r"other\o.bin"), &content("o", 50_000));
    let root = t.s("");
    let ctx = ctx_for(&root);
    let mut r = scan_dir(&root);
    let before_root = r.tree.allocated;
    let cache_before = diskwhy_core::view::find_dir(&r.tree, &t.s("cache")).unwrap().last().unwrap().allocated;
    let other_before = diskwhy_core::view::find_dir(&r.tree, &t.s("other")).unwrap().last().unwrap().allocated;
    let prot = Protector::from_system();
    let q = TempTree::new("refresh-q");
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    let rep = action::refresh::refresh_dirs(&mut r, &ex.affected_dirs);
    assert_eq!(rep.rescanned.len(), 1);
    let cache = diskwhy_core::view::find_dir(&r.tree, &t.s("cache")).unwrap().last().unwrap().allocated;
    assert_eq!(cache, 0);
    assert!(r.tree.allocated < before_root && before_root - r.tree.allocated >= 300_000);
    assert_eq!(diskwhy_core::view::find_dir(&r.tree, &t.s("other")).unwrap().last().unwrap().allocated, other_before);
    assert_eq!(r.buckets.total(), r.used_bytes as i128, "刷新后归属不变式仍成立");
    // 空的 cache\sub 已经删掉、树里也没了；还原时它会被重新建出来，刷新要落到树里最近的上层 cache
    assert!(diskwhy_core::view::find_dir(&r.tree, &t.s(r"cache\sub")).is_none());
    let rr = quarantine::restore(Path::new(ex.manifest_path.as_ref().unwrap()), &log).unwrap();
    let rep2 = action::refresh::refresh_dirs(&mut r, &rr.touched_dirs);
    assert_eq!(rep2.rescanned, vec![t.s("cache")], "{:?}", rr.touched_dirs);
    let cache_after = diskwhy_core::view::find_dir(&r.tree, &t.s("cache")).unwrap().last().unwrap().allocated;
    assert_eq!(cache_after, cache_before);
    assert_eq!(r.tree.allocated, before_root);
}

#[test]
fn guidance_gives_commands_but_never_runs_them() {
    use diskwhy_core::action::guidance::{guidance_for, ALLOWED_SETTINGS};
    use diskwhy_core::explain::{Category, Explanation, How, Tier};
    let mk = |id: &str, tier: Tier, how: Vec<How>| Explanation {
        tier,
        owner: None,
        category: Category::System,
        what: String::new(),
        why: String::new(),
        lose: String::new(),
        lose_kind: None,
        lose_short: None,
        how,
        how_detail: "说明".into(),
        gain_bytes: 0,
        gain_note: String::new(),
        source: Source::Rule { id: id.into(), group: "x".into(), inherited_from: None },
        confidence: 1.0,
        refs: vec![],
        verified_on: None,
        notes: vec![],
        clues: vec![],
        ai: None,
    };
    let g = guidance_for(&mk("winsxs", Tier::Keep, vec![How::BuiltinTool]));
    assert!(g.iter().any(|x| x.command.as_deref() == Some("DISM /Online /Cleanup-Image /StartComponentCleanup") && x.needs_admin));
    let g = guidance_for(&mk("hiberfil", Tier::Review, vec![How::BuiltinTool]));
    assert!(g.iter().any(|x| x.command.as_deref() == Some("powercfg /h off")));
    let g = guidance_for(&mk("windows-old", Tier::Review, vec![How::BuiltinTool]));
    assert!(g.iter().any(|x| x.command.as_deref() == Some("cleanmgr /d C:")));
    assert!(g.iter().any(|x| x.settings_uri.as_deref() == Some("ms-settings:storagesense")));
    let g = guidance_for(&mk("conda-pkgs", Tier::Safe, vec![How::BuiltinTool]));
    assert!(g.iter().any(|x| x.command.as_deref() == Some("conda clean --all")));
    let g = guidance_for(&mk("docker-data-vhdx", Tier::Keep, vec![How::Migrate, How::BuiltinTool]));
    assert!(g.iter().any(|x| x.explain.contains("Disk image location")));
    let g = guidance_for(&mk("wsl-distro-vhdx", Tier::Keep, vec![How::Migrate]));
    assert!(g.iter().any(|x| x.explain.contains("wsl --export")));
    let g = guidance_for(&mk("wechat-files", Tier::Keep, vec![How::Migrate]));
    assert!(g.iter().any(|x| x.explain.contains("文件管理")));
    // 所有可一键打开的设置页都在白名单里
    for id in ["winsxs", "hiberfil", "windows-old", "recycle-bin", "conda-pkgs", "docker-data-vhdx", "wechat-files"] {
        for x in guidance_for(&mk(id, Tier::Keep, vec![How::BuiltinTool])) {
            if let Some(u) = x.settings_uri {
                assert!(ALLOWED_SETTINGS.iter().any(|(a, _)| *a == u), "{u}");
            }
        }
    }
}

// ───────────────────────── 隔离区：彻底清空 / 不能写的位置 ─────────────────────────

#[test]
fn quarantine_purge_removes_only_that_batch() {
    let t = TempTree::new("purge");
    write_old(&t.p(r"cache\x.bin"), &content("x", 5000));
    write_old(&t.p(r"cache\y.bin"), &content("y", 7000));
    write_old(&t.p(r"other\keep.bin"), &content("k", 100));
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::new("purge-q");
    // 隔离区根目录里另放一个不相关的文件，清空批次时不能动它
    write_old(&q.p("unrelated.txt"), b"not ours");
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!(ex.ok, 2);
    let mp = PathBuf::from(ex.manifest_path.unwrap());
    // 不在 DiskWhy-Quarantine 里的“清单”一律拒绝
    let fake = q.p("manifest.json");
    fs::copy(&mp, &fake).unwrap();
    let err = quarantine::purge(&fake, &log).unwrap_err();
    assert!(err.contains("拒绝"), "{err}");
    fs::remove_file(&fake).unwrap();
    let rep = quarantine::purge(&mp, &log).unwrap();
    assert!(rep.removed, "{:?}", rep.error);
    assert_eq!(rep.deleted_entries, 2);
    assert!(!mp.parent().unwrap().exists(), "批次目录应被删掉");
    assert!(q.p("unrelated.txt").exists(), "隔离区根目录里别的文件不能动");
    assert!(t.p(r"other\keep.bin").exists());
    assert!(quarantine::list_batches(&[q.s("")]).is_empty());
}

#[test]
fn quarantine_unwritable_destination_is_blocked_before_anything_moves() {
    let t = TempTree::new("unwritable");
    write_old(&t.p(r"cache\x.bin"), b"x");
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::new("unwritable-q");
    // 拒绝 Everyone 在这个文件夹里建子文件夹 / 写文件（测后恢复）
    let out = Command::new("icacls").arg(q.s("")).args(["/deny", "*S-1-1-0:(AD,WD)"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let w = rep.warnings.iter().find(|w| w.code == "quarantine_bad_dest").expect("不能写的隔离区必须被拦下");
    assert!(w.blocking && w.message.contains("建不了暂存区文件夹"), "{}", w.message);
    assert!(!rep.can_execute);
    let log = ActionLog::open(&t.p("actions.log"));
    let err = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap_err();
    println!("预检：{}\n执行：{err}", w.message);
    assert!(t.p(r"cache\x.bin").exists(), "什么都不能动");
    let _ = Command::new("icacls").arg(q.s("")).args(["/remove:d", "*S-1-1-0"]).output();
}

// ───────────────────────── 阶段 G：预检提速 ─────────────────────────

/// 占用探测（一次打开：DELETE | 读属性，全共享）先筛出被占用的文件，只把这些交给 Restart Manager，
/// 分组、多会话并发；进程名照样报告，进程不会被关。
#[test]
fn probe_then_restart_manager_flags_only_locked_files() {
    use diskwhy_core::action::winops;
    let t = TempTree::new("probe-rm");
    let mut locked_paths = Vec::new();
    let mut free_paths = Vec::new();
    for i in 0..30 {
        let rel = format!(r"cache\{}\f{i:02}.bin", if i % 2 == 0 { "a" } else { "b" });
        write_old(&t.p(&rel), format!("free {i}").as_bytes());
        free_paths.push(t.p(&rel));
    }
    for i in 0..13 {
        let rel = format!(r"cache\{}\locked{i:02}.bin", if i < 9 { "a" } else { "b" });
        write_old(&t.p(&rel), format!("locked {i}").as_bytes());
        locked_paths.push(t.p(&rel));
    }
    // 不共享（share 0）占住 12 个；第 13 个只给读写共享、不给 FILE_SHARE_DELETE —— 同样算被占用
    let mut handles: Vec<fs::File> = locked_paths[..12]
        .iter()
        .map(|p| fs::OpenOptions::new().read(true).write(true).share_mode(0).open(p).unwrap())
        .collect();
    handles.push(fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).open(&locked_paths[12]).unwrap());
    // 另一个文件被打开但给了全部共享（含删除）：不算被占用
    let shared_ok = t.p(r"cache\a\shared-delete-ok.bin");
    write_old(&shared_ok, b"shared");
    let _h_shared = fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2 | 0x4).open(&shared_ok).unwrap();
    free_paths.push(shared_ok.clone());

    // 1. 探测函数本身
    let (probe, fp) = winops::probe_and_fingerprint(&free_paths[0].to_string_lossy(), false);
    assert_eq!(probe, Ok(()));
    let fp = fp.unwrap();
    let fp2 = winops::fingerprint(&free_paths[0].to_string_lossy(), true).unwrap();
    assert_eq!((fp.volume, fp.id, fp.size, fp.mtime_ft), (fp2.volume, fp2.id, fp2.size, fp2.mtime_ft), "一次打开取到的指纹必须和单独取的一致");
    let (probe, fp) = winops::probe_and_fingerprint(&locked_paths[0].to_string_lossy(), false);
    assert!(matches!(probe, Err(c) if winops::is_sharing_violation(c)), "被占用的文件应该探测到共享冲突：{probe:?}");
    assert!(fp.is_ok(), "被占用的文件也要能取到指纹（只读属性不受共享模式限制）：{fp:?}");
    let (probe, _) = winops::probe_and_fingerprint(&shared_ok.to_string_lossy(), false);
    assert_eq!(probe, Ok(()), "别人打开时给了 FILE_SHARE_DELETE 的文件不算被占用");

    // 2. 整个预检：小分组（每组 4 个）、3 个会话并发 → 至少 4 组
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    plan.safe_direct = false; // 阶段 J：测的是回收站 / 暂存区本身，safe 档也按策略走
    let opts = PreflightOptions { rm_chunk: 4, rm_sessions: 3, ..PreflightOptions::default() };
    let prog = action::Progress::default();
    let t0 = std::time::Instant::now();
    let rep = action::preflight_with(&mut plan, &prot, &opts, &prog);
    let v = prog.view();
    println!("预检 {:?}，进度 {v:?}，分段 {:?}", t0.elapsed(), rep.timing_ms);
    let me = std::process::id();
    let item = &plan.items[0];
    let dec = |p: &Path| {
        let s = p.to_string_lossy().to_string();
        let i = item.entries.iter().position(|e| e.path.eq_ignore_ascii_case(&s)).unwrap_or_else(|| panic!("清单里没有 {s}"));
        (&item.entries[i], &item.decisions[i])
    };
    for p in &locked_paths {
        match dec(p).1 {
            Decision::Skip { reason: SkipReason::Locked { processes } } => {
                assert!(processes.iter().any(|x| x.pid == me), "{}：Restart Manager 应该报告当前进程 PID {me}，实际 {processes:?}", p.display())
            }
            d => panic!("{} 被占用，应该跳过，实际 {d:?}", p.display()),
        }
    }
    for p in &free_paths {
        let (e, d) = dec(p);
        assert_eq!(d, &Decision::Delete, "{} 没被占用，应该可以删", p.display());
        assert!(e.fp.is_some(), "能删的文件必须带指纹（执行前重新校验要用）");
    }
    assert_eq!(rep.locked_count, 13);
    assert!(rep.locked_processes.iter().any(|x| x.pid == me));
    assert_eq!((v.checked, v.to_check), (item.entries.len(), item.entries.len()));
    assert_eq!(v.locked, 13);
    assert!(v.rm_total >= 4 && v.rm_done == v.rm_total, "应该分成至少 4 组问 Restart Manager：{v:?}");
    assert!(rep.can_execute && !rep.pending);
    // 进程没被关：句柄都还能用
    for h in handles.iter_mut().take(12) {
        h.write_all(b" still mine").unwrap();
    }
}

/// 快速计划（不枚举）马上给出估算，弹层用它先出现；估算不能执行，取消的预检也不能执行。
#[test]
fn quick_plan_quote_is_pending_and_cancelled_preflight_cannot_execute() {
    let t = TempTree::new("quote");
    for i in 0..5 {
        write_old(&t.p(&format!(r"cache\q{i}.bin")), &content(&format!("q{i}"), 5000));
    }
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let reqs = [req(&t.s("cache"))];
    let t0 = std::time::Instant::now();
    let plan = action::build_plan_quick(&r, &ctx, &prot, &reqs, Strategy::RecycleBin);
    let q = action::quote(&plan);
    println!("快速计划 + 估算 {:?}", t0.elapsed());
    assert!(plan.items[0].entries.is_empty(), "快速计划不枚举内容");
    assert!(q.pending && !q.can_execute);
    assert_eq!(q.items[0].will_delete_count, 0);
    assert!(q.total_bytes > 0 && q.total_bytes == plan.items[0].scan_allocated, "估算按扫描时的大小：{} vs {}", q.total_bytes, plan.items[0].scan_allocated);
    let log = ActionLog::open(&t.p("actions.log"));
    assert!(action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).is_err(), "没预检的计划不能执行");

    // 取消：预检尽快返回，计划不标记为已预检，执行被拒绝，文件原封不动
    let mut plan2 = action::build_plan_quick(&r, &ctx, &prot, &reqs, Strategy::RecycleBin);
    let prog = action::Progress::default();
    action::enumerate_plan(&mut plan2, &prog);
    assert_eq!(plan2.items[0].entries.len(), 5);
    prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    let rep = action::preflight_with(&mut plan2, &prot, &PreflightOptions::default(), &prog);
    assert!(rep.pending && !rep.can_execute && !plan2.preflight_done);
    let err = action::execute(&plan2, &prot, &ExecOptions::new(&log), |_| {}).unwrap_err();
    assert!(err.contains("预检"), "{err}");
    assert!((0..5).all(|i| t.p(&format!(r"cache\q{i}.bin")).exists()));

    // 正常做完：和一次性 build_plan + preflight 结果一样
    let mut plan3 = action::build_plan_quick(&r, &ctx, &prot, &reqs, Strategy::RecycleBin);
    action::enumerate_plan(&mut plan3, &action::Progress::default());
    let rep3 = action::preflight(&mut plan3, &prot, &PreflightOptions::default());
    assert!(!rep3.pending && rep3.can_execute && plan3.preflight_done);
    assert_eq!(rep3.total_entries, 5);
}

/// 父目录真实位置的缓存：清单项下面的文件夹按“清单项真实位置 + 相对路径”推出来，结果必须和实际解析一致；
/// 清单项本身经过链接指到受保护目录时照样拦下。
#[test]
fn parent_cache_derived_paths_match_real_resolution() {
    use diskwhy_core::action::protect::ParentCache;
    use diskwhy_core::action::winops::final_path;
    let t = TempTree::new("pcache");
    fs::create_dir_all(t.p(r"real\a\b\c")).unwrap();
    mklink_j(&t.p("via-link"), &t.p("real"));
    let root = t.s("via-link");
    let cache = ParentCache::with_roots([root.as_str()]);
    for rel in ["", r"\a", r"\a\b", r"\A\B\c"] {
        let p = format!("{root}{rel}");
        let derived = cache.resolve(&p).expect("应该能解析");
        let real = final_path(&p).expect("实际解析");
        assert!(derived.eq_ignore_ascii_case(&real), "{p}：推出来的 {derived} 和实际的 {real} 不一致");
        assert!(real.to_lowercase().contains(r"\real"), "{real}");
    }
    // 不在清单项下面：照旧实际解析
    let other = t.s("real");
    assert_eq!(cache.resolve(&other).map(|x| x.to_lowercase()), final_path(&other).map(|x| x.to_lowercase()));
    // 清单项是指向 C:\Windows 的联接：里面的条目用缓存检查照样被拦
    let win = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    mklink_j(&t.p("win-link"), Path::new(&win));
    let wl = t.s("win-link");
    let prot = Protector::from_system();
    let c2 = ParentCache::with_roots([wl.as_str()]);
    let entry = format!(r"{wl}\System32\drivers\etc\hosts");
    let a = prot.entry_blocked(&entry);
    let b = prot.entry_blocked_cached(&entry, &c2);
    assert!(a.is_some() && b.is_some(), "经过链接指到 Windows 目录的条目必须被拦：{a:?} / {b:?}");
}

// ───────────────────────── 阶段 J：直接删除 / 统一队列 / 停止 / 整夹回收 / 跨盘 ─────────────────────────

/// 一次扫描回收站，把原路径在 `originals` 里的项（文件或整夹）挪回原处，并删掉对应的 $I（只动我们自己的）。
/// 返回挪回来的个数。
fn restore_from_bin(vol: &str, originals: &[String]) -> usize {
    let want: std::collections::HashMap<String, String> = originals.iter().map(|o| (o.to_lowercase(), o.clone())).collect();
    let sizes: std::collections::HashSet<u64> = originals.iter().map(|o| 28 + 2 * (o.encode_utf16().count() as u64 + 1)).collect();
    let rb = Path::new(vol).join("$Recycle.Bin");
    let mut n = 0;
    let Ok(sids) = fs::read_dir(&rb) else { return 0 };
    for sid in sids.flatten() {
        let Ok(rd) = fs::read_dir(sid.path()) else { continue };
        for f in rd.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            if !fname.starts_with("$I") || !f.metadata().map(|m| sizes.contains(&m.len())).unwrap_or(false) {
                continue;
            }
            let Ok(b) = fs::read(f.path()) else { continue };
            if b.len() < 28 || u64::from_le_bytes(b[0..8].try_into().unwrap()) != 2 {
                continue;
            }
            let chars = u32::from_le_bytes(b[24..28].try_into().unwrap()) as usize;
            let u: Vec<u16> = b[28..(28 + chars * 2).min(b.len())].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            let orig = String::from_utf16_lossy(&u);
            if let Some(o) = want.get(&orig.to_lowercase()) {
                let r = sid.path().join(format!("$R{}", &fname[2..]));
                if let Some(parent) = Path::new(o).parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if fs::rename(&r, o).is_ok() {
                    let _ = fs::remove_file(f.path());
                    n += 1;
                }
            }
        }
    }
    n
}

fn read_log(p: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(p).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// safe 档直接删除：9 条护栏照样生效——受保护路径、热文件、被占用（预检时 / 执行时）、指纹变了、
/// 链接只删链接本身、云文件（预检时 / 执行时变成云文件），只读文件也能删；报告按去向分计。
#[test]
fn permanent_delete_honours_all_guardrails() {
    use diskwhy_core::action::Dest;
    let t = TempTree::new("perm");
    for i in 0..3 {
        write_old(&t.p(&format!(r"target\keep{i}.txt")), &content(&format!("k{i}"), 1000));
    }
    let target_hash = sha(&t.p(r"target\keep0.txt"));
    for n in ["a", "b", "changed", "locked-pre", "locked-exec", "cloud-exec", "ro"] {
        write_old(&t.p(&format!(r"cache\sub\{n}.bin")), &content(n, 3000));
    }
    write_old(&t.p(r"cache\.ssh\key"), b"secret"); // 预检时就因保护路径跳过
    write_old(&t.p(r"cache\cloud.bin"), b"cloud");
    assert!(Command::new("attrib").arg("+O").arg(t.p(r"cache\cloud.bin")).output().unwrap().status.success());
    fs::write(t.p(r"cache\hot.bin"), b"hot").unwrap();
    mklink_j(&t.p(r"cache\lnk"), &t.p("target"));
    let mut perm = fs::metadata(t.p(r"cache\sub\ro.bin")).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(t.p(r"cache\sub\ro.bin"), perm).unwrap();
    let _pre_lock = fs::OpenOptions::new().read(true).share_mode(0x1).open(t.p(r"cache\sub\locked-pre.bin")).unwrap();
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    // 策略选的是回收站，但 safe 档 + safe_direct（默认）→ 直接删除
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    assert!(plan.safe_direct);
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.by_dest.recycle.count, 0, "safe 档不进回收站：{:?}", rep.by_dest);
    assert!(rep.by_dest.permanent.count >= 6, "{:?}", rep.by_dest);
    assert!(!rep.warnings.iter().any(|w| w.code == "recycle_low_space" || w.code == "recycle_over_capacity"), "没东西进回收站就不该有回收站提示：{:?}", rep.warnings);
    assert!(rep.est_secs > 0.0 && rep.scan_total_bytes > 0);
    assert_eq!(rep.skipped_by_reason.get("protected").map(|c| c.count), Some(1), "{:?}", rep.skipped_by_reason);
    assert_eq!(rep.skipped_by_reason.get("hot").map(|c| c.count), Some(1));
    assert_eq!(rep.skipped_by_reason.get("cloud").map(|c| c.count), Some(1));
    assert_eq!(rep.skipped_by_reason.get("locked").map(|c| c.count), Some(1));
    assert_eq!(rep.skipped_bytes, rep.skipped_by_reason.values().map(|c| c.bytes).sum::<u64>());
    // 预检之后：改一个、占住一个（不给删除共享）、把一个变成“云文件”
    fs::OpenOptions::new().append(true).open(t.p(r"cache\sub\changed.bin")).unwrap().write_all(b"x").unwrap();
    backdate(&t.p(r"cache\sub\changed.bin"), 7200);
    let _exec_lock = fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).open(t.p(r"cache\sub\locked-exec.bin")).unwrap();
    assert!(Command::new("attrib").arg("+O").arg(t.p(r"cache\sub\cloud-exec.bin")).output().unwrap().status.success());
    // 再塞一条“预检说能删”、但其实在受保护路径里的条目：执行时的保护路径检查必须拦住
    let ssh_key = t.s(r"cache\.ssh\key");
    {
        let item = &mut plan.items[0];
        let i = item.entries.iter().position(|e| e.path == ssh_key).unwrap();
        item.entries[i].fp = Some(action::winops::fingerprint(&ssh_key, true).unwrap());
        item.decisions[i] = Decision::Delete;
    }
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    drop(log);
    let det = |n: &str| ex.items[0].details.iter().find(|d| d.path.ends_with(n)).map(|d| (d.result.clone(), d.reason.clone()));
    for n in ["a.bin", "b.bin", "ro.bin"] {
        assert!(!t.p(&format!(r"cache\sub\{n}")).exists(), "{n} 应该被直接删掉");
    }
    assert!(det("changed.bin").unwrap().1.contains("大小"), "{:?}", det("changed.bin"));
    assert!(det("locked-exec.bin").unwrap().1.contains("占用"), "{:?}", det("locked-exec.bin"));
    assert_eq!(det("cloud-exec.bin").unwrap().0, "skipped");
    assert!(det(r".ssh\key").unwrap().0 == "skipped", "{:?}", det(r".ssh\key"));
    for n in [r"sub\changed.bin", r"sub\locked-pre.bin", r"sub\locked-exec.bin", r"sub\cloud-exec.bin", r".ssh\key", "cloud.bin", "hot.bin"] {
        assert!(t.p(&format!(r"cache\{n}")).exists(), "{n} 必须还在");
    }
    assert!(fs::symlink_metadata(t.p(r"cache\lnk")).is_err(), "链接本身被删掉");
    assert_eq!(sha(&t.p(r"target\keep0.txt")), target_hash, "链接目标完好");
    assert_eq!(ex.failed, 0, "{:?}", ex.items[0].details);
    assert_eq!(ex.by_dest.permanent.count, 4, "a / b / ro + 链接：{:?}", ex.by_dest);
    assert_eq!(ex.by_dest.recycle.count + ex.by_dest.quarantine.count, 0);
    assert_eq!(ex.ok, 4);
    assert!(ex.skipped_by_reason.get("changed").is_some() && ex.skipped_by_reason.get("locked").map(|c| c.count) == Some(2), "{:?}", ex.skipped_by_reason);
    assert!(ex.duration_ms > 0 || ex.ok > 0);
    let lines = read_log(&logp);
    assert_eq!(lines.iter().filter(|l| l["op"] == "delete" && l["result"] == "ok").count(), 3);
    assert_eq!(lines.iter().filter(|l| l["op"] == "start").count(), 1);
    assert_eq!(lines.iter().filter(|l| l["op"] == "summary").count(), 1);
    // 执行时才发现的热文件（把热窗口调成 100 年）：同样跳过
    let t2 = TempTree::new("perm-hot");
    write_old(&t2.p(r"cache\x.bin"), b"x");
    let root2 = t2.s("");
    let r2 = scan_dir(&root2);
    let mut plan2 = action::build_plan(&r2, &ctx_for(&root2), &prot, &[req(&t2.s("cache"))], Strategy::Permanent);
    action::preflight(&mut plan2, &prot, &PreflightOptions::default());
    let log2 = ActionLog::open(&t2.p("actions.log"));
    let mut o = ExecOptions::new(&log2);
    o.hot_window_secs = 100 * 365 * 86_400;
    let ex2 = action::execute(&plan2, &prot, &o, |_| {}).unwrap();
    assert_eq!((ex2.ok, ex2.skipped), (0, 1));
    assert!(ex2.skipped_by_reason.contains_key("hot"));
    assert!(t2.p(r"cache\x.bin").exists());
    let _ = Dest::Permanent;
    // 清理只读属性，让 TempTree 能删干净
    let _ = Command::new("attrib").args(["-R", "-O", "/S"]).arg(format!("{}\\*", t.0.display())).output();
}

/// safe 档直接删、review 档按策略（暂存区）；用户明确选回收站时（safe_direct = false）safe 也按策略。
#[test]
fn safe_goes_direct_review_follows_strategy() {
    let t = TempTree::new("split");
    for i in 0..5 {
        write_old(&t.p(&format!(r"cache\c{i}.bin")), &content(&format!("c{i}"), 2000));
        write_old(&t.p(&format!(r"review\deep\r{i}.bin")), &content(&format!("r{i}"), 2000));
    }
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::new("split-q");
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache")), req(&t.s("review"))], Strategy::Quarantine { dest_root: q.s("") });
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!((rep.by_dest.permanent.count, rep.by_dest.quarantine.count, rep.by_dest.recycle.count), (5, 5, 0), "{:?}", rep.by_dest);
    assert!(rep.can_execute, "{:?}", rep.warnings);
    let log = ActionLog::open(&t.p("actions.log"));
    let events: std::sync::Mutex<Vec<action::ExecProgress>> = std::sync::Mutex::new(Vec::new());
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |p| events.lock().unwrap().push(p.clone())).unwrap();
    assert_eq!((ex.ok, ex.failed), (10, 0), "{:?}", ex.items);
    assert_eq!((ex.by_dest.permanent.count, ex.by_dest.quarantine.count), (5, 5));
    let last = events.lock().unwrap().last().cloned().expect("至少有最后一次进度");
    assert_eq!((last.done, last.total), (10, 10));
    assert!(last.cost_total > 0.0 && (last.cost_done - last.cost_total).abs() < 1e-6, "{last:?}");
    assert_eq!(last.total_bytes, ex.ok_bytes);
    assert!(!t.p(r"review\deep").exists(), "暂存后变空的子文件夹删掉");
    let m = Manifest::load(Path::new(ex.manifest_path.as_ref().unwrap())).unwrap();
    assert_eq!(m.entries.len(), 5, "只有 review 档进暂存区");
    assert!(m.entries.iter().all(|e| e.original.contains(r"\review\")));
    let rr = quarantine::restore(Path::new(ex.manifest_path.as_ref().unwrap()), &log).unwrap();
    assert_eq!(rr.restored, 5);
    // 用户明确选回收站 → safe_direct = false：safe 档也进回收站（这里只看预检的去向，不真的回收）
    let mut plan2 = action::build_plan(&r, &ctx, &prot, &[req(&t.s("review"))], Strategy::RecycleBin);
    plan2.safe_direct = false;
    let rep2 = action::preflight(&mut plan2, &prot, &PreflightOptions::default());
    assert_eq!((rep2.by_dest.recycle.count, rep2.by_dest.permanent.count), (5, 0));
}

/// 统一队列 + 停止：多个清单项、上千个文件，处理一部分后停止 → 报告 cancelled、剩下的原样不动，
/// 只删“里面的东西全处理完”的空文件夹；统计守恒（成功 + 跳过 + 失败 + 没处理 = 计划总数）。
#[test]
fn cancel_stops_quickly_and_leaves_the_rest_untouched() {
    let t = TempTree::new("cancel");
    let mut all = Vec::new();
    for d in 0..30 {
        for f in 0..100 {
            let rel = format!(r"cache\d{d:02}\f{f:03}.bin");
            write_old(&t.p(&rel), b"0123456789");
            all.push(rel);
        }
    }
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Permanent);
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.total_entries, 3000);
    let log = ActionLog::open(&t.p("actions.log"));
    let cancel = AtomicBool::new(false);
    let mut o = ExecOptions::new(&log);
    o.cancel = Some(&cancel);
    o.workers = 1;
    o.progress_ms = 1;
    let ex = action::execute(&plan, &prot, &o, |p| {
        if p.done >= 300 {
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    })
    .unwrap();
    assert!(ex.cancelled);
    let left = all.iter().filter(|rel| t.p(rel).exists()).count();
    println!("停止：成功 {}，没处理 {}，还在 {left}", ex.ok, ex.not_processed.count);
    assert!(ex.ok >= 300 && ex.ok < 3000, "{}", ex.ok);
    assert_eq!(ex.ok + ex.skipped + ex.failed + ex.not_processed.count, 3000);
    assert_eq!(left, ex.not_processed.count, "没处理的必须都还在");
    // 文件夹：全删光的删掉了，还有文件的留着
    for d in 0..30 {
        let dir = t.p(&format!(r"cache\d{d:02}"));
        let remaining = (0..100).filter(|f| dir.join(format!("f{f:03}.bin")).exists()).count();
        assert_eq!(dir.exists(), remaining > 0, "d{d:02} 还剩 {remaining} 个");
    }
    let lines = read_log(&t.p("actions.log"));
    assert!(lines.iter().any(|l| l["op"] == "cancel"));
    assert!(lines.iter().any(|l| l["op"] == "summary" && l["reason"].as_str().unwrap().contains("已停止")));
}

/// 回收站模式的整夹回收：满足条件的子文件夹整个进回收站（日志仍逐个文件记），条件不满足
/// （预检后多了一个文件）的退回逐个文件，多出来的文件不动。测完把我们自己的东西从回收站挪回来。
#[test]
fn recycle_whole_dir_when_strictly_safe_else_per_file() {
    let t = TempTree::new("rdir");
    for i in 0..40 {
        write_old(&t.p(&format!(r"cache\whole\w{i:02}.bin")), &content(&format!("w{i}"), 500));
    }
    write_old(&t.p(r"cache\whole\inner\x.bin"), b"inner");
    for i in 0..20 {
        write_old(&t.p(&format!(r"cache\grown\g{i:02}.bin")), &content(&format!("g{i}"), 500));
    }
    write_old(&t.p(r"cache\top.bin"), b"top");
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    plan.safe_direct = false;
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.total_entries, 62);
    write_old(&t.p(r"cache\grown\new-after-plan.txt"), b"not in plan");
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    drop(log);
    let vol = volume_root_of(&root).unwrap();
    let mut originals = vec![t.s(r"cache\whole"), t.s(r"cache\top.bin")];
    originals.extend((0..20).map(|i| t.s(&format!(r"cache\grown\g{i:02}.bin"))));
    let lines = read_log(&logp);
    let restored = restore_from_bin(&vol, &originals);
    assert_eq!((ex.ok, ex.failed), (62, 0), "{:?}", ex.items);
    assert_eq!(ex.by_dest.recycle.count, 62);
    assert_eq!(lines.iter().filter(|l| l["op"] == "recycle_dir").count(), 1, "整夹回收一次（whole）");
    assert_eq!(lines.iter().filter(|l| l["method"] == "ifileoperation_dir").count(), 41 + 1, "41 个文件逐个记 + 1 条文件夹记录");
    assert_eq!(lines.iter().filter(|l| l["op"] == "recycle" && l["method"] == "ifileoperation").count(), 21);
    assert!(t.p(r"cache\grown\new-after-plan.txt").exists(), "预检之后多出来的文件不能动");
    assert_eq!(restored, 22, "whole 整夹 + top + 20 个 grown 都该在回收站里");
    assert!(t.p(r"cache\whole\inner\x.bin").exists() && t.p(r"cache\grown\g00.bin").exists());
    assert_eq!(fs::read(t.p(r"cache\whole\w07.bin")).unwrap(), content("w7", 500));
}

/// 跨盘暂存：链接数 > 1 的文件跳过（硬链接，删了不腾空间）；CopyFileExW 收到停止信号时中断、半截的复制品删掉。
#[test]
fn cross_volume_quarantine_skips_hardlinks_and_copy_can_be_cancelled() {
    let Ok(base) = std::env::var("DISKWHY_TEST_XVOL_DIR") else {
        println!("SKIP：没设 DISKWHY_TEST_XVOL_DIR");
        return;
    };
    let base = PathBuf::from(base);
    fs::create_dir_all(&base).unwrap();
    let t = TempTree::new("xhl");
    write_old(&t.p(r"cache\plain.bin"), &content("plain", 5000));
    write_old(&t.p(r"cache\linked.bin"), &content("linked", 5000));
    fs::hard_link(t.p(r"cache\linked.bin"), t.p(r"other-name.bin")).unwrap();
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::in_dir(&base, "xhl-q");
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
    plan.safe_direct = false;
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.skipped_by_reason.get("hardlink").map(|c| c.count), Some(1), "{:?}", rep.skipped_by_reason);
    let hl = rep.items[0].skipped.iter().find(|s| s.code == "hardlink").unwrap();
    assert_eq!(hl.reason, "硬链接，删了不腾空间");
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!((ex.ok, ex.by_dest.quarantine.count), (1, 1));
    assert!(t.p(r"cache\linked.bin").exists());
    let rr = quarantine::restore(Path::new(ex.manifest_path.as_ref().unwrap()), &log).unwrap();
    assert_eq!(rr.restored, 1);
    // CopyFileExW 中断
    let big = t.p("big.bin");
    write_old(&big, &vec![7u8; 8 << 20]);
    let dst = q.p("big-copy.bin");
    let flag = std::sync::atomic::AtomicI32::new(1);
    let e = action::winops::copy_file_cancellable(&big.to_string_lossy(), &dst.to_string_lossy(), Some(&flag)).unwrap_err();
    assert_eq!(e, action::winops::ERROR_REQUEST_ABORTED);
    assert!(!dst.exists(), "半截的复制品必须删掉");
    let err = quarantine::move_file_cancellable(&big.to_string_lossy(), &dst.to_string_lossy(), Some(&flag)).unwrap_err();
    assert!(err.starts_with("已停止"), "{err}");
    assert!(big.exists() && !dst.exists(), "停止后原文件不动、没有复制品");
}

/// 暂存区落点：盘根不可写时的回退链；提权子进程参数检查；新旧文件夹名都认。
#[test]
fn quarantine_destination_fallback_chain() {
    use diskwhy_core::action::quarantine::{elevated_mkdir_from_args, mkdir_exit, prepare_dest_with, DestStatus};
    assert!(matches!(prepare_dest_with(r"C:\Windows", false, &[], 3), DestStatus::Failed { .. }));
    assert_eq!(elevated_mkdir_from_args(&["diskwhy.exe".into()]), None);
    assert_eq!(elevated_mkdir_from_args(&["x".into(), "--mkdir-quarantine".into(), r"C:\Windows".into(), "--grant-sid".into(), "S-1-5-21-1".into()]), Some(mkdir_exit::BAD_ARGS));
    assert_eq!(elevated_mkdir_from_args(&["x".into(), "--mkdir-quarantine".into(), r"C:\".into(), "--grant-sid".into(), "S-1-1-0;rm".into()]), Some(mkdir_exit::BAD_ARGS));
    assert_eq!(quarantine::mkdir_params(r"D:\", "S-1-5-21-9"), r#"--mkdir-quarantine "D:\\" --grant-sid S-1-5-21-9"#);
    // 真机：D:\ 根目录普通用户建不了文件夹（docs/PHASE_J_PLAN.md §0）
    let Ok(base) = std::env::var("DISKWHY_TEST_XVOL_DIR") else { return };
    let vol = volume_root_of(&base).unwrap();
    let q_at_root = Path::new(&vol).join(quarantine::DIR_NAME);
    let root_writable = quarantine::dir_writable(&vol);
    println!("{vol} 根目录可写：{root_writable}，{} 已存在：{}", q_at_root.display(), q_at_root.exists());
    if !root_writable && !q_at_root.exists() {
        assert_eq!(prepare_dest_with(&vol, false, &[], 3), DestStatus::NeedsElevation);
        assert!(matches!(prepare_dest_with(&vol, false, &[], 1), DestStatus::Failed { .. }), "非管理员直接给“选文件夹”");
        // ③ 以前用过的位置仍可写 → 用它
        assert_eq!(prepare_dest_with(&vol, false, &[base.clone()], 3), DestStatus::Ok { dest_root: base.clone() });
        assert!(!Path::new(&base).join(quarantine::DIR_NAME).exists(), "只是试探，不留下文件夹");
    } else {
        assert_eq!(prepare_dest_with(&vol, false, &[], 3), DestStatus::Ok { dest_root: vol.clone() });
    }
    // 旧名 DiskWhy-Quarantine 下的批次照样能列出来
    let q = TempTree::in_dir(Path::new(&base), "oldname");
    let dir = q.p(r"DiskWhy-Quarantine\20260101-000000-aa");
    fs::create_dir_all(&dir).unwrap();
    let mut m = Manifest::new("20260101-000000-aa", &q.s(""), &dir.to_string_lossy());
    m.entries.push(quarantine::ManifestEntry {
        original: r"C:\nowhere\x.bin".into(),
        stored: Some(dir.join(r"C\nowhere\x.bin").to_string_lossy().to_string()),
        kind: EntryKind::File,
        size: 1,
        bytes: 4096,
        mtime: 0,
        quarantined_at: 0,
        item: r"C:\nowhere".into(),
        source: "t".into(),
        tier: diskwhy_core::explain::Tier::Safe,
        method: "rename".into(),
        status: "stored".into(),
        note: String::new(),
    });
    m.save(&dir.join(quarantine::MANIFEST)).unwrap();
    let b = quarantine::list_batches(&[q.s("")]);
    assert_eq!(b.len(), 1);
    let log = ActionLog::open(&q.p("actions.log"));
    let pr = quarantine::purge(&dir.join(quarantine::MANIFEST), &log).unwrap();
    assert!(pr.removed);
}

/// 日志轮转（> 阈值改名为 actions.<时间>.log，留最近几份、不覆盖）；回收站上限 / 用量读得到。
#[test]
fn log_rotation_and_recycle_capacity_readable() {
    use diskwhy_core::action::log::{rotated_logs, rotated_path, ROTATE_KEEP};
    let t = TempTree::new("rot");
    let p = t.p("actions.log");
    fs::write(&p, vec![b'x'; 2000]).unwrap();
    fs::write(rotated_path(&p), b"older").unwrap(); // 旧版本留下的 actions.1.log
    let log = ActionLog::open_rotating(&p, 1000);
    log.write(&diskwhy_core::action::log::LogRecord::new("b", "start", "p"));
    drop(log);
    let r = rotated_logs(&p);
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(fs::metadata(&r[0]).unwrap().len(), 2000, "旧日志改名为 actions.<时间>.log");
    assert_eq!(fs::read_to_string(&p).unwrap().lines().count(), 1);
    // 阶段 J 复核：连着轮转（每次清理 / 还原都会打开日志）不能冲掉出事那次的日志——第一份（2000 个 x）一直在，
    // 直到攒够 ROTATE_KEEP 份之后才按时间删最早的；旧版本的 actions.1.log 永远不动
    for k in 0..ROTATE_KEEP - 1 {
        fs::write(&p, vec![b'y'; 1500 + k]).unwrap();
        drop(ActionLog::open_rotating(&p, 1000));
        assert!(rotated_logs(&p).iter().any(|x| fs::metadata(x).map(|m| m.len()).unwrap_or(0) == 2000), "第 {k} 次轮转后第一份还在");
    }
    assert_eq!(rotated_logs(&p).len(), ROTATE_KEEP);
    fs::write(&p, vec![b'z'; 3000]).unwrap();
    drop(ActionLog::open_rotating(&p, 1000));
    assert_eq!(rotated_logs(&p).len(), ROTATE_KEEP, "最多留 {ROTATE_KEEP} 份");
    assert_eq!(fs::read(rotated_path(&p)).unwrap(), b"older", "旧版本的 actions.1.log 不动");
    let vol = volume_root_of(&t.s("")).unwrap();
    let t0 = std::time::Instant::now();
    let usage = action::winops::recycle_bin_usage(&vol);
    let cap = action::winops::recycle_bin_max_capacity(&vol);
    println!("{vol} 回收站：{usage:?}（{:?}），上限 {cap:?}，卷 {:?}", t0.elapsed(), action::winops::volume_guid(&vol));
    assert!(action::winops::volume_guid(&vol).is_some());
    assert!(usage.is_some());
}

// ───────────────────────── 阶段 J harden（PHASE_J_CRASH.md）─────────────────────────

/// 回收站：每一项的结果来自 IFileOperationProgressSink::PostDeleteItem——进了回收站 / 被占用（带 HRESULT）/ 打不开。
#[test]
fn recycle_items_reports_per_item_results() {
    use action::winops::{self, RecycleOutcome};
    let t = TempTree::new("ritems");
    write_old(&t.p("a.bin"), b"a");
    write_old(&t.p("b.bin"), b"b");
    // b 被别的程序以“读写共享、不共享删除”打开着
    let hold = fs::OpenOptions::new().read(true).share_mode(1 | 2).open(t.p("b.bin")).unwrap();
    let r = {
        let _com = winops::ComSta::init();
        winops::recycle_items(&[t.s("a.bin"), t.s("b.bin"), t.s("missing.bin")])
    };
    drop(hold);
    let vol = volume_root_of(&t.s("")).unwrap();
    let restored = restore_from_bin(&vol, &[t.s("a.bin")]);
    println!("{r:?}");
    assert_eq!(r[0], RecycleOutcome::Recycled);
    assert!(matches!(&r[1], RecycleOutcome::Failed { hr, .. } if *hr < 0), "被占用的应带失败 HRESULT：{:?}", r[1]);
    assert!(matches!(&r[2], RecycleOutcome::Failed { .. }));
    assert_eq!(restored, 1);
    assert!(t.p("b.bin").exists());
}

/// 预检之后才被占用的文件：回收时不再报“回收站操作被中止（你点了否）”，而是“跳过 · 被占用”。
#[test]
fn recycle_locked_after_preflight_is_skipped_locked_not_failed() {
    let t = TempTree::new("rlock");
    for i in 0..3 {
        write_old(&t.p(&format!(r"cache\f{i}.bin")), &content(&format!("f{i}"), 1000));
    }
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    plan.safe_direct = false;
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let hold = fs::OpenOptions::new().read(true).share_mode(1 | 2).open(t.p(r"cache\f1.bin")).unwrap();
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    drop(log);
    drop(hold);
    let vol = volume_root_of(&root).unwrap();
    let restored = restore_from_bin(&vol, &[t.s(r"cache\f0.bin"), t.s(r"cache\f2.bin")]);
    assert_eq!((ex.ok, ex.skipped, ex.failed), (2, 1, 0), "{:?}", ex.items);
    assert_eq!(ex.skipped_by_reason.get("locked").map(|c| c.count), Some(1));
    assert_eq!(restored, 2);
    assert!(t.p(r"cache\f1.bin").exists());
    let lines = read_log(&logp);
    let l = lines.iter().find(|l| l["op"] == "recycle" && l["result"] == "skipped").expect("占用的那条记成 skipped");
    assert!(l["reason"].as_str().unwrap().contains("占用"), "{l}");
}

/// 稀疏文件：逻辑大小很大、几乎不占盘。只有真的设上了稀疏属性才返回 true（否则 set_len 会真占空间，不能做）。
fn make_sparse(p: &Path, size: u64) -> bool {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).unwrap();
    }
    let Ok(f) = fs::OpenOptions::new().read(true).write(true).create_new(true).open(p) else { return false };
    const FSCTL_SET_SPARSE: u32 = 0x0009_00C4;
    let mut ret = 0u32;
    let ok = unsafe {
        windows_sys::Win32::System::IO::DeviceIoControl(
            f.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut ret,
            std::ptr::null_mut(),
        )
    } != 0;
    let sparse = ok && fs::metadata(p).map(|m| m.file_attributes() & 0x200 != 0).unwrap_or(false);
    if !sparse {
        drop(f);
        let _ = fs::remove_file(p);
        return false;
    }
    f.set_len(size).is_ok()
}

/// 比回收站上限还大的文件：预检就标成“跳过 · 回收站放不下”，执行时不送进回收站（否则 Windows 会直接永久删除），
/// 原样不动；同一项里的小文件照常进回收站。用稀疏文件，不真的占空间。
#[test]
fn recycle_refuses_file_bigger_than_bin_capacity() {
    let Ok(base) = std::env::var("DISKWHY_TEST_XVOL_DIR") else {
        println!("SKIP：没设 DISKWHY_TEST_XVOL_DIR（要一个空间充足的盘放稀疏文件）");
        return;
    };
    let t = TempTree::in_dir(Path::new(&base), "rbig");
    write_old(&t.p(r"cache\small.bin"), b"small");
    let vol = volume_root_of(&t.s("")).unwrap();
    let info = action::winops::RecycleBinInfo::read(&vol);
    println!("{vol} 回收站设置：{info:?}");
    let Some(cap) = info.max_bytes.filter(|_| !info.nuke) else {
        println!("SKIP：读不到回收站上限，或这个盘的回收站是“直接删除”");
        return;
    };
    let big = t.p(r"cache\big.bin");
    if !make_sparse(&big, cap + (1 << 20)) {
        println!("SKIP：建不了稀疏文件");
        return;
    }
    backdate(&big, 7200);
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    plan.safe_direct = false;
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.skipped_by_reason.get("recycle_unavailable").map(|c| c.count), Some(1), "{:?}", rep.skipped_by_reason);
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    drop(log);
    let restored = restore_from_bin(&vol, &[t.s(r"cache\small.bin")]);
    assert!(big.exists(), "太大的文件不能被送进回收站（会被直接删掉）");
    assert_eq!(fs::metadata(&big).unwrap().len(), cap + (1 << 20));
    assert_eq!((ex.ok, ex.by_dest.recycle.count, ex.by_dest.permanent.count), (1, 1, 0));
    assert_eq!(ex.skipped_by_reason.get("recycle_unavailable").map(|c| c.count), Some(1));
    assert_eq!(restored, 1);
    // 判断规则本身
    let b = action::winops::RecycleBinInfo { max_bytes: Some(100), nuke: false };
    assert!(b.refuse(100).is_none() && b.refuse(101).is_some());
    assert!(action::winops::RecycleBinInfo { max_bytes: None, nuke: true }.refuse(1).is_some());
    assert!(action::winops::RecycleBinInfo { max_bytes: Some(0), nuke: false }.refuse(1).is_some());
}

/// 正在运行的 Chromium / Electron 程序的磁盘缓存：程序用“全共享（含删除）”打开缓存文件，占用探测看不出来，
/// 但 Restart Manager 能查到 → 整个缓存文件夹跳过（被占用），没人用的缓存照常删。
#[test]
fn live_chromium_cache_is_skipped_even_when_opened_with_share_delete() {
    let t = TempTree::new("chrome");
    write_old(&t.p(r"cache\app\GPUCache\index"), b"gpu-index");
    write_old(&t.p(r"cache\app\GPUCache\data_0"), &content("d0", 4000));
    write_old(&t.p(r"cache\app\GPUCache\data_1"), &content("d1", 4000));
    write_old(&t.p(r"cache\app\Cache\Cache_Data\index"), b"idx");
    write_old(&t.p(r"cache\app\Cache\Cache_Data\0123456789abcdef_0"), &content("e0", 3000));
    write_old(&t.p(r"cache\app\Cache\Cache_Data\index-dir\the-real-index"), b"real");
    write_old(&t.p(r"cache\other.bin"), &content("o", 2000));
    // “某个程序”正在用 GPUCache：FILE_SHARE_READ | WRITE | DELETE 打开 index（Chromium 的做法）
    let hold = fs::OpenOptions::new().read(true).share_mode(1 | 2 | 4).open(t.p(r"cache\app\GPUCache\index")).unwrap();
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    assert!(plan.safe_direct, "safe 档默认直接删除");
    let dirs = action::preflight::chromium_cache_dirs(&plan);
    assert_eq!(dirs.len(), 2, "{dirs:?}");
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    println!("{:?} / {:?}", rep.skipped_by_reason, rep.timing_ms);
    assert_eq!(rep.skipped_by_reason.get("locked").map(|c| c.count), Some(3), "GPUCache 里 3 个都跳过：{:?}", rep.skipped_by_reason);
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    drop(log);
    drop(hold);
    assert_eq!((ex.ok, ex.failed), (4, 0), "{:?}", ex.items);
    assert!(t.p(r"cache\app\GPUCache\index").exists() && t.p(r"cache\app\GPUCache\data_0").exists() && t.p(r"cache\app\GPUCache\data_1").exists());
    assert!(!t.p(r"cache\app\Cache\Cache_Data\index").exists() && !t.p(r"cache\other.bin").exists());
}

/// DiskWhy 自己的数据（日志 / 设置 / 界面的 WebView2 数据）受保护：正在运行时删自己的 EBWebView 缓存会弄坏自己的界面。
#[test]
fn protect_own_app_data() {
    let vars: Vec<(String, String)> = [("USERPROFILE", r"C:\Users\alice"), ("LOCALAPPDATA", r"C:\Users\alice\AppData\Local")]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    let p = Protector::with(&vars, &[], &[r"C:\".into()]);
    assert_blocked(&p, r"C:\Users\alice\AppData\Local\com.diskwhy.desktop\EBWebView\Default\Cache\Cache_Data\index", "DiskWhy");
    assert_blocked(&p, r"C:\Users\alice\AppData\Local\com.diskwhy.desktop", "DiskWhy");
    assert_blocked(&p, r"C:\Users\alice\AppData\Local\DiskWhy\actions.log", "DiskWhy");
    assert_allowed(&p, r"C:\Users\alice\AppData\Local\Temp\x.tmp");
    assert_allowed(&p, r"C:\Users\alice\AppData\Local\Google\Chrome\User Data\Default\Cache");
}

// ───────────────────────── 阶段 J 复核修正的回归测试 ─────────────────────────

/// 回收站收不下太长的路径（≥ 260 个字符时 IFileOperation 不报错、直接永久删除）：预检标成“跳过 · 回收站放不下”，
/// 执行时也不送（哪怕预检的决定被改回“删”）；同一个文件夹里的短路径文件照常进回收站，而且**不整夹回收**
/// （整夹送进去会连短路径的一起被直接删掉）。一个都不能被永久删除。测完把我们自己的东西从回收站挪回来。
#[test]
fn recycle_never_nukes_long_paths_or_their_folder() {
    let Ok(base) = std::env::var("DISKWHY_TEST_XVOL_DIR") else {
        println!("SKIP：没设 DISKWHY_TEST_XVOL_DIR");
        return;
    };
    fs::create_dir_all(&base).unwrap();
    let t = TempTree::in_dir(Path::new(&base), "longp");
    for i in 0..20 {
        write_old(&t.p(&format!(r"cache\sub\s{i:02}.bin")), &content(&format!("s{i}"), 300));
    }
    write_old(&t.p(r"cache\top.bin"), b"top");
    // 凑一个 290 多字符的路径（std 会自动加 \\?\ 前缀）
    let mut deep = String::from(r"cache\sub");
    while t.s(&deep).len() < 270 {
        deep.push_str(r"\dddddddddddddddddddd");
    }
    let long_rel = format!(r"{deep}\long-name-file.bin");
    write_old(&t.p(&long_rel), &content("long", 500));
    let long_path = t.s(&long_rel);
    assert!(long_path.encode_utf16().count() >= 290, "{}", long_path.len());
    let root = t.s("");
    let ctx = ctx_for(&root);
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    plan.safe_direct = false;
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.skipped_by_reason.get("recycle_unavailable").map(|c| c.count), Some(1), "{:?}", rep.skipped_by_reason);
    // 执行时的第二道检查：把预检的决定改回“删”
    {
        let item = &mut plan.items[0];
        let i = item.entries.iter().position(|e| e.path == long_path).expect("清单里有长路径文件");
        assert!(matches!(item.decisions[i], Decision::Skip { reason: SkipReason::RecycleUnavailable { .. } }));
        item.decisions[i] = Decision::Delete;
    }
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    drop(log);
    let vol = volume_root_of(&root).unwrap();
    let mut originals: Vec<String> = (0..20).map(|i| t.s(&format!(r"cache\sub\s{i:02}.bin"))).collect();
    originals.push(t.s(r"cache\top.bin"));
    originals.push(t.s(r"cache\sub")); // 万一整夹进去了也挪回来
    let restored = restore_from_bin(&vol, &originals);
    let lines = read_log(&logp);
    assert!(Path::new(&format!(r"\\?\{long_path}")).exists(), "长路径文件必须原样还在");
    assert_eq!(ex.by_dest.nuked.count, 0, "一个都不能被 Windows 直接删掉：{:?}", ex.by_dest);
    assert_eq!(ex.by_dest.permanent.count, 0);
    assert_eq!((ex.ok, ex.by_dest.recycle.count), (21, 21), "{:?}", ex.items[0].details);
    assert_eq!(ex.skipped_by_reason.get("recycle_unavailable").map(|c| c.count), Some(1));
    assert_eq!(lines.iter().filter(|l| l["op"] == "recycle_dir").count(), 0, "有长路径的文件夹不能整夹回收");
    assert_eq!(restored, 21, "21 个短路径文件都在回收站里（挪回来了）");
    assert!(ex.notes.is_empty(), "没出问题就没有“注意”：{:?}", ex.notes);
}

/// 数据放在 MFT 文件记录里的小文件（几十字节）删掉不腾出任何簇：预检按 0 字节算，大一点的照常按簇取整。
#[test]
fn tiny_mft_resident_files_count_as_zero_bytes() {
    let t = TempTree::new("tiny");
    for i in 0..10 {
        write_old(&t.p(&format!(r"cache\t{i}.bin")), &content(&format!("t{i}"), 64));
    }
    write_old(&t.p(r"cache\big.bin"), &content("big", 5000));
    let root = t.s("");
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx_for(&root), &prot, &[req(&t.s("cache"))], Strategy::Permanent);
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let cluster = volume_space(&volume_root_of(&root).unwrap()).unwrap().cluster_size;
    assert_eq!(rep.total_entries, 11);
    assert_eq!(rep.total_bytes, 5000u64.div_ceil(cluster) * cluster, "64 字节的文件不占簇：{}", rep.total_bytes);
}

/// 扫描（首页 / 清理页的数字）和预检（确认弹层）对小文件用同一条规则：常驻 MFT 的按 0，其余按簇取整。
/// 2026-10-07 在 D: 实测：短名字时 720 字节还常驻、750 起整簇；这里取 64 / 400 / 700（常驻）和 800 / 1000 / 5000（不常驻）。
#[test]
fn scan_and_preflight_agree_on_tiny_files() {
    let t = TempTree::new("tiny2");
    let sizes = [0usize, 64, 400, 700, 800, 1000, 5000];
    for (i, n) in sizes.iter().enumerate() {
        for k in 0..20 {
            write_old(&t.p(&format!(r"cache\s{i}\f{k:02}.bin")), &content(&format!("{i}-{k}"), *n));
        }
    }
    let root = t.s("");
    let vol = volume_space(&volume_root_of(&root).unwrap()).unwrap();
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx_for(&root), &prot, &[req(&t.s("cache"))], Strategy::Permanent);
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let cache = r.tree.children.iter().find(|c| c.name == "cache").expect("cache");
    for (i, n) in sizes.iter().enumerate() {
        let scan_bytes = cache.children.iter().find(|c| c.name == format!("s{i}")).expect("s{i}").allocated;
        let sub = format!("\\s{i}\\");
        let pre_bytes: u64 = plan.items[0].entries.iter().filter(|e| e.path.contains(&sub)).map(|e| e.bytes).sum();
        assert_eq!(scan_bytes, pre_bytes, "{n} 字节 × 20：扫描 {scan_bytes} vs 预检 {pre_bytes}");
        if vol.ntfs {
            let expect = if *n <= 700 { 0 } else { 20 * (*n as u64).div_ceil(vol.cluster_size) * vol.cluster_size };
            assert_eq!(scan_bytes, expect, "{n} 字节 × 20（NTFS）");
        }
    }
}

/// 预检时没人用、执行前才打开的浏览器缓存：执行开始时再问一次 Restart Manager，整个缓存文件夹跳过（被占用）。
#[test]
fn chromium_cache_opened_after_preflight_is_skipped_at_execute() {
    let t = TempTree::new("chrome2");
    write_old(&t.p(r"cache\app\GPUCache\index"), b"gpu-index");
    write_old(&t.p(r"cache\app\GPUCache\data_0"), &content("d0", 4000));
    write_old(&t.p(r"cache\app\GPUCache\data_1"), &content("d1", 4000));
    write_old(&t.p(r"cache\other.bin"), &content("o", 2000));
    let root = t.s("");
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx_for(&root), &prot, &[req(&t.s("cache"))], Strategy::Permanent);
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.total_entries, 4, "预检时没人用：{:?}", rep.skipped_by_reason);
    // 现在“浏览器”打开了：全共享（含删除），删除探测看不出来
    let hold = fs::OpenOptions::new().read(true).share_mode(1 | 2 | 4).open(t.p(r"cache\app\GPUCache\index")).unwrap();
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    drop(log);
    drop(hold);
    assert_eq!((ex.ok, ex.failed), (1, 0), "{:?}", ex.items);
    assert_eq!(ex.skipped_by_reason.get("locked").map(|c| c.count), Some(3), "{:?}", ex.skipped_by_reason);
    assert!(t.p(r"cache\app\GPUCache\data_0").exists() && t.p(r"cache\app\GPUCache\index").exists());
    assert!(!t.p(r"cache\other.bin").exists());
}

/// 暂存：执行中途程序被结束时，manifest.json 还是空的，移进来的文件只记在 manifest.journal 里——
/// 读批次（暂存区页 / 还原）要把它们认出来；整份保存后 journal 删掉。
#[test]
fn quarantine_journal_survives_kill_and_is_merged() {
    let q = TempTree::new("journal");
    let dir = q.p(r"DiskWhy暂存区\20261006-000000-jj");
    fs::create_dir_all(&dir).unwrap();
    let mp = dir.join(quarantine::MANIFEST);
    let m = Manifest::new("20261006-000000-jj", &q.s(""), &dir.to_string_lossy());
    m.save(&mp).unwrap();
    let entry = |n: usize| quarantine::ManifestEntry {
        original: format!(r"C:\nowhere\f{n}.bin"),
        stored: Some(dir.join(format!(r"C\nowhere\f{n}.bin")).to_string_lossy().to_string()),
        kind: EntryKind::File,
        size: 10,
        bytes: 4096,
        mtime: 0,
        quarantined_at: 0,
        item: r"C:\nowhere".into(),
        source: "t".into(),
        tier: diskwhy_core::explain::Tier::Review,
        method: "rename".into(),
        status: "stored".into(),
        note: String::new(),
    };
    {
        let j = quarantine::Journal::open(&mp);
        for n in 0..3 {
            assert!(j.append(&entry(n)));
        }
        // 被结束的那一刻：最后一行只写了一半
        drop(j);
        let mut f = fs::OpenOptions::new().append(true).open(quarantine::journal_path(&mp)).unwrap();
        f.write_all(br#"{"original":"C:\\nowhere\\half"#).unwrap();
    }
    let loaded = Manifest::load(&mp).unwrap();
    assert_eq!(loaded.entries.len(), 3, "journal 里完整的 3 行都认出来，半行跳过");
    let b = quarantine::list_batches(&[q.s("")]);
    assert_eq!(b.len(), 1, "只有 journal 有记录的批次也要在暂存区里列出来");
    assert_eq!(b[0].stored_entries, 3);
    loaded.save(&mp).unwrap();
    assert!(!quarantine::journal_path(&mp).exists(), "整份写好后 journal 删掉");
    assert_eq!(Manifest::load(&mp).unwrap().entries.len(), 3, "不重复");
}

/// 暂存执行：每移进来一个都写进 journal；结束时整份 manifest 写好、journal 删掉。
#[test]
fn quarantine_execute_writes_journal_then_compacts() {
    let t = TempTree::new("jexec");
    for i in 0..5 {
        write_old(&t.p(&format!(r"review\r{i}.bin")), &content(&format!("r{i}"), 2000));
    }
    let root = t.s("");
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let q = TempTree::new("jexec-q");
    let mut plan = action::build_plan(&r, &ctx_for(&root), &prot, &[req(&t.s("review"))], Strategy::Quarantine { dest_root: q.s("") });
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!(ex.by_dest.quarantine.count, 5);
    let mp = PathBuf::from(ex.manifest_path.as_ref().unwrap());
    assert!(!quarantine::journal_path(&mp).exists(), "结束后 journal 已并进 manifest.json");
    assert_eq!(Manifest::load(&mp).unwrap().entries.len(), 5);
    assert!(ex.notes.is_empty(), "正常暂存不出“注意”：{:?}", ex.notes);
    let lines = read_log(&t.p("actions.log"));
    assert_eq!(lines.iter().find(|l| l["op"] == "start").unwrap()["method"].as_str().unwrap(), format!("暂存区（{}）", q.s("")));
    let rr = quarantine::restore(&mp, &log).unwrap();
    assert_eq!(rr.restored, 5);
    assert!(rr.batch_removed, "还原后批次文件夹删干净（journal 不能挡着）");
}

/// 一开始就停止（一块都没开始）：什么都不动——连清单项里原来就空着的子文件夹也不删；报告“已停止”。
/// 只有可放心清理、直接删除的计划，日志里的方式写“直接删除”（不是策略的“回收站”）。
#[test]
fn stop_before_start_touches_nothing_and_method_is_honest() {
    let t = TempTree::new("stop0");
    for i in 0..5 {
        write_old(&t.p(&format!(r"cache\f{i}.bin")), &content(&format!("f{i}"), 1000));
    }
    fs::create_dir_all(t.p(r"cache\empty-before")).unwrap();
    let root = t.s("");
    let r = scan_dir(&root);
    let prot = Protector::from_system();
    let mut plan = action::build_plan(&r, &ctx_for(&root), &prot, &[req(&t.s("cache"))], Strategy::RecycleBin);
    action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let logp = t.p("actions.log");
    let log = ActionLog::open(&logp);
    let cancel = AtomicBool::new(true);
    let mut o = ExecOptions::new(&log);
    o.cancel = Some(&cancel);
    let ex = action::execute(&plan, &prot, &o, |_| {}).unwrap();
    drop(log);
    assert!(ex.cancelled);
    assert_eq!((ex.ok, ex.not_processed.count), (0, 5));
    assert!(t.p(r"cache\empty-before").exists(), "没开始的项里原来就空着的文件夹不能删");
    assert_eq!(ex.items[0].dirs_removed, 0);
    let lines = read_log(&logp);
    assert!(!lines.iter().any(|l| l["op"] == "rmdir"));
    assert_eq!(lines.iter().find(|l| l["op"] == "start").unwrap()["method"], "直接删除");
    // 不停止：照常删、空文件夹也删
    let log = ActionLog::open(&logp);
    let ex2 = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert!(!ex2.cancelled);
    assert_eq!(ex2.ok, 5);
    assert!(!t.p(r"cache\empty-before").exists());
}

/// 提权子进程改权限：在同一个句柄上做，联接（junction）一律拒绝（不会跟过去改目标的权限）；只接受个人账户的 SID。
#[test]
fn elevated_grant_refuses_junctions_and_group_sids() {
    use action::winops::{current_user_sid, grant_modify_plain_dir};
    let t = TempTree::new("grant");
    let target = t.p("target");
    fs::create_dir_all(&target).unwrap();
    let plain = t.p("plain");
    fs::create_dir_all(&plain).unwrap();
    mklink_j(&t.p("jn"), &target);
    let sid = current_user_sid().expect("当前用户 SID");
    assert!(quarantine::valid_sid(&sid), "{sid}");
    for bad in ["S-1-1-0", "S-1-5-11", "S-1-5-32-545", "S-1-5-21-", "S-1-5-21-1--2", "S-1-5-21-1-x"] {
        assert!(!quarantine::valid_sid(bad), "{bad} 必须拒绝");
    }
    assert!(quarantine::valid_sid("S-1-12-1-111-222-333-444"));
    let before = Command::new("icacls").arg(&target).output().unwrap().stdout;
    assert_eq!(grant_modify_plain_dir(&t.s("jn"), &sid), Err(336), "联接：拒绝");
    assert_eq!(Command::new("icacls").arg(&target).output().unwrap().stdout, before, "联接指向的文件夹权限没变");
    assert_eq!(grant_modify_plain_dir(&plain.to_string_lossy(), &sid), Ok(()));
    let after = String::from_utf8_lossy(&Command::new("icacls").arg(&plain).output().unwrap().stdout).to_string();
    println!("{after}");
    assert!(after.contains("(OI)(CI)") && after.contains("(M)"), "{after}");
    assert_eq!(grant_modify_plain_dir(&plain.to_string_lossy(), "S-1-x"), Err(1337));
}
