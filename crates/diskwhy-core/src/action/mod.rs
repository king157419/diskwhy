//! 阶段 D：带护栏的动作（清理清单 → 预检 → 执行 → 可撤销）。
//!
//! 流程：
//! 1. [`build_plan`]：GUI 的清理清单（只有路径）→ [`Plan`]。**档位、来源、规则都由后端用当前扫描结果和
//!    规则库重新算**，不信任前端传来的任何说明；被拦下的项不枚举内容。能进清单的项会重新枚举磁盘上的
//!    实际内容（扫描树里每个目录只留了前 50 个文件，不能拿来当删除清单），给每个文件记下指纹
//!    （文件 ID / 大小 / 修改时间）。
//! 2. [`preflight`]：逐项、逐条检查，给出阻止 / 跳过 / 警告（见 [`preflight`] 模块说明的 9 条护栏）。
//! 3. [`execute`]：只处理预检标记为“删”的条目；执行前每个条目再取一次指纹，对不上就跳过；
//!    回收站走 `IFileOperation`，隔离区走 `MoveFileExW`（跨卷：复制 → 逐字节校验 → 删除）；
//!    每条结果写进 `%LOCALAPPDATA%\DiskWhy\actions.log`（JSON lines）。最多 4 个线程并发，
//!    失败不回滚已成功的项。
//! 4. 隔离区可以一键还原（[`quarantine::restore`]）或彻底清空（[`quarantine::purge`]）。
//!
//! `builtin_tool` / `migrate` 只给指引（[`guidance`]），**本阶段不自动运行任何系统命令**。

pub mod execute;
pub mod guidance;
pub mod log;
pub mod plan;
pub mod preflight;
pub mod protect;
pub mod quarantine;
pub mod refresh;
pub mod winops;

pub use execute::{execute, ExecOptions, ExecProgress, ExecReport, ItemResult};
pub use plan::{build_plan, build_plan_quick, enumerate_plan};
pub use preflight::{preflight, preflight_with, quote, PreflightOptions, PreflightReport};

use crate::explain::{How, Source, Tier};
use serde::{Deserialize, Serialize};
use winops::{Fingerprint, LockingProcess};

/// 预览里每项最多列多少条。
pub const PREVIEW_LIMIT: usize = 200;
/// 30 分钟内修改过的文件视为“正在用”，跳过。
pub const HOT_WINDOW_SECS: i64 = 30 * 60;
/// 执行并发上限。
pub const MAX_WORKERS: usize = 4;
/// 单个清单项最多枚举多少条（超过就整项拒绝，让用户分批）。
pub const MAX_ENTRIES_PER_ITEM: usize = 500_000;
/// 回收站所在卷可用空间低于这个比例时提示“删了不会立刻腾空间”。
pub const LOW_SPACE_RATIO: f64 = 0.10;

/// 怎么删。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Strategy {
    /// 送进回收站（默认）。
    RecycleBin,
    /// 移到 `<dest_root>\DiskWhy-Quarantine\<时间戳>\<原路径映射>`，保留 7 天，可一键还原。
    Quarantine { dest_root: String },
}

impl Strategy {
    pub fn label(&self) -> String {
        match self {
            Strategy::RecycleBin => "回收站".into(),
            Strategy::Quarantine { dest_root } => format!("隔离区（{dest_root}）"),
        }
    }
}

/// 清理清单里的一项（前端只传路径；`expect_rule` 是加入清单时看到的规则，用来发现“说明已经变了”）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    pub path: String,
    #[serde(default)]
    pub expect_rule: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    /// 联接 / 符号链接 / 挂载点：不跟随，只处理链接自身。
    Link,
    /// 云文件目录（OneDrive 等）：不进去，整个跳过。
    CloudDir,
}

/// 清单项里实际会被处理的一条（文件或链接）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub kind: EntryKind,
    pub attrs: u32,
    pub reparse_tag: u32,
    /// 逻辑大小。
    pub size: u64,
    /// 能腾出的字节：磁盘占用按簇取整；硬链接（还有别的名字）为 0。
    pub bytes: u64,
    pub mtime: i64,
    /// 生成清单时的指纹（云占位不打开，没有指纹）。
    pub fp: Option<Fingerprint>,
    /// 链接是否是目录型（联接 / 目录符号链接）。
    #[serde(default)]
    pub link_is_dir: bool,
}

/// 跳过的原因。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkipReason {
    /// 30 分钟内修改过。
    Hot { minutes_ago: i64 },
    /// 云占位 / 云文件（删了可能连云端一起删，或触发下载）。
    Cloud,
    /// 被进程占用（Restart Manager 报告，只报告不关进程）。
    Locked { processes: Vec<LockingProcess> },
    /// 没有删除权限。
    NoPermission { code: u32 },
    /// 受保护路径。
    Protected { why: String },
    /// 生成清单后被改过 / 替换过。
    Changed { what: String },
    /// 已经不在了。
    Missing,
    /// 读不到信息。
    Unreadable { code: u32 },
}

impl SkipReason {
    pub fn code(&self) -> &'static str {
        match self {
            SkipReason::Hot { .. } => "hot",
            SkipReason::Cloud => "cloud",
            SkipReason::Locked { .. } => "locked",
            SkipReason::NoPermission { .. } => "no_permission",
            SkipReason::Protected { .. } => "protected",
            SkipReason::Changed { .. } => "changed",
            SkipReason::Missing => "missing",
            SkipReason::Unreadable { .. } => "unreadable",
        }
    }

    pub fn text(&self) -> String {
        match self {
            SkipReason::Hot { minutes_ago } => format!("{minutes_ago} 分钟前刚改过（30 分钟内修改的文件可能正在用，跳过）"),
            SkipReason::Cloud => "云端文件 / 占位文件（OneDrive 等）：删了可能连云端的一起删，DiskWhy 不碰".into(),
            SkipReason::Locked { processes } if processes.is_empty() => "正在被别的程序占用（没查到是哪个进程）".into(),
            SkipReason::Locked { processes } => format!(
                "正在被占用：{}（DiskWhy 只报告，不会关掉它们）",
                processes.iter().map(|p| format!("{}（PID {}）", p.name, p.pid)).collect::<Vec<_>>().join("、")
            ),
            SkipReason::NoPermission { code } => format!("没有删除权限（Win32 错误 {code}）"),
            SkipReason::Protected { why } => why.clone(),
            SkipReason::Changed { what } => format!("生成清单之后被改过：{what}（防止删掉被替换的文件，跳过）"),
            SkipReason::Missing => "已经不在了".into(),
            SkipReason::Unreadable { code } => format!("读不到文件信息（Win32 错误 {code}）"),
        }
    }
}

/// 预检对一条的决定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Decision {
    /// 还没预检。
    Pending,
    /// 删（进回收站 / 隔离区）。
    Delete,
    /// 链接：只删链接自身（同卷隔离时移动链接自身）。
    RemoveLink,
    Skip { reason: SkipReason },
}

/// 清单里的一项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub path: String,
    pub is_dir: bool,
    /// 扫描结果里的占用（给界面对照）。
    pub scan_allocated: u64,
    pub tier: Tier,
    /// 解释来源是 Unknown（界面上的灰色）。
    pub unknown: bool,
    pub source: Source,
    pub source_label: String,
    pub rule_id: Option<String>,
    pub what: String,
    pub owner: Option<String>,
    pub lose: String,
    pub how: Vec<How>,
    /// 被拦下的原因（档位 / 保护路径 / 规则不一致 / …）。拦下的项不会执行。
    pub blocked: Option<String>,
    /// 全部条目（文件 / 链接 / 云目录）。
    pub entries: Vec<Entry>,
    /// 与 `entries` 一一对应。
    pub decisions: Vec<Decision>,
    /// 子文件夹（最深的在前）；里面的东西处理完变空了就删掉。清单项本身的文件夹保留。
    pub dirs: Vec<String>,
    /// 枚举时出的错（列不出的子目录等）。
    pub enum_errors: Vec<String>,
    /// 所在卷的簇大小（预检取到指纹后按它把磁盘占用取整）。
    #[serde(default = "default_cluster")]
    pub cluster: u64,
}

fn default_cluster() -> u64 {
    4096
}

/// 计划 + 预检的进度（阶段 G）。各阶段在工作线程里更新这些计数，界面层自己定时读（不在热路径上发消息）；
/// `cancel` 置位后枚举 / 预检尽快停下，计划不会标记为已预检，不能执行。
#[derive(Debug, Default)]
pub struct Progress {
    /// 0 = 生成清单，1 = 列文件，2 = 逐个检查，3 = 问是哪些程序在占用，4 = 完成
    pub phase: std::sync::atomic::AtomicU8,
    pub dirs: std::sync::atomic::AtomicUsize,
    pub entries: std::sync::atomic::AtomicUsize,
    pub checked: std::sync::atomic::AtomicUsize,
    pub to_check: std::sync::atomic::AtomicUsize,
    pub locked: std::sync::atomic::AtomicUsize,
    pub rm_done: std::sync::atomic::AtomicUsize,
    pub rm_total: std::sync::atomic::AtomicUsize,
    pub cancel: std::sync::atomic::AtomicBool,
}

/// [`Progress`] 某一刻的读数（发给界面）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProgressView {
    /// `plan` / `list` / `check` / `who` / `done`
    pub phase: String,
    pub dirs: usize,
    pub entries: usize,
    pub checked: usize,
    pub to_check: usize,
    pub locked: usize,
    pub rm_done: usize,
    pub rm_total: usize,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn set_phase(&self, p: u8) {
        self.phase.store(p, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn view(&self) -> ProgressView {
        use std::sync::atomic::Ordering::Relaxed;
        ProgressView {
            phase: match self.phase.load(Relaxed) {
                0 => "plan",
                1 => "list",
                2 => "check",
                3 => "who",
                _ => "done",
            }
            .into(),
            dirs: self.dirs.load(Relaxed),
            entries: self.entries.load(Relaxed),
            checked: self.checked.load(Relaxed),
            to_check: self.to_check.load(Relaxed),
            locked: self.locked.load(Relaxed),
            rm_done: self.rm_done.load(Relaxed),
            rm_total: self.rm_total.load(Relaxed),
        }
    }
}

impl PlanItem {
    /// 会被删掉的条目（预检之后才准）。
    pub fn will_delete(&self) -> impl Iterator<Item = (&Entry, &Decision)> {
        self.entries.iter().zip(self.decisions.iter()).filter(|(_, d)| matches!(d, Decision::Delete | Decision::RemoveLink))
    }
    pub fn entry_count(&self) -> usize {
        self.will_delete().count()
    }
    pub fn bytes(&self) -> u64 {
        self.will_delete().map(|(e, _)| e.bytes).sum()
    }
    /// 前 N 条实际会删的文件。
    pub fn entries_preview(&self, n: usize) -> Vec<EntryView> {
        self.will_delete().take(n).map(|(e, d)| EntryView::of(e, d)).collect()
    }
}

/// 一次清理的计划。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub created_at: i64,
    pub scan_root: String,
    pub strategy: Strategy,
    pub items: Vec<PlanItem>,
    /// 计划层面的提示（合并了重复项等）。
    pub warnings: Vec<String>,
    /// 预检做过了没有（没做过不许执行）。
    pub preflight_done: bool,
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().filter(|i| i.blocked.is_none()).map(|i| i.bytes()).sum()
    }
}

/// 给界面看的一条。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryView {
    pub path: String,
    pub kind: EntryKind,
    pub bytes: u64,
    pub mtime: i64,
    /// 附注（“链接：只删链接本身”“硬链接，删了不腾空间”）。
    pub note: Option<String>,
}

impl EntryView {
    pub fn of(e: &Entry, d: &Decision) -> Self {
        let note = match (e.kind, d) {
            (EntryKind::Link, _) => Some(format!(
                "这是{}：不跟进去，只删链接本身，它指向的内容不受影响",
                if e.link_is_dir { "目录联接 / 目录链接" } else { "文件符号链接" }
            )),
            _ if e.fp.is_some_and(|f| f.links > 1) => Some(format!("硬链接（共 {} 个名字）：删掉这个名字不腾空间", e.fp.unwrap().links)),
            _ => None,
        };
        EntryView { path: e.path.clone(), kind: e.kind, bytes: e.bytes, mtime: e.mtime, note }
    }
}

/// 生成一个短 id（时间 + 计数）。
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{prefix}{}-{:x}{:02x}", fmt_stamp(crate::explain::now_unix()), (nanos % 0xFFFF) as u32, n % 256)
}

/// Unix 秒 → `YYYYMMDD-HHMMSS`（本地时区不可靠地取不到，统一用 UTC，并在界面上标注）。
pub fn fmt_stamp(unix: i64) -> String {
    let (y, m, d, hh, mm, ss) = civil(unix);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// Unix 秒 → ISO 8601（UTC）。
pub fn fmt_iso(unix: i64) -> String {
    let (y, m, d, hh, mm, ss) = civil(unix);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Howard Hinnant 的 civil_from_days。
fn civil(unix: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, (secs / 3600) as u32, ((secs % 3600) / 60) as u32, (secs % 60) as u32)
}

#[cfg(test)]
mod tests {
    #[test]
    fn stamps() {
        assert_eq!(super::fmt_iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::fmt_iso(1_790_186_441), "2026-09-23T18:00:41Z");
        assert_eq!(super::fmt_stamp(951_782_400), "20000229-000000");
    }
}
