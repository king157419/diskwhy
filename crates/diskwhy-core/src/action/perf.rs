//! 执行耗时模型（阶段 J）：预检报告的 `est_secs`、执行进度的成本加权都用它。
//!
//! 每种操作记“**单线程**时每个文件的耗时”（毫秒，含执行前的重新校验和保护路径检查）；多线程的加速按
//! Amdahl 定律算：`speedup(p) = p / (1 + (p − 1) × serial_frac)`（文件系统元数据操作在多线程下互相等锁，
//! 实测 16 线程直接删除只快 3.4 倍，见下面的基准）。估计：
//!
//! ```text
//! est_secs = Σ 个数 × 单文件耗时 / speedup(p) / 1000          ← 逐个文件，多线程
//!          + (子文件夹数 × rmdir_ms + 暂存条数 × manifest_ms) / 1000   ← 收尾，单线程，不打折
//!          + 跨盘字节 / 吞吐 + 0.2
//! ```
//!
//! 学习：每次执行完，收尾两项直接按实测的“每个文件夹 / 每条”往里拉（≥ 200 个才学）；其余按这次实际做了的
//! 工作量算出“预测用时”，和实测墙钟时间（扣掉固定开销和收尾的实测时间）的比值 k，把这次做过
//! （≥ 200 个）的操作的单文件耗时按指数滑动平均（α = 0.5）往 `原值 × k` 拉；跨盘字节 ≥ 64 MB 时吞吐往
//! `原值 / k` 拉。写回 `%LOCALAPPDATA%\DiskWhy\perf.json`。
//!
//! 默认值（2026-10-06 本机实测，16 线程，`tests/bench_tests.rs`，数据在 D: 盘）：
//! | 操作 | 实测 | 单线程等效 |
//! |---|---|---|
//! | 直接删除 | 1 线程 20,000 个 4.08 s（4,901 个/秒）；16 线程 200,070 个 11.96 s（16,724 个/秒，含收尾）；2026-10-07 复测 16 线程 10 万个 64 字节文件扣掉收尾 3.7–4.3 s | 0.15 ms，serial_frac 0.25 |
//! | 同盘暂存（改名） | 16 线程 200,070 个 38.15 s（5,244 个/秒）；2026-10-07 复测 10 万个扣掉收尾 17.8–20.0 s | 0.65 ms |
//! | 回收站逐个 | 1 线程 600 个 2.70 s（222 个/秒）；3 线程 600 个 1.81 s（332 个/秒） | 4.5 ms |
//! | 回收站整夹 | 2 线程 800 个 0.30 s（2,624 个/秒） | ≈ 0.7 ms |
//! | 跨盘暂存（C: → D:，小文件） | 8 线程 2,000 个 2.18 s（916 个/秒） | 3.0 ms + 字节 / 150 MB/s |
//!
//! | 收尾：删空文件夹（单线程，含先列一遍） | 2026-10-07：2,100 个 0.25–0.30 ms/个；20,500 个 0.23–0.26 ms/个（5 秒多） | 0.25 ms |
//! | 收尾：写 manifest.json | 2026-10-07：10 万条 159–171 ms | 0.002 ms/条 |
//!
//! 回收站默认取 4.0 ms（逐个和整夹混合、偏保守）；以上都会被每次执行的实测慢慢校正。
//! 2026-10-07 标定后（`bench_tests`，D: 盘 10 万个 64 字节文件）预计 / 实际：直接删除 2,100 个文件夹 5.2 s / 4.5–5.0 s，
//! 20,500 个文件夹 9.8 s / 9.3 s；同盘暂存 2,100 个文件夹 20.2 s / 18.7 s，20,500 个文件夹 24.8 s / 25.6 s。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 耗时模型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerfModel {
    /// 直接删除（含链接只删自身）：单线程每个文件的毫秒数。
    pub permanent_ms: f64,
    /// 送回收站（逐个文件 / 整个文件夹混合，按文件数平均）。
    pub recycle_ms: f64,
    /// 同盘暂存（改名）。
    pub rename_ms: f64,
    /// 跨盘暂存每个文件的固定开销（不含拷贝字节的时间）。
    pub xvol_ms: f64,
    /// 跨盘拷贝总吞吐（MB/s）。
    pub xvol_mb_s: f64,
    /// 收尾：删空文件夹（每个文件夹，含先列一遍看空不空）。收尾是**单线程**做的（做完最后一块的那个线程），
    /// 不享受多线程加速。
    pub rmdir_ms: f64,
    /// 收尾：整份写暂存区清单 `manifest.json`（每条）。也是单线程。
    #[serde(default = "default_manifest_ms")]
    pub manifest_ms: f64,
    /// Amdahl 串行比例（0–1）：多线程加速 = p / (1 + (p − 1) × serial_frac)。
    #[serde(default = "default_serial")]
    pub serial_frac: f64,
    /// 已经学习过几次。
    #[serde(default)]
    pub runs: u32,
}

fn default_serial() -> f64 {
    0.25
}

fn default_manifest_ms() -> f64 {
    MANIFEST_MS
}

impl Default for PerfModel {
    fn default() -> Self {
        // 本机基准（2026-10-06，D: 盘，release 构建，见模块说明里的表）。
        PerfModel {
            permanent_ms: PERMANENT_MS,
            recycle_ms: RECYCLE_MS,
            rename_ms: RENAME_MS,
            xvol_ms: XVOL_MS,
            xvol_mb_s: 150.0,
            rmdir_ms: RMDIR_MS,
            manifest_ms: MANIFEST_MS,
            serial_frac: 0.25,
            runs: 0,
        }
    }
}

/// 直接删除（单线程，每个文件）。
const PERMANENT_MS: f64 = 0.15;
/// 回收站（单线程，每个文件）。
const RECYCLE_MS: f64 = 4.0;
/// 同盘改名（单线程，每个文件）。
const RENAME_MS: f64 = 0.65;
/// 跨盘每个文件的固定开销（复制建文件 + 逐字节比对打开两次 + 删除原文件）。
const XVOL_MS: f64 = 3.0;
/// 收尾删空文件夹（每个）。
const RMDIR_MS: f64 = 0.25;
/// 收尾写 manifest.json（每条）。
const MANIFEST_MS: f64 = 0.002;

/// 要估计的工作量。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Workload {
    pub permanent: usize,
    pub recycle: usize,
    pub rename: usize,
    pub xvol: usize,
    pub xvol_bytes: u64,
    /// 链接（直接删链接本身，按“直接删除”算）。
    pub links: usize,
    /// 收尾要看一遍（空了就删）的子文件夹个数。
    pub dirs: usize,
    /// 收尾要写进 manifest.json 的条数（去暂存区的）。
    #[serde(default)]
    pub manifest: usize,
}

impl Workload {
    pub fn entries(&self) -> usize {
        self.permanent + self.recycle + self.rename + self.xvol + self.links
    }
}

/// 一次执行的实测（各线程累计的忙碌时间，毫秒）。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Measured {
    pub permanent: (usize, f64),
    pub recycle: (usize, f64),
    pub rename: (usize, f64),
    /// (个数, 毫秒, 字节)
    pub xvol: (usize, f64, u64),
    /// 收尾看过的文件夹（个数, 毫秒）：列一遍 + 空了就删。
    pub rmdir: (usize, f64),
    /// 其他（跳过、记日志等）的忙碌时间。
    pub other_ms: f64,
    /// 收尾写 manifest.json（条数, 毫秒）。
    #[serde(default)]
    pub manifest: (usize, f64),
}

impl Measured {
    pub fn merge(&mut self, o: &Measured) {
        self.permanent.0 += o.permanent.0;
        self.permanent.1 += o.permanent.1;
        self.recycle.0 += o.recycle.0;
        self.recycle.1 += o.recycle.1;
        self.rename.0 += o.rename.0;
        self.rename.1 += o.rename.1;
        self.xvol.0 += o.xvol.0;
        self.xvol.1 += o.xvol.1;
        self.xvol.2 += o.xvol.2;
        self.rmdir.0 += o.rmdir.0;
        self.rmdir.1 += o.rmdir.1;
        self.other_ms += o.other_ms;
        self.manifest.0 += o.manifest.0;
        self.manifest.1 += o.manifest.1;
    }
    pub fn busy_ms(&self) -> f64 {
        self.permanent.1 + self.recycle.1 + self.rename.1 + self.xvol.1 + self.rmdir.1 + self.other_ms + self.manifest.1
    }
    /// 收尾（单线程）实际用的毫秒数。
    pub fn finalize_ms(&self) -> f64 {
        self.rmdir.1 + self.manifest.1
    }
    /// 这次实际做了的工作量。
    pub fn workload(&self) -> Workload {
        Workload {
            permanent: self.permanent.0,
            recycle: self.recycle.0,
            rename: self.rename.0,
            xvol: self.xvol.0,
            xvol_bytes: self.xvol.2,
            links: 0,
            dirs: self.rmdir.0,
            manifest: self.manifest.0,
        }
    }
}

/// 指数滑动平均的权重。0.5：清理一周才一两次，要两三次就贴近本机（2026-10-07 前是 0.3；收尾拆出来单独学之后
/// 逐个文件的那部分噪声小了）。
const ALPHA: f64 = 0.5;
const MIN_SAMPLE: usize = 200;
/// 固定开销（秒）：起线程、写 start / summary、量剩余空间等。
const OVERHEAD_S: f64 = 0.2;

fn ema(old: f64, new: f64) -> f64 {
    if !new.is_finite() || new <= 0.0 {
        return old;
    }
    old * (1.0 - ALPHA) + new * ALPHA
}

impl PerfModel {
    /// 默认位置 `%LOCALAPPDATA%\DiskWhy\perf.json`。
    pub fn default_path() -> Option<PathBuf> {
        crate::explain::llm::app_data_dir().map(|d| d.join("perf.json"))
    }

    /// 读文件；没有 / 坏了就用默认值。
    ///
    /// 2026-10-07 之前写的文件（没有 `manifest_ms`）里的 `rmdir_ms` 从来没学过（旧版不学它，值是旧默认 0.1），
    /// 而且旧版把收尾时间算进了逐个文件的耗时：这种文件的 `rmdir_ms` 换成新默认值。
    pub fn load(path: &Path) -> Self {
        let Some(v) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else {
            return Self::default();
        };
        let old_format = v.get("manifest_ms").is_none();
        let Some(mut m) = serde_json::from_value::<PerfModel>(v).ok().filter(|m| m.sane()) else {
            return Self::default();
        };
        if old_format {
            m.rmdir_ms = RMDIR_MS;
        }
        m
    }

    pub fn load_default() -> Self {
        Self::default_path().map(|p| Self::load(&p)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let s = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, s).map_err(|e| format!("写 {} 失败：{e}", path.display()))
    }

    fn sane(&self) -> bool {
        let pos = |x: f64| x.is_finite() && x > 0.0 && x < 10_000.0;
        pos(self.permanent_ms) && pos(self.recycle_ms) && pos(self.rename_ms) && pos(self.xvol_ms) && pos(self.xvol_mb_s) && pos(self.rmdir_ms)
            && pos(self.manifest_ms)
            && (0.0..=1.0).contains(&self.serial_frac)
    }

    /// 实际能用上几个线程（队列一块 256 条，块数少于线程数时用不满）。
    pub fn effective_threads(&self, w: &Workload, workers: usize, items: usize) -> f64 {
        let chunks = w.entries().div_ceil(super::CHUNK) + items.saturating_sub(1);
        workers.max(1).min(chunks.max(1)) as f64
    }

    /// p 个线程相对单线程的加速。
    pub fn speedup(&self, p: f64) -> f64 {
        let p = p.max(1.0);
        p / (1.0 + (p - 1.0) * self.serial_frac)
    }

    /// 单个条目的单线程成本（毫秒），进度条按它加权（跨盘拷贝另按字节，见 [`PerfModel::copy_ms`]）。
    pub fn op_ms(&self, dest: super::Dest, cross_volume: bool, is_link: bool) -> f64 {
        use super::Dest::*;
        if is_link {
            return self.permanent_ms;
        }
        match (dest, cross_volume) {
            (Permanent, _) => self.permanent_ms,
            (Recycle, _) => self.recycle_ms,
            (Quarantine, false) => self.rename_ms,
            (Quarantine, true) => self.xvol_ms,
        }
    }

    /// 拷贝字节的毫秒数（`threads` 个线程同时拷时按最多 2 路并行算，磁盘带宽有限）。
    pub fn copy_ms(&self, bytes: u64, threads: f64) -> f64 {
        bytes as f64 / (self.xvol_mb_s * 1e6) * 1000.0 / threads.clamp(1.0, 2.0)
    }

    /// 逐个文件的工作（多线程并行做）的单线程毫秒数。
    fn work_ms(&self, w: &Workload) -> f64 {
        w.permanent as f64 * self.permanent_ms
            + w.links as f64 * self.permanent_ms
            + w.recycle as f64 * self.recycle_ms
            + w.rename as f64 * self.rename_ms
            + w.xvol as f64 * self.xvol_ms
    }

    /// 收尾（单线程，墙钟毫秒）：删空文件夹 + 写 manifest.json。
    pub fn finalize_ms(&self, w: &Workload) -> f64 {
        w.dirs as f64 * self.rmdir_ms + w.manifest as f64 * self.manifest_ms
    }

    /// 估计秒数：并行部分按加速比折算，收尾和拷字节不折算。
    pub fn estimate_secs(&self, w: &Workload, workers: usize, items: usize) -> f64 {
        if w.entries() == 0 {
            return 0.0;
        }
        let p = self.effective_threads(w, workers, items);
        (self.work_ms(w) / self.speedup(p) + self.finalize_ms(w) + self.copy_ms(w.xvol_bytes, 1.0)) / 1000.0 + OVERHEAD_S
    }

    /// 用一次执行的实测更新。`threads` = 实际用了几个线程，`wall_ms` = 墙钟时间。
    pub fn learn(&mut self, m: &Measured, wall_ms: f64, threads: usize) {
        let w = m.workload();
        if w.entries() < MIN_SAMPLE || wall_ms <= 0.0 {
            return;
        }
        // 收尾是单线程的、实测得到每一步的时间：直接按“每个文件夹 / 每条”的实测往里拉
        if m.rmdir.0 >= MIN_SAMPLE {
            self.rmdir_ms = ema(self.rmdir_ms, m.rmdir.1 / m.rmdir.0 as f64).clamp(0.005, 50.0);
        }
        if m.manifest.0 >= MIN_SAMPLE {
            self.manifest_ms = ema(self.manifest_ms, m.manifest.1 / m.manifest.0 as f64).clamp(0.0005, 5.0);
        }
        // 其余（并行部分）：墙钟时间扣掉固定开销和收尾，和预测比
        let p = self.effective_threads(&w, threads, 1);
        let pred = self.work_ms(&w) / self.speedup(p) + self.copy_ms(w.xvol_bytes, 1.0);
        let actual = (wall_ms - OVERHEAD_S * 1000.0 - m.finalize_ms()).max(wall_ms * 0.3);
        if pred <= 0.0 {
            return;
        }
        let k = (actual / pred).clamp(0.2, 5.0);
        if w.permanent >= MIN_SAMPLE {
            self.permanent_ms = ema(self.permanent_ms, self.permanent_ms * k);
        }
        if w.recycle >= MIN_SAMPLE {
            self.recycle_ms = ema(self.recycle_ms, self.recycle_ms * k);
        }
        if w.rename >= MIN_SAMPLE {
            self.rename_ms = ema(self.rename_ms, self.rename_ms * k);
        }
        if w.xvol >= MIN_SAMPLE {
            self.xvol_ms = ema(self.xvol_ms, self.xvol_ms * k);
        }
        if w.xvol_bytes >= 64 << 20 {
            self.xvol_mb_s = ema(self.xvol_mb_s, self.xvol_mb_s / k).clamp(5.0, 5000.0);
        }
        self.runs += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_and_learn() {
        let m = PerfModel::default();
        let w = Workload { permanent: 100_000, ..Default::default() };
        let one = m.estimate_secs(&w, 1, 1);
        let many = m.estimate_secs(&w, 16, 1);
        assert!(many < one, "{many} vs {one}");
        assert!((m.speedup(16.0) - 16.0 / (1.0 + 15.0 * 0.25)).abs() < 1e-9);
        assert_eq!(m.estimate_secs(&Workload::default(), 16, 0), 0.0);
        // 实际比预测慢 2 倍 → 直接删除的单文件耗时变大，估计变大
        let mut m2 = m.clone();
        let wall = (m.estimate_secs(&w, 16, 1) - 0.2) * 1000.0 * 2.0 + 200.0;
        m2.learn(&Measured { permanent: (100_000, 1.0), ..Default::default() }, wall, 16);
        assert!(m2.permanent_ms > m.permanent_ms, "{} vs {}", m2.permanent_ms, m.permanent_ms);
        assert!(m2.estimate_secs(&w, 16, 1) > many);
        assert_eq!(m2.recycle_ms, m.recycle_ms, "这次没做的操作不变");
        assert_eq!(m2.runs, 1);
        // 样本太少不学
        let mut m3 = m.clone();
        m3.learn(&Measured { recycle: (10, 1000.0), ..Default::default() }, 100.0, 16);
        assert_eq!(m3, m);
        // 坏文件 → 默认值
        let dir = std::env::temp_dir().join(format!("dw-perf-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("perf.json");
        std::fs::write(&p, b"{not json").unwrap();
        assert_eq!(PerfModel::load(&p), PerfModel::default());
        m2.save(&p).unwrap();
        // serde_json 默认的小数解析可能差最后一位（1 ulp），按近似比
        let back = PerfModel::load(&p);
        assert!((back.permanent_ms - m2.permanent_ms).abs() < 1e-12 && back.runs == m2.runs, "{back:?} vs {m2:?}");
        assert_eq!(PerfModel { permanent_ms: m2.permanent_ms, ..back }, m2);
        // 旧版 perf.json（没有 manifest_ms）照常能读，缺的取默认
        std::fs::write(&p, br#"{"permanent_ms":0.3,"recycle_ms":4.0,"rename_ms":0.65,"xvol_ms":3.0,"xvol_mb_s":150.0,"rmdir_ms":0.1,"serial_frac":0.25,"runs":3}"#).unwrap();
        let old = PerfModel::load(&p);
        assert_eq!((old.permanent_ms, old.manifest_ms, old.runs), (0.3, PerfModel::default().manifest_ms, 3));
        assert_eq!(old.rmdir_ms, PerfModel::default().rmdir_ms, "旧文件里没学过的 rmdir_ms 换成新默认");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 收尾（删空文件夹、写 manifest.json）是单线程的：估计里不按加速比打折；学习时按实测的每个 / 每条往里拉，
    /// 并且从墙钟时间里扣掉，不把收尾算到逐个文件的耗时上。
    #[test]
    fn finalize_is_serial_and_learned_separately() {
        let m = PerfModel::default();
        let base = Workload { rename: 100_000, ..Default::default() };
        let fin = Workload { dirs: 20_000, manifest: 100_000, ..base };
        let d = m.estimate_secs(&fin, 16, 1) - m.estimate_secs(&base, 16, 1);
        let want = (20_000.0 * m.rmdir_ms + 100_000.0 * m.manifest_ms) / 1000.0;
        assert!((d - want).abs() < 1e-9, "收尾 {d} 秒，应为 {want}（不打折）");
        // 实测：收尾 20,000 个文件夹每个 0.5 ms（10 秒），manifest 10 万条 2 秒；并行部分和预测一样
        let par = m.estimate_secs(&base, 16, 1) - 0.2;
        let meas = Measured { rename: (100_000, 1.0), rmdir: (20_000, 10_000.0), manifest: (100_000, 2_000.0), ..Default::default() };
        let mut m2 = m.clone();
        m2.learn(&meas, par * 1000.0 + 12_000.0 + 200.0, 16);
        assert!((m2.rmdir_ms - (m.rmdir_ms * (1.0 - ALPHA) + 0.5 * ALPHA)).abs() < 1e-9, "{}", m2.rmdir_ms);
        assert!((m2.manifest_ms - (m.manifest_ms * (1.0 - ALPHA) + 0.02 * ALPHA)).abs() < 1e-9, "{}", m2.manifest_ms);
        assert!((m2.rename_ms - m.rename_ms).abs() < 1e-6, "收尾慢不该算到改名上：{} vs {}", m2.rename_ms, m.rename_ms);
    }
}
