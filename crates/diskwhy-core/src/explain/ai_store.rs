//! AI 自动识别（2026-10-11 用户决定「免费智谱 + 自动识别」）：扫描完，DiskWhy 自己把认不出来的大文件夹 / 大文件
//! 问 AI，答案记在磁盘上（`%LOCALAPPDATA%\DiskWhy\ai-cache.json`），以后解释时直接带上，界面标「AI 推测」。
//!
//! 铁律：AI 的答案**只是附在解释上的一段说明**（[`super::Explanation::ai`] / `TreemapNode.ai`）——档位、来源、分类、
//! 后果标签、能腾多少、各档合计、清理候选、方块图颜色全都和“未知”时一模一样。AI 永远不能让任何东西变成可清理。
//!
//! - [`AiStore`]：路径（小写）→ [`AiNote`]，内存里一份、每次写入都落盘（先写临时文件再改名），最多 3000 条（按日期丢最旧的）；
//!   文件坏了 / 不存在就当空的，绝不 panic。
//! - [`ai_candidates`]：扫描树里值得问的项（≥ 门槛、解释是“未知”、还没问过），从大到小。
//! - 批量问的执行器在 [`super::ai_batch`]。

use super::pattern::split_lower;
use super::{explain_with_inherited, ExplainCtx, Source, Target};
use crate::fs::join;
use crate::model::{Node, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

/// AI 对一个文件夹 / 文件的推测（界面上标「AI 推测」）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiNote {
    /// 这是什么。
    pub what: String,
    /// 可能属于哪个软件（不知道为空）。
    pub owner: String,
    /// 为什么会有。
    pub why: String,
    /// 删了会失去什么（AI 没说时为空，界面用自己的兜底文字）。
    pub lose: String,
    /// 0~0.9（AI 推测永远不算“确认”）。
    pub confidence: f32,
    /// 回答的模型名。
    pub model: String,
    /// 问的日期 `YYYY-MM-DD`（UTC）。
    pub at: String,
    /// 问的时候它占多少（allocated 字节）。
    pub size: u64,
}

/// 缓存最多记多少条（超过时按 `at` 丢最旧的；同一天的先丢小的）。
pub const AI_STORE_CAP: usize = 3000;

/// AI 答案缓存。线程安全；`ExplainCtx.ai` 是 `Arc<AiStore>`，换设置时复制上下文也共用同一份。
#[derive(Debug, Default)]
pub struct AiStore {
    map: RwLock<HashMap<String, AiNote>>,
    /// 落盘位置；None = 只在内存里（测试）。
    path: Option<PathBuf>,
    /// 写文件串行化（两个线程同时 put 时不会互相覆盖临时文件）。
    save_lock: Mutex<()>,
}

fn key(path: &str) -> String {
    path.to_lowercase()
}

impl AiStore {
    /// 只在内存里（测试 / 没有 %LOCALAPPDATA% 时）。
    pub fn memory() -> Self {
        AiStore::default()
    }

    /// 从文件读；文件不存在、读不了、不是合法 JSON、版本不认识 → 空的（但以后 put 仍写到这个位置）。
    /// 开头的 UTF-8 BOM（记事本另存过）不碍事；某一条坏了只丢那一条，其余照读（不会因为一条坏了丢掉整份缓存）。
    pub fn load_from(path: &Path) -> Self {
        let items: Vec<(String, AiNote)> = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s.trim_start_matches('\u{feff}')).ok())
            .filter(|v| v.get("version").and_then(|x| x.as_u64()) == Some(1))
            .and_then(|mut v| match v.get_mut("items").map(serde_json::Value::take) {
                Some(serde_json::Value::Object(o)) => Some(o),
                _ => None,
            })
            .map(|o| o.into_iter().filter_map(|(k, v)| serde_json::from_value::<AiNote>(v).ok().map(|n| (k, n))).collect())
            .unwrap_or_default();
        let mut map: HashMap<String, AiNote> = HashMap::with_capacity(items.len());
        for (k, v) in items {
            if !k.trim().is_empty() && !v.what.trim().is_empty() {
                map.insert(key(&k), v);
            }
        }
        trim_to_cap(&mut map, AI_STORE_CAP, None);
        AiStore { map: RwLock::new(map), path: Some(path.to_path_buf()), save_lock: Mutex::new(()) }
    }

    /// `%LOCALAPPDATA%\DiskWhy\ai-cache.json`；找不到 %LOCALAPPDATA% 时只在内存里。
    pub fn load_default() -> Self {
        match Self::default_path() {
            Some(p) => Self::load_from(&p),
            None => Self::memory(),
        }
    }

    pub fn default_path() -> Option<PathBuf> {
        super::llm::app_data_dir().map(|d| d.join("ai-cache.json"))
    }

    /// 落盘位置（只在内存里时 None）。
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 按完整路径取（大小写不敏感）。
    pub fn get(&self, path: &str) -> Option<AiNote> {
        self.map.read().unwrap_or_else(|e| e.into_inner()).get(&key(path)).cloned()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.map.read().unwrap_or_else(|e| e.into_inner()).contains_key(&key(path))
    }

    pub fn len(&self) -> usize {
        self.map.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 记一条（同一路径覆盖旧的），超过上限时丢最旧的，然后落盘。
    /// 内存里一定记上；落盘失败返回 Err（调用方记日志即可，不影响这次使用）。
    pub fn put(&self, path: &str, note: AiNote) -> Result<(), String> {
        let k = key(path);
        {
            let mut m = self.map.write().unwrap_or_else(|e| e.into_inner());
            m.insert(k.clone(), note);
            trim_to_cap(&mut m, AI_STORE_CAP, Some(&k));
        }
        self.save()
    }

    /// 把当前内容写到磁盘：先写 `<文件>.<进程号>-<序号>.tmp` 并刷到盘上（`sync_all`），再改名覆盖——写一半断电不会留下半个文件，
    /// 也不会出现“改名生效了、内容还没落盘”的空文件。临时文件名每次都不一样：同时开着两个 DiskWhy 时不会互相写坏对方的临时文件。
    pub fn save(&self) -> Result<(), String> {
        let Some(p) = &self.path else { return Ok(()) };
        let _g = self.save_lock.lock().unwrap_or_else(|e| e.into_inner());
        let text = {
            let m = self.map.read().unwrap_or_else(|e| e.into_inner());
            serde_json::to_string(&FileFormatRef { version: 1, items: &m }).map_err(|e| e.to_string())?
        };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{e}"))?;
        }
        static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
        let mut tmp = p.as_os_str().to_owned();
        tmp.push(format!(".{}-{}.tmp", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
        let tmp = PathBuf::from(tmp);
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("写 {} 失败：{e}", tmp.display())
        })?;
        std::fs::rename(&tmp, p).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("改名到 {} 失败：{e}", p.display())
        })
    }
}

/// 磁盘上的格式：`{"version":1,"items":{"<小写完整路径>":AiNote,…}}`（读的时候一条一条地认，见 [`AiStore::load_from`]）。
#[derive(Serialize)]
struct FileFormatRef<'a> {
    version: u32,
    items: &'a HashMap<String, AiNote>,
}

/// 超过 `cap` 条时按 (`at`, `size`) 从小到大丢（最旧的先丢，同一天的先丢小的）；`keep` 这条（刚写入的）不丢。
fn trim_to_cap(m: &mut HashMap<String, AiNote>, cap: usize, keep: Option<&str>) {
    if m.len() <= cap {
        return;
    }
    let mut ks: Vec<(String, u64, String)> =
        m.iter().filter(|(k, _)| Some(k.as_str()) != keep).map(|(k, v)| (v.at.clone(), v.size, k.clone())).collect();
    ks.sort();
    let extra = m.len() - cap;
    for (_, _, k) in ks.into_iter().take(extra) {
        m.remove(&k);
    }
}

/// 今天的日期 `YYYY-MM-DD`（UTC）。
pub fn today() -> String {
    super::heuristics::ymd(super::now_unix())
}

// ───────────────────────── 值得问 AI 的项 ─────────────────────────

/// 自动识别的默认门槛：≥ 100 MB。
pub const AI_MIN_BYTES: u64 = 100 * 1024 * 1024;
/// 自动识别一次最多问多少项。
pub const AI_MAX_ITEMS: usize = 40;

/// 节点本身能不能拿去问（无权限 / 系统保留 / 硬链接多余名字 / 映射 / 云占位 / 没跟随的链接都不问）。
fn askable(n: &Node) -> bool {
    n.kind != NodeKind::Rest
        && !n.inaccessible
        && !n.system_reserved
        && !n.hardlink_dup
        && !n.mapped
        && !n.cloud_placeholder
        && !n.reparse_skipped
}

/// 一项占它最近的“要问 / 问过的”上级的这么多（‰）以上，就当是同一样东西，不再单独问（见 [`ai_candidates`]）。
pub const AI_SAME_AS_PARENT_PERMILLE: u64 = 900;

/// 扫描树里值得问 AI 的项：文件夹和文件，占用 ≥ `min_bytes`、解释的来源是“未知”、还没问过（不在 `ctx.ai` 里）。
/// 不含扫描根本身、“其余小文件合计”、无权限 / 系统保留 / 硬链接多余名字 / 映射 / 云占位 / 没跟随的链接，
/// 也不进个人文件夹（桌面、文档、下载……，见 [`super::PROTECTED_USER_DIRS`]）。按占用从大到小，最多 `cap` 项。
/// 返回 (完整路径, 占用)。
///
/// **装的主要是个人资料的不自动问**（[`personal_content`]）：文件夹里看得到的字节一半以上是文档 / 图片 / 视频 / 音乐
/// （.pdf .docx .jpg .mp4 .mp3……，[`PERSONAL_EXTS`]），或者单个文件本身就是这些类型——不管它在哪（`C:\资料`、
/// 用户文件夹根上的文件、`D:\照片`、搬走了的“文档”文件夹……），这种东西的名字多半是私人的，问 AI 也问不出什么。
/// 这样的文件夹自己不问，里面的子项照样一个一个看（各自按同样的规则判断）。
///
/// **别人的东西不自动问**（按路径判断，不只看本机的环境变量）：任何盘上的 `X:\Users` 本身不问（它的子文件夹名就是
/// 各个账号的用户名），`X:\Users\<某人>` 只要不是本机当前用户（`%USERPROFILE%`）就整棵子树都不看——管理员身份扫描时
/// 能读到别的账号的文件夹、打开别的电脑的快照、第二块盘上留着旧系统的 Users，这些都可能把别人的私人文件名发出去。
/// 手动“问 AI”不受影响（用户自己点的）。
///
/// **一条“套娃”链只问最上面那个**：一项占它最近的要问（或问过）的上级 ≥ 90%（[`AI_SAME_AS_PARENT_PERMILLE`]）时，
/// 它和上级几乎是同一堆东西，不再单独问——真机上 `pip\cache\http-v2\8\c\7\8\9\<一个 .body 文件>`、
/// `Clipchamp…\LocalState\EBWebView\Default\IndexedDB\…` 这种一层套一层、大小几乎一样的链，
/// 不去重会占掉一批 40 项里的一大半，问出来的也是同一句话（集成测试 2026-10-12）。
/// 中间隔着认得出来的文件夹就重新算（那是另一样东西）。
///
/// 小于门槛的子树整个跳过（子项不会比父目录大），所以在上百万个文件的树上也只解释几百到几千个节点。
pub fn ai_candidates(tree: &Node, ctx: &ExplainCtx, min_bytes: u64, cap: usize) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = Vec::new();
    if cap == 0 || tree.kind != NodeKind::Dir {
        return out;
    }
    let mut chain: Vec<&Node> = Vec::new();
    let home = ctx.env.get("USERPROFILE").map(split_lower);
    let w = Walk { ctx, min_bytes: min_bytes.max(1), home: home.as_deref() };
    walk_candidates(&w, tree, None, &mut chain, &mut out);
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.truncate(cap);
    out
}

struct Walk<'c> {
    ctx: &'c ExplainCtx,
    min_bytes: u64,
    /// 本机当前用户的用户文件夹（小写段，`%USERPROFILE%`）。
    home: Option<&'c [String]>,
}

/// 自动识别眼里的“个人资料”扩展名（小写，不带点）：文档、图片（含相机 RAW）、视频、音乐。
pub const PERSONAL_EXTS: &[&str] = &[
    // 文档
    "pdf", "doc", "docx", "docm", "dot", "dotx", "xls", "xlsx", "xlsm", "xlsb", "ppt", "pptx", "pptm", "txt", "md", "wps", "et", "dps",
    // 图片
    "jpg", "jpeg", "png", "heic", "heif", "gif", "bmp", "webp", "tif", "tiff", "raw", "cr2", "cr3", "nef", "arw", "dng",
    // 视频
    "mp4", "mov", "mkv", "avi", "wmv", "m4v",
    // 音乐
    "mp3", "flac", "wav", "m4a",
];

/// 文件名的扩展名是不是 [`PERSONAL_EXTS`] 之一（大小写不敏感，不分配内存——大树上要调很多次）。
pub fn is_personal_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() && !e.is_empty() && e.len() <= 5 => PERSONAL_EXTS.iter().any(|p| p.eq_ignore_ascii_case(e)),
        _ => false,
    }
}

/// 判断“个人资料占多少”时一个文件夹最多看多少个节点（够判断了；大树上不至于每个候选都走一遍整棵子树）。
const PERSONAL_MAX_VISIT: usize = 20_000;

/// 自动识别不问的“个人资料”：文件 → 扩展名是 [`PERSONAL_EXTS`] 之一；文件夹 → 子树里看得到的文件字节（不含“其余小文件合计”，
/// 最多看 [`PERSONAL_MAX_VISIT`] 个节点）**一半以上**是这些类型。
pub fn personal_content(n: &Node) -> bool {
    match n.kind {
        NodeKind::File => is_personal_name(&n.name),
        NodeKind::Rest => false,
        NodeKind::Dir => {
            let (mut seen, mut personal) = (0u64, 0u64);
            let mut visited = 0usize;
            let mut stack: Vec<&Node> = vec![n];
            while let Some(x) = stack.pop() {
                visited += 1;
                if visited > PERSONAL_MAX_VISIT {
                    break;
                }
                match x.kind {
                    NodeKind::File => {
                        seen += x.allocated;
                        if is_personal_name(&x.name) {
                            personal += x.allocated;
                        }
                    }
                    NodeKind::Dir => stack.extend(x.children.iter()),
                    NodeKind::Rest => {}
                }
            }
            seen > 0 && (personal as u128) * 2 > seen as u128
        }
    }
}

/// 小写段是 `x:\users`（任何盘）。
fn users_root(segs: &[String]) -> bool {
    segs.len() == 2 && segs[0].len() == 2 && segs[0].ends_with(':') && segs[1] == "users"
}

/// 小写段在 `x:\users\<某人>` 之下（含自身），而那不是本机当前用户的用户文件夹。
fn foreign_profile(segs: &[String], home: Option<&[String]>) -> bool {
    segs.len() >= 3 && users_root(&segs[..2]) && home.is_none_or(|h| h.len() != 3 || segs[..3] != h[..])
}

/// `size` 占 `cover`（最近的要问 / 问过的上级的占用）≥ 90%：和上级算同一样东西。
fn same_as_cover(size: u64, cover: Option<u64>) -> bool {
    cover.is_some_and(|a| a > 0 && (size as u128) * 1000 >= (a as u128) * (AI_SAME_AS_PARENT_PERMILLE as u128))
}

/// `cover`：最近的“要问 / 问过的”上级的占用（中间隔着认得出来的文件夹时是 None）。
fn walk_candidates<'a>(w: &Walk, node: &'a Node, cover: Option<u64>, chain: &mut Vec<&'a Node>, out: &mut Vec<(String, u64)>) {
    let ctx = w.ctx;
    chain.push(node);
    // 子项的“上层规则继承”只跟这一层的祖先链有关，算一次（和 view::listing 一样）。
    let mut inherited = None;
    let mut inherited_done = false;
    for c in &node.children {
        if c.allocated < w.min_bytes || c.kind == NodeKind::Rest {
            continue;
        }
        let mut is_users_root = false;
        if c.kind == NodeKind::Dir {
            let segs = split_lower(&c.path);
            // 桌面 / 文档 / 图片 / 视频 / 音乐 / 下载 / OneDrive 及其下面：是用户自己的东西，自动问会把私人文件名
            // 发出去，也问不出什么（手动“问 AI”仍然可以）。别的账号的用户文件夹同理。整棵子树都不看。
            if ctx.is_protected(&segs) || foreign_profile(&segs, w.home) {
                continue;
            }
            is_users_root = users_root(&segs);
        }
        // 子项的 cover：自己要问 / 问过 → 自己；和上级算同一样 → 还是上级的；认得出来 → 重新算；
        // 不能问（无权限、X:\Users……）、主要是个人资料 → 照旧
        let mut sub_cover = cover;
        if askable(c) && !is_users_root {
            let path = if c.kind == NodeKind::Dir { c.path.clone() } else { join(&node.path, &c.name) };
            if ctx.ai.contains(&path) {
                if !same_as_cover(c.allocated, cover) {
                    sub_cover = Some(c.allocated);
                }
            } else {
                if !inherited_done {
                    inherited = ctx.inherited(chain);
                    inherited_done = true;
                }
                let t = Target { node: c, path: &path, parent: Some(node), ancestors: chain };
                if explain_with_inherited(&t, inherited.as_ref(), ctx).source == Source::Unknown {
                    // 个人资料（文档 / 照片 / 视频 / 音乐为主）不问：名字多半是私人的（只在要问的时候才算，认得出来的不用算）
                    if !same_as_cover(c.allocated, cover) && !personal_content(c) {
                        out.push((path, c.allocated));
                        sub_cover = Some(c.allocated);
                    }
                } else {
                    sub_cover = None;
                }
            }
        }
        if c.kind == NodeKind::Dir && !c.children.is_empty() {
            walk_candidates(w, c, sub_cover, chain, out);
        }
    }
    chain.pop();
}
