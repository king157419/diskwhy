//! 启发式补充（2026-09-24 审计）：用户自己放在任意盘上的“大东西”。
//!
//! 规则库的路径模式必须以 `%变量%` 开头，管不到 `D:\steam`、`D:\conda` 这种用户自己挑的位置，
//! 所以这些只能按**名字和目录结构**来认：
//!
//! - 聊天记录：任何盘上的 `xwechat_files` / `WeChat Files`（微信）、`Tencent Files`（QQ）、`WXWork`（企业微信）→ keep；
//!   其中每个账号下的 `cache` / `temp` → review。
//! - 游戏：Steam 库（`steamapps`）、腾讯游戏 / Epic 目录、`.minecraft`、Unity / 虚幻引擎游戏 → keep，“用平台自带的卸载”；
//!   游戏自动录的精彩片段（`wonderfulVideos…`）→ review。
//! - 虚拟机：VMware（`.vmx`）/ VirtualBox（`.vbox`）/ Hyper-V（`.vmcx`、`Virtual Hard Disks`）→ keep + 迁移。
//! - conda：环境集合 `envs`、单个环境（有 `conda-meta` 且上层叫 `envs`）、base 安装 → keep；包缓存 `pkgs` → review。
//! - 安卓 / Flutter：Android SDK、`system-images` 等、Gradle 用户目录及 `caches` / `wrapper`、`.android`（AVD）、
//!   Flutter SDK 与 `bin\cache`、pub 缓存 → review（写清楚哪些是能重新下载的缓存、哪些是 SDK 本体）。
//! - Rust 编译产物：带 `.rustc_info.json` + `CACHEDIR.TAG` 的文件夹（`CARGO_TARGET_DIR` 可以叫任何名字）→ review。
//!
//! [`structural`] 只看名字和直接子项（便宜），既用于当前节点，也通过 [`inherited`] 用于上层文件夹；
//! [`by_content`] 要看子树统计（虚幻引擎 `.pak` 占比），只用于当前节点。
//! 结果和其他启发式一样由 `heuristics::finish` 收尾：档位永远不会是 safe。

use super::heuristics::{child, ext_of, has_child, Hint, SubtreeStats};
use super::pattern::split_lower;
use super::{Category, ExplainCtx, How, Target, Tier};
use crate::model::{Node, NodeKind};
use crate::view::human_bytes;

/// 启发式对应的参考链接（`heuristics::finish` 填到 `Explanation.refs`）。
pub fn refs_for(name: &str) -> &'static [&'static str] {
    match name {
        n if n.starts_with("steam-") => &["https://help.steampowered.com/en/faqs/view/4BD4-4528-6B2E-8327"],
        "conda-env" | "conda-envs" | "conda-base" | "conda-root" => {
            &["https://docs.conda.io/projects/conda/en/latest/user-guide/tasks/manage-environments.html"]
        }
        "conda-pkgs" => &["https://docs.conda.io/projects/conda/en/latest/commands/clean.html"],
        "android-sdk" | "android-sdk-part" => &["https://developer.android.com/tools/sdkmanager"],
        "android-user-home" | "android-avd" => {
            &["https://developer.android.com/tools/variables", "https://developer.android.com/studio/run/managing-avds"]
        }
        "gradle-home" | "gradle-cache" => &["https://docs.gradle.org/current/userguide/directory_layout.html"],
        "flutter-sdk" | "flutter-cache" => &["https://docs.flutter.dev/reference/flutter-cli"],
        "pub-cache" => &["https://dart.dev/tools/pub/cmd/pub-cache"],
        "rust-target" => &["https://doc.rust-lang.org/cargo/reference/build-cache.html"],
        "vm-virtualbox" => &["https://www.virtualbox.org/manual/topics/working-with-vms.html"],
        "vm-hyperv" => &["https://learn.microsoft.com/en-us/powershell/module/hyper-v/move-vmstorage"],
        "pnpm-store" => &["https://pnpm.io/cli/store"],
        "ollama-models-dir" => &["https://docs.ollama.com/faq"],
        "driver-packages" => &["https://learn.microsoft.com/en-us/windows-hardware/drivers/install/driver-store"],
        _ => &[],
    }
}

// ───────────────────────── 小工具 ─────────────────────────

/// 原始大小写的路径段（和 `split_lower` 一一对应）。
fn orig_segs(path: &str) -> Vec<&str> {
    path.split(['\\', '/']).filter(|s| !s.is_empty()).collect()
}

fn is_dir(n: &Node) -> bool {
    n.kind == NodeKind::Dir
}

fn name_lc(n: &Node) -> String {
    n.name.to_lowercase()
}

/// 直接子项里有没有某个扩展名的文件。
fn has_file_ext(n: &Node, exts: &[&str]) -> bool {
    n.children.iter().any(|c| c.kind == NodeKind::File && exts.contains(&ext_of(&c.name).as_str()))
}

/// 子文件夹大小清单：“名字 大小、名字 大小…”。
fn sizes_of<'a>(items: impl Iterator<Item = &'a Node>, k: usize) -> String {
    let mut v: Vec<&Node> = items.collect();
    v.sort_by(|a, b| b.allocated.cmp(&a.allocated));
    let total = v.len();
    let mut s: Vec<String> = v.iter().take(k).map(|c| format!("{} {}", c.name, human_bytes(c.allocated))).collect();
    if total > k {
        s.push(format!("……共 {total} 个"));
    }
    s.join("、")
}

#[allow(clippy::too_many_arguments)]
fn hint(
    name: &'static str,
    tier: Tier,
    category: Category,
    what: impl Into<String>,
    why: impl Into<String>,
    lose: impl Into<String>,
    how: Vec<How>,
    how_detail: impl Into<String>,
    confidence: f32,
    owner: Option<String>,
    clues: Vec<String>,
) -> Hint {
    Hint {
        name,
        tier,
        category,
        what: what.into(),
        why: why.into(),
        lose: lose.into(),
        how,
        how_detail: how_detail.into(),
        confidence,
        owner,
        clues,
        lose_kind: None,
        lose_short: None,
    }
}

// ───────────────────────── 入口 ─────────────────────────

/// 只看名字、路径和直接子项的判断（顺序即优先级）。
pub(super) fn structural(t: &Target, _ctx: &ExplainCtx) -> Option<Hint> {
    specific(t).or_else(|| dev_sdk_container(t)).or_else(|| user_projects(t)).or_else(|| dominant_child(t, 2))
}

/// 认得出“这是什么”的具体判断（不含“这里面装着什么”的容器判断）。
fn specific(t: &Target) -> Option<Hint> {
    chat_files(t)
        .or_else(|| game_highlights(t))
        .or_else(|| steam(t))
        .or_else(|| game_dirs(t))
        .or_else(|| vm_folder(t))
        .or_else(|| conda(t))
        .or_else(|| android(t))
        .or_else(|| gradle(t))
        .or_else(|| flutter_and_pub(t))
        .or_else(|| rust_target(t))
        .or_else(|| tool_stores(t))
        .or_else(|| installed_elsewhere(t))
}

/// 需要子树统计的判断：虚幻引擎游戏资源包。
pub(super) fn by_content(t: &Target, s: &SubtreeStats) -> Option<Hint> {
    const UE: &[&str] = &["pak", "ucas", "utoc"];
    let n = t.node;
    let share = if n.kind == NodeKind::File {
        if UE.contains(&ext_of(&n.name).as_str()) {
            1.0
        } else {
            0.0
        }
    } else {
        s.share(UE)
    };
    if share >= 0.5 && n.allocated > 200 * 1024 * 1024 {
        return Some(hint(
            "game-files",
            Tier::Keep,
            Category::App,
            "游戏的资源包（虚幻引擎的 .pak / .ucas / .utoc）",
            "虚幻引擎做的游戏把地图、模型、贴图、声音打包成这些大文件，游戏运行时直接读取",
            "直接删游戏就坏了，启动器会要求“修复”并重新下载几十 GB。不玩了请用安装它的平台卸载。",
            vec![How::Manual],
            "用安装它的平台（Steam / Epic / WeGame / 游戏自己的启动器）卸载，或在平台里“移动安装位置”；不要直接删文件夹。",
            0.7,
            None,
            vec![format!(".pak/.ucas/.utoc 占 {:.0}%", share * 100.0)],
        ));
    }
    None
}

/// 当前节点认不出来时，看上层文件夹（最多 8 层）能不能用 [`structural`] 认出来。
pub(super) fn inherited(t: &Target, ctx: &ExplainCtx) -> Option<Hint> {
    let anc = t.ancestors;
    for i in (0..anc.len()).rev().take(8) {
        let a = anc[i];
        if a.path.is_empty() || a.kind != NodeKind::Dir {
            continue;
        }
        let at = Target { node: a, path: &a.path, parent: if i > 0 { Some(anc[i - 1]) } else { None }, ancestors: &anc[..i] };
        if let Some(mut h) = structural(&at, ctx) {
            // 容器类的说法（“这里面主要是聊天记录”“conda 环境集合”）不往下传。
            if matches!(
                h.name,
                "chat-files-container" | "conda-envs" | "conda-root" | "game-tencent-root" | "dev-sdk-container" | "user-projects" | "dominant-child" | "steam-root"
            ) {
                return None;
            }
            h.clues.insert(0, format!("它在「{}」里面，按上层文件夹来推测", a.path));
            h.confidence = (h.confidence - 0.1).max(0.3);
            return Some(h);
        }
    }
    None
}

// ───────────────────────── 聊天记录 ─────────────────────────

const CHAT_ROOTS: &[(&str, &str)] = &[("xwechat_files", "微信"), ("wechat files", "微信"), ("tencent files", "QQ"), ("wxwork", "企业微信")];

fn chat_app(seg_lc: &str) -> Option<&'static str> {
    CHAT_ROOTS.iter().find(|(r, _)| *r == seg_lc).map(|(_, a)| *a)
}

fn chat_files(t: &Target) -> Option<Hint> {
    let segs = split_lower(t.path);
    let orig = orig_segs(t.path);
    if let Some((i, app)) = segs.iter().enumerate().find_map(|(i, s)| chat_app(s).map(|a| (i, a))) {
        let root = orig.get(i).copied().unwrap_or("");
        let sub = segs.get(i + 2).map(String::as_str);
        let sub2 = segs.get(i + 3).map(String::as_str);
        let is_cache = matches!(sub, Some("cache" | "temp")) || (sub == Some("filestorage") && matches!(sub2, Some("cache" | "temp")));
        let mut clues = vec![format!("路径里有「{root}」：{app}存聊天记录和聊天文件的文件夹（不管放在哪个盘都叫这个名字）")];
        if let Some(acc) = orig.get(i + 1) {
            clues.push(format!("账号文件夹：{acc}"));
        }
        if is_cache {
            return Some(hint(
                "chat-files-cache",
                Tier::Review,
                Category::Cache,
                format!("{app}聊天文件夹里的缓存（图片、视频的缩略图和临时文件）"),
                format!("{app}看图片、视频时生成的缓存，放在每个账号的聊天文件夹里"),
                "删了聊天记录本身不丢，但一部分图片、视频要重新从服务器加载；已经过期的原图 / 视频可能就看不了了。",
                vec![How::Manual],
                format!("优先用{app}自带的“设置 → 存储空间 / 文件管理”清理缓存，不要直接删整个账号文件夹。"),
                0.7,
                Some(app.to_string()),
                clues,
            ));
        }
        clues.push(format!("{app}“设置 → 文件管理”里显示的才是当前在用的位置；同一个账号如果在好几个地方都有，其他的可能是改位置前留下的旧副本，先确认再处理"));
        return Some(hint(
            "chat-files",
            Tier::Keep,
            Category::Userdata,
            format!("{app}的聊天记录和聊天文件（收到的图片、视频、文件）"),
            format!("{app}把每个账号的聊天数据库和收发的文件放在这里；很多人会把它改到 D 盘这类空间大的盘"),
            "删了聊天记录和收到的文件就没了，无法恢复（除非手机上还有或做过备份）。",
            vec![How::Migrate, How::Manual],
            format!("想换盘：在{app}“设置 → 文件管理”里改存储位置，让{app}自己搬。想省空间：用{app}自带的存储空间管理删大文件。不要直接删文件夹。"),
            0.8,
            Some(app.to_string()),
            clues,
        ));
    }
    // 自己不是聊天文件夹，但里面几乎全是（比如 D:\wxliaotianjilu 里只有一个 xwechat_files）。
    let n = t.node;
    if is_dir(n) && n.allocated > 0 {
        for c in n.children.iter().filter(|c| is_dir(c)) {
            if let Some(app) = chat_app(&name_lc(c)) {
                if c.allocated as f64 >= n.allocated as f64 * 0.8 {
                    return Some(hint(
                        "chat-files-container",
                        Tier::Keep,
                        Category::Userdata,
                        format!("放{app}聊天记录的文件夹"),
                        format!("里面主要是「{}」（{app}的聊天记录和聊天文件），看起来是你在{app}里把存储位置改到了这里", c.name),
                        format!("删了{app}的聊天记录和收到的文件就没了。"),
                        vec![How::Migrate, How::Manual],
                        format!("要搬走请在{app}“设置 → 文件管理”里改位置；不要直接删。"),
                        0.75,
                        Some(app.to_string()),
                        vec![format!("「{}」占这里的 {:.0}%（{}）", c.name, c.allocated as f64 * 100.0 / n.allocated as f64, human_bytes(c.allocated))],
                    ));
                }
            }
        }
    }
    None
}

// ───────────────────────── 游戏 ─────────────────────────

fn game_highlights(t: &Target) -> Option<Hint> {
    let segs = split_lower(t.path);
    let orig = orig_segs(t.path);
    let i = segs.iter().position(|s| s.starts_with("wonderfulvideos"))?;
    let valorant = segs.iter().any(|s| s == "tencent games" || s.contains("valorant") || s == "aclos" || s == "aclos-highlight");
    Some(hint(
        "game-highlights",
        Tier::Review,
        Category::Userdata,
        "游戏自动录下来的精彩片段视频",
        format!(
            "文件夹名「{}」和《无畏契约》国服“无畏时刻”自动录像的文件夹名一致（本机线索：它{}）。开着这个功能时每局都会录，时间长了会非常大",
            orig.get(i).copied().unwrap_or(""),
            if valorant { "和 VALORANT / Tencent Games / ACLOS 放在一起" } else { "的名字" }
        ),
        "删了这些录像就没了，游戏本身不受影响。想留的片段先挪走。",
        vec![How::Manual, How::Migrate],
        "在游戏客户端的录像 / 高光设置里查看和删除，或把保存位置改到别的盘、关掉自动录制；不想留的也可以直接删（默认进回收站，回收站在同一个盘，要清空回收站才真正腾出空间）。",
        0.6,
        if valorant { Some("无畏契约（腾讯）".into()) } else { None },
        vec!["腾讯没有公开说明这个文件夹，这是按文件夹名和位置推测的".into()],
    ))
}

fn keep_game(name: &'static str, what: String, why: String, how_detail: String, owner: Option<String>, clues: Vec<String>) -> Hint {
    hint(
        name,
        Tier::Keep,
        Category::App,
        what,
        why,
        "直接删文件夹，平台会认为游戏损坏、要求重新下载；有些游戏的存档和设置也在安装目录里，会一起丢。",
        vec![How::Manual, How::Migrate],
        how_detail,
        0.75,
        owner,
        clues,
    )
}

fn steam(t: &Target) -> Option<Hint> {
    let segs = split_lower(t.path);
    let orig = orig_segs(t.path);
    let n = t.node;
    let steam_how = "在 Steam 里操作：库里右键游戏 → 管理 → 卸载；或 设置 → 存储空间，选中游戏“移动”到别的库文件夹。不要直接删文件夹。".to_string();
    if let Some(j) = segs.iter().position(|s| s == "steamapps") {
        let tail = &segs[j + 1..];
        return Some(match tail.first().map(String::as_str) {
            None => keep_game(
                "steam-library",
                "Steam 游戏库（Steam 下载的所有游戏都在这里）".into(),
                "Steam 库文件夹里的 steamapps：common 是游戏本体，appmanifest_*.acf 是每个游戏的安装记录，workshop 是创意工坊内容".into(),
                steam_how,
                Some("Steam".into()),
                vec![format!("子项：{}", sizes_of(n.children.iter().filter(|c| is_dir(c)), 6))],
            ),
            Some("common") if tail.len() == 1 => keep_game(
                "steam-library",
                "Steam 游戏库里所有游戏的安装文件夹".into(),
                "每个子文件夹是一个游戏".into(),
                steam_how,
                Some("Steam".into()),
                vec![format!("游戏：{}", sizes_of(n.children.iter().filter(|c| is_dir(c)), 8))],
            ),
            Some("common") => {
                let game = orig.get(j + 2).copied().unwrap_or("");
                keep_game(
                    "steam-game",
                    format!("Steam 游戏「{game}」的安装文件"),
                    "Steam 把每个游戏装在 steamapps\\common\\<游戏名> 下".into(),
                    format!("在 Steam 库里右键「{game}」→ 管理 → 卸载；或 属性 → 已安装文件 → 移动安装文件夹。不要直接删。"),
                    Some("Steam".into()),
                    Vec::new(),
                )
            }
            Some("shadercache" | "downloading" | "temp") => hint(
                "steam-cache",
                Tier::Review,
                Category::Cache,
                "Steam 的着色器缓存 / 下载中的临时文件",
                "Steam 预编译的着色器，或正在下载、更新的游戏的临时文件",
                "着色器缓存删了游戏头几次运行会重新编译、可能卡一点；downloading 删了正在进行的下载要重来。不丢游戏和存档。",
                vec![How::Delete],
                "完全退出 Steam 后删除；着色器缓存也可以在 Steam 设置里关掉预缓存。",
                0.7,
                Some("Steam".into()),
                Vec::new(),
            ),
            Some("workshop") => hint(
                "steam-workshop",
                Tier::Review,
                Category::Userdata,
                "Steam 创意工坊下载的模组、地图等",
                "你在创意工坊订阅的内容下载到这里",
                "直接删了下次启动游戏 Steam 会重新下载；不想要请在创意工坊里取消订阅。",
                vec![How::Manual],
                "在 Steam 创意工坊里取消订阅不需要的内容，Steam 会自己删。",
                0.7,
                Some("Steam".into()),
                Vec::new(),
            ),
            Some(_) => keep_game(
                "steam-library",
                "Steam 游戏库里的文件".into(),
                "在 steamapps 里，是 Steam 管理游戏用的".into(),
                steam_how,
                Some("Steam".into()),
                Vec::new(),
            ),
        });
    }
    if is_dir(n) && has_child(n, "steamapps") && ["steamui", "steam.exe", "steamclient64.dll", "package"].iter().any(|x| has_child(n, x)) {
        return Some(keep_game(
            "steam-root",
            "Steam 客户端和它的游戏库".into(),
            "Steam 装在这里；steamapps 是游戏库，userdata 是各账号的本地设置和部分游戏存档，appcache / logs / depotcache 是缓存和日志".into(),
            "不玩了先在 Steam 里卸载游戏，再在“设置 > 应用”里卸载 Steam；只想腾空间就在 Steam 里卸载或移动游戏。".into(),
            Some("Steam".into()),
            vec![format!("子项：{}", sizes_of(n.children.iter().filter(|c| is_dir(c)), 6))],
        ));
    }
    None
}

fn game_dirs(t: &Target) -> Option<Hint> {
    let segs = split_lower(t.path);
    let orig = orig_segs(t.path);
    let n = t.node;
    let platform_how = "用安装它的平台卸载（WeGame / Epic / 游戏自己的启动器，或“设置 > 应用”），或在平台里“移动安装位置”；不要直接删文件夹。".to_string();
    // “Tencent Games” 文件夹本身：里面可能同时有游戏本体和几百 GB 的自动录像，分开说。
    let tencent_root = if segs.last().is_some_and(|s| s == "tencent games") {
        Some(n)
    } else {
        child(n, "Tencent Games").filter(|c| is_dir(c) && c.allocated as f64 >= n.allocated as f64 * 0.8)
    };
    if let (Some(root), true) = (tencent_root, is_dir(n)) {
        let videos: u64 = root.children.iter().filter(|c| c.name.to_lowercase().starts_with("wonderfulvideos")).map(|c| c.allocated).sum();
        let games: Vec<&Node> = root.children.iter().filter(|c| is_dir(c) && !c.name.to_lowercase().starts_with("wonderfulvideos")).collect();
        let mut clues = vec![format!("游戏：{}", if games.is_empty() { "（没有）".to_string() } else { sizes_of(games.into_iter(), 6) })];
        if videos > 0 {
            clues.push(format!("wonderfulVideos… 录像文件夹合计 {}（{:.0}%）——这部分是自动录像，可以在游戏里清理", human_bytes(videos), videos as f64 * 100.0 / root.allocated.max(1) as f64));
        }
        return Some(hint(
            "game-tencent-root",
            Tier::Keep,
            Category::App,
            if videos > 0 { "腾讯游戏的文件夹：游戏本体 + 游戏自动录的精彩片段视频" } else { "腾讯游戏的文件夹（WeGame / 游戏启动器装的游戏）" },
            "腾讯游戏（WeGame 或游戏自己的启动器）默认把游戏装在 Tencent Games 下；《无畏契约》的“无畏时刻”录像也会放在这里的 wonderfulVideos… 文件夹",
            "整个删游戏就没了（要重新下载几十 GB），录像也一起没了。请点进去分别处理：游戏用平台卸载，录像在游戏里清理。",
            vec![How::Manual],
            platform_how,
            0.7,
            Some("腾讯游戏".into()),
            clues,
        ));
    }
    if let Some(j) = segs.iter().position(|s| s == "tencent games") {
        let game = orig.get(j + 1).copied().unwrap_or("");
        let valorant = segs.get(j + 1).is_some_and(|g| g == "valorant");
        return Some(keep_game(
            "game-tencent",
            if game.is_empty() { "腾讯游戏的安装文件夹".into() } else { format!("腾讯游戏「{game}」的安装文件") },
            if valorant { "《无畏契约》国服客户端装在这里（live\\ShooterGame\\Content\\Paks 是游戏资源包）".into() } else { "腾讯游戏（WeGame 或游戏自己的启动器）默认装在 Tencent Games 下".into() },
            platform_how,
            Some(if valorant { "无畏契约（腾讯）".into() } else { "腾讯游戏".into() }),
            Vec::new(),
        ));
    }
    if let Some(j) = segs.iter().position(|s| s == "epic games") {
        if segs.len() > j + 1 {
            let game = orig.get(j + 1).copied().unwrap_or("");
            return Some(keep_game(
                "game-epic",
                format!("Epic 游戏「{game}」的安装文件"),
                "Epic Games 启动器默认把游戏装在 Epic Games\\<游戏名> 下".into(),
                "在 Epic Games 启动器的库里点游戏右边的“…”→ 卸载 / 管理 → 移动。不要直接删。".into(),
                Some("Epic Games".into()),
                Vec::new(),
            ));
        }
    }
    let mc_layout = is_dir(n)
        && ["versions", "assets", "libraries"].iter().all(|x| has_child(n, x))
        && ["launcher_profiles.json", "PCL.ini", "options.txt", "saves", "mods"].iter().any(|x| has_child(n, x));
    if segs.iter().any(|s| s == ".minecraft") || mc_layout {
        return Some(hint(
            "game-minecraft",
            Tier::Keep,
            Category::Userdata,
            "Minecraft 的游戏目录（存档、模组、资源包、截图都在里面）",
            "Minecraft 启动器把游戏版本、存档（saves）、模组（mods）、资源包都放在 .minecraft 下",
            "saves 是你的存档，删了就没了；versions / libraries / assets 删了启动器会重新下载。",
            vec![How::Manual, How::Migrate],
            "先备份 saves；只想腾空间可以在启动器里删不玩的版本。",
            0.75,
            Some("Minecraft".into()),
            Vec::new(),
        ));
    }
    if is_dir(n) {
        let nm = name_lc(n);
        let unity = has_child(n, "UnityPlayer.dll")
            || (nm.ends_with("_data") && ["StreamingAssets", "Managed", "il2cpp_data", "globalgamemanagers"].iter().any(|x| has_child(n, x)));
        if unity || has_child(n, ".egstore") {
            return Some(keep_game(
                "game-files",
                "一个游戏的安装文件".into(),
                if unity { "里面有 Unity 引擎游戏的标志（UnityPlayer.dll 或 *_Data 里的 StreamingAssets / Managed）".into() } else { "里面有 .egstore（Epic Games 启动器装的游戏）".into() },
                platform_how,
                None,
                Vec::new(),
            ));
        }
    }
    None
}

// ───────────────────────── 虚拟机 ─────────────────────────

fn vm_folder(t: &Target) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    let vm_keep = |name: &'static str, what: &str, how_detail: &str| {
        hint(
            name,
            Tier::Keep,
            Category::Vm,
            what.to_string(),
            "虚拟机的整台“电脑”（系统、软件、里面的文件）都装在这个文件夹的虚拟磁盘文件里",
            "删了等于把那台虚拟机连同里面所有文件一起删掉，无法恢复。",
            vec![How::Migrate, How::Manual],
            how_detail.to_string(),
            0.75,
            None,
            vec![format!("子项：{}", sizes_of(n.children.iter(), 5))],
        )
    };
    if has_file_ext(n, &["vmx"]) {
        return Some(vm_keep(
            "vm-vmware",
            "VMware 虚拟机（.vmx 配置 + .vmdk 虚拟磁盘）",
            "要搬：先在 VMware 里把虚拟机关机（不要挂起），把整个文件夹移到新位置，再在 VMware 里“文件 → 打开”新位置的 .vmx。不用了：在 VMware 里“管理 → 从磁盘中删除”。",
        ));
    }
    if has_file_ext(n, &["vbox"]) {
        return Some(vm_keep(
            "vm-virtualbox",
            "VirtualBox 虚拟机（.vbox 配置 + .vdi 等虚拟磁盘）",
            "要搬：在 VirtualBox 管理器里右键这台虚拟机 → 移动（Move），或用 VBoxManage movevm。不用了：右键 → 删除，选“删除所有文件”。",
        ));
    }
    let nm = name_lc(n);
    if has_file_ext(n, &["vmcx", "vmrs"]) || ((nm == "virtual hard disks" || nm == "virtual machines") && has_file_ext(n, &["vhdx", "avhdx", "vhd", "vmcx"])) {
        return Some(vm_keep(
            "vm-hyperv",
            "Hyper-V 虚拟机的文件（配置或虚拟磁盘）",
            "要搬：Hyper-V 管理器里对虚拟机点“移动”，或 PowerShell 的 Move-VMStorage。不用了：先在 Hyper-V 管理器里删除虚拟机，再删磁盘文件。",
        ));
    }
    None
}

// ───────────────────────── conda ─────────────────────────

fn is_conda_env(n: &Node) -> bool {
    is_dir(n) && has_child(n, "conda-meta")
}

fn conda(t: &Target) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    let nm = name_lc(n);
    let parent_is_envs = t.parent.is_some_and(|p| p.name.eq_ignore_ascii_case("envs"));
    if !is_conda_env(n) && has_child(n, "python.exe") && child(n, "Lib").is_some_and(|l| has_child(l, "site-packages")) {
        return Some(hint(
            "python-runtime",
            Tier::Review,
            Category::App,
            "一套独立的 Python 运行环境（python.exe + 装好的库）",
            "有的软件 / 整合包自带一份 Python（比如 AI 工具整合包里的 runtime 文件夹），也可能是你自己解压的便携版 Python",
            "删了依赖它的软件或脚本就跑不起来，要重新下载整合包或重装库。如果它属于某个软件，跟着那个软件一起处理，别单独删。",
            vec![How::Manual],
            "先看它上一层是什么软件：不用那个软件了就整个删掉那个软件的文件夹；还在用就别动。",
            0.65,
            None,
            vec!["里面有 python.exe 和 Lib\\site-packages".into()],
        ));
    }
    if is_conda_env(n) && parent_is_envs {
        return Some(hint(
            "conda-env",
            Tier::Keep,
            Category::App,
            format!("conda 环境「{}」（{}）", n.name, human_bytes(n.allocated)),
            "用 conda create 建的一套独立 Python 环境：解释器加这个环境里装的所有库",
            "删了用这个环境的项目就跑不起来，要重新创建并重装所有库（可能很慢、版本也可能对不上）。",
            vec![How::Manual, How::Migrate],
            format!("不用了：conda env remove -n {}（会列出要删的东西）。conda 不支持直接剪切环境文件夹，想换盘请先 conda env export > env.yml 导出，再在新位置 conda env create -f env.yml 重建。", n.name),
            0.8,
            Some("conda".into()),
            vec!["里面有 conda-meta，上层文件夹叫 envs".into()],
        ));
    }
    if is_conda_env(n) && (has_child(n, "condabin") || child(n, "Scripts").is_some_and(|s| has_child(s, "conda.exe"))) {
        return Some(hint(
            "conda-base",
            Tier::Keep,
            Category::App,
            "conda（Miniconda / Anaconda）的 base 安装，conda 命令本身在这里",
            "安装 Miniconda / Anaconda 时建的基础环境；其他环境、包缓存可能也放在它下面或旁边",
            "删了 conda 命令就没了，所有 conda 环境都没法再用 conda 管理。",
            vec![How::Manual],
            "不用了请在“设置 > 应用”里卸载 Miniconda / Anaconda；想省空间先用 conda clean --all 清包缓存。",
            0.8,
            Some("conda".into()),
            vec!["里面有 conda-meta 和 condabin / Scripts\\conda.exe".into()],
        ));
    }
    if nm == "envs" {
        let envs: Vec<&Node> = n.children.iter().filter(|c| is_conda_env(c)).collect();
        if !envs.is_empty() {
            return Some(hint(
                "conda-envs",
                Tier::Keep,
                Category::App,
                format!("conda 的环境集合（{} 个环境）", envs.len()),
                "每个子文件夹是一个用 conda create 建的环境",
                "整个删会让所有 conda 环境都没了。请按下面的大小挑不用的环境单独删除。",
                vec![How::Manual, How::Migrate],
                "conda env list 看有哪些环境；不用的用 conda env remove -n 名字 删除。想让新环境建在别的盘：conda config --add envs_dirs 新路径。",
                0.8,
                Some("conda".into()),
                vec![format!("各环境大小：{}", sizes_of(envs.into_iter(), 12))],
            ));
        }
    }
    if nm == "pkgs"
        && (has_child(n, "urls.txt") || has_child(n, "urls") || has_child(n, "cache"))
        && n.children.iter().filter(|c| is_dir(c)).take(80).any(|c| has_child(c, "info"))
    {
        return Some(hint(
            "conda-pkgs",
            Tier::Review,
            Category::Cache,
            "conda 下载和解压过的包缓存",
            "conda 安装包时先下载、解压到这里，再装进各个环境",
            "用 conda clean 清理不影响已经建好的环境；以后装同样的包要重新下载。不要手动删，交给 conda clean 判断哪些还在用。",
            vec![How::BuiltinTool],
            "conda clean --all（会先列出要删什么，确认后再删）。",
            0.75,
            Some("conda".into()),
            vec!["名叫 pkgs，里面有 urls.txt / cache，子文件夹里有 info（conda 包的结构）".into()],
        ));
    }
    if has_child(n, "envs") && has_child(n, "pkgs") && child(n, "envs").is_some_and(|e| e.children.iter().any(is_conda_env)) {
        return Some(hint(
            "conda-root",
            Tier::Keep,
            Category::App,
            "conda 的主文件夹（环境 envs + 包缓存 pkgs）",
            "你把 conda 的环境目录和包缓存放在了这里",
            "整个删会让所有 conda 环境都没了。请点进去看：envs 里挑不用的环境删，pkgs 用 conda clean 清。",
            vec![How::Manual],
            "点进去分别处理 envs 和 pkgs。",
            0.75,
            Some("conda".into()),
            vec![format!("子项：{}", sizes_of(n.children.iter().filter(|c| is_dir(c)), 6))],
        ));
    }
    None
}

// ───────────────────────── Android / Gradle / Flutter ─────────────────────────

const SDK_PARTS: &[&str] = &["platform-tools", "build-tools", "platforms", "system-images", "emulator", "cmdline-tools", "ndk", "cmake", "sources"];

fn is_android_sdk(n: &Node) -> bool {
    is_dir(n) && SDK_PARTS.iter().filter(|p| has_child(n, p)).count() >= 3
}

fn android(t: &Target) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    let sdk_how = "Android Studio → Settings → Languages & Frameworks → Android SDK（SDK Manager）里取消勾选不用的项目；或命令行 sdkmanager --uninstall \"包名\"。";
    if is_android_sdk(n) {
        return Some(hint(
            "android-sdk",
            Tier::Review,
            Category::App,
            "Android SDK（安卓开发工具包）",
            "开发安卓 / Flutter 应用要用：platform-tools（adb）、build-tools、platforms 是编译必需的本体；system-images（模拟器系统镜像）、ndk、emulator 是按需装的大件",
            "整个删安卓项目就编译不了、模拟器也起不来，要重新下载。按需卸载 system-images、ndk、旧版 build-tools / platforms 最省空间，以后要用再装。",
            vec![How::BuiltinTool],
            sdk_how,
            0.75,
            Some("Android SDK".into()),
            vec![format!("子项：{}", sizes_of(n.children.iter().filter(|c| is_dir(c)), 8))],
        ));
    }
    if let Some(p) = t.parent {
        if is_android_sdk(p) {
            let (what, lose) = match name_lc(n).as_str() {
                "system-images" => ("Android 模拟器的系统镜像（每个安卓版本 / 架构一份）", "删了用到它的模拟器启动不了，要重新下载镜像（每个 1~几 GB）。不用的版本在 SDK Manager 里卸载。"),
                "ndk" => ("Android NDK（写 C/C++ 原生代码用的工具链，可能装了好几个版本）", "只有用到原生代码（或某些插件）的项目需要；删了要用时 Android Studio 会提示重新下载。旧版本可以卸载。"),
                "emulator" => ("Android 模拟器程序", "删了模拟器起不来，要在 SDK Manager 里重新安装。"),
                "build-tools" | "platforms" => ("Android 编译工具 / 各安卓版本的开发平台（可能有好几个版本）", "项目用到的版本必须留着；旧版本可以在 SDK Manager 里卸载，缺了编译时会提示下载。"),
                "sources" => ("Android 源码（给调试时看系统源码用）", "删了只是调试时看不到系统源码。"),
                "ndk-bundle" | "cmake" => ("Android 原生开发用的 CMake / 旧版 NDK", "只有原生代码项目需要；要用时可重新下载。"),
                _ => return None,
            };
            return Some(hint(
                "android-sdk-part",
                Tier::Review,
                Category::App,
                what,
                "Android SDK 的组成部分，用 SDK Manager 按需安装",
                lose,
                vec![How::BuiltinTool],
                sdk_how,
                0.75,
                Some("Android SDK".into()),
                vec![format!("在 Android SDK「{}」里", p.path)],
            ));
        }
    }
    if has_child(n, "avd") && ["debug.keystore", "adbkey", "cache", "analytics.settings"].iter().any(|x| has_child(n, x)) {
        return Some(hint(
            "android-user-home",
            Tier::Review,
            Category::Userdata,
            "Android 工具的用户目录（默认是 ~\\.android，可用 ANDROID_USER_HOME 改）",
            "avd 是你建的安卓模拟器（含模拟器里装的应用和数据）；debug.keystore 是调试签名；cache 是缓存",
            "avd 删了模拟器和里面的数据都没了（可重建）；debug.keystore 删了会自动重新生成，但之前装到手机上的调试版应用要卸载重装。",
            vec![How::Manual],
            "模拟器在 Android Studio 的 Device Manager 里删；cache 可以删；debug.keystore 别动。",
            0.7,
            Some("Android SDK".into()),
            vec![format!("子项：{}", sizes_of(n.children.iter(), 6))],
        ));
    }
    if name_lc(n).ends_with(".avd") && t.parent.is_some_and(|p| p.name.eq_ignore_ascii_case("avd")) {
        return Some(hint(
            "android-avd",
            Tier::Review,
            Category::Vm,
            format!("一台 Android 模拟器「{}」（系统盘、快照和里面装的应用）", n.name.trim_end_matches(".avd")),
            "Android Studio 建的模拟器，userdata、快照（snapshots）、SD 卡镜像都在这个文件夹里",
            "删了这台模拟器和里面的数据就没了，可以重新建。",
            vec![How::BuiltinTool],
            "Android Studio → Device Manager 里删除这台模拟器，或只“Wipe Data”、删掉不用的快照。",
            0.75,
            Some("Android Emulator".into()),
            Vec::new(),
        ));
    }
    if name_lc(n) == "avd" && n.children.iter().any(|c| c.name.to_lowercase().ends_with(".avd")) {
        return Some(hint(
            "android-avd",
            Tier::Review,
            Category::Vm,
            "Android 模拟器（AVD），每个 .avd 文件夹是一台模拟器",
            "Android Studio 建的模拟器，里面有模拟器的系统盘、快照和你装的应用",
            "删了那台模拟器和里面的数据就没了，可以重新建。",
            vec![How::BuiltinTool],
            "Android Studio → Device Manager 里删不用的模拟器（或 avdmanager delete avd -n 名字）；只想腾空间可以先“Wipe Data”或删快照。",
            0.75,
            Some("Android Emulator".into()),
            vec![format!("模拟器：{}", sizes_of(n.children.iter().filter(|c| is_dir(c)), 6))],
        ));
    }
    None
}

fn is_gradle_home(n: &Node) -> bool {
    is_dir(n) && has_child(n, "caches") && (has_child(n, "wrapper") || has_child(n, "daemon") || has_child(n, "native"))
}

fn gradle(t: &Target) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    if is_gradle_home(n) {
        return Some(hint(
            "gradle-home",
            Tier::Review,
            Category::Cache,
            "Gradle 的用户目录（GRADLE_USER_HOME，默认 ~\\.gradle）",
            "caches 是下载的依赖和构建缓存，wrapper\\dists 是各项目用的 Gradle 版本，daemon 是后台进程的日志；gradle.properties 是你的配置",
            "caches、wrapper\\dists、daemon 删了下次构建会自动重新下载（要联网、第一次很慢），不丢代码；gradle.properties / init.d 是配置，别删。Gradle 自己也会定期清理长期没用的缓存。",
            vec![How::Manual],
            "关掉 Android Studio / IDE 并停掉 Gradle（gradle --stop）后，删 caches、wrapper\\dists、daemon；配置文件留着。",
            0.75,
            Some("Gradle".into()),
            vec![format!("子项：{}", sizes_of(n.children.iter(), 6))],
        ));
    }
    if let Some(p) = t.parent {
        if is_gradle_home(p) {
            let nm = name_lc(n);
            let what = match nm.as_str() {
                "caches" => "Gradle 下载的依赖包和构建缓存",
                "wrapper" => "Gradle Wrapper 下载的各版本 Gradle 本体",
                "daemon" => "Gradle 后台进程的日志和注册信息",
                _ => return None,
            };
            return Some(hint(
                "gradle-cache",
                Tier::Review,
                Category::Cache,
                what,
                "Gradle 构建时自动下载 / 生成，可重新得到",
                "删了下次构建会重新下载（要联网、第一次很慢），不丢代码。",
                vec![How::Delete],
                "先关掉 IDE 并执行 gradle --stop，再删除。",
                0.75,
                Some("Gradle".into()),
                vec![format!("在 Gradle 用户目录「{}」里", p.path)],
            ));
        }
    }
    None
}

fn is_flutter_bin(n: &Node) -> bool {
    is_dir(n) && (has_child(n, "flutter.bat") || has_child(n, "flutter")) && (has_child(n, "dart.bat") || has_child(n, "cache") || has_child(n, "internal"))
}

fn flutter_and_pub(t: &Target) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    let nm = name_lc(n);
    if has_child(n, "packages") && child(n, "bin").is_some_and(is_flutter_bin) {
        return Some(hint(
            "flutter-sdk",
            Tier::Review,
            Category::App,
            "Flutter SDK（Flutter 开发工具包）",
            "flutter 命令、框架源码（packages）；bin\\cache 里是它自己下载的 Dart SDK 和各平台引擎文件",
            "整个删 Flutter 项目就跑不起来了，要重新下载 SDK。只删 bin\\cache 的话下次运行 flutter 会自动重新下载（几百 MB 到几 GB）。",
            vec![How::Manual],
            "不用 Flutter 了可以整个删（记得从 PATH 里去掉）；还在用就别动，或者只清 bin\\cache 后运行 flutter precache 重新下载需要的部分。",
            0.75,
            Some("Flutter".into()),
            vec![format!("子项：{}", sizes_of(n.children.iter().filter(|c| is_dir(c)), 6))],
        ));
    }
    if nm == "bin" && is_flutter_bin(n) && has_child(n, "cache") {
        return Some(hint(
            "flutter-sdk",
            Tier::Review,
            Category::App,
            "Flutter 的命令和它下载的工具缓存（cache 里是 Dart SDK 和引擎文件）",
            "flutter 命令第一次运行时把 Dart SDK、各平台引擎下载到 bin\\cache",
            "flutter.bat 等命令别删；cache 删了下次运行 flutter 会重新下载。",
            vec![How::Manual],
            "只清 cache 子文件夹，然后运行 flutter precache（或下次正常运行 flutter）重新下载。",
            0.7,
            Some("Flutter".into()),
            Vec::new(),
        ));
    }
    if nm == "cache" && t.parent.is_some_and(is_flutter_bin) {
        return Some(hint(
            "flutter-cache",
            Tier::Review,
            Category::Cache,
            "Flutter 下载的 Dart SDK 和各平台引擎文件",
            "flutter 命令自动下载到 bin\\cache，版本升级时会更新",
            "删了下次运行 flutter 会自动重新下载（要联网，几百 MB 到几 GB），不丢项目代码。",
            vec![How::Delete],
            "关掉 IDE 和正在运行的 flutter / dart 进程后删除，再运行 flutter precache。",
            0.75,
            Some("Flutter".into()),
            Vec::new(),
        ));
    }
    let pub_named = matches!(nm.as_str(), ".pub-cache" | "pub_cache" | "pub-cache");
    let pub_shaped = has_child(n, "hosted") && (has_child(n, "hosted-hashes") || has_child(n, "active_roots") || has_child(n, "git"));
    if pub_shaped || (pub_named && has_child(n, "hosted")) {
        return Some(hint(
            "pub-cache",
            Tier::Review,
            Category::Cache,
            "Dart / Flutter 下载的依赖包缓存（pub cache）",
            "flutter pub get / dart pub get 把项目依赖下载到这里，所有项目共用",
            "删了下次 pub get 会重新下载，不丢代码；用 dart pub global activate 装的全局命令也在这里，会一起没掉。",
            vec![How::Delete],
            "dart pub cache clean（或 flutter pub cache clean），之后在项目里 pub get 重新下载。",
            0.75,
            Some("Dart / Flutter".into()),
            Vec::new(),
        ));
    }
    None
}

// ───────────────────────── Rust 编译产物 ─────────────────────────

fn rust_target(t: &Target) -> Option<Hint> {
    let n = t.node;
    // cargo 在 target 目录里写 .rustc_info.json 和 CACHEDIR.TAG（老版本 / 中断的编译可能没有后者）。
    let markers = has_child(n, ".rustc_info.json") && ["CACHEDIR.TAG", "debug", "release"].iter().any(|x| has_child(n, x));
    let classic = name_lc(n) == "target" && t.parent.is_some_and(|p| has_child(p, "Cargo.toml"));
    if !(is_dir(n) && (markers || classic)) {
        return None;
    }
    Some(hint(
        "rust-target",
        Tier::Review,
        Category::Build,
        "Rust 项目的编译产物（cargo 的 target 文件夹）",
        "cargo build / test 生成的中间文件和程序；用 CARGO_TARGET_DIR 或 build.target-dir 指定位置时，它可以叫任何名字、放在任何地方",
        "删了只是下次要重新编译（几分钟到几十分钟），不丢源代码。正在编译时别删。",
        vec![How::Delete],
        "在项目目录执行 cargo clean，或直接删这个文件夹。",
        0.8,
        None,
        vec!["里面有 .rustc_info.json 和 CACHEDIR.TAG（cargo 建的编译缓存标记）".into()],
    ))
}

// ───────────────────────── 容器：这里面装着什么 ─────────────────────────

/// 给子文件夹造一个 Target（容器判断只看它自己和它的直接子项，不需要祖先链）。
fn child_target<'a>(c: &'a Node, parent: &'a Node) -> Target<'a> {
    Target { node: c, path: &c.path, parent: Some(parent), ancestors: &[] }
}

/// 几个开发工具包放在一起（比如 D:\desk\sdk 里有 android、gradle、flutter、pub_cache）。
fn dev_sdk_container(t: &Target) -> Option<Hint> {
    const SDK_HINTS: &[&str] = &["android-sdk", "android-user-home", "gradle-home", "flutter-sdk", "pub-cache", "conda-root", "conda-envs", "conda-base"];
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    let found: Vec<(String, String)> = n
        .children
        .iter()
        .filter(|c| is_dir(c) && !c.path.is_empty())
        .filter_map(|c| {
            let h = specific(&child_target(c, n))?;
            SDK_HINTS.contains(&h.name).then(|| (format!("{}（{}）", c.name, human_bytes(c.allocated)), h.what))
        })
        .collect();
    if found.len() < 2 {
        return None;
    }
    Some(hint(
        "dev-sdk-container",
        Tier::Review,
        Category::App,
        "放开发工具包（SDK）和它们缓存的文件夹",
        "里面有好几个开发工具：SDK 本体（编译要用）和它们下载的缓存（可以重新下载）混在一起",
        "整个删的话，用到这些工具的项目都编译不了、要重新下载好几 GB。请点进去分别看：缓存类（Gradle caches、pub 缓存、Flutter bin\\cache）可以删，SDK 本体按需在各自的管理工具里卸载。",
        vec![How::Manual],
        "点进去逐个处理，每个子文件夹都有单独的说明。",
        0.7,
        None,
        found.into_iter().map(|(a, b)| format!("{a}：{b}")).collect(),
    ))
}

const PROJECT_MARKERS: &[&str] =
    &[".git", "package.json", "Cargo.toml", "pyproject.toml", "go.mod", "pom.xml", "build.gradle", "build.gradle.kts", "CMakeLists.txt", "requirements.txt", "pubspec.yaml"];

fn is_project(n: &Node) -> bool {
    is_dir(n) && (PROJECT_MARKERS.iter().any(|m| has_child(n, m)) || has_file_ext(n, &["sln", "csproj"]))
}

/// 用户自己的代码 / 项目文件夹（不在系统目录和 AppData 里）。
fn user_projects(t: &Target) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    let segs = split_lower(t.path);
    // 系统目录、AppData 和各种“装好的库”（Python site-packages、node_modules、conda 包缓存）里的 package.json /
    // pyproject.toml 不是用户的项目。
    if in_system_area(&segs)
        || segs.iter().any(|s| matches!(s.as_str(), "site-packages" | "dist-packages" | "node_modules" | "pkgs" | ".cargo" | ".rustup" | "vendor"))
        || has_child(n, "site-packages")
        || name_lc(n) == "lib"
    {
        return None;
    }
    if name_lc(n) == ".git" {
        return Some(hint(
            "user-projects",
            Tier::Keep,
            Category::Userdata,
            "git 的版本库（这个项目的全部提交历史）",
            "git 把项目的每一次提交、分支和配置都存在 .git 里",
            "删了项目的历史记录、没推送到远程的提交和分支就全没了，只剩当前文件。",
            vec![How::Manual],
            "想省空间可以在项目里运行 git gc；不要删 .git。",
            0.8,
            Some("git".into()),
            Vec::new(),
        ));
    }
    if has_child(n, ".git") {
        return Some(hint(
            "user-projects",
            Tier::Keep,
            Category::Userdata,
            "一个 git 代码仓库（项目文件夹）",
            "里面有 .git，是用 git 管理的项目——你自己写的或者克隆下来的代码",
            "删了项目文件和没推送的改动就没了。项目里的编译产物（node_modules、target 等）会单独说明，可以按需清。",
            vec![How::Manual],
            "要腾空间就清项目里的编译产物 / 依赖（node_modules、target、.venv 等）；整个项目不要了先确认代码已经推送到远程。",
            0.7,
            None,
            Vec::new(),
        ));
    }
    if is_project(n) {
        return Some(hint(
            "user-projects",
            Tier::Keep,
            Category::Userdata,
            "一个代码项目",
            "里面有 package.json / Cargo.toml / pyproject.toml / *.sln 之类的项目文件，看起来是你自己的（或下载来的）项目",
            "删了项目文件就没了。项目里的编译产物（node_modules、target 等）会单独说明，可以按需清。",
            vec![How::Manual],
            "要腾空间就清项目里的编译产物和依赖；整个项目不要了自己确认后再删。",
            0.6,
            None,
            Vec::new(),
        ));
    }
    let mut projects: Vec<&Node> = Vec::new();
    for c in n.children.iter().filter(|c| is_dir(c)) {
        if is_project(c) {
            projects.push(c);
        } else {
            projects.extend(c.children.iter().filter(|g| is_project(g)));
        }
    }
    if projects.len() < 2 {
        return None;
    }
    Some(hint(
        "user-projects",
        Tier::Keep,
        Category::Userdata,
        format!("放代码项目的文件夹（找到 {} 个项目）", projects.len()),
        "里面有好几个带 .git / package.json / Cargo.toml / pyproject.toml 等标志的项目，看起来是你自己的工作文件夹",
        "这是你自己的文件，DiskWhy 不会替你判断能不能删。项目里的编译产物和依赖（node_modules、target、.venv）会单独说明，可以按需清。",
        vec![How::Manual],
        "点进去看哪个项目大；清编译产物和依赖最省事，项目本身自己决定。",
        0.6,
        None,
        vec![format!("项目：{}", sizes_of(projects.into_iter(), 8))],
    ))
}

/// 自己认不出来，但里面几乎全是（≥ 80%）一个认得出来的子文件夹（比如 DiskWhy-build 里只有一个 Rust target）。
fn dominant_child(t: &Target, depth: u8) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) || n.allocated == 0 {
        return None;
    }
    let c = n.children.iter().filter(|c| is_dir(c) && !c.path.is_empty()).max_by_key(|c| c.allocated)?;
    if (c.allocated as f64) < n.allocated as f64 * 0.8 {
        return None;
    }
    let ct = child_target(c, n);
    // 子文件夹自己也可能只是“几乎全是某某”的一层包装（D:\Games → Minecraft → .minecraft），最多再往下看一层。
    let h = specific(&ct).or_else(|| if depth > 1 { dominant_child(&ct, depth - 1) } else { None })?;
    Some(Hint {
        name: "dominant-child",
        what: format!("里面几乎全是「{}」：{}", c.name, h.what),
        clues: {
            let mut v = vec![format!("「{}」占这里的 {:.0}%（{}）", c.name, c.allocated as f64 * 100.0 / n.allocated as f64, human_bytes(c.allocated))];
            v.extend(h.clues);
            v
        },
        confidence: (h.confidence - 0.1).max(0.3),
        // 后果跟着认出来的那个子文件夹走，不是“dominant-child”这个名字。
        lose_kind: h.lose_kind.or_else(|| super::heuristics::heuristic_lose_kind(h.name, h.tier)),
        lose_short: h.lose_short.or_else(|| super::heuristics::heuristic_lose_short(h.name)),
        ..h
    })
}

// ───────────────────────── 其他工具的仓库 / 用户自己装在别处的软件 ─────────────────────────

/// 系统目录、Program Files、AppData（这些地方有规则库和注册表归属来管）。
fn in_system_area(segs: &[String]) -> bool {
    segs.iter().any(|s| s == "appdata")
        || segs.get(1).is_some_and(|s| matches!(s.as_str(), "windows" | "program files" | "program files (x86)" | "programdata" | "$recycle.bin"))
}

/// pnpm 的全局内容寻址仓库、Ollama 的模型文件夹（用户常把它们挪到别的盘）。
fn tool_stores(t: &Target) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    let nm = name_lc(n);
    let pnpm_version_dir = |d: &Node| is_dir(d) && d.name.len() >= 2 && d.name.to_lowercase().starts_with('v') && has_child(d, "files") && has_child(d, "index");
    if nm == ".pnpm-store" || nm == "pnpm-store" || pnpm_version_dir(n) || n.children.iter().any(pnpm_version_dir) && nm.contains("pnpm") {
        return Some(hint(
            "pnpm-store",
            Tier::Review,
            Category::Cache,
            "pnpm 的全局包仓库（所有项目共用的 npm 包，按内容去重存放）",
            "pnpm 把下载过的包存在这里，再用硬链接放进各项目的 node_modules，所以看起来大，其实各项目共用一份",
            "直接删会让已有项目的 node_modules 里的硬链接失效或需要重新下载；用 pnpm store prune 只删没有项目在用的包，最安全。",
            vec![How::BuiltinTool],
            "在任意目录运行 pnpm store prune（删掉没有被任何项目引用的包）；pnpm store path 可以看当前用的是哪个仓库。",
            0.75,
            Some("pnpm".into()),
            Vec::new(),
        ));
    }
    if has_child(n, "blobs") && has_child(n, "manifests") && child(n, "blobs").is_some_and(|b| b.children.iter().any(|c| c.name.to_lowercase().starts_with("sha256"))) {
        return Some(hint(
            "ollama-models-dir",
            Tier::Review,
            Category::Model,
            "Ollama 的模型文件夹（blobs 里是模型权重，manifests 记录装了哪些模型）",
            "Ollama 默认把模型放在 ~\\.ollama\\models，可以用 OLLAMA_MODELS 环境变量改到别的位置（官方 FAQ）；这里的结构和它一样",
            "直接删 blobs 会让已下载的模型全坏掉，要重新下载（每个几 GB）。",
            vec![How::BuiltinTool, How::Migrate],
            "用 ollama list 看有哪些模型，ollama rm 模型名 删不用的；想换盘就设置 OLLAMA_MODELS 后把整个文件夹搬过去。",
            0.75,
            Some("Ollama".into()),
            Vec::new(),
        ));
    }
    None
}

/// 名字像版本号的文件夹（6.1.0.653、9.9.33-52230、7.73.9）。
fn looks_like_version(name: &str) -> bool {
    let digits = name.chars().filter(|c| c.is_ascii_digit()).count();
    digits >= 3 && name.chars().next().is_some_and(|c| c.is_ascii_digit()) && name.contains('.') && name.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '_')
}

/// 装在系统目录以外（比如 D:\QQ、D:\quark）的软件，以及 Visual Studio、驱动安装包文件夹。
fn installed_elsewhere(t: &Target) -> Option<Hint> {
    let n = t.node;
    if !is_dir(n) {
        return None;
    }
    let segs = split_lower(t.path);
    if let Some(j) = segs.iter().position(|s| s == "microsoft visual studio") {
        if j + 1 <= segs.len() {
            return Some(hint(
                "visual-studio-install",
                Tier::Review,
                Category::App,
                "Visual Studio 的安装目录（程序本体）",
                "Visual Studio Installer 可以把 Visual Studio 装到任意盘；数字文件夹（17、18）是大版本号，下面是 Community / Professional 等版本",
                "直接删 Visual Studio 就坏了，而且 Installer 里的记录会残留。",
                vec![How::BuiltinTool],
                "打开 Visual Studio Installer：“修改”去掉不用的工作负载 / 组件，或“卸载”；Installer 的“删除下载缓存”选项也能省空间。",
                0.7,
                Some("Visual Studio".into()),
                Vec::new(),
            ));
        }
    }
    if in_system_area(&segs) {
        return None;
    }
    let uninstaller = n.children.iter().find(|c| {
        c.kind == NodeKind::File && {
            let l = c.name.to_lowercase();
            l == "uninstall.exe" || l.starts_with("unins0") && l.ends_with(".exe") || l == "uninst.exe"
        }
    });
    if let Some(u) = uninstaller {
        let versions: Vec<&Node> = n
            .children
            .iter()
            .chain(child(n, "versions").map(|v| v.children.iter()).into_iter().flatten())
            .filter(|c| is_dir(c) && looks_like_version(&c.name))
            .collect();
        let mut clues = vec![format!("里面有卸载程序 {}", u.name)];
        if versions.len() >= 2 {
            clues.push(format!(
                "有 {} 个版本号文件夹（{}）：通常只有最新的一个在用，旧的多半是升级残留，但 DiskWhy 不能确认，建议用软件自己的修复 / 重装来清理",
                versions.len(),
                sizes_of(versions.into_iter(), 5)
            ));
        }
        return Some(hint(
            "app-install-elsewhere",
            Tier::Review,
            Category::App,
            format!("软件「{}」的安装目录（程序本体）", n.name),
            "你把这个软件装在了这里（有卸载程序）；聊天记录、下载的文件一般不在安装目录里",
            "直接删文件夹软件就打不开了，开始菜单、注册表里的记录也会残留。想腾空间请正常卸载。",
            vec![How::Manual],
            "在“设置 > 应用 > 已安装的应用”里卸载（或运行文件夹里的卸载程序）；不要直接删文件夹。",
            0.6,
            Some(n.name.clone()),
            clues,
        ));
    }
    let nm = name_lc(n);
    if segs.len() == 2 && matches!(nm.as_str(), "driver" | "drivers" | "驱动" | "驱动程序") {
        return Some(hint(
            "driver-packages",
            Tier::Review,
            Category::Installer,
            "你存的驱动安装包（显卡、芯片组、声卡等）",
            "装驱动时下载 / 解压出来的安装文件；装好之后，Windows 会把驱动另存进 System32\\DriverStore，系统用的是那一份",
            "删了已经装好的驱动不受影响；以后重装系统或回滚驱动时要重新下载。电脑厂商的驱动包有时不好再找到，先确认。",
            vec![How::Manual],
            "确认驱动都已经装好、以后能从官网重新下载，再删不需要的旧版本安装包。",
            0.5,
            None,
            vec![format!("子项：{}", sizes_of(n.children.iter().filter(|c| is_dir(c)), 6))],
        ));
    }
    None
}
