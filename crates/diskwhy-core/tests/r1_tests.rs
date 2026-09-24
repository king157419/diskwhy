//! 第 1 轮评审（安全 / 产品 / 冷启动）之后补的测试。
//!
//! 核心是 `safety_trap_corpus`：把安全评审探针（`_ops/S22/r1-safety-probe`）A、C 两节里的“用户数据陷阱”
//! 原样搬进来，用**内置规则 + 继承**逐个解释，断言没有一个是 safe，并且 `tier_summary` 的 safe 字节里
//! 一个字节的陷阱都没有。TEMP 取默认值、`D:\`、`C:\Users\bob`、`C:\` 四种。

use diskwhy_core::explain::env::EnvMap;
use diskwhy_core::explain::llm::LlmSettings;
use diskwhy_core::explain::registry::RegistrySnapshot;
use diskwhy_core::explain::rules::RuleSet;
use diskwhy_core::explain::{tier_summary, ExplainCtx, Explanation, Source, Tier};
use diskwhy_core::view::explain_path;
use diskwhy_core::*;
use std::collections::BTreeMap;

const U: &str = r"C:\Users\bob";

fn env_with(temp: &str) -> EnvMap {
    EnvMap::from_pairs(&[
        ("SystemDrive", "C:"),
        ("SystemRoot", r"C:\Windows"),
        ("WINDIR", r"C:\Windows"),
        ("ProgramFiles", r"C:\Program Files"),
        ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
        ("ProgramData", r"C:\ProgramData"),
        ("USERPROFILE", U),
        ("LOCALAPPDATA", r"C:\Users\bob\AppData\Local"),
        ("APPDATA", r"C:\Users\bob\AppData\Roaming"),
        ("TEMP", temp),
        ("PUBLIC", r"C:\Users\Public"),
    ])
}

fn ctx(temp: &str) -> (ExplainCtx, Vec<String>) {
    let env = env_with(temp);
    let (rules, errs) = RuleSet::embedded(&env);
    (ExplainCtx::new(env, rules, RegistrySnapshot::from_apps(Vec::new()), LlmSettings::default()), errs)
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
        files: 1,
        dirs: 0,
        children: vec![],
    }
}

/// 内存里的树：文件路径 → 大小（和探针一样的造法）。
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
            children.push(leaf(n, *sz));
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

/// C 盘上的陷阱（安全评审 A 节 + 更多）。返回 (树里的文件, 每个陷阱的“根”路径和它的字节数)。
fn c_drive() -> (Vec<(String, u64)>, Vec<(String, u64)>) {
    let la = format!(r"{U}\AppData\Local");
    let ra = format!(r"{U}\AppData\Roaming");
    let tmp = format!(r"{la}\Temp");
    let files: Vec<(String, u64)> = vec![
        // 陷阱
        (format!(r"{tmp}\x\WeChat Files\wxid_abc\Msg\MicroMsg.db"), 5_000_000),
        (format!(r"{tmp}\x\WeChat Files\wxid_abc\FileStorage\File\合同.pdf"), 900_000),
        (format!(r"{tmp}\clone\myproj\.git\HEAD"), 100),
        (format!(r"{tmp}\clone\myproj\src\main.rs"), 9_000),
        (format!(r"{tmp}\clone\myproj\Cargo.toml"), 300),
        (format!(r"{tmp}\conda\envs\py311\conda-meta\history"), 100),
        (format!(r"{tmp}\conda\envs\py311\python.exe"), 90_000),
        (format!(r"{tmp}\Rar$EXa1234.5678\毕业论文终稿.docx"), 2_000_000),
        (format!(r"{tmp}\report-final.xlsx"), 70_000),
        (format!(r"{tmp}\vm\win11.vhdx"), 3_000_000),
        (format!(r"{la}\Microsoft\Windows\INetCache\Content.Outlook\AB12CD34\报价单(修改).xlsx"), 80_000),
        (format!(r"{ra}\SomeApp\Cache\data_0"), 8_000),
        (format!(r"{ra}\SomeApp\Cache\my_notes.txt"), 3_000),
        (format!(r"{ra}\SomePWA\Service Worker\CacheStorage\abc\0"), 700_000),
        (format!(r"{la}\JetBrains\IntelliJIdea2024.1\caches\content.dat"), 700_000),
        (format!(r"{la}\JetBrains\IntelliJIdea2024.1\LocalHistory\changes.storageData"), 700_000),
        (format!(r"{ra}\Tencent\xwechat\radium\users\abc\applet\x.wxapkg"), 700_000),
        (format!(r"{la}\Packages\Foo_123\LocalCache\Local\Temp\notes\todo.docx"), 7_000),
        (format!(r"{la}\Packages\CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc\LocalState\ext4.vhdx"), 60_000_000),
        // 真正的临时文件 / 缓存（应该仍然 safe）
        (format!(r"{tmp}\abc123.tmp"), 20_000_000),
        (format!(r"{tmp}\pip-unpack-1\wheel.whl"), 1_000_000),
        (format!(r"{ra}\RealApp\Cache\Cache_Data\index"), 1_000),
        (format!(r"{ra}\RealApp\Cache\Cache_Data\data_0"), 500_000),
        (format!(r"{ra}\RealApp\Cache\Cache_Data\f_00001a"), 500_000),
        // 普通的 keep
        (format!(r"{U}\Documents\WeChat Files\wxid_abc\Msg\MicroMsg.db"), 5_000_000),
        (format!(r"{U}\.ssh\id_ed25519"), 400),
        (format!(r"{U}\source\repos\App1\Program.cs"), 400),
        (format!(r"{ra}\Telegram Desktop\tdata\key_datas"), 400),
    ];
    let traps: Vec<(String, u64)> = vec![
        (format!(r"{tmp}\x\WeChat Files"), 5_900_000),
        (format!(r"{tmp}\clone\myproj"), 9_400),
        (format!(r"{tmp}\conda\envs\py311"), 90_100),
        (format!(r"{tmp}\Rar$EXa1234.5678"), 2_000_000),
        (format!(r"{tmp}\report-final.xlsx"), 70_000),
        (format!(r"{tmp}\vm"), 3_000_000),
        (format!(r"{la}\Microsoft\Windows\INetCache\Content.Outlook"), 80_000),
        (format!(r"{ra}\SomeApp\Cache"), 11_000),
        (format!(r"{ra}\SomePWA\Service Worker\CacheStorage"), 700_000),
        (format!(r"{la}\JetBrains\IntelliJIdea2024.1\caches"), 700_000),
        (format!(r"{ra}\Tencent\xwechat\radium"), 700_000),
        (format!(r"{la}\Packages\Foo_123\LocalCache\Local\Temp\notes"), 7_000),
        (format!(r"{la}\Packages\CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc\LocalState"), 60_000_000),
    ];
    (files, traps)
}

/// 陷阱本身、它下面的每一层、它上面到规则根的每一层，都不能是 safe。
fn assert_no_safe_trap(t: &Node, c: &ExplainCtx, traps: &[(String, u64)], label: &str) {
    for (p, _) in traps {
        let e = ex(t, c, p);
        assert_ne!(e.tier, Tier::Safe, "[{label}] 陷阱 {p} 被标成了 safe（{}）", src(&e));
        // 往下：陷阱里的每个文件夹
        let (_, node) = diskwhy_core::view::find_node(t, p).unwrap();
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            for ch in n.children.iter().filter(|x| x.kind == NodeKind::Dir) {
                let e = ex(t, c, &ch.path);
                assert_ne!(e.tier, Tier::Safe, "[{label}] 陷阱里面的 {} 被标成了 safe（{}）", ch.path, src(&e));
                stack.push(ch);
            }
        }
    }
}

#[test]
fn safety_trap_corpus() {
    let (files, traps) = c_drive();
    let t = tree("C:", &files);
    let la = format!(r"{U}\AppData\Local");
    let tmp = format!(r"{la}\Temp");

    // 1. 默认 TEMP：逐个解释。
    let (c, errs) = ctx(&tmp);
    assert!(errs.is_empty(), "{errs:#?}");
    assert_no_safe_trap(&t, &c, &traps, "默认 TEMP");

    // 具体档位：keep 级的陷阱是 keep，其余 review。
    let tier = |p: &str| ex(&t, &c, p).tier;
    assert_eq!(tier(&format!(r"{tmp}\x\WeChat Files")), Tier::Keep);
    assert_eq!(tier(&format!(r"{tmp}\x\WeChat Files\wxid_abc\Msg")), Tier::Keep);
    assert_eq!(tier(&format!(r"{tmp}\conda\envs\py311")), Tier::Keep);
    assert_eq!(tier(&format!(r"{tmp}\vm")), Tier::Keep, "文件夹里大部分是虚拟磁盘");
    assert_eq!(tier(&format!(r"{tmp}\vm\win11.vhdx")), Tier::Keep);
    assert_eq!(tier(&format!(r"{tmp}\clone\myproj")), Tier::Review);
    assert_eq!(tier(&format!(r"{tmp}\clone\myproj\src")), Tier::Review, "在代码仓库里面");
    assert_eq!(tier(&format!(r"{tmp}\clone")), Tier::Review, "下面有代码仓库");
    assert_eq!(tier(&format!(r"{tmp}\report-final.xlsx")), Tier::Review);
    let rar = ex(&t, &c, &format!(r"{tmp}\Rar$EXa1234.5678"));
    assert!(matches!(&rar.source, Source::Rule { id, .. } if id == "temp-opened-from-archive"), "{}", src(&rar));
    assert!(rar.lose.contains("只在这里"));
    let outlook = ex(&t, &c, &format!(r"{la}\Microsoft\Windows\INetCache\Content.Outlook\AB12CD34"));
    assert!(matches!(&outlook.source, Source::Rule { id, .. } if id == "outlook-attachment-temp"), "{}", src(&outlook));
    assert_eq!(outlook.tier, Tier::Review);
    let wsl = ex(&t, &c, &format!(r"{la}\Packages\CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc\LocalState"));
    assert_eq!(wsl.tier, Tier::Keep);
    assert!(wsl.how_detail.contains("别用“重置”"), "{}", wsl.how_detail);
    let generic = ex(&t, &c, &format!(r"{U}\AppData\Roaming\SomeApp\Cache"));
    assert!(matches!(&generic.source, Source::Rule { id, .. } if id == "chromium-generic-cache"));
    assert_eq!(generic.tier, Tier::Review, "混着 my_notes.txt，严格核验不过");
    assert!(generic.notes.iter().any(|n| n.contains("标准格式")), "{:?}", generic.notes);

    // Temp 本身：里面混着用户数据 → review，能腾多少分开写。
    let temp = ex(&t, &c, &tmp);
    assert_eq!(temp.tier, Tier::Review);
    assert!(temp.gain_note.contains("可以清理") && temp.gain_note.contains("看起来是你的数据"), "{}", temp.gain_note);
    assert!(temp.gain_bytes > 0 && temp.gain_bytes < t_alloc(&t, &tmp));

    // 真正的缓存仍然 safe（护栏不能把一切都变成 review）。
    assert_eq!(tier(&format!(r"{tmp}\pip-unpack-1")), Tier::Safe);
    assert_eq!(tier(&format!(r"{tmp}\abc123.tmp")), Tier::Safe);
    assert_eq!(tier(&format!(r"{U}\AppData\Roaming\RealApp\Cache")), Tier::Safe);

    // 2. 汇总：safe 字节里没有任何陷阱。
    let s = tier_summary(&t, &c);
    let trap_bytes: u64 = traps.iter().map(|x| x.1).sum();
    let honest_safe: u64 = 20_000_000 + 1_000_000 + 1_001_000;
    assert_eq!(s.safe_bytes, honest_safe, "safe 只应包含真正的临时文件和缓存：{s:#?}");
    assert!(s.guarded_bytes > 0 && s.guarded_bytes <= trap_bytes, "{s:#?}");
    assert!(s.top_keep.iter().chain(&s.top_review).any(|r| r.id.starts_with("guard:")), "护栏改记的部分要在列表里：{s:#?}");

    // 3. TEMP 被改到奇怪的位置：user-temp 的 %TEMP% 模式要被跳过（带警告），D 盘上的东西不能因此变 safe。
    let d_files: Vec<(String, u64)> = vec![
        (r"D:\Weixin\xwechat_files\wxid_abc_1234\db_storage\message\message_0.db".into(), 5_000_000),
        (r"D:\VMs\ubuntu\disk.vhdx".into(), 90_000_000),
        (r"D:\conda\envs\d2l\conda-meta\history".into(), 100),
        (r"D:\work\notes.txt".into(), 100),
    ];
    let d = tree("D:", &d_files);
    for temp in [r"D:\", r"C:\Users\bob", r"C:\", r"C:\Users\bob\AppData\Local"] {
        let (c2, warns) = ctx(temp);
        assert!(
            warns.iter().any(|w| w.contains("user-temp") && w.contains("%TEMP%")),
            "TEMP={temp} 时应警告并跳过 %TEMP% 模式：{warns:#?}"
        );
        for p in [r"D:\Weixin\xwechat_files", r"D:\VMs\ubuntu", r"D:\conda\envs\d2l", r"D:\work", r"D:\"] {
            let e = ex(&d, &c2, p);
            assert_ne!(e.tier, Tier::Safe, "TEMP={temp}：{p} 被标成了 safe（{}）", src(&e));
        }
        let sd = tier_summary(&d, &c2);
        assert_eq!(sd.safe_bytes, 0, "TEMP={temp}：D 盘 safe 应为 0：{sd:#?}");
        for p in [format!(r"{U}\.ssh"), format!(r"{U}\source\repos\App1"), format!(r"{U}\AppData\Roaming\Telegram Desktop\tdata")] {
            assert_ne!(ex(&t, &c2, &p).tier, Tier::Safe, "TEMP={temp}：{p}");
        }
        // 兜底的 %LOCALAPPDATA%\Temp 仍然有效
        assert!(matches!(&ex(&t, &c2, &format!(r"{tmp}\pip-unpack-1")).source, Source::Rule { id, .. } if id == "user-temp"));
        // 陷阱在这种环境下同样不是 safe
        assert_no_safe_trap(&t, &c2, &traps, temp);
    }
}

fn t_alloc(t: &Node, p: &str) -> u64 {
    diskwhy_core::view::find_node(t, p).unwrap().1.allocated
}

/// 静态校验：内置规则里每条 safe 都有出处或标了本机观察；没有 ** 却不带 when_child 的非缓存名。
#[test]
fn safe_rules_have_provenance() {
    let (c, errs) = ctx(r"C:\Users\bob\AppData\Local\Temp");
    assert!(errs.is_empty(), "{errs:#?}");
    let mut observed = Vec::new();
    for r in c.rules.rules.iter().filter(|r| r.tier == Tier::Safe) {
        assert!(!r.refs.is_empty() || r.observed_only, "safe 规则 {} 没有出处", r.id);
        if r.refs.is_empty() {
            observed.push(r.id.as_str());
        }
    }
    // 只在本机观察过的 safe 规则应该是少数，并且名单固定（新增时要在这里显式登记）。
    observed.sort();
    assert_eq!(
        observed,
        vec![
            "directx-shader-cache",
            "docker-logs",
            "feishu-update",
            "gpu-shader-cache",
            "netease-music-cache",
            "netease-music-logs",
            "squirrel-temp",
            "steam-caches",
            "tencent-logs",
            "vscode-family-cached",
            "wechat-logs",
            "wechat-update",
        ]
    );
}

/// 第 1 轮安全评审第 9、16 条：发给 AI 的东西再收紧；只有认不出的项能问；远程地址必须 https。
#[test]
fn llm_redaction_and_gates() {
    use diskwhy_core::explain::llm;
    use diskwhy_core::explain::Target;
    use diskwhy_core::view::find_node;
    // 评审探针 E 节的原样例子
    let zs = tree(
        "C:",
        &[
            (r"C:\Users\张三\AppData\Local\Mystery\张三的简历.docx".into(), 9000),
            (r"C:\Users\张三\AppData\Local\Mystery\13812345678-身份证正反面.jpg".into(), 8000),
            (r"C:\Users\张三\AppData\Local\Mystery\wxid_q1w2e3r4t5\a".into(), 7000),
            (r"C:\Users\张三\AppData\Local\Mystery\zhangsan@company.com.ost".into(), 6000),
            (r"C:\Users\张三\AppData\Local\Mystery\id_rsa".into(), 500),
            (r"C:\Users\张三\AppData\Local\Mystery\张三备份\x".into(), 500),
            (r"C:\Users\张三\AppData\Local\Mystery\123456789\y".into(), 500),
        ],
    );
    let (chain, node) = find_node(&zs, r"C:\Users\张三\AppData\Local\Mystery").unwrap();
    let tg = Target { node, path: &node.path, parent: chain.last().copied(), ancestors: &chain };
    let inp = llm::build_input_with(&tg, &["张三".to_string()]);
    let body = serde_json::to_string(&inp).unwrap();
    for leak in ["张三", "简历", "13812345678", "身份证", "q1w2e3r4t5", "zhangsan", "company", "id_rsa", "123456789"] {
        assert!(!body.contains(leak), "请求里还有「{leak}」：{body}");
    }
    assert!(inp.children.contains(&r"wxid_<隐藏>\".to_string()), "{:?}", inp.children);
    assert!(inp.children.contains(&r"~备份\".to_string()), "两个字的中文用户名也要换掉：{:?}", inp.children);
    assert!(inp.children.contains(&r"<数字>\".to_string()), "{:?}", inp.children);
    assert!(inp.children.iter().any(|c| c.starts_with(".docx 文件 × 1")), "{:?}", inp.children);
    assert_eq!(inp.parent, r"…\AppData\Local");
    // 非 Users 路径：只留最近两层
    let dd = tree("D:", &[(r"D:\李四-个人资料\某某科技有限公司\离职材料\x\a.bin".into(), 100)]);
    let (chain, node) = find_node(&dd, r"D:\李四-个人资料\某某科技有限公司\离职材料\x").unwrap();
    let tg = Target { node, path: &node.path, parent: chain.last().copied(), ancestors: &chain };
    let i2 = llm::build_input_with(&tg, &["bob".to_string()]);
    assert_eq!(i2.parent, r"…\某某科技有限公司\离职材料");
    assert!(!serde_json::to_string(&i2).unwrap().contains("李四"));
    assert_eq!(llm::mask_sensitive("a@b.co"), "<邮箱>");
    assert_eq!(llm::mask_sensitive("QQ_1234"), "QQ_1234", "4 位以内的数字不打码");

    // 地址：远程 http 拒绝，本机 http / 任意 https 放行
    assert!(llm::check_url("http://api.example.com/v1/chat/completions").is_err());
    assert!(llm::check_url("http://192.168.1.5:11434/v1/chat/completions").is_err());
    for ok in ["https://api.example.com/v1", "http://localhost:11434/v1", "http://127.0.0.1:9/v1", "http://[::1]:8080/v1"] {
        assert!(llm::check_url(ok).is_ok(), "{ok}");
    }
    assert!(llm::check_url("ftp://x").is_err());

    // 只有认不出的项能问：规则认得的（Temp）、启发式认得的（聊天数据）都拒绝
    let (c, _) = ctx(r"C:\Users\bob\AppData\Local\Temp");
    let (files, _) = c_drive();
    let t = tree("C:", &files);
    for p in [r"C:\Users\bob\AppData\Local\Temp\abc123.tmp", r"C:\Users\bob\Documents\WeChat Files"] {
        let (chain, node) = find_node(&t, p).unwrap();
        let tg = Target { node, path: p, parent: chain.last().copied(), ancestors: &chain };
        assert!(llm::ensure_askable(&tg, &c).is_err(), "{p} 不该能问 AI");
    }
    let (chain, node) = find_node(&t, r"C:\Users\bob\.ssh").unwrap();
    let tg = Target { node, path: r"C:\Users\bob\.ssh", parent: chain.last().copied(), ancestors: &chain };
    let e = diskwhy_core::explain::explain(&tg, &c);
    assert_eq!(llm::ensure_askable(&tg, &c).is_ok(), e.is_unknown());
}

/// 第 1 轮安全评审第 10 条：“文档”被移到 D 盘后，safe 规则 / 启发式落在那里也要降级。
#[test]
fn known_folders_are_protected() {
    // 真实系统：至少读到“文档”和“下载”，而且都是绝对路径、不带结尾反斜杠。
    let real = diskwhy_core::win::known_personal_folders();
    assert!(real.len() >= 2, "{real:?}");
    assert!(real.iter().all(|p| p.len() > 3 && p.chars().nth(1) == Some(':') && !p.ends_with('\\')), "{real:?}");
    // 假环境：文档在 D:\我的文档
    let (mut c, _) = ctx(r"C:\Users\bob\AppData\Local\Temp");
    let segs = diskwhy_core::explain::pattern::split_lower(r"D:\我的文档\Chrome\Cache");
    assert!(!c.is_protected(&segs));
    c.add_protected(&[r"D:\我的文档".to_string()]);
    assert!(c.is_protected(&segs));
    // 放在那里的 Chromium 缓存（签名严格）也只能是 review
    let files: Vec<(String, u64)> = vec![
        (r"D:\我的文档\Chrome\Cache\Cache_Data\index".into(), 1_000),
        (r"D:\我的文档\Chrome\Cache\Cache_Data\data_0".into(), 1_000),
    ];
    let t = tree("D:", &files);
    let e = ex(&t, &c, r"D:\我的文档\Chrome\Cache");
    assert_eq!(e.tier, Tier::Review, "{}", src(&e));
}

/// 第 1 轮冷启动评审：链接显示“不占空间，不用管”；0 字节的不进“未知”；未知项只说一次、给结论、替用户总结线索。
#[test]
fn links_empty_and_unknown_wording() {
    let (c, _) = ctx(r"C:\Users\bob\AppData\Local\Temp");
    let mut t = tree(
        "C:",
        &[
            (r"C:\Documents and Settings\x".into(), 0),
            (r"C:\Users\bob\AppData\Local\EmptyThing\x".into(), 0),
            (r"C:\Users\bob\AppData\Local\OpenAI\Codex\bin\codex.exe".into(), 9_000_000),
            (r"C:\Users\bob\AppData\Local\OpenAI\Codex\bin\helper.dll".into(), 1_000_000),
            (r"C:\Users\bob\AppData\Local\OpenAI\Codex\config.json".into(), 1_000),
        ],
    );
    // 把 Documents and Settings 标成没跟进去的链接（扫描器对重解析点就是这样记的）
    let link = t.children.iter_mut().find(|n| n.name == "Documents and Settings").unwrap();
    link.reparse_skipped = true;
    link.children.clear();
    link.allocated = 0;
    let e = ex(&t, &c, r"C:\Documents and Settings");
    assert!(matches!(&e.source, Source::Heuristic { name } if name == "reparse-link"), "{}", src(&e));
    assert!(e.what.contains("不占空间") && e.gain_bytes == 0);
    let e = ex(&t, &c, r"C:\Users\bob\AppData\Local\EmptyThing");
    assert!(matches!(&e.source, Source::Heuristic { name } if name == "empty"), "{}", src(&e));
    assert!(e.what.contains("不占空间"));
    let e = ex(&t, &c, r"C:\Users\bob\AppData\Local\OpenAI");
    assert!(e.is_unknown());
    assert_eq!(e.what, "还没认出来，先别动");
    assert_eq!(e.lose, "先不动它不会有任何损失。");
    assert_eq!(e.what.matches("不确定").count() + e.why.matches("不确定").count() + e.lose.matches("不确定").count(), 0, "不要一直说“不确定”");
    assert!(e.clues[0].contains("程序文件") && e.clues[0].contains("Codex"), "线索要替用户总结一句：{:?}", e.clues);
}

/// 第 1 轮产品评审第 1 条：沙箱这几层不再沿用“Claude 的登录信息”；只装着 cargo 编译目录的外层文件夹也认得出。
#[test]
fn sandbox_layers_and_build_container() {
    let (c, _) = ctx(r"C:\Users\bob\AppData\Local\Temp");
    let pkg = r"C:\Users\bob\AppData\Local\Packages\Claude_pzs8sxrjxfjjc";
    let t = tree(
        "C:",
        &[
            (format!(r"{pkg}\LocalCache\Local\DiskWhy-build\target\CACHEDIR.TAG"), 177),
            (format!(r"{pkg}\LocalCache\Local\DiskWhy-build\target\.rustc_info.json"), 1_111),
            (format!(r"{pkg}\LocalCache\Local\DiskWhy-build\target\debug\x.rlib"), 9_000_000),
            (format!(r"{pkg}\Settings\settings.dat"), 8_000),
        ],
    );
    let lc = ex(&t, &c, &format!(r"{pkg}\LocalCache"));
    assert!(matches!(&lc.source, Source::Rule { id, .. } if id == "store-app-localcache"), "{}", src(&lc));
    assert!(!lc.what.contains("登录信息"));
    let local = ex(&t, &c, &format!(r"{pkg}\LocalCache\Local"));
    assert!(matches!(&local.source, Source::Rule { id, .. } if id == "store-app-sandbox-appdata"), "{}", src(&local));
    let b = ex(&t, &c, &format!(r"{pkg}\LocalCache\Local\DiskWhy-build"));
    assert!(matches!(&b.source, Source::Heuristic { name } if name == "rust-target"), "{}", src(&b));
    assert!(b.notes.iter().any(|n| n.contains("沙箱")), "{:?}", b.notes);
}
