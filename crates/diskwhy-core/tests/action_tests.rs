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

/// 在 `X:\$Recycle.Bin\<SID>\$I*` 里找原路径以 `name` 结尾的记录；返回 ($I, $R, 记录的原路径)。
fn find_in_recycle_bin(vol: &str, name: &str) -> Option<(PathBuf, PathBuf, String)> {
    let rb = Path::new(vol).join("$Recycle.Bin");
    for sid in fs::read_dir(&rb).ok()?.flatten() {
        let Ok(rd) = fs::read_dir(sid.path()) else { continue };
        for f in rd.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            if !fname.starts_with("$I") {
                continue;
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
            if orig.to_lowercase().ends_with(&name.to_lowercase()) {
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
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    assert_eq!(rep.total_entries, 1);
    let log = ActionLog::open(&t.p("actions.log"));
    let ex = action::execute(&plan, &prot, &ExecOptions::new(&log), |_| {}).unwrap();
    assert_eq!((ex.ok, ex.failed), (1, 0), "{:?}", ex.items);
    assert!(!t.p(&rel).exists(), "原位置应该没有了");
    let vol = volume_root_of(&t.s("")).unwrap();
    let (i_path, r_path, recorded) = find_in_recycle_bin(&vol, &name).expect("回收站里必须能找到这个文件（$Recycle.Bin 下的 $I 记录）");
    assert!(r_path.exists(), "对应的 $R 数据文件必须存在：{}", r_path.display());
    assert_eq!(fs::metadata(&r_path).unwrap().len(), data.len() as u64);
    assert_eq!(sha(&r_path), h0, "回收站里的内容必须与原文件一致");
    println!("回收站记录：{} → {}（原路径记录为 {recorded}）", i_path.display(), r_path.display());
    // 只动我们自己这一条：把 $R 移回原位，删掉对应的 $I
    fs::rename(&r_path, t.p(&rel)).unwrap();
    fs::remove_file(&i_path).unwrap();
    assert_eq!(sha(&t.p(&rel)), h0, "从回收站还原后哈希一致");
    assert!(find_in_recycle_bin(&vol, &name).is_none());
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
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let vol = volume_root_of(&root).unwrap();
    let sp = volume_space(&vol).unwrap();
    let low = (sp.free_bytes as f64) < sp.total_bytes as f64 * 0.10;
    let w = rep.warnings.iter().find(|w| w.code == "recycle_low_space");
    assert_eq!(w.is_some(), low, "可用 {} / {}，警告：{:?}", sp.free_bytes, sp.total_bytes, rep.warnings);
    if let Some(w) = w {
        assert!(w.message.contains("回收站") && w.message.contains("清空回收站") && w.message.contains("隔离"));
        println!("{vol} 可用 {:.1}% → 警告：{}", sp.free_bytes as f64 * 100.0 / sp.total_bytes as f64, w.message);
    }
    // 可切换到的隔离区盘不包含源所在的盘
    assert!(rep.quarantine_choices.iter().all(|d| !d.root.eq_ignore_ascii_case(&vol)));
    // 隔离区放在同一个盘上 → 提示不会腾空间
    let q = TempTree::new("space-q");
    let mut plan2 = action::build_plan(&r, &ctx, &prot, &[req(&t.s("cache"))], Strategy::Quarantine { dest_root: q.s("") });
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
    assert_eq!(lines.len(), 2 + 1 + 1 + 1 + 2 + 1);
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
    let rep = action::preflight(&mut plan, &prot, &PreflightOptions::default());
    let w = rep.warnings.iter().find(|w| w.code == "quarantine_bad_dest").expect("不能写的隔离区必须被拦下");
    assert!(w.blocking && w.message.contains("建不了隔离区文件夹"), "{}", w.message);
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
