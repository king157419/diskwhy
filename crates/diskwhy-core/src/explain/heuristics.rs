//! 启发式：规则库没命中时按顺序尝试。
//!
//! 1. 目录内容签名（聊天数据、Chromium 缓存、Python 环境、node_modules、Rust target、虚拟环境、.NET bin/obj、
//!    模型权重、虚拟磁盘、日志/转储、下载目录里的安装包、临时文件、媒体文件）；
//! 2. 归属推断：注册表 Uninstall 快照里的 InstallLocation / DisplayName / Publisher；
//! 3. 年龄：最近修改超过 180 天，附一句“已经 N 个月没动过”；缓存类写“很久没用了”；
//! 4. 都不行：`Source::Unknown`，列出看到的线索。
//!
//! **档位护栏**：启发式只有一种情况能给 safe——[`SAFE_HEURISTICS`] 里的 `chromium-cache`，
//! 而且要满足“文件夹名是 Chromium 的缓存目录名 + 里面每个文件都是 Chromium 磁盘缓存的标准文件名”
//! 并且不在桌面 / 文档等个人文件夹下。其余一律 review 或 keep（[`finish`] 里兜底强制，
//! `explain::enforce_guards` 再兜一次）。
//!
//! 看的是扫描树（内存里的 [`Node`]）：每个目录只保留了前 N 大文件，小文件合并成 Rest 节点，
//! 所以“旁边有没有 package.json / Cargo.toml / *.csproj”这种判断在树里找不到、且父目录有 Rest
//! 节点时，会退回去直接问文件系统（只查文件是否存在，不读内容）。

use super::pattern::split_lower;
use super::registry::{normalize_name, InstalledApp};
use super::{gain, Category, ExplainCtx, Explanation, How, Source, Target, Tier};
use crate::model::{Node, NodeKind};
use crate::view::human_bytes;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// 最近修改超过多少天算“很久没动”。
pub const STALE_DAYS: i64 = 180;
/// 允许给出 safe 的启发式（只有签名极严格的 Chromium 磁盘缓存）。
pub const SAFE_HEURISTICS: &[&str] = &["chromium-cache"];
/// Chromium 用磁盘缓存格式（index + data_N / f_xxxxxx）存东西的文件夹名（小写）。
const CHROMIUM_CACHE_DIRS: &[&str] =
    &["cache_data", "gpucache", "grshadercache", "graphitedawncache", "dawncache", "dawngraphitecache", "dawnwebgpucache", "shadercache"];
/// 聊天软件的数据文件夹名（小写）。用户改过存储位置时规则库按 %变量% 认不出来，靠名字兜底。
const CHAT_DATA_DIRS: &[(&str, &str)] =
    &[("wechat files", "微信"), ("xwechat_files", "微信"), ("tencent files", "QQ"), ("wxwork", "企业微信")];
/// 统计子树时最多看多少个节点（大目录采样，保证列表秒开）。
const MAX_VISIT: usize = 30_000;
/// 单个模型文件至少多大才算“模型权重”。
const MODEL_MIN_FILE: u64 = 100 * 1024 * 1024;

const MODEL_EXTS: &[&str] = &["safetensors", "gguf", "onnx", "pt", "pth", "bin", "ckpt"];
const VDISK_EXTS: &[&str] = &["vhdx", "vhd", "vmdk", "qcow2", "avhdx", "vdi"];
const LOG_EXTS: &[&str] = &["log", "etl", "dmp", "mdmp", "hdmp"];
const INSTALLER_EXTS: &[&str] = &["iso", "msi", "exe", "msix", "msixbundle", "appx"];
const MEDIA_EXTS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "wmv", "flv", "webm", "m4v", "ts", "jpg", "jpeg", "png", "heic", "gif", "bmp", "webp",
    "raw", "cr2", "nef", "arw", "dng", "mp3", "flac", "wav", "m4a", "aac", "ogg", "ape",
];

// ───────────────────────── 子树统计 ─────────────────────────

/// 子树（采样）统计：扩展名分布、最大的文件、最近修改时间。
#[derive(Debug, Clone, Default)]
pub struct SubtreeStats {
    /// 看到的普通文件个数（不含 Rest 合计里的）。
    pub files_seen: u64,
    /// 看到的普通文件字节（allocated，不含 Rest）。
    pub bytes_seen: u64,
    /// Rest 合计节点的字节（扩展名未知）。
    pub rest_bytes: u64,
    /// (扩展名, 字节, 个数)，按字节降序。无扩展名记为空字符串。
    pub ext: Vec<(String, u64, u64)>,
    /// 最大的几个文件 (名字, 字节)。
    pub biggest: Vec<(String, u64)>,
    /// 子树里最新的修改时间（Unix 秒，0 = 未知）。
    pub newest_mtime: i64,
    /// 节点太多，只看了一部分。
    pub truncated: bool,
}

impl SubtreeStats {
    /// 某组扩展名占“看到的文件字节”的比例（0~1）。
    pub fn share(&self, exts: &[&str]) -> f64 {
        if self.bytes_seen == 0 {
            return 0.0;
        }
        let b: u64 = self.ext.iter().filter(|(e, _, _)| exts.contains(&e.as_str())).map(|x| x.1).sum();
        b as f64 / self.bytes_seen as f64
    }
    /// 某组扩展名占文件个数的比例（0~1）。
    pub fn count_share(&self, exts: &[&str]) -> f64 {
        if self.files_seen == 0 {
            return 0.0;
        }
        let c: u64 = self.ext.iter().filter(|(e, _, _)| exts.contains(&e.as_str())).map(|x| x.2).sum();
        c as f64 / self.files_seen as f64
    }
    /// 某组扩展名里最大的单个文件。
    pub fn biggest_with(&self, exts: &[&str]) -> u64 {
        self.biggest.iter().filter(|(n, _)| exts.contains(&ext_of(n).as_str())).map(|x| x.1).max().unwrap_or(0)
    }
}

/// 小写扩展名（不含点）；没有扩展名返回空串。
pub fn ext_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() && !e.is_empty() && e.len() <= 12 && !e.contains(' ') => e.to_lowercase(),
        _ => String::new(),
    }
}

/// 统计一个节点的子树（最多看 [`MAX_VISIT`] 个节点）。
pub fn subtree_stats(node: &Node) -> SubtreeStats {
    let mut s = SubtreeStats::default();
    let mut ext: HashMap<String, (u64, u64)> = HashMap::new();
    let mut biggest: Vec<(String, u64)> = Vec::new();
    let mut visited = 0usize;
    let mut stack: Vec<&Node> = vec![node];
    while let Some(n) = stack.pop() {
        visited += 1;
        if visited > MAX_VISIT {
            s.truncated = true;
            break;
        }
        s.newest_mtime = s.newest_mtime.max(n.mtime);
        match n.kind {
            NodeKind::File => {
                s.files_seen += 1;
                s.bytes_seen += n.allocated;
                let e = ext.entry(ext_of(&n.name)).or_default();
                e.0 += n.allocated;
                e.1 += 1;
                biggest.push((n.name.clone(), n.allocated));
                if biggest.len() > 64 {
                    biggest.sort_by(|a, b| b.1.cmp(&a.1));
                    biggest.truncate(16);
                }
            }
            NodeKind::Rest => s.rest_bytes += n.allocated,
            NodeKind::Dir => stack.extend(n.children.iter()),
        }
    }
    biggest.sort_by(|a, b| b.1.cmp(&a.1));
    biggest.truncate(16);
    s.biggest = biggest;
    let mut v: Vec<(String, u64, u64)> = ext.into_iter().map(|(k, (b, c))| (k, b, c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    s.ext = v;
    s
}

// ───────────────────────── 小工具 ─────────────────────────

fn child<'a>(n: &'a Node, name: &str) -> Option<&'a Node> {
    n.children.iter().find(|c| c.kind != NodeKind::Rest && c.name.eq_ignore_ascii_case(name))
}

fn has_child(n: &Node, name: &str) -> bool {
    child(n, name).is_some()
}

fn parent_path(t: &Target) -> Option<String> {
    if let Some(p) = t.parent {
        if !p.path.is_empty() {
            return Some(p.path.clone());
        }
    }
    let trimmed = t.path.trim_end_matches('\\');
    trimmed.rsplit_once('\\').map(|(a, _)| if a.len() == 2 && a.ends_with(':') { format!("{a}\\") } else { a.to_string() })
}

fn parent_truncated(t: &Target) -> bool {
    t.parent.is_none_or(|p| p.children.iter().any(|c| c.kind == NodeKind::Rest))
}

/// 父目录里有没有叫 `name` 的项。树里找不到且父目录的小文件被合并过时，直接问文件系统。
fn sibling_exists(t: &Target, name: &str) -> bool {
    if let Some(p) = t.parent {
        if has_child(p, name) {
            return true;
        }
    }
    if parent_truncated(t) {
        if let Some(pp) = parent_path(t) {
            return Path::new(&pp).join(name).exists();
        }
    }
    false
}

/// 父目录里有没有某种扩展名的文件。
fn sibling_with_ext(t: &Target, exts: &[&str]) -> Option<String> {
    if let Some(p) = t.parent {
        if let Some(c) = p.children.iter().find(|c| c.kind == NodeKind::File && exts.contains(&ext_of(&c.name).as_str())) {
            return Some(c.name.clone());
        }
    }
    if parent_truncated(t) {
        if let Some(pp) = parent_path(t) {
            if let Ok(rd) = std::fs::read_dir(&pp) {
                for e in rd.flatten().take(5000) {
                    let n = e.file_name().to_string_lossy().to_string();
                    if exts.contains(&ext_of(&n).as_str()) && e.file_type().map(|f| f.is_file()).unwrap_or(false) {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// 路径按 `\` 切成段（去掉空段）。
fn segments(path: &str) -> Vec<&str> {
    path.split(['\\', '/']).filter(|s| !s.is_empty()).collect()
}

fn seg_count(path: &str) -> usize {
    segments(path).len()
}

/// 名字（不分大小写）等于 `name` 的段的下标，从浅到深。
fn segment_indices(path: &str, name: &str) -> Vec<usize> {
    segments(path).iter().enumerate().filter(|(_, s)| s.eq_ignore_ascii_case(name)).map(|(i, _)| i).collect()
}

fn first_segment(path: &str, name: &str) -> Option<usize> {
    segment_indices(path, name).into_iter().next()
}

/// 前 `n` 段拼回路径（`C:` 单独一段时补成 `C:\`）。
fn prefix_path(path: &str, n: usize) -> String {
    let s = segments(path);
    let n = n.min(s.len());
    if n == 0 {
        return String::new();
    }
    let joined = s[..n].join("\\");
    if n == 1 && joined.ends_with(':') {
        format!("{joined}\\")
    } else {
        joined
    }
}

/// 严格的 Chromium 磁盘缓存文件名：`index`、`data_N`、`f_十六进制`、`16 位十六进制_0/_1/_s`（simple 格式）。
fn is_chromium_cache_file(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    if l == "index" {
        return true;
    }
    if let Some(d) = l.strip_prefix("data_") {
        return !d.is_empty() && d.chars().all(|c| c.is_ascii_digit());
    }
    if let Some(h) = l.strip_prefix("f_") {
        return !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some((h, suf)) = l.split_once('_') {
        return h.len() == 16 && h.chars().all(|c| c.is_ascii_hexdigit()) && matches!(suf, "0" | "1" | "s");
    }
    false
}

/// 目录里每一项都是 Chromium 磁盘缓存的标准文件（可以有 `index-dir\the-real-index`），并且有 index 和至少一个数据文件。
/// 被合并进“其余小文件”的看不到名字——看得到的最大 50 个都合格就算合格；index 被合并掉时去文件系统确认它存在。
fn is_chromium_disk_cache(n: &Node) -> bool {
    let (mut has_index, mut has_data, mut has_rest) = (false, false, false);
    for c in &n.children {
        match c.kind {
            NodeKind::Rest => has_rest = true,
            NodeKind::File => {
                if !is_chromium_cache_file(&c.name) {
                    return false;
                }
                if c.name.eq_ignore_ascii_case("index") {
                    has_index = true;
                } else {
                    has_data = true;
                }
            }
            NodeKind::Dir => {
                let ok = c.name.eq_ignore_ascii_case("index-dir")
                    && c.children.iter().all(|x| x.kind == NodeKind::File && x.name.eq_ignore_ascii_case("the-real-index"));
                if !ok {
                    return false;
                }
            }
        }
    }
    if !has_index && has_rest && !n.path.is_empty() {
        has_index = Path::new(&n.path).join("index").is_file();
    }
    has_index && has_data
}

/// 签名严格的 Chromium 缓存：返回线索；不够严格返回 None。
fn strict_chromium_cache(t: &Target) -> Option<String> {
    let n = t.node;
    let name = n.name.to_lowercase();
    let real = |c: &&Node| c.kind != NodeKind::Rest;
    if CHROMIUM_CACHE_DIRS.contains(&name.as_str()) && is_chromium_disk_cache(n) {
        return Some(format!("文件夹叫 {}，里面全是 Chromium 磁盘缓存的标准文件（index、data_N / f_xxxxxx）", n.name));
    }
    if name == "cache" {
        // 新版：Cache\Cache_Data（新版 Chrome 旁边还有个很小的 No_Vary_Search）；老版：Cache 里直接是 index + data_N。
        let mut cache_data_ok = false;
        let all_ok = n.children.iter().filter(real).all(|c| {
            if c.kind == NodeKind::Dir && c.name.eq_ignore_ascii_case("Cache_Data") {
                cache_data_ok = is_chromium_disk_cache(c);
                cache_data_ok
            } else if c.kind == NodeKind::Dir && c.name.eq_ignore_ascii_case("No_Vary_Search") {
                c.children.iter().all(|x| x.kind == NodeKind::File && (x.name.ends_with(".baj") || x.name.ends_with(".baf")))
            } else {
                false
            }
        });
        if all_ok && cache_data_ok {
            return Some("文件夹叫 Cache，里面是 Cache_Data，而且 Cache_Data 里全是 Chromium 磁盘缓存的标准文件".into());
        }
        if is_chromium_disk_cache(n) {
            return Some("文件夹叫 Cache，里面全是 Chromium 老版磁盘缓存的标准文件（index、data_N）".into());
        }
        return None;
    }
    if name == "code cache" {
        let subs: Vec<&Node> = n.children.iter().filter(real).collect();
        let all_ok = !subs.is_empty()
            && subs.iter().all(|c| {
                c.kind == NodeKind::Dir
                    && ["js", "wasm", "webui_js"].contains(&c.name.to_lowercase().as_str())
                    && (c.children.is_empty() || is_chromium_disk_cache(c))
            });
        if all_ok && subs.iter().any(|c| is_chromium_disk_cache(c)) {
            return Some("文件夹叫 Code Cache，里面只有 js / wasm，都是 Chromium 磁盘缓存的标准文件".into());
        }
        return None;
    }
    if ["js", "wasm", "webui_js"].contains(&name.as_str())
        && t.parent.is_some_and(|p| p.name.eq_ignore_ascii_case("Code Cache"))
        && is_chromium_disk_cache(n)
    {
        return Some(format!("在 Code Cache 里，文件夹叫 {}，里面全是 Chromium 磁盘缓存的标准文件", n.name));
    }
    None
}

/// 聊天软件的数据（不在规则库覆盖的默认位置时）：keep + 迁移。
fn chat_data(t: &Target, ctx: &ExplainCtx) -> Option<Hint> {
    let segs = segments(t.path);
    let (idx, app) = segs.iter().enumerate().find_map(|(i, s)| {
        let l = s.to_lowercase();
        CHAT_DATA_DIRS.iter().find(|(d, _)| *d == l).map(|(_, app)| (i, *app))
    })?;
    // 程序安装目录下（比如 Program Files (x86)\WXWork）是程序本体，不是聊天数据。
    let lower = split_lower(t.path);
    let in_program = ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .any(|v| ctx.env.get(v).is_some_and(|base| under(&lower, &split_lower(base)) || lower == split_lower(base)));
    if in_program {
        return None;
    }
    let data_dir = prefix_path(t.path, idx + 1);
    Some(Hint {
        name: "chat-data",
        tier: Tier::Keep,
        category: Category::Userdata,
        what: format!("{app}的聊天数据（聊天记录、收到的文件、图片和视频）"),
        why: format!("{app}把每个账号的聊天记录和收到的文件都存在「{}」这个文件夹里", segs[idx]),
        lose: "删了聊天记录和收到的文件都会丢，手机上没有、也没有漫游的就再也找不回来。".into(),
        how: vec![How::Migrate],
        how_detail: format!("在{app}的“设置 → 存储空间 / 文件管理 / 存储管理”里改存储位置，让它自己搬到别的盘；想腾空间用同一页面里的“清理”按钮，不要在资源管理器里直接删。"),
        confidence: 0.7,
        owner: Some(app.to_string()),
        clues: vec![format!("路径里有「{}」这一层（{data_dir}），这是{app}存聊天数据的文件夹名", segs[idx])],
    })
}

fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// Unix 秒 → YYYY-MM-DD（UTC，够用）。
pub fn ymd(unix: i64) -> String {
    if unix <= 0 {
        return "未知".into();
    }
    // Howard Hinnant 的 civil_from_days。
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// “已经 N 个月没动过”（不到 180 天返回 None）。
pub fn stale_note(newest: i64, now: i64) -> Option<String> {
    if newest <= 0 || now <= newest {
        return None;
    }
    let days = (now - newest) / 86_400;
    if days < STALE_DAYS {
        return None;
    }
    let months = days / 30;
    Some(if months >= 24 {
        format!("已经 {} 年多没动过了（最近一次修改在 {}）", months / 12, ymd(newest))
    } else {
        format!("已经 {months} 个月没动过了（最近一次修改在 {}）", ymd(newest))
    })
}

/// 缓存类（规则或启发式判定 category = cache）超过 180 天没动：提示“很久没用”。
pub fn long_unused_note(newest: i64, now: i64) -> Option<String> {
    stale_note(newest, now).map(|s| format!("很久没用了：{s}。对应的软件可能已经不怎么用，甚至已经卸载了。"))
}

/// 子树里最新的修改时间，最多看 `limit` 个节点（给规则命中的缓存算“很久没用”用，比 [`subtree_stats`] 轻）。
pub fn newest_mtime(node: &Node, limit: usize) -> i64 {
    let mut newest = 0i64;
    let mut stack: Vec<&Node> = vec![node];
    let mut visited = 0usize;
    while let Some(n) = stack.pop() {
        visited += 1;
        if visited > limit {
            break;
        }
        newest = newest.max(n.mtime);
        if n.kind == NodeKind::Dir {
            stack.extend(n.children.iter());
        }
    }
    newest
}

fn ext_summary(s: &SubtreeStats, n: usize) -> String {
    if s.bytes_seen == 0 {
        return "没有看到普通文件".into();
    }
    s.ext
        .iter()
        .take(n)
        .map(|(e, b, c)| {
            let name = if e.is_empty() { "无扩展名".to_string() } else { format!(".{e}") };
            format!("{name} {}（{} 个）", pct(*b as f64 / s.bytes_seen as f64), c)
        })
        .collect::<Vec<_>>()
        .join("、")
}

fn top_children_summary(n: &Node, k: usize) -> String {
    let v: Vec<String> = n
        .children
        .iter()
        .take(k)
        .map(|c| {
            let nm = if c.kind == NodeKind::Rest { "其余小文件".to_string() } else if c.kind == NodeKind::Dir { format!("{}\\", c.name) } else { c.name.clone() };
            format!("{nm}（{}）", human_bytes(c.allocated))
        })
        .collect();
    v.join("、")
}

// ───────────────────────── 结果构造 ─────────────────────────

struct Hint {
    name: &'static str,
    tier: Tier,
    category: Category,
    what: String,
    why: String,
    lose: String,
    how: Vec<How>,
    how_detail: String,
    confidence: f32,
    owner: Option<String>,
    clues: Vec<String>,
}

/// 统一收尾：档位兜底（只有 [`SAFE_HEURISTICS`] 且不在个人文件夹下才能 safe）、能腾多少、年龄提示。
fn finish(h: Hint, t: &Target, stats: &SubtreeStats, ctx: &ExplainCtx) -> Explanation {
    let protected = ctx.is_protected(&split_lower(t.path));
    let safe_ok = h.tier == Tier::Safe && SAFE_HEURISTICS.contains(&h.name) && !protected;
    let tier = if h.tier == Tier::Safe && !safe_ok { Tier::Review } else { h.tier };
    let (gain_bytes, gain_note) = gain(tier, &h.how, t.node.allocated);
    let mut notes = Vec::new();
    if tier == Tier::Safe {
        notes.push("规则库里没有这一条，是 DiskWhy 看文件夹内容认出来的：名字和里面每个文件的名字都是 Chromium 磁盘缓存的标准格式，这种缓存删了程序会自己重建。".to_string());
    } else if h.tier == Tier::Safe && protected {
        notes.push("看内容像是能放心删的缓存，但它在桌面 / 文档 / 下载 / OneDrive 等个人文件夹下面，为稳妥起见标成“需要你判断”。".to_string());
    } else if h.tier == Tier::Keep {
        notes.push("这是 DiskWhy 根据文件夹里的内容推测的，不是规则库里核实过的说法；因为删错的代价很大，按“建议保留”处理。".to_string());
    } else {
        notes.push("这是 DiskWhy 根据文件夹里的内容推测的，不是规则库里核实过的说法，所以最多只标“需要你判断”。".to_string());
    }
    let newest = if stats.newest_mtime > 0 { stats.newest_mtime } else { t.node.mtime };
    let age = if h.category == Category::Cache { long_unused_note(newest, ctx.now) } else { stale_note(newest, ctx.now) };
    if let Some(s) = age {
        notes.push(s);
    }
    let owner = h.owner.or_else(|| owner_from_path(t.path, ctx).filter(|o| o.installed).map(|o| o.label()));
    Explanation {
        tier,
        owner,
        category: h.category,
        what: h.what,
        why: h.why,
        lose: h.lose,
        how: h.how,
        how_detail: h.how_detail,
        gain_bytes,
        gain_note,
        source: Source::Heuristic { name: h.name.to_string() },
        confidence: h.confidence,
        refs: Vec::new(),
        verified_on: None,
        notes,
        clues: h.clues,
    }
}

// ───────────────────────── 入口 ─────────────────────────

/// 规则没命中时调用：签名 → 注册表归属 → 未知。
pub fn explain_unmatched(t: &Target, ctx: &ExplainCtx) -> Explanation {
    let stats = subtree_stats(t.node);
    if let Some(h) = signature(t, &stats, ctx) {
        return finish(h, t, &stats, ctx);
    }
    if let Some(h) = registry_owner(t, ctx) {
        return finish(h, t, &stats, ctx);
    }
    unknown(t, &stats, ctx)
}

/// 目录内容签名，按固定顺序尝试，第一个命中的算数。
fn signature(t: &Target, s: &SubtreeStats, ctx: &ExplainCtx) -> Option<Hint> {
    let n = t.node;
    let name = n.name.to_lowercase();
    let is_dir = n.kind == NodeKind::Dir;

    // 0. 聊天软件的数据（用户把存储位置改到别处时，规则库按 %变量% 认不出来）：keep + 迁移。
    if let Some(h) = chat_data(t, ctx) {
        return Some(h);
    }

    // 1. Chromium 类缓存。签名严格（名字 + 每个文件名都对得上）→ safe；只是“像”→ review。
    if is_dir {
        if let Some(clue) = strict_chromium_cache(t) {
            return Some(Hint {
                name: "chromium-cache",
                tier: Tier::Safe,
                category: Category::Cache,
                what: "某个用 Chromium 内核的程序（浏览器或 Electron 应用）的缓存".into(),
                why: "程序打开网页或界面时把图片、脚本、显卡着色器等存一份，下次打开更快".into(),
                lose: "不丢聊天记录、文档、登录状态或设置；删掉后程序第一次打开会慢一点，缓存会自己重新长出来。程序开着时部分文件删不掉，跳过即可。".into(),
                how: vec![How::Delete],
                how_detail: "先退出对应的程序（托盘里也要退），再删这个文件夹；程序下次启动会自动重建。".into(),
                confidence: 0.75,
                owner: None,
                clues: vec![clue],
            });
        }
        let new_fmt = child(n, "Cache_Data").is_some_and(|c| has_child(c, "index"));
        let old_fmt = has_child(n, "index") && (has_child(n, "data_0") || has_child(n, "data_1"));
        if new_fmt || old_fmt {
            return Some(Hint {
                name: "chromium-cache-mixed",
                tier: Tier::Review,
                category: Category::Cache,
                what: "看起来是某个用 Chromium 内核的程序（浏览器或 Electron 应用）的网页缓存，但里面还混着别的东西".into(),
                why: "程序打开网页或界面时把图片、脚本等存一份，下次打开更快".into(),
                lose: "缓存部分删了只是下次打开慢一点；但这个文件夹的名字或内容和标准的 Chromium 缓存不完全一样，混着的那些文件是什么 DiskWhy 说不准，所以没标“可放心清理”。".into(),
                how: vec![How::Delete],
                how_detail: "点进去看看：Cache_Data、GPUCache、Code Cache 这些子文件夹一般可以删；别的先弄清楚再说。删之前先退出对应的程序。".into(),
                confidence: 0.6,
                owner: None,
                clues: vec![if new_fmt { "里面有 Cache_Data\\index（Chromium 新版缓存格式），但文件夹名或其他内容对不上标准格式".into() } else { "里面有 index 和 data_0/data_1（Chromium 老版缓存格式），但还有别的文件".into() }],
            });
        }
    }

    // 2 & 5. Python 环境 / 虚拟环境。
    if is_dir {
        let has_cfg = has_child(n, "pyvenv.cfg");
        let named_venv = name == ".venv" || name == "venv";
        let site = name == "site-packages"
            || has_child(n, "site-packages")
            || child(n, "Lib").is_some_and(|l| has_child(l, "site-packages"))
            || child(n, "lib").is_some_and(|l| l.children.iter().any(|c| c.name.to_lowercase().starts_with("python") && has_child(c, "site-packages")));
        let conda = has_child(n, "conda-meta");
        // conda 的 envs 目录（或其中一个环境）装在规则库不认识的位置（比如 D:\conda\envs）：和规则 conda-envs 一样 keep + 迁移。
        let parent_is_envs = t.parent.is_some_and(|p| p.name.eq_ignore_ascii_case("envs"));
        let envs_root = name == "envs" && sibling_exists(t, "conda-meta");
        if (conda && parent_is_envs) || envs_root {
            return Some(Hint {
                name: "conda-env",
                tier: Tier::Keep,
                category: Category::Userdata,
                what: if envs_root { "conda 的环境目录（每个子文件夹是一套独立的 Python 和库）".into() } else { format!("conda 环境「{}」（一套独立的 Python 和库）", n.name) },
                why: "给不同项目准备的运行环境，用 conda create 建出来的".into(),
                lose: "删了环境里的库和配置都没了，用它的项目跑不起来，要照原样重建（很费时，版本还不一定装得回来）。".into(),
                how: vec![How::Migrate],
                how_detail: "要迁到别的盘：先 conda env export -n 环境名 > env.yml 备份，在 .condarc 的 envs_dirs 里加新位置，再 conda env create -f env.yml 重建，确认能用后 conda env remove -n 旧环境。不用的环境用 conda env remove -n 环境名 删，不要直接删文件夹。".into(),
                confidence: 0.7,
                owner: Some("conda（Anaconda / Miniconda）".into()),
                clues: vec![if envs_root { "文件夹叫 envs，旁边有 conda-meta（conda 安装目录的标志）".into() } else { "里面有 conda-meta，上一层文件夹叫 envs".into() }],
            });
        }
        if has_cfg || (named_venv && site) {
            return Some(Hint {
                name: "python-venv",
                tier: Tier::Review,
                category: Category::Build,
                what: "Python 虚拟环境（某个项目单独装的一套 Python 库）".into(),
                why: "开发者给每个项目单独装一套依赖，避免互相冲突".into(),
                lose: "删了这个项目就跑不起来，要重新创建虚拟环境并重装依赖（pip install -r requirements.txt 之类），花时间但一般不丢你写的代码。不确定是谁的就别删。".into(),
                how: vec![How::Delete],
                how_detail: "确认对应项目不用了再删；要用时在项目目录里重新创建（python -m venv .venv 或 uv venv）并重装依赖。".into(),
                confidence: 0.75,
                owner: None,
                clues: vec![if has_cfg { "里面有 pyvenv.cfg".into() } else { format!("文件夹名是 {}，里面有 site-packages", n.name) }],
            });
        }
        if site || conda {
            return Some(Hint {
                name: "python-env",
                tier: Tier::Review,
                category: Category::App,
                what: if conda { "一套 conda / Python 环境（Python 解释器加一堆装好的库）".into() } else { "一套 Python 环境（Python 解释器或装好的库）".into() },
                why: "装 Python 或某些自带 Python 的软件时生成，程序运行要用".into(),
                lose: "删了依赖这套环境的程序和脚本会跑不起来，要重新安装；一般不含你的文档，但可能有你自己装的库和配置。".into(),
                how: vec![How::Manual],
                how_detail: "如果是 conda 环境，用 conda env remove -n 环境名 删除；如果是某个软件自带的，卸载那个软件。".into(),
                confidence: 0.6,
                owner: None,
                clues: vec![if conda { "里面有 conda-meta（conda 环境的标志）".into() } else { "里面有 site-packages（Python 装库的地方）".into() }],
            });
        }
    }

    // 2b. 在某个 site-packages 里面（Python 环境里装的库）。
    if let Some(i) = first_segment(t.path, "site-packages").filter(|&i| i + 1 < seg_count(t.path)) {
        let env_dir = prefix_path(t.path, i);
        return Some(Hint {
            name: "python-env",
            tier: Tier::Review,
            category: Category::App,
            what: "某个 Python 环境里装的库".into(),
            why: "pip / conda 装库时放在 site-packages 里，程序运行要用".into(),
            lose: "单独删其中一个库，用到它的程序会报错；想腾空间应该删整个环境，或者用 pip uninstall 卸载库。".into(),
            how: vec![How::Manual],
            how_detail: "用 pip uninstall 包名 卸载，或整个环境不用了再删整个环境。".into(),
            confidence: 0.6,
            owner: None,
            clues: vec![format!("在 {env_dir} 里面")],
        });
    }

    // 3. node_modules：本身叫 node_modules，或者在某个 node_modules 里面。提示是哪个项目的。
    if let Some(i) = first_segment(t.path, "node_modules") {
        let project = prefix_path(t.path, i);
        let project_name = project.trim_end_matches('\\').rsplit('\\').next().unwrap_or(&project).to_string();
        let own = i + 1 == seg_count(t.path);
        let has_pkg = if own { sibling_exists(t, "package.json") } else { Path::new(&project).join("package.json").exists() };
        let (hname, what, clue, conf) = if has_pkg {
            (
                "node-modules",
                if own { format!("项目「{project_name}」下载的 JavaScript 依赖包") } else { format!("项目「{project_name}」的 JavaScript 依赖包里的一部分") },
                format!("在 node_modules 里；对应的项目是 {project}（那里有 package.json）"),
                0.75,
            )
        } else {
            (
                "node-modules-orphan",
                format!("一个 JavaScript 依赖包文件夹（node_modules），原来应该属于「{project_name}」"),
                format!("在 {project} 下面，但那里没有 package.json：对应的项目可能已经删了或挪走了，这可能是遗留下来的"),
                0.5,
            )
        };
        return Some(Hint {
            name: hname,
            tier: Tier::Review,
            category: Category::Build,
            what,
            why: "运行 npm / pnpm / yarn install 时把项目需要的包全部下载到这里".into(),
            lose: "删了对应项目暂时跑不起来，在项目目录里重新 npm install 就能装回来（要联网、要等一会儿），不丢你写的代码。".into(),
            how: vec![How::Delete],
            how_detail: format!("确认项目不在运行，删掉整个 {project}\\node_modules（不要只删一部分）；要用时在项目目录执行 npm install（或 pnpm install / yarn）。"),
            confidence: conf,
            owner: Some(format!("项目「{project_name}」（{project}）")),
            clues: vec![clue],
        });
    }

    // 4. Rust target：本身叫 target 或在 target 里面，且 target 旁边有 Cargo.toml。
    for i in segment_indices(t.path, "target") {
        let project = prefix_path(t.path, i);
        let own = i + 1 == seg_count(t.path);
        let has_cargo = if own { sibling_exists(t, "Cargo.toml") } else { Path::new(&project).join("Cargo.toml").exists() };
        if has_cargo {
            let project_name = project.trim_end_matches('\\').rsplit('\\').next().unwrap_or(&project).to_string();
            return Some(Hint {
                name: "rust-target",
                tier: Tier::Review,
                category: Category::Build,
                what: if own { format!("Rust 项目「{project_name}」的编译产物") } else { format!("Rust 项目「{project_name}」编译产物的一部分") },
                why: "cargo build / cargo test 编译时生成的中间文件和程序".into(),
                lose: "删了只是下次要重新编译（可能要几分钟到几十分钟），不丢源代码。".into(),
                how: vec![How::Delete],
                how_detail: format!("在 {project} 里执行 cargo clean，或直接删整个 target 文件夹。"),
                confidence: 0.8,
                owner: Some(format!("项目「{project_name}」（{project}）")),
                clues: vec![format!("在 target 文件夹里，旁边有 Cargo.toml（项目在 {project}）")],
            });
        }
    }

    // 6. .NET bin/obj（旁边有 *.csproj，且 bin 和 obj 同时存在）。
    if is_dir && (name == "bin" || name == "obj") {
        let other = if name == "bin" { "obj" } else { "bin" };
        if sibling_exists(t, other) {
            if let Some(proj) = sibling_with_ext(t, &["csproj", "fsproj", "vbproj"]) {
                return Some(Hint {
                    name: "dotnet-build",
                    tier: Tier::Review,
                    category: Category::Build,
                    what: ".NET 项目的编译产物".into(),
                    why: "dotnet build / Visual Studio 编译时生成".into(),
                    lose: "删了下次要重新编译，不丢源代码。".into(),
                    how: vec![How::Delete],
                    how_detail: "在项目目录执行 dotnet clean，或删掉 bin 和 obj 两个文件夹。".into(),
                    confidence: 0.75,
                    owner: None,
                    clues: vec![format!("bin 和 obj 同时存在，旁边有 {proj}")],
                });
            }
        }
    }

    // 7. 模型权重。
    let model_share = if n.kind == NodeKind::File { if MODEL_EXTS.contains(&ext_of(&n.name).as_str()) { 1.0 } else { 0.0 } } else { s.share(MODEL_EXTS) };
    let model_big = if n.kind == NodeKind::File { n.allocated } else { s.biggest_with(MODEL_EXTS) };
    if model_share >= 0.6 && model_big > MODEL_MIN_FILE {
        return Some(Hint {
            name: "model-weights",
            tier: Tier::Review,
            category: Category::Model,
            what: "看起来是 AI 模型文件（模型权重）".into(),
            why: "AI 工具（本地大模型、画图、语音、识别等）运行时要读取这些文件".into(),
            lose: "删了用到它的 AI 工具要重新下载，文件很大、下载很慢，有的还要重新配置；不确定是哪个工具的就先别删。".into(),
            how: vec![How::Migrate, How::Delete],
            how_detail: "不常用可以删，要用时再下载；常用的话建议在对应工具里把模型目录改到别的盘（很多工具支持环境变量或设置项）。".into(),
            confidence: 0.6,
            owner: None,
            clues: vec![format!("模型类文件（.safetensors/.gguf/.onnx/.pt/.bin 等）占 {}，最大的单个 {}", pct(model_share), human_bytes(model_big))],
        });
    }

    // 8. 虚拟磁盘。
    let vd_share = if n.kind == NodeKind::File { if VDISK_EXTS.contains(&ext_of(&n.name).as_str()) { 1.0 } else { 0.0 } } else { s.share(VDISK_EXTS) };
    if vd_share >= 0.5 {
        return Some(Hint {
            name: "virtual-disk",
            tier: Tier::Keep,
            category: Category::Vm,
            what: "虚拟磁盘文件（一整台虚拟机或 Linux 子系统的“硬盘”）".into(),
            why: "虚拟机、WSL、Docker 等把整个系统和里面的数据装在这种文件里".into(),
            lose: "删了等于把那台虚拟机/子系统连同里面所有文件一起删掉，无法恢复。".into(),
            how: vec![How::Migrate],
            how_detail: "在对应软件里把它迁到别的盘（WSL 用 wsl --export / --import，Docker Desktop 在设置里改磁盘位置，Hyper-V 用“移动”向导）；不用了就在对应软件里删除那台虚拟机。".into(),
            confidence: 0.7,
            owner: None,
            clues: vec![format!("虚拟磁盘文件（.vhdx/.vhd/.vmdk/.qcow2）占 {}", pct(vd_share))],
        });
    }

    // 9. 日志 / 转储。
    let log_share = if n.kind == NodeKind::File { if LOG_EXTS.contains(&ext_of(&n.name).as_str()) { 1.0 } else { 0.0 } } else { s.share(LOG_EXTS) };
    if log_share > 0.7 {
        return Some(Hint {
            name: "logs-dumps",
            tier: Tier::Review,
            category: Category::Log,
            what: "日志或崩溃转储文件".into(),
            why: "程序记录运行过程或崩溃现场，方便排查问题".into(),
            lose: "一般只是丢掉排查问题的记录，程序照常用；如果正在找厂商排查故障，先别删。".into(),
            how: vec![How::Delete],
            how_detail: "关掉对应程序后删除；正在写入的日志会删不掉，跳过即可。".into(),
            confidence: 0.6,
            owner: None,
            clues: vec![format!(".log/.etl/.dmp 占 {}", pct(log_share))],
        });
    }

    // 10. 下载目录里的安装包。
    if let Some(home) = ctx.env.get("USERPROFILE") {
        let dl = split_lower(&format!("{home}\\Downloads"));
        let segs = split_lower(t.path);
        if segs.len() > dl.len() && segs[..dl.len()] == dl[..] {
            let inst_share =
                if n.kind == NodeKind::File { if INSTALLER_EXTS.contains(&ext_of(&n.name).as_str()) { 1.0 } else { 0.0 } } else { s.share(INSTALLER_EXTS) };
            if inst_share > 0.5 {
                return Some(Hint {
                    name: "downloaded-installer",
                    tier: Tier::Review,
                    category: Category::Installer,
                    what: "下载下来的安装包 / 系统镜像".into(),
                    why: "你（或浏览器）下载软件时保存的安装文件".into(),
                    lose: "软件装好后安装包一般就用不上了；删了以后想重装得重新下载。个别安装包是现在很难再下到的旧版本，先确认。".into(),
                    how: vec![How::Delete],
                    how_detail: "确认对应软件已经装好、以后不需要再装，删掉即可（默认进回收站）。".into(),
                    confidence: 0.6,
                    owner: None,
                    clues: vec![format!("在“下载”文件夹里，.iso/.msi/.exe 占 {}", pct(inst_share))],
                });
            }
        }
    }

    // 11. 临时文件。
    let tmp_share = if n.kind == NodeKind::File {
        if ext_of(&n.name) == "tmp" { 1.0 } else { 0.0 }
    } else {
        s.share(&["tmp"]).max(s.count_share(&["tmp"]))
    };
    if tmp_share > 0.5 {
        return Some(Hint {
            name: "temp-files",
            tier: Tier::Review,
            category: Category::Cache,
            what: "临时文件".into(),
            why: "程序运行或安装过程中产生的中间文件，本该用完就删，但经常被遗留".into(),
            lose: "通常不丢东西；正在运行的程序可能还在用其中一部分（这种会删不掉，跳过即可）。".into(),
            how: vec![How::Delete],
            how_detail: "关掉相关程序后删除；删不掉的说明还在用，跳过。".into(),
            confidence: 0.55,
            owner: None,
            clues: vec![format!(".tmp 文件占 {}", pct(tmp_share))],
        });
    }

    // 12.（附加）媒体文件：视频 / 图片 / 音频占大头。
    let media_share = if n.kind == NodeKind::File { if MEDIA_EXTS.contains(&ext_of(&n.name).as_str()) { 1.0 } else { 0.0 } } else { s.share(MEDIA_EXTS) };
    if media_share > 0.7 && t.node.allocated > 10 * 1024 * 1024 {
        let top: Vec<String> = s.ext.iter().take(3).filter(|x| !x.0.is_empty()).map(|x| format!(".{}", x.0)).collect();
        return Some(Hint {
            name: "media-files",
            tier: Tier::Review,
            category: Category::Userdata,
            what: "一堆视频 / 图片 / 音频文件".into(),
            why: "可能是你自己保存的，也可能是某个软件自动录制或缓存的（比如游戏精彩时刻、直播回放、聊天里收到的图片视频）".into(),
            lose: "如果是你自己的照片视频，删了就没了；如果是软件的缓存，删了可能要重新加载。DiskWhy 分不清是哪种，请先打开看看。".into(),
            how: vec![How::Manual],
            how_detail: "打开文件夹看一眼：是自己要的就迁到别的盘；是软件自动生成、已经不需要的，再删。".into(),
            confidence: 0.5,
            owner: None,
            clues: vec![format!("媒体文件占 {}（主要是 {}）", pct(media_share), top.join("、"))],
        });
    }
    None
}

// ───────────────────────── 注册表归属 ─────────────────────────

/// 推断出的归属。
#[derive(Debug, Clone)]
pub struct OwnerGuess {
    pub name: String,
    pub publisher: String,
    /// 在注册表 Uninstall 里找到了。
    pub installed: bool,
    /// 怎么推断的：安装目录 / 名字 / 发布者 / 文件夹名。
    pub via: &'static str,
}

impl OwnerGuess {
    pub fn label(&self) -> String {
        if self.installed && !self.publisher.is_empty() {
            format!("{}（发布者 {}）", self.name, self.publisher)
        } else {
            self.name.clone()
        }
    }
}

fn under(segs: &[String], base: &[String]) -> bool {
    segs.len() > base.len() && segs[..base.len()] == base[..]
}

/// 名字匹配：规范化后相等，或一方以另一方开头（短的一方至少 4 个字符）。
pub fn names_match(folder_norm: &str, app_norm: &str) -> bool {
    if folder_norm.is_empty() || app_norm.is_empty() {
        return false;
    }
    if folder_norm == app_norm {
        return true;
    }
    let (short, long) = if folder_norm.chars().count() <= app_norm.chars().count() { (folder_norm, app_norm) } else { (app_norm, folder_norm) };
    short.chars().count() >= 4 && long.starts_with(short)
}

/// 从路径推断“这是哪个软件的”：先看注册表安装目录，再看 AppData / ProgramData / Program Files 下的第一层文件夹名。
/// 找不到注册表记录时，返回文件夹名本身（`installed = false`）。
pub fn owner_from_path(path: &str, ctx: &ExplainCtx) -> Option<OwnerGuess> {
    let segs = split_lower(path);
    // 1. 落在某个安装目录之下（取最长的那个）。
    let mut best: Option<(&InstalledApp, usize)> = None;
    for a in &ctx.registry.apps {
        if a.location_norm.is_empty() {
            continue;
        }
        let base = split_lower(&a.location_norm);
        if (segs.len() >= base.len() && segs[..base.len()] == base[..]) && best.is_none_or(|b| base.len() > b.1) {
            best = Some((a, base.len()));
        }
    }
    if let Some((a, _)) = best {
        return Some(OwnerGuess { name: a.display_name.clone(), publisher: a.publisher.clone(), installed: true, via: "安装目录" });
    }
    // 2. 容器目录下的前几层文件夹名（比如 ProgramData\NVIDIA Corporation\NVIDIA App）。
    let folders = app_folder_names(&segs, ctx)?;
    let folder1 = folders[0].clone();
    let original = original_segment(path, &folder1);
    let cands: Vec<String> = folders.iter().map(|f| normalize_name(f)).filter(|c| c.chars().count() >= 2).collect();
    //   a) 规范化后完全相等（任何一层）——最可靠；
    for c in &cands {
        if let Some(a) = ctx.registry.apps.iter().find(|a| a.name_norm == *c) {
            return Some(OwnerGuess { name: a.display_name.clone(), publisher: a.publisher.clone(), installed: true, via: "名字" });
        }
    }
    //   b) 前缀匹配，但只接受唯一的那个（“NVIDIA”能前缀匹配一堆 NVIDIA 软件，就不猜具体是哪个）。
    for c in &cands {
        let hits: Vec<&InstalledApp> = ctx.registry.apps.iter().filter(|a| names_match(c, &a.name_norm)).collect();
        let distinct: HashSet<&str> = hits.iter().map(|a| a.name_norm.as_str()).collect();
        if distinct.len() == 1 {
            let a = hits[0];
            return Some(OwnerGuess { name: a.display_name.clone(), publisher: a.publisher.clone(), installed: true, via: "名字" });
        }
    }
    let c1 = normalize_name(&folder1);
    if c1.chars().count() >= 3 {
        let pubs: Vec<&InstalledApp> = ctx.registry.apps.iter().filter(|a| names_match(&c1, &a.publisher_norm)).collect();
        if let Some(first) = pubs.first() {
            if pubs.len() == 1 {
                return Some(OwnerGuess { name: first.display_name.clone(), publisher: first.publisher.clone(), installed: true, via: "发布者" });
            }
            // 同一发布者装了好几个软件：只说到厂商，不猜具体是哪个。
            return Some(OwnerGuess { name: format!("{} 家的软件", first.publisher), publisher: String::new(), installed: true, via: "发布者（多个）" });
        }
    }
    Some(OwnerGuess { name: original, publisher: String::new(), installed: false, via: "文件夹名" })
}

/// 在路径里找回某个小写段的原始大小写。
fn original_segment(path: &str, lower: &str) -> String {
    path.split(['\\', '/']).find(|s| s.to_lowercase() == *lower).unwrap_or(lower).to_string()
}

/// 软件数据常见的容器目录；返回其下最多三层文件夹名（小写，从浅到深）。
fn app_folder_names(segs: &[String], ctx: &ExplainCtx) -> Option<Vec<String>> {
    let bases: Vec<(Vec<String>, bool)> = [
        ("LOCALAPPDATA", "Programs", false),
        ("LOCALAPPDATA", "Packages", true),
        ("LOCALAPPDATA", "", false),
        ("APPDATA", "", false),
        ("USERPROFILE", "AppData\\LocalLow", false),
        ("ProgramData", "", false),
        ("ProgramFiles", "", false),
        ("ProgramFiles(x86)", "", false),
    ]
    .iter()
    .filter_map(|(var, sub, pkg)| {
        let v = ctx.env.get(var)?;
        let p = if sub.is_empty() { v.to_string() } else { format!("{v}\\{sub}") };
        Some((split_lower(&p), *pkg))
    })
    .collect();
    for (base, is_pkg) in bases {
        if under(segs, &base) {
            let mut f1 = segs[base.len()].clone();
            if is_pkg {
                // 商店应用包名：Publisher.AppName_发布者哈希 → AppName
                f1 = f1.split('_').next().unwrap_or(&f1).to_string();
                if let Some((_, app)) = f1.rsplit_once('.') {
                    f1 = app.to_string();
                }
            }
            let mut v = vec![f1];
            v.extend(segs.iter().skip(base.len() + 1).take(2).cloned());
            return Some(v);
        }
    }
    None
}

fn registry_owner(t: &Target, ctx: &ExplainCtx) -> Option<Hint> {
    let g = owner_from_path(t.path, ctx)?;
    if !g.installed {
        return None;
    }
    let label = g.label();
    if g.via == "安装目录" {
        return Some(Hint {
            name: "registry-install-dir",
            tier: Tier::Review,
            category: Category::App,
            what: format!("已安装软件「{}」的安装目录（程序本体）", g.name),
            why: "安装软件时放程序文件的地方".into(),
            lose: format!("直接删文件夹会让「{}」打不开，卸载信息也会残留。想腾空间请正常卸载它。", g.name),
            how: vec![How::Manual],
            how_detail: format!("在“设置 > 应用 > 已安装的应用”里找到「{}」卸载，不要直接删文件夹。", g.name),
            confidence: 0.7,
            owner: Some(label),
            clues: vec!["路径落在注册表里登记的安装目录之下".into()],
        });
    }
    // 在 Program Files / AppData\Local\Programs 下：这是程序文件夹，不是“数据”。
    let segs = split_lower(t.path);
    let in_program_dir = [("ProgramFiles", ""), ("ProgramFiles(x86)", ""), ("LOCALAPPDATA", "\\Programs")].iter().any(|(v, sub)| {
        ctx.env.get(v).is_some_and(|base| under(&segs, &split_lower(&format!("{base}{sub}"))))
    });
    if in_program_dir {
        let who = if g.via == "发布者（多个）" { g.name.clone() } else { format!("已安装软件「{}」", g.name) };
        return Some(Hint {
            name: "registry-program-dir",
            tier: Tier::Review,
            category: Category::App,
            what: format!("看起来是{who}的程序文件夹"),
            why: "安装软件时放程序文件的地方".into(),
            lose: "直接删文件夹会让相关软件打不开，卸载信息也会残留。想腾空间请在系统设置里正常卸载。".into(),
            how: vec![How::Manual],
            how_detail: "在“设置 > 应用 > 已安装的应用”里按大小排序，卸载不用的软件，不要直接删文件夹。".into(),
            confidence: 0.5,
            owner: Some(label),
            clues: vec![format!("在程序安装目录下，文件夹名和注册表里已安装软件的{}对得上", if g.via.starts_with("发布者") { "发布者" } else { "名字" })],
        });
    }
    if g.via == "发布者（多个）" {
        let publisher = g.name.trim_end_matches(" 家的软件").to_string();
        let mut names: Vec<&str> =
            ctx.registry.apps.iter().filter(|a| a.publisher.trim() == publisher).map(|a| a.display_name.as_str()).collect();
        names.sort();
        names.dedup();
        return Some(Hint {
            name: "registry-appdata",
            tier: Tier::Review,
            category: Category::Userdata,
            what: format!("看起来是「{publisher}」家某个软件存放数据的地方"),
            why: "这家厂商的软件把设置、缓存、下载的更新等放在这里".into(),
            lose: "这家装了好几个软件，DiskWhy 分不清是哪一个的，也分不清里面哪些是缓存。直接删可能让其中某个软件出问题。".into(),
            how: vec![How::Manual],
            how_detail: "点进去看子文件夹名，通常能看出是哪个软件；用那个软件自己的清理功能，或卸载不用的软件。".into(),
            confidence: 0.4,
            owner: Some(g.name.clone()),
            clues: vec![format!("文件夹名和注册表里的发布者「{publisher}」对得上；这家装了：{}", names.iter().take(6).copied().collect::<Vec<_>>().join("、"))],
        });
    }
    Some(Hint {
        name: "registry-appdata",
        tier: Tier::Review,
        category: Category::Userdata,
        what: format!("看起来是已安装软件「{}」存放数据的地方", g.name),
        why: "软件把设置、登录状态、缓存、下载的内容等放在这里".into(),
        lose: "里面可能混着缓存（能删）和设置、登录信息、你在这个软件里的文件（不能删），DiskWhy 分不清。直接整个删掉，这个软件可能丢设置、要重新登录，甚至丢数据。".into(),
        how: vec![How::Manual],
        how_detail: format!("优先用「{}」自己的“清理缓存 / 存储管理”功能；或者点进去看看哪些子文件夹是缓存。", g.name),
        confidence: 0.5,
        owner: Some(label),
        clues: vec![format!("文件夹名和注册表里已安装软件的{}对得上", if g.via == "发布者" { "发布者" } else { "名字" })],
    })
}

// ───────────────────────── 未知 ─────────────────────────

/// 完全认不出来：列出线索。
pub fn unknown(t: &Target, s: &SubtreeStats, ctx: &ExplainCtx) -> Explanation {
    let n = t.node;
    let mut clues = Vec::new();
    if n.kind == NodeKind::Dir && !n.children.is_empty() {
        clues.push(format!("最大的几个子项：{}", top_children_summary(n, 5)));
    }
    if s.bytes_seen > 0 {
        clues.push(format!("主要文件类型（按大小）：{}{}", ext_summary(s, 5), if s.truncated { "（内容太多，只看了一部分）" } else { "" }));
    }
    if n.kind == NodeKind::Dir {
        clues.push(format!("一共 {} 个文件、{} 个文件夹", n.files, n.dirs));
    }
    let newest = if s.newest_mtime > 0 { s.newest_mtime } else { n.mtime };
    match stale_note(newest, ctx.now) {
        Some(x) => clues.push(x),
        None if newest > 0 => clues.push(format!("最近一次修改在 {}", ymd(newest))),
        None => {}
    }
    let segs = split_lower(t.path);
    let in_appdata = segs.iter().any(|x| x == "appdata");
    clues.push(if in_appdata {
        "在 AppData 下面：一般是某个软件自己存的数据（缓存、设置或下载的内容），不是你手动放的文件".into()
    } else if segs.get(1).is_some_and(|x| x == "users") {
        "在用户文件夹里，但不在 AppData 下：可能是你自己放的，也可能是某个工具建的".into()
    } else {
        "不在用户文件夹里".into()
    });
    if let Some(g) = owner_from_path(t.path, ctx) {
        if !g.installed {
            clues.push(format!("所在的软件文件夹叫「{}」，但注册表“已安装的应用”里找不到对得上的软件（可能是绿色版、已卸载的残留，或者名字对不上）", g.name));
        }
    }
    let how = vec![How::Manual];
    Explanation {
        tier: Tier::Review,
        owner: None,
        category: Category::Unknown,
        what: "我也不确定这是什么".into(),
        why: "规则库里没有它，内容上也没看出明显特征。".into(),
        lose: "不知道。在弄清楚之前请不要删——可以根据下面的线索搜一下文件夹名，或者点“问 AI”让 AI 猜一猜。".into(),
        how,
        how_detail: "先弄清楚是哪个软件的：搜索文件夹名、看看里面的文件，或者问 AI。".into(),
        gain_bytes: 0,
        gain_note: format!("占 {}；弄清楚是什么之前不估算能腾多少", human_bytes(n.allocated)),
        source: Source::Unknown,
        confidence: 0.0,
        refs: Vec::new(),
        verified_on: None,
        notes: Vec::new(),
        clues,
    }
}
