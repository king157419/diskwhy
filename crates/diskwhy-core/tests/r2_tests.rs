//! 第 2 轮安全评审（`_ops/S22/评审-r2-safety.md`）严重项的测试。
//!
//! - S1-A：被合并进“其余 N 个文件”的文档，护栏以前看不见（探针 R 节，真实扫描）。
//! - S1-B：往下找陷阱超过上限就放行（探针 T 节）。
//! - S2：safe 的上层吞掉下层 review 规则的字节（探针 N 节；另加“Temp 里只有 Rar$EX + .psd”）。
//! - 合并阶段 D 用的删除闸门：`delete_block_reason` / `batch_deletable`，在陷阱语料上断言
//!   “整块删会碰到用户数据的项一律拦下”（阶段 D 的 `action::plan` 在 master 上，合并后照这个接）。

use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::LlmSettings;
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::RuleSet;
use diskwhy_core::explain::{batch_deletable, delete_block_reason, explain, tier_summary, ExplainCtx, Explanation, Source, Target, Tier};
use diskwhy_core::view::{explain_path, find_node};
use diskwhy_core::*;
use std::collections::BTreeMap;

const U: &str = r"C:\Users\bob";

fn env_home(home: &str, temp: &str) -> EnvMap {
    EnvMap::from_pairs(&[
        ("SystemDrive", "C:"),
        ("SystemRoot", r"C:\Windows"),
        ("WINDIR", r"C:\Windows"),
        ("ProgramFiles", r"C:\Program Files"),
        ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
        ("ProgramData", r"C:\ProgramData"),
        ("USERPROFILE", home),
        ("LOCALAPPDATA", &format!(r"{home}\AppData\Local")),
        ("APPDATA", &format!(r"{home}\AppData\Roaming")),
        ("TEMP", temp),
        ("PUBLIC", r"C:\Users\Public"),
    ])
}

fn ctx_with(env: EnvMap) -> ExplainCtx {
    let (rules, errs) = RuleSet::embedded(&env);
    let _ = errs;
    ExplainCtx::new(env, rules, RegistrySnapshot::from_apps(Vec::new()), LlmSettings::default())
}

fn ctx_default() -> ExplainCtx {
    ctx_with(env_home(U, &format!(r"{U}\AppData\Local\Temp")))
}

fn leaf(name: &str, size: u64) -> Node {
    Node {
        name: name.to_string(),
        path: String::new(),
        kind: NodeKind::File,
        logical: size,
        allocated: size,
        mtime: 1_750_000_000,
        attrs: 0x20,
        reparse_tag: 0,
        inaccessible: false,
        error_code: 0,
        reparse_skipped: false,
        cloud_placeholder: false,
        system_reserved: false,
        hardlink_dup: false,
        mapped: false,
        rest_flags: 0,
        files: 1,
        dirs: 0,
        children: vec![],
    }
}

/// 内存里的树：文件路径 → 大小。名字以 `<rest:FLAGS:N>` 开头的“文件”造成一个 Rest 节点。
fn tree(drive: &str, entries: &[(String, u64)]) -> Node {
    #[derive(Default)]
    struct D {
        dirs: BTreeMap<String, D>,
        files: Vec<(String, u64)>,
    }
    let mut root = D::default();
    let prefix = format!("{drive}\\");
    for (p, sz) in entries {
        let rel = p.strip_prefix(&prefix).expect("drive");
        let parts: Vec<&str> = rel.split('\\').filter(|s| !s.is_empty()).collect();
        let mut cur = &mut root;
        for part in &parts[..parts.len() - 1] {
            cur = cur.dirs.entry(part.to_string()).or_default();
        }
        cur.files.push((parts[parts.len() - 1].to_string(), *sz));
    }
    fn build(name: &str, path: &str, d: &D) -> Node {
        let mut children: Vec<Node> = Vec::new();
        for (n, sub) in &d.dirs {
            let p = if path.ends_with('\\') { format!("{path}{n}") } else { format!("{path}\\{n}") };
            children.push(build(n, &p, sub));
        }
        for (n, sz) in &d.files {
            if let Some(spec) = n.strip_prefix("<rest:") {
                // <rest:FLAGS:N>
                let spec = spec.trim_end_matches('>');
                let mut it = spec.split(':');
                let flags: u8 = it.next().unwrap().parse().unwrap();
                let count: u64 = it.next().unwrap().parse().unwrap();
                let mut r = leaf(&format!("（其余 {count} 个文件）"), *sz);
                r.kind = NodeKind::Rest;
                r.rest_flags = flags;
                r.files = count;
                children.push(r);
            } else {
                children.push(leaf(n, *sz));
            }
        }
        children.sort_by(|a, b| b.allocated.cmp(&a.allocated));
        let mut me = leaf(name, 0);
        me.path = path.to_string();
        me.kind = NodeKind::Dir;
        me.attrs = 0x10;
        me.allocated = children.iter().map(|c| c.allocated).sum();
        me.logical = me.allocated;
        me.files = children.iter().map(|c| c.files).sum();
        me.dirs = children.iter().map(|c| if c.kind == NodeKind::Dir { 1 + c.dirs } else { 0 }).sum();
        me.children = children;
        me
    }
    build(&prefix, &prefix, &root)
}

fn ex(t: &Node, c: &ExplainCtx, p: &str) -> Explanation {
    explain_path(t, p, c).unwrap_or_else(|| panic!("树里没有 {p}")).1
}

fn src(e: &Explanation) -> String {
    match &e.source {
        Source::Rule { id, inherited_from, .. } => format!("规则 {id}{}", if inherited_from.is_some() { "（继承）" } else { "" }),
        Source::Heuristic { name } => format!("启发式 {name}"),
        Source::Llm { model } => format!("AI {model}"),
        Source::Unknown => "未知".into(),
    }
}

/// 解释某个目录下的 Rest 节点。
fn ex_rest(t: &Node, c: &ExplainCtx, dir: &str) -> Explanation {
    let (mut chain, node) = find_node(t, dir).unwrap();
    let rest = node.children.iter().find(|x| x.kind == NodeKind::Rest).expect("没有 Rest");
    chain.push(node);
    let target = Target { node: rest, path: &node.path, parent: Some(node), ancestors: &chain };
    explain(&target, c)
}

// ───────────────────────── S1-A：其余小文件里的文档 ─────────────────────────

#[test]
fn s1a_rest_flags_from_names() {
    use diskwhy_core::explain::heuristics::rest_flags_of;
    assert_eq!(rest_flags_of(["a.tmp", "b.log"].into_iter()), REST_CHECKED);
    assert_eq!(rest_flags_of(["a.tmp", "投标方案-终稿.DOCX"].into_iter()), REST_CHECKED | REST_DOC);
    assert_eq!(rest_flags_of(["x.vhdx"].into_iter()), REST_CHECKED | REST_VDISK);
    assert_eq!(rest_flags_of(["Database.kdbx", "id_rsa", "wallet.dat", "server.ppk"].into_iter()), REST_CHECKED | REST_SECRET);
    // cacert.pem（PyInstaller 解压的证书包）不算密钥
    assert_eq!(rest_flags_of(["cacert.pem", "id_rsa.pub"].into_iter()), REST_CHECKED);
}

/// 探针 R 节原样：60 个 64 KB 的 .tmp + 1 个 20 KB 的 docx，真实扫描，docx 被合并进“其余”。
#[test]
fn s1a_real_scan_docx_in_rest_is_not_safe() {
    let base = std::env::temp_dir().join(format!("diskwhy-r2-s1a-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let run = |with_doc: bool| {
        let home = base.join(if with_doc { "doc" } else { "nodoc" }).join("home");
        let home = home.to_string_lossy().to_string();
        let rtemp = format!(r"{home}\AppData\Local\Temp");
        std::fs::create_dir_all(&rtemp).unwrap();
        for i in 0..60 {
            std::fs::write(format!(r"{rtemp}\cache{i:02}.tmp"), vec![0u8; 64 * 1024]).unwrap();
        }
        if with_doc {
            std::fs::write(format!(r"{rtemp}\投标方案-终稿.docx"), vec![0u8; 20 * 1024]).unwrap();
        }
        std::fs::create_dir_all(format!(r"{rtemp}\sub")).unwrap();
        std::fs::write(format!(r"{rtemp}\sub\x.tmp"), vec![0u8; 4096]).unwrap();
        let c = ctx_with(env_home(&home, &rtemp));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let r = diskwhy_core::scan::scan(&home, &ScanOptions::default(), &cancel, |_| {}).expect("scan");
        (r, c, rtemp)
    };

    // 有 docx：docx 看不见（在 Rest 里），但 Rest 的标志位记下了“有文档”
    let (r, c, rtemp) = run(true);
    let (_, tnode) = find_node(&r.tree, &rtemp).unwrap();
    assert!(!tnode.children.iter().any(|x| x.name.ends_with(".docx")), "docx 应该被合并进其余（测试前提）");
    let rest = tnode.children.iter().find(|x| x.kind == NodeKind::Rest).expect("有 Rest");
    assert_eq!(rest.rest_flags, REST_CHECKED | REST_DOC);
    let e = ex(&r.tree, &c, &rtemp);
    assert_eq!(e.tier, Tier::Review, "Temp 不能再是 safe：{} / {}", src(&e), e.gain_note);
    assert!(e.hold.is_some(), "整块删要被扣下");
    assert!(e.gain_note.contains("另有 1 处"), "{}", e.gain_note);
    assert!(e.notes.iter().any(|n| n.contains("里面有文档")), "{:?}", e.notes);
    assert!(!batch_deletable(&e));
    assert!(delete_block_reason(&e).is_some());
    // Rest 本身
    let er = ex_rest(&r.tree, &c, &rtemp);
    assert_eq!(er.tier, Tier::Review, "{}", src(&er));
    assert!(er.hold.is_some());
    // 汇总：Rest 的字节记到了“需判断”，Temp 这一处在可清理清单里标“混着别的东西”
    let s = tier_summary(&r.tree, &c);
    assert_eq!(s.guarded_bytes, rest.allocated, "{:?}", s.guard_items.iter().map(|g| (&g.id, g.bytes)).collect::<Vec<_>>());
    assert!(s.guard_items.iter().any(|g| g.id == "guard:hidden-files" && g.tier == Tier::Review));
    let item = s.safe_items.iter().find(|i| i.path.eq_ignore_ascii_case(&rtemp)).expect("Temp 在清单里");
    assert!(item.mixed);
    assert_eq!(item.bytes, s.safe_bytes);

    // 对照：没有 docx 时 Temp 仍然 safe（Rest 标志位只有“看过”）
    let (r, c, rtemp) = run(false);
    let e = ex(&r.tree, &c, &rtemp);
    assert_eq!(e.tier, Tier::Safe, "{} / {:?}", src(&e), e.notes);
    assert!(e.hold.is_none());
    assert!(batch_deletable(&e));
    let s = tier_summary(&r.tree, &c);
    assert_eq!(s.guarded_bytes, 0);
    assert!(!s.safe_items.iter().find(|i| i.path.eq_ignore_ascii_case(&rtemp)).unwrap().mixed);

    let _ = std::fs::remove_dir_all(&base);
}

/// 没记名字的 Rest（旧快照 / 手工造的树）：按“看不到”处理。只标了“看过”的：放行。
#[test]
fn s1a_unchecked_rest_is_review() {
    let c = ctx_default();
    let tmp = format!(r"{U}\AppData\Local\Temp");
    for (flags, want) in [(0u8, Tier::Review), (REST_CHECKED, Tier::Safe), (REST_CHECKED | REST_SECRET, Tier::Review), (REST_CHECKED | REST_VDISK, Tier::Review)] {
        let files = vec![(format!(r"{tmp}\a.tmp"), 5_000_000u64), (format!(r"{tmp}\<rest:{flags}:30>"), 300_000)];
        let t = tree("C:", &files);
        let e = ex(&t, &c, &tmp);
        assert_eq!(e.tier, want, "flags={flags:#x} {} {:?}", src(&e), e.notes);
        let er = ex_rest(&t, &c, &tmp);
        let s = tier_summary(&t, &c);
        if want == Tier::Safe {
            assert_eq!(er.tier, Tier::Safe);
            assert_eq!(s.safe_bytes, 5_300_000);
        } else {
            assert_ne!(er.tier, Tier::Safe, "flags={flags:#x}");
            assert!(e.hold.is_some() && er.hold.is_some());
            assert_eq!(s.safe_bytes, 5_000_000, "flags={flags:#x}");
            assert_eq!(s.guarded_bytes, 300_000);
            let keepish = flags & (REST_SECRET | REST_VDISK) != 0;
            assert_eq!(er.tier, if keepish { Tier::Keep } else { Tier::Review }, "flags={flags:#x}");
        }
    }
}

/// 看得见的密钥 / 密码库文件（第 2 轮 S1 的“密钥类”）：Temp 不能 safe，文件本身 keep。
#[test]
fn s1a_visible_secret_files() {
    let c = ctx_default();
    let tmp = format!(r"{U}\AppData\Local\Temp");
    let files = vec![(format!(r"{tmp}\a.tmp"), 5_000_000u64), (format!(r"{tmp}\Database.kdbx"), 40_000), (format!(r"{tmp}\id_rsa"), 3_000)];
    let t = tree("C:", &files);
    let e = ex(&t, &c, &tmp);
    assert_eq!(e.tier, Tier::Review, "{}", src(&e));
    assert_eq!(e.gain_bytes, 5_000_000);
    for f in ["Database.kdbx", "id_rsa"] {
        let e = ex(&t, &c, &format!(r"{tmp}\{f}"));
        assert_eq!(e.tier, Tier::Keep, "{f}: {}", src(&e));
    }
    let s = tier_summary(&t, &c);
    assert_eq!(s.safe_bytes, 5_000_000);
    assert_eq!(s.keep_bytes, 43_000);
}

// ───────────────────────── S1-B：看不全不放行 ─────────────────────────

/// 探针 T 节：Temp 里 700×50 个小文件，深处一个 600 MB 的 vhdx 和一个 WeChat Files。
#[test]
fn s1b_truncated_search_demotes_safe() {
    let tmp = format!(r"{U}\AppData\Local\Temp");
    let mut files: Vec<(String, u64)> = Vec::new();
    for d in 0..700 {
        for f in 0..50 {
            files.push((format!(r"{tmp}\d{d:03}\f{f:02}.tmp"), 1_000));
        }
    }
    // 按大小排在最后（最后才会被看到）的一个很小的聊天数据
    files.push((format!(r"{tmp}\zz\WeChat Files\wxid_a\Msg\MicroMsg.db"), 1));
    let t = tree("C:", &files);

    // 上限调到 3 万（和评审探针一样）：看不全 → review，并扣下整块删
    let mut c = ctx_default();
    c.trap_visit = 30_000;
    let e = ex(&t, &c, &tmp);
    assert_ne!(e.tier, Tier::Safe, "{} {:?}", src(&e), e.notes);
    assert!(e.hold.is_some());
    assert!(!batch_deletable(&e));
    assert!(e.notes.iter().any(|n| n.contains("太多")), "{:?}", e.notes);

    // 小一点的子目录照样能看全：d000 是 safe
    let e0 = ex(&t, &c, &format!(r"{tmp}\d000"));
    assert_eq!(e0.tier, Tier::Safe);

    // 默认上限（100 万）：看得全，找到那处聊天数据 → review，能腾的扣掉它
    let c = ctx_default();
    let e = ex(&t, &c, &tmp);
    assert_eq!(e.tier, Tier::Review, "{} {:?}", src(&e), e.notes);
    assert_eq!(e.gain_bytes, 35_000_000);

    // 探针 T 原样：深处一个 600 MB 的 vhdx 和一个 WeChat Files（1 GB 的 keep 占一半以上 → 整个 Temp 按保留说明）
    files.push((format!(r"{tmp}\big\disk.vhdx"), 600_000_000));
    files.push((format!(r"{tmp}\big\backup\WeChat Files\wxid_a\Msg\MicroMsg.db"), 400_000_000));
    let t = tree("C:", &files);
    let e = ex(&t, &c, &tmp);
    assert_eq!(e.tier, Tier::Keep, "{} {:?}", src(&e), e.notes);
    assert!(delete_block_reason(&e).is_some());

    // 汇总和这一行一致：safe 只有那 3500 万字节的 .tmp，1 GB 是保留
    let s = tier_summary(&t, &c);
    assert_eq!(s.safe_bytes, 35_000_000);
    assert_eq!(s.keep_bytes, 1_000_000_001);
    let item = s.safe_items.iter().find(|i| i.path.eq_ignore_ascii_case(&tmp)).unwrap();
    assert!(item.mixed);
}

// ───────────────────────── S2：下层 review 规则 ─────────────────────────

/// 任务点名的例子：Temp 里只有 Rar$EX + .psd（不是 Office / PDF）和普通临时文件，没有别的陷阱。
#[test]
fn s2_temp_with_only_rar_psd() {
    let c = ctx_default();
    let tmp = format!(r"{U}\AppData\Local\Temp");
    let rar = format!(r"{tmp}\Rar$EXa77.888");
    let files = vec![(format!(r"{rar}\设计稿-改.psd"), 30_000_000u64), (format!(r"{tmp}\abc.tmp"), 10_000_000)];
    let t = tree("C:", &files);

    let er = ex(&t, &c, &rar);
    assert_eq!(er.tier, Tier::Review);
    assert!(matches!(&er.source, Source::Rule { id, .. } if id == "temp-opened-from-archive"), "{}", src(&er));

    let e = ex(&t, &c, &tmp);
    assert_eq!(e.tier, Tier::Review, "Temp 不能再整块 safe：{} / {}", src(&e), e.gain_note);
    assert_eq!(e.gain_bytes, 10_000_000, "能腾多少要扣掉 Rar$EX：{}", e.gain_note);
    assert!(e.gain_note.contains("另有 1 处"), "{}", e.gain_note);
    assert!(e.notes.iter().any(|n| n.contains("Rar$EXa77.888")), "要列出路径：{:?}", e.notes);
    assert!(e.hold.is_some());
    assert!(!batch_deletable(&e));
    assert!(delete_block_reason(&e).is_some());
    // 普通临时文件仍然 safe、能批量删
    let ef = ex(&t, &c, &format!(r"{tmp}\abc.tmp"));
    assert_eq!(ef.tier, Tier::Safe);
    assert!(batch_deletable(&ef));

    let s = tier_summary(&t, &c);
    assert_eq!(s.safe_bytes, 10_000_000);
    assert_eq!(s.review_bytes, 30_000_000);
    let item = s.safe_items.iter().find(|i| i.path.eq_ignore_ascii_case(&tmp)).unwrap();
    assert!(item.mixed);
    assert_eq!(item.bytes, 10_000_000);
}

/// 探针 N 节原样（Temp 和 INetCache 两处嵌套）。
#[test]
fn s2_probe_n_nested_rules() {
    let c = ctx_default();
    let la = format!(r"{U}\AppData\Local");
    let tmp = format!(r"{la}\Temp");
    let inet = format!(r"{la}\Microsoft\Windows\INetCache");
    let files = vec![
        (format!(r"{tmp}\Rar$EXa77.888\设计稿-改.psd"), 30_000_000u64),
        (format!(r"{tmp}\Temp1_合同扫描.zip\扫描件.jpg"), 2_000_000),
        (format!(r"{tmp}\junk.tmp"), 5_000_000),
        (format!(r"{tmp}\Temp12_旅行照片.zip\a.jpg"), 3_000_000),
        (format!(r"{inet}\Content.Outlook\ZX81QW\现场照片.jpg"), 9_000_000),
        (format!(r"{inet}\Content.Outlook\ZX81QW\报价附件.zip"), 1_000_000),
        (format!(r"{inet}\IE\ABCD1234\x.dat"), 100_000),
    ];
    let t = tree("C:", &files);
    let e = ex(&t, &c, &tmp);
    assert_eq!(e.tier, Tier::Review, "{}", src(&e));
    // Temp12_… 两位数编号的规则没覆盖（评审 G5，一般项，这一轮没改），仍按临时文件算
    assert_eq!(e.gain_bytes, 5_000_000 + 3_000_000, "{}", e.gain_note);
    let ei = ex(&t, &c, &inet);
    assert_eq!(ei.tier, Tier::Review, "{}", src(&ei));
    assert_eq!(ei.gain_bytes, 100_000, "{}", ei.gain_note);
    assert!(ei.hold.is_some());
    let s = tier_summary(&t, &c);
    assert_eq!(s.safe_bytes, 8_100_000);
    assert_eq!(s.review_bytes, 42_000_000);
    for p in [&tmp, &inet] {
        assert!(s.safe_items.iter().find(|i| i.path.eq_ignore_ascii_case(p)).unwrap().mixed, "{p}");
    }
}

// ───────────────────────── 删除闸门（给阶段 D 合并用） ─────────────────────────

/// 第 1 轮 `safety_trap_corpus` 的 C 盘陷阱（照抄）+ 第 2 轮的 R / N 类陷阱。
fn corpus() -> (Vec<(String, u64)>, Vec<String>, Vec<String>) {
    let la = format!(r"{U}\AppData\Local");
    let ra = format!(r"{U}\AppData\Roaming");
    let tmp = format!(r"{la}\Temp");
    let files: Vec<(String, u64)> = vec![
        (format!(r"{tmp}\x\WeChat Files\wxid_abc\Msg\MicroMsg.db"), 5_000_000),
        (format!(r"{tmp}\x\WeChat Files\wxid_abc\FileStorage\File\合同.pdf"), 900_000),
        (format!(r"{tmp}\clone\myproj\.git\HEAD"), 100),
        (format!(r"{tmp}\clone\myproj\src\main.rs"), 9_000),
        (format!(r"{tmp}\conda\envs\py311\conda-meta\history"), 100),
        (format!(r"{tmp}\conda\envs\py311\python.exe"), 90_000),
        (format!(r"{tmp}\Rar$EXa1234.5678\毕业论文终稿.docx"), 2_000_000),
        (format!(r"{tmp}\Rar$EXa77.888\设计稿-改.psd"), 30_000_000),
        (format!(r"{tmp}\report-final.xlsx"), 70_000),
        (format!(r"{tmp}\vm\win11.vhdx"), 3_000_000),
        (format!(r"{tmp}\keys\id_ed25519"), 400),
        (format!(r"{tmp}\many\<rest:129:40>"), 400_000),
        (format!(r"{tmp}\many\a.tmp"), 1_000_000),
        (format!(r"{la}\Microsoft\Windows\INetCache\Content.Outlook\AB12CD34\报价单(修改).xlsx"), 80_000),
        (format!(r"{la}\Microsoft\Windows\INetCache\Content.Outlook\AB12CD34\现场照片.jpg"), 800_000),
        (format!(r"{la}\Microsoft\Windows\INetCache\IE\X1\a.dat"), 100_000),
        (format!(r"{ra}\SomeApp\Cache\data_0"), 8_000),
        (format!(r"{ra}\SomeApp\Cache\my_notes.txt"), 3_000),
        (format!(r"{la}\Packages\CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc\LocalState\ext4.vhdx"), 60_000_000),
        // 真正的临时文件 / 缓存
        (format!(r"{tmp}\abc123.tmp"), 20_000_000),
        (format!(r"{tmp}\pip-unpack-1\wheel.whl"), 1_000_000),
        (format!(r"{ra}\RealApp\Cache\Cache_Data\index"), 1_000),
        (format!(r"{ra}\RealApp\Cache\Cache_Data\data_0"), 500_000),
        (format!(r"{ra}\RealApp\Cache\Cache_Data\f_00001a"), 500_000),
    ];
    let traps: Vec<String> = vec![
        format!(r"{tmp}\x\WeChat Files"),
        format!(r"{tmp}\clone\myproj"),
        format!(r"{tmp}\conda\envs\py311"),
        format!(r"{tmp}\Rar$EXa1234.5678"),
        format!(r"{tmp}\Rar$EXa77.888"),
        format!(r"{tmp}\report-final.xlsx"),
        format!(r"{tmp}\vm"),
        format!(r"{tmp}\keys\id_ed25519"),
        format!(r"{tmp}\many\*rest*"),
        format!(r"{la}\Microsoft\Windows\INetCache\Content.Outlook"),
        format!(r"{ra}\SomeApp\Cache"),
        format!(r"{la}\Packages\CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc\LocalState"),
    ];
    let clean: Vec<String> = vec![format!(r"{tmp}\abc123.tmp"), format!(r"{tmp}\pip-unpack-1"), format!(r"{ra}\RealApp\Cache\Cache_Data"), format!(r"{la}\Microsoft\Windows\INetCache\IE")];
    (files, traps, clean)
}

fn is_prefix(dir: &str, p: &str) -> bool {
    let d = dir.trim_end_matches('\\').to_lowercase();
    let p = p.to_lowercase();
    p == d || p.starts_with(&format!("{d}\\"))
}

/// 树里每一项（文件夹、文件、其余小文件合计）：只要整块删会碰到陷阱（它就是陷阱、在陷阱里面、或者陷阱在它下面），
/// `delete_block_reason` 就必须给出原因，`batch_deletable` 必须是 false。真正的缓存 / 临时文件照常能批量删。
#[test]
fn delete_gate_blocks_every_item_touching_a_trap() {
    let (files, traps, clean) = corpus();
    let t = tree("C:", &files);
    let c = ctx_default();
    let mut checked = 0;
    let mut stack: Vec<(Vec<&Node>, &Node)> = vec![(Vec::new(), &t)];
    while let Some((chain, n)) = stack.pop() {
        for ch in &n.children {
            let path = match ch.kind {
                NodeKind::Dir => ch.path.clone(),
                NodeKind::File => format!("{}\\{}", n.path.trim_end_matches('\\'), ch.name),
                NodeKind::Rest => n.path.clone(),
            };
            // 陷阱清单里 Rest 写成“目录\*rest*”
            let touch_path = if ch.kind == NodeKind::Rest { format!(r"{path}\*rest*") } else { path.clone() };
            let mut anc = chain.clone();
            anc.push(n);
            let target = Target { node: ch, path: &path, parent: Some(n), ancestors: &anc };
            let e = explain(&target, &c);
            let touches = traps.iter().any(|tr| is_prefix(&touch_path, tr) || is_prefix(tr, &touch_path));
            if touches {
                assert!(
                    delete_block_reason(&e).is_some() && !batch_deletable(&e),
                    "{path}（{:?}）碰到了陷阱却没被拦下：{:?} {} hold={:?}",
                    ch.kind,
                    e.tier,
                    src(&e),
                    e.hold
                );
                checked += 1;
            }
            if ch.kind == NodeKind::Dir {
                stack.push((anc, ch));
            }
        }
    }
    assert!(checked > 40, "至少核对了 {checked} 项");
    for p in &clean {
        let e = ex(&t, &c, p);
        assert!(batch_deletable(&e), "{p} 应该能批量删：{:?} {} {:?}", e.tier, src(&e), e.hold);
    }
}

/// TEMP=D:\ 时（第 1 轮严重问题 2 的场景）D 盘上的东西一律不能整块删。
#[test]
fn delete_gate_temp_at_drive_root() {
    let c = ctx_with(env_home(U, r"D:\"));
    let files = vec![
        (r"D:\WeChat Files\wxid_a\Msg\MicroMsg.db".to_string(), 5_000_000u64),
        (r"D:\vm\ubuntu.vhdx".to_string(), 9_000_000),
        (r"D:\proj\.git\HEAD".to_string(), 100),
        (r"D:\资料\年终总结.docx".to_string(), 300_000),
        (r"D:\a.tmp".to_string(), 1_000),
    ];
    let t = tree("D:", &files);
    for p in [r"D:\WeChat Files", r"D:\vm", r"D:\proj", r"D:\资料"] {
        let e = ex(&t, &c, p);
        assert!(!batch_deletable(&e), "{p}：{:?} {}", e.tier, src(&e));
        assert!(delete_block_reason(&e).is_some() || e.tier != Tier::Safe, "{p}");
    }
    assert_eq!(tier_summary(&t, &c).safe_bytes, 0);
}

/// 包管理器缓存里包自带的 PDF、测试证书不算“你的文件”（真实 C 盘上 cargo registry、uv cache 被误判过，决定 70）；
/// 但虚拟磁盘、聊天数据照样拦。Temp 不在这个名单里，照查。
#[test]
fn package_caches_ignore_bundled_docs_and_test_keys() {
    let c = ctx_default();
    let cargo = format!(r"{U}\.cargo\registry");
    let uv = format!(r"{U}\AppData\Local\uv\cache");
    let npm = format!(r"{U}\AppData\Local\npm-cache");
    let files = vec![
        (format!(r"{cargo}\src\index.crates.io-x\openssl-0.10.81\test\identity.p12"), 4_000u64),
        (format!(r"{cargo}\src\index.crates.io-x\openssl-0.10.81\src\lib.rs"), 40_000),
        (format!(r"{uv}\archive-v0\abc\matplotlib\mpl-data\images\hand.pdf"), 8_000),
        (format!(r"{uv}\archive-v0\abc\matplotlib\<rest:129:80>"), 90_000),
        (format!(r"{uv}\archive-v0\abc\matplotlib\__init__.py"), 50_000),
        (format!(r"{npm}\_cacache\content-v2\x\data"), 60_000),
        // uv 放的空 .git 文件（让 git 别把缓存当仓库），不是代码仓库
        (format!(r"{uv}\sdists-v9\.git"), 0),
        (format!(r"{uv}\sdists-v9\.gitignore"), 0),
        (format!(r"{npm}\vm\disk.vhdx"), 1_000),
    ];
    let t = tree("C:", &files);
    assert_eq!(ex(&t, &c, &cargo).tier, Tier::Safe, "{:?}", ex(&t, &c, &cargo).notes);
    assert_eq!(ex(&t, &c, &uv).tier, Tier::Safe, "{:?}", ex(&t, &c, &uv).notes);
    let en = ex(&t, &c, &npm);
    assert_eq!(en.tier, Tier::Review, "npm-cache 里的虚拟磁盘仍要拦：{}", src(&en));
    let s = tier_summary(&t, &c);
    assert_eq!(s.safe_bytes, 4_000 + 40_000 + 8_000 + 90_000 + 50_000 + 60_000);
    assert_eq!(s.keep_bytes, 1_000);
    for p in [&cargo, &uv] {
        assert!(!s.safe_items.iter().find(|i| i.path.eq_ignore_ascii_case(p)).unwrap().mixed, "{p}");
    }
}
