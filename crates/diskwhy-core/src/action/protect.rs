//! 保护路径：**硬编码、不可配置、不可关闭**。
//!
//! 删除请求落在下面这些路径本身、它们里面、或者是它们的祖先（上层文件夹）时直接拒绝：
//!
//! | 类别 | 范围 |
//! |---|---|
//! | 任何卷根（`X:\`） | 本身（卷根是所有东西的祖先） |
//! | `X:\Windows`（任何盘）、`%SystemRoot%` | 本身及里面全部；特别标出 `WinSxS`、`Installer` |
//! | `X:\Program Files*`（任何盘，含 `(x86)`）、`%ProgramFiles%` 等 | 本身及里面全部 |
//! | `X:\ProgramData\Microsoft\Windows` | 本身及里面全部（祖先 `ProgramData`、`ProgramData\Microsoft` 也拒绝） |
//! | 用户的 桌面 / 文档 / 图片 / 视频 / 音乐 / 下载 | 根及其直接子项（`X:\Users\任何人\<名字>` + 已知文件夹的真实位置） |
//! | `.ssh` / `.gnupg` / `.aws` | 路径里任何一段是这些名字 → 本身及里面全部 |
//! | OneDrive 根 | `%OneDrive*%` 和 `X:\Users\任何人\OneDrive*` 本身 |
//! | DiskWhy 自己的数据（阶段 J） | `%LOCALAPPDATA%\DiskWhy`、`%LOCALAPPDATA%\com.diskwhy.desktop`（界面的 WebView2 数据）本身及里面全部 |
//! | `$Recycle.Bin` / `System Volume Information` / `WinSxS` | 路径里任何一段是这些名字 → 本身及里面全部 |
//!
//! 比较前先规范化：去掉 `\\?\`、`/` 换成 `\`、去掉每段结尾的 `.` 和空格（Windows 会自动忽略）、
//! 大小写不敏感；含 `..`、NTFS 流（`:`）、UNC / 设备路径、相对路径一律拒绝；短文件名（`PROGRA~1`）
//! 先用 GetLongPathNameW 展开，展不开就拒绝。另外再用 GetFinalPathNameByHandleW 解析一次真实位置
//! （路径里有联接 / 符号链接指到受保护目录时也能拦住），两者任一受保护就拒绝。

use super::winops;

/// 受保护的具体位置（从环境变量 / 已知文件夹 / 盘符得到）。
#[derive(Debug, Clone)]
struct Place {
    segs: Vec<String>,
    label: String,
    depth: Depth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Depth {
    /// 本身和里面全部。
    Tree,
    /// 本身和直接子项。
    SelfAndChildren,
    /// 只有本身。
    SelfOnly,
}

/// 个人文件夹名（`X:\Users\<谁>\<名字>`）。
const PERSONAL: &[(&str, &str)] =
    &[("desktop", "桌面"), ("documents", "文档"), ("pictures", "图片"), ("videos", "视频"), ("music", "音乐"), ("downloads", "下载")];
/// 路径里出现这些段名就拒绝（本身及里面全部）。
const SECRET_SEGS: &[&str] = &[".ssh", ".gnupg", ".aws"];
const SYSTEM_SEGS: &[(&str, &str)] =
    &[("$recycle.bin", "回收站（$Recycle.Bin）"), ("system volume information", "System Volume Information（还原点 / 卷影副本）"), ("winsxs", "WinSxS（Windows 组件存储）")];

/// 保护检查器。
#[derive(Debug, Clone)]
pub struct Protector {
    places: Vec<Place>,
    /// 是否在检查时调用文件系统（GetLongPathNameW / GetFinalPathNameByHandleW）。测试纯字符串规则时可关。
    resolve: bool,
}

/// 规范化结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
    /// `C:\A\b` 形式（盘符大写、无结尾反斜杠，卷根为 `C:\`）。
    pub path: String,
    /// 小写段：第一段是 `c:`。
    pub segs: Vec<String>,
}

/// 规范化路径。拒绝的情况返回 Err(原因)。
pub fn normalize(input: &str) -> Result<Normalized, String> {
    let raw = input.trim().replace('/', "\\");
    if raw.is_empty() {
        return Err("路径为空".into());
    }
    let s = if let Some(rest) = raw.strip_prefix(r"\\?\") {
        if rest.len() >= 4 && rest[..4].eq_ignore_ascii_case("UNC\\") {
            return Err("不支持网络路径（UNC）".into());
        }
        rest.to_string()
    } else if raw.starts_with(r"\\") {
        return Err("不支持网络路径或设备路径（\\\\ 开头）".into());
    } else {
        raw
    };
    let b = s.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return Err(format!("只支持带盘符的完整路径：{input}"));
    }
    if b.len() > 2 && b[2] != b'\\' {
        return Err(format!("不是完整路径（盘符后面缺少 \\）：{input}"));
    }
    let drive = format!("{}:", (b[0] as char).to_ascii_uppercase());
    let mut out_segs: Vec<String> = Vec::new();
    for comp in s[2..].split('\\').filter(|c| !c.is_empty()) {
        if comp == "." {
            continue;
        }
        if comp.contains(':') {
            return Err(format!("路径里不能有冒号（NTFS 数据流）：{input}"));
        }
        // Windows 会忽略每一段结尾的点和空格：`C:\Windows.` 就是 `C:\Windows`。
        let trimmed = comp.trim_end_matches(['.', ' ']);
        if trimmed.is_empty() {
            // “..”、“...” 等
            return Err(format!("路径里不能有“..”：{input}"));
        }
        out_segs.push(trimmed.to_string());
    }
    let path = if out_segs.is_empty() { format!("{drive}\\") } else { format!("{drive}\\{}", out_segs.join("\\")) };
    let mut segs = vec![drive.to_lowercase()];
    segs.extend(out_segs.iter().map(|x| x.to_lowercase()));
    Ok(Normalized { path, segs })
}

fn looks_short_name(seg: &str) -> bool {
    // 8.3 短名的特征：主名里有 ~ 后跟数字，例如 PROGRA~1、DOCUME~2.TXT
    let stem = seg.split('.').next().unwrap_or(seg);
    stem.char_indices().any(|(i, c)| c == '~' && stem[i + 1..].chars().next().is_some_and(|d| d.is_ascii_digit()))
}

fn starts_with(segs: &[String], prefix: &[String]) -> bool {
    segs.len() >= prefix.len() && segs[..prefix.len()] == prefix[..]
}

impl Protector {
    /// 生产用：当前进程环境变量 + 已知文件夹真实位置 + 本机所有盘符。
    pub fn from_system() -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let mut vars = Vec::new();
        for k in [
            "SystemRoot",
            "windir",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "CommonProgramFiles",
            "CommonProgramFiles(x86)",
            "ProgramData",
            "USERPROFILE",
            "OneDrive",
            "OneDriveConsumer",
            "OneDriveCommercial",
            "LOCALAPPDATA",
        ] {
            if let Some(v) = env(k) {
                vars.push((k.to_string(), v));
            }
        }
        let known = winops::personal_known_folders();
        let drives: Vec<String> = crate::win::list_fixed_drives().into_iter().map(|d| d.root).collect();
        let mut p = Protector::with(&vars, &known, &drives);
        p.resolve = true;
        p
    }

    /// 测试 / 生产共用：给定环境变量、已知文件夹、盘符。不访问文件系统（`resolve = false`）。
    pub fn with(vars: &[(String, String)], known: &[(String, &str)], drives: &[String]) -> Self {
        let mut places = Vec::new();
        let mut add = |path: &str, label: String, depth: Depth| {
            if let Ok(n) = normalize(path) {
                places.push(Place { segs: n.segs, label, depth });
            }
        };
        for d in drives {
            add(d, format!("卷根 {d}"), Depth::SelfOnly);
            add(&format!("{d}$Recycle.Bin"), "回收站（$Recycle.Bin）".into(), Depth::Tree);
            add(&format!("{d}System Volume Information"), "System Volume Information（还原点 / 卷影副本）".into(), Depth::Tree);
        }
        let get = |k: &str| vars.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
        for k in ["SystemRoot", "windir"] {
            if let Some(v) = get(k) {
                add(&format!("{v}\\WinSxS"), "WinSxS（Windows 组件存储）".into(), Depth::Tree);
                add(&format!("{v}\\Installer"), "Windows Installer 缓存（Windows\\Installer）".into(), Depth::Tree);
                add(&v, "Windows 系统目录".into(), Depth::Tree);
            }
        }
        for k in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "CommonProgramFiles", "CommonProgramFiles(x86)"] {
            if let Some(v) = get(k) {
                add(&v, format!("程序安装目录（{v}）"), Depth::Tree);
            }
        }
        if let Some(v) = get("ProgramData") {
            add(&format!("{v}\\Microsoft\\Windows"), "ProgramData\\Microsoft\\Windows（系统共享数据）".into(), Depth::Tree);
        }
        if let Some(home) = get("USERPROFILE") {
            for (name, label) in PERSONAL {
                add(&format!("{home}\\{name}"), format!("你的{label}文件夹"), Depth::SelfAndChildren);
            }
            for s in SECRET_SEGS {
                add(&format!("{home}\\{s}"), format!("密钥 / 凭据目录（{s}）"), Depth::Tree);
            }
            add(&home, "你的用户目录".into(), Depth::SelfOnly);
        }
        for (path, label) in known {
            add(path, format!("你的{label}文件夹（{path}）"), Depth::SelfAndChildren);
        }
        // 阶段 J（PHASE_J_CRASH.md 隐患 5）：DiskWhy 自己的数据——日志 / 设置 / 执行状态（%LOCALAPPDATA%\DiskWhy）
        // 和界面的 WebView2 数据（%LOCALAPPDATA%\com.diskwhy.desktop\EBWebView，正在运行时删它的缓存会弄坏自己的界面）。
        if let Some(v) = get("LOCALAPPDATA") {
            add(&format!("{v}\\DiskWhy"), "DiskWhy 自己的日志和设置".into(), Depth::Tree);
            add(&format!("{v}\\com.diskwhy.desktop"), "DiskWhy 自己的界面数据（WebView2）".into(), Depth::Tree);
        }
        for k in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(v) = get(k) {
                add(&v, format!("OneDrive 根目录（{v}）"), Depth::SelfOnly);
            }
        }
        Protector { places, resolve: false }
    }

    /// 检查一个删除请求。返回 `Some(原因)` 表示拒绝。
    pub fn check(&self, input: &str) -> Option<String> {
        self.check_with(input, &|p: &str| winops::final_path(p))
    }

    /// 同 [`Protector::check`]，但父目录的真实位置由 `resolve_parent` 给出（预检里用 [`ParentCache`]：
    /// 同一个文件夹里的几千个文件只解析一次父目录）。检查逻辑、拒绝条件和 `check` 完全相同。
    pub fn check_with(&self, input: &str, resolve_parent: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        let n = match normalize(input) {
            Ok(n) => n,
            Err(e) => return Some(e),
        };
        let mut n = n;
        if n.segs.iter().any(|s| looks_short_name(s)) {
            let resolved = if self.resolve { winops::long_name(&n.path).and_then(|p| normalize(&p).ok()) } else { None };
            match resolved {
                Some(r) if !r.segs.iter().any(|s| looks_short_name(s)) => n = r,
                _ => return Some(format!("路径里有 8.3 短文件名（如 PROGRA~1），无法确认真实位置，拒绝：{input}")),
            }
        }
        if let Some(r) = self.check_segs(&n.segs) {
            return Some(r);
        }
        if self.resolve {
            // 真实位置：只解析父目录（跟随父目录链上的联接 / 符号链接），最后一段不跟随——
            // 要删的如果本身是链接，删的是链接自身。
            if n.segs.len() >= 2 {
                let (parent, name) = n.path.rsplit_once('\\').unwrap();
                let parent = if parent.len() == 2 { format!("{parent}\\") } else { parent.to_string() };
                if let Some(real_parent) = resolve_parent(&parent) {
                    if let Ok(real) = normalize(&crate::fs::join(&real_parent, name)) {
                        if real.segs != n.segs {
                            if let Some(r) = self.check_segs(&real.segs) {
                                return Some(format!("{r}（实际位置：{}，路径里有链接指向受保护目录）", real.path));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// 只按字符串规则检查（已规范化的小写段）。
    pub fn check_segs(&self, segs: &[String]) -> Option<String> {
        if segs.len() <= 1 {
            return Some(format!("{} 是卷根（整个盘），不能删除", segs.first().map(|s| s.to_uppercase() + "\\").unwrap_or_default()));
        }
        // 1. 任何位置出现的系统段 / 密钥段
        for s in segs {
            for (name, label) in SYSTEM_SEGS {
                if s == name {
                    return Some(format!("受保护：{label}，DiskWhy 永远不删这里（本身和里面的东西都不删）"));
                }
            }
            if SECRET_SEGS.contains(&s.as_str()) {
                return Some(format!("受保护：{s} 是密钥 / 凭据目录，DiskWhy 永远不删"));
            }
        }
        let first = segs[1].as_str();
        // 2. 任何盘上的 Windows / Program Files* / ProgramData\Microsoft\Windows
        if first == "windows" {
            if segs.get(2).is_some_and(|x| x == "installer") {
                return Some("受保护：Windows\\Installer 是软件卸载 / 修复要用的安装包缓存，删了会导致软件无法卸载或更新".into());
            }
            return Some("受保护：Windows 系统目录（本身和里面的东西都不删）".into());
        }
        if first.starts_with("program files") {
            return Some(format!("受保护：程序安装目录（{}），卸载软件请用系统的“应用”设置", segs[1]));
        }
        if first == "programdata" {
            let pmw = ["programdata", "microsoft", "windows"];
            let depth = segs.len() - 1;
            if depth <= 2 && segs[1..].iter().zip(pmw.iter()).all(|(a, b)| a == b) {
                return Some("受保护：这是 ProgramData\\Microsoft\\Windows 的上层文件夹，不能整个删除".into());
            }
            if depth >= 3 && segs[1..4].iter().zip(pmw.iter()).all(|(a, b)| a == b) {
                return Some("受保护：ProgramData\\Microsoft\\Windows 是系统共享数据".into());
            }
        }
        // 3. X:\Users\<谁>\<个人文件夹>（根 + 直接子项）、OneDrive 根；以及它们的祖先
        if first == "users" {
            if segs.len() <= 3 {
                return Some("受保护：这是用户目录（或其上层），里面有桌面、文档等个人文件夹，不能整个删除".into());
            }
            let third = segs[3].as_str();
            if let Some((_, label)) = PERSONAL.iter().find(|(n, _)| *n == third) {
                if segs.len() <= 5 {
                    return Some(format!("受保护：你的{label}文件夹（根目录和它下面第一层）不能由 DiskWhy 删除，请自己在资源管理器里处理"));
                }
            }
            if third.starts_with("onedrive") && segs.len() == 4 {
                return Some("受保护：OneDrive 根目录".into());
            }
        }
        // 4. 具体位置（环境变量 / 已知文件夹 / 盘符）：本身、里面（按深度）或祖先
        for p in &self.places {
            if starts_with(segs, &p.segs) {
                let extra = segs.len() - p.segs.len();
                let hit = match p.depth {
                    Depth::Tree => true,
                    Depth::SelfAndChildren => extra <= 1,
                    Depth::SelfOnly => extra == 0,
                };
                if hit {
                    return Some(format!("受保护：{}", p.label));
                }
            } else if p.segs.len() > segs.len() && starts_with(&p.segs, segs) {
                return Some(format!("受保护：这是「{}」的上层文件夹，删了会连它一起删掉", p.label));
            }
        }
        None
    }

    /// 清单里的单个条目（文件夹里面的文件）是否受保护：只看“本身或里面”的规则（条目不会是祖先）。
    pub fn entry_blocked(&self, path: &str) -> Option<String> {
        self.check(path)
    }

    /// 同 [`Protector::entry_blocked`]，父目录真实位置走缓存。
    pub fn entry_blocked_cached(&self, path: &str, cache: &ParentCache) -> Option<String> {
        self.check_with(path, &|p: &str| cache.resolve(p))
    }
}

/// 保护原因的短说法（≤ 6 个字，清理页上不能勾的那一行显示；完整原因悬停看）。
pub fn short_reason(why: &str) -> &'static str {
    let w = why.to_lowercase();
    if w.contains("卷根") {
        "整个盘"
    } else if w.contains("diskwhy 自己") {
        "本程序数据"
    } else if w.contains("密钥") {
        "密钥目录"
    } else if w.contains("回收站") {
        "回收站"
    } else if w.contains("程序安装目录") {
        "程序目录"
    } else if w.contains("onedrive") {
        "OneDrive"
    } else if w.contains("用户目录") {
        "用户目录"
    } else if w.contains("你的") {
        "个人文件夹"
    } else if w.contains("短文件名") || (w.contains("路径") && !w.contains("受保护")) {
        "路径不明"
    } else {
        "系统目录"
    }
}

/// 父目录真实位置（GetFinalPathNameByHandleW）的缓存，一次预检内有效。
///
/// 阶段 G 实测（本机默认勾选 39.6 万个文件、6.5 万个文件夹）：逐个文件解析父目录要 40 秒，
/// 占保护路径检查时间的 93%。两层优化：
/// 1. 同一个父目录的结果对里面的所有文件都一样，每个文件夹只算一次；
/// 2. 清单项里面的文件夹：枚举时**只进入不是 reparse point 的普通文件夹**（链接只作为条目、不跟进去），
///    所以 清单项下面某个文件夹的真实位置 = 清单项本身的真实位置 + 相对路径，只需对清单项本身调用一次
///    GetFinalPathNameByHandleW（[`ParentCache::with_roots`]）。不在任何清单项下面的父目录照旧实际解析。
///
/// 枚举之后文件夹被人换成链接这种竞态，由执行时的重新校验兜住：`execute::reverify` 仍然逐条、不走缓存地
/// 完整检查保护路径（含实际解析父目录），并核对文件 ID。
#[derive(Default)]
pub struct ParentCache {
    map: std::sync::RwLock<std::collections::HashMap<String, Option<String>>>,
    /// (小写的清单项路径, 清单项路径, 它的真实位置)
    roots: Vec<(String, String, Option<String>)>,
}

impl ParentCache {
    /// `roots`：清单项（文件夹）路径。每个调用一次 GetFinalPathNameByHandleW。
    pub fn with_roots<'a>(roots: impl IntoIterator<Item = &'a str>) -> Self {
        let mut v: Vec<(String, String, Option<String>)> =
            roots.into_iter().map(|r| (r.to_lowercase(), r.to_string(), winops::final_path(r))).collect();
        // 最长的在前（嵌套的清单项取最近的那个）
        v.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        ParentCache { map: Default::default(), roots: v }
    }

    pub fn resolve(&self, parent: &str) -> Option<String> {
        if let Some(v) = self.map.read().unwrap().get(parent) {
            return v.clone();
        }
        let v = self.derive(parent).unwrap_or_else(|| winops::final_path(parent));
        self.map.write().unwrap().insert(parent.to_string(), v.clone());
        v
    }

    /// 在某个清单项下面：真实位置 = 清单项的真实位置 + 相对路径。不在任何清单项下面返回 None（要实际解析）。
    fn derive(&self, parent: &str) -> Option<Option<String>> {
        for (rl, r, real) in &self.roots {
            let Some(prefix) = parent.get(..r.len()) else { continue };
            if prefix.to_lowercase() != *rl {
                continue;
            }
            if parent.len() == r.len() {
                return Some(real.clone());
            }
            if parent.as_bytes()[r.len()] == b'\\' && !r.ends_with('\\') {
                let rest = &parent[r.len() + 1..];
                return Some(real.as_ref().map(|x| crate::fs::join(x, rest)));
            }
        }
        None
    }
    pub fn len(&self) -> usize {
        self.map.read().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_variants() {
        assert_eq!(normalize(r"\\?\c:\windows\").unwrap().path, r"C:\windows");
        assert_eq!(normalize("c:/Windows/System32").unwrap().segs, vec!["c:", "windows", "system32"]);
        assert_eq!(normalize(r"C:\Windows. \x").unwrap().segs, vec!["c:", "windows", "x"]);
        assert_eq!(normalize("d:").unwrap().path, r"D:\");
        assert!(normalize(r"C:\a\..\Windows").is_err());
        assert!(normalize(r"\\server\share\x").is_err());
        assert!(normalize(r"\\?\UNC\server\share").is_err());
        assert!(normalize(r"relative\path").is_err());
        assert!(normalize(r"C:\a:stream").is_err());
        assert!(looks_short_name("PROGRA~1"));
        assert!(!looks_short_name("my~file"));
    }

    #[test]
    fn short_reasons_fit_six_chars() {
        let p = Protector::with(
            &[
                ("SystemRoot".into(), r"C:\Windows".into()),
                ("ProgramFiles".into(), r"C:\Program Files".into()),
                ("USERPROFILE".into(), r"C:\Users\me".into()),
                ("LOCALAPPDATA".into(), r"C:\Users\me\AppData\Local".into()),
            ],
            &[],
            &[r"C:\".into()],
        );
        let cases = [
            (r"C:\", "整个盘"),
            (r"C:\Windows\Temp", "系统目录"),
            (r"C:\Program Files\App", "程序目录"),
            (r"C:\Users\me\Documents", "个人文件夹"),
            (r"C:\Users\me", "用户目录"),
            (r"C:\Users\me\.ssh\id", "密钥目录"),
            (r"C:\$Recycle.Bin\x", "回收站"),
            (r"C:\Users\me\AppData\Local\DiskWhy\x", "本程序数据"),
        ];
        for (path, want) in cases {
            let why = p.check(path).unwrap_or_else(|| panic!("{path} 应受保护"));
            let s = short_reason(&why);
            assert_eq!(s, want, "{path}: {why}");
            assert!(s.chars().count() <= 6);
        }
    }
}
