//! 扫描完自动问 AI 的批量执行器（不依赖 Tauri：怎么问、怎么通知界面都由调用方传进来，测试用假的 ask 和很短的等待）。
//!
//! - **一次只问一个**（智谱免费账号并发很低，1302 = 并发太高），每两次请求之间停一下；
//! - 可重试的错误（限流 / 服务忙 / 超时 / 断线）：等 3 秒再试、再不行等 8 秒再试——**每项最多发 3 次请求**
//!   （[`AI_MAX_ATTEMPTS`]），还不行就算这一项失败、接着问下一项；
//! - 配置错误（密钥 / 模型名 / 地址 / 余额 / 连不上）：整批停下，`error` = 短中文；
//! - 连续 3 项失败：也停下，`error` = 最后一项的原因；
//! - 这一项已经不在扫描结果里了（刚被清理掉）：跳过，不算失败；
//! - 每项之间、每次等待中、排队等请求锁之后都看取消标记（新的扫描开始、换了设置、关了 AI 时取消）；
//!   拿到请求锁之后还要再核对一次（[`recheck_batch`]）：设置变了 → 取消，这一项排队时已经被问过了 → 跳过，都不发请求。
//!
//! 每次状态变化都调 `on_status`；每个成功的答案先写进 [`AiStore`]（落盘）再调 `on_answer`。
//!
//! 界面壳（src-tauri）里和别的功能打交道的几块也放在这里，好测：[`ask_one`]（每问一项：等扫描结果、取信息、排队、
//! 再看一次取消标记和设置、发请求）、[`ask_manual`]（手动“问 AI”排队拿到锁之后的核对）和 [`BatchSlot`]
//! （“当前这一批”的代号 / 取消标记 / 进度）。

use super::ai_store::{AiNote, AiStore};
use super::llm::{AskError, LlmSettings, MSG_ASKED, MSG_NOT_FOUND};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 自动识别的进度（Tauri 事件 `ai-status` 的内容，也是 `ai_status` 命令的返回值）。
///
/// `done` = 已经处理完的项（答上来的 + 失败的），`failed` ⊆ `done`；`error` 只在整批停下（配置错误 / 连续失败）时有。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiStatus {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    pub error: Option<String>,
}

/// 每项最多发几次请求（第一次 + 重试）。默认的 [`BatchConfig::backoff`] 有 `AI_MAX_ATTEMPTS - 1` 段等待。
pub const AI_MAX_ATTEMPTS: usize = 3;

/// 批量执行的节奏（测试里换成毫秒级）。
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// 可重试的错误之后依次等多久再试（长度 = 最多重试几次；每项最多发 `1 + backoff.len()` 次请求）。
    pub backoff: Vec<Duration>,
    /// 两次请求之间停多久。
    pub pause: Duration,
    /// 连续这么多项失败就停下。
    pub max_consecutive_failures: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        BatchConfig {
            // 3 次请求：第一次，等 3 秒，第二次，等 8 秒，第三次
            backoff: vec![Duration::from_secs(3), Duration::from_secs(8)],
            pause: Duration::from_millis(300),
            max_consecutive_failures: 3,
        }
    }
}

/// 这一批是怎么结束的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchEnd {
    /// 全部问完（有的可能失败了，见 `failed`）。
    Finished,
    /// 被取消。
    Cancelled,
    /// 配置错误或连续失败，停下了（原因 = 短中文，同 `AiStatus.error`）。
    Stopped(String),
}

/// 等 `d`；期间每 25 毫秒看一次取消标记。被取消返回 false。
fn wait(d: Duration, cancel: &AtomicBool) -> bool {
    let end = Instant::now() + d;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        let now = Instant::now();
        if now >= end {
            return true;
        }
        std::thread::sleep((end - now).min(Duration::from_millis(25)));
    }
}

/// 依次问 `items`（完整路径, 占用），一次一个。已经在 `store` 里的直接算完成（不发请求）。
/// 返回最后的状态和结束方式；最后的状态一定也通过 `on_status` 发过（`running = false`）。
pub fn run_batch(
    items: &[(String, u64)],
    cfg: &BatchConfig,
    cancel: &AtomicBool,
    store: &AiStore,
    mut ask: impl FnMut(&str) -> Result<AiNote, AskError>,
    mut on_status: impl FnMut(&AiStatus),
    mut on_answer: impl FnMut(&str, &AiNote),
) -> (AiStatus, BatchEnd) {
    let mut st = AiStatus { running: !items.is_empty(), total: items.len(), ..Default::default() };
    let finish = |mut st: AiStatus, end: BatchEnd, on_status: &mut dyn FnMut(&AiStatus)| {
        st.running = false;
        if let BatchEnd::Stopped(m) = &end {
            st.error = Some(m.clone());
        }
        on_status(&st);
        (st, end)
    };
    on_status(&st);
    if items.is_empty() {
        return (st, BatchEnd::Finished);
    }
    let mut consecutive = 0usize;
    let mut asked_any = false;
    for (path, _) in items {
        if cancel.load(Ordering::SeqCst) {
            return finish(st, BatchEnd::Cancelled, &mut on_status);
        }
        // 这期间用户可能手动问过了
        if store.contains(path) {
            st.done += 1;
            on_status(&st);
            continue;
        }
        if asked_any && !wait(cfg.pause, cancel) {
            return finish(st, BatchEnd::Cancelled, &mut on_status);
        }
        asked_any = true;
        let mut attempt = 0usize;
        loop {
            match ask(path) {
                Ok(note) => {
                    // 内存里一定记上；落盘失败不影响这次（下次 put 会整份重写）
                    let _ = store.put(path, note.clone());
                    on_answer(path, &note);
                    consecutive = 0;
                    st.done += 1;
                    on_status(&st);
                    break;
                }
                Err(e) if e.cancelled => {
                    return finish(st, BatchEnd::Cancelled, &mut on_status);
                }
                Err(e) if e.fatal => {
                    return finish(st, BatchEnd::Stopped(e.message), &mut on_status);
                }
                // 不用问了（扫描结果里已经没有它）：算问完，不算失败，也不打断“连续失败”的计数
                Err(e) if e.skipped => {
                    st.done += 1;
                    on_status(&st);
                    break;
                }
                Err(e) if e.retryable && attempt < cfg.backoff.len() => {
                    if !wait(cfg.backoff[attempt], cancel) {
                        return finish(st, BatchEnd::Cancelled, &mut on_status);
                    }
                    attempt += 1;
                }
                Err(e) => {
                    consecutive += 1;
                    st.done += 1;
                    st.failed += 1;
                    if consecutive >= cfg.max_consecutive_failures.max(1) {
                        return finish(st, BatchEnd::Stopped(e.message), &mut on_status);
                    }
                    on_status(&st);
                    break;
                }
            }
        }
    }
    finish(st, BatchEnd::Finished, &mut on_status)
}

// ───────────────────────── 界面壳用：问一项 ─────────────────────────

/// 扫描结果暂时不在时最多等多久（清理 / 还原后的刷新会把它拿走一会儿，大盘上要几秒到几十秒）。
pub const RESULT_WAIT: Duration = Duration::from_secs(120);

/// 自动识别问一项（界面壳的 `ai_ask_one` 就是它；拆出来是为了能测）：
///
/// 1. `current()` 取当前的扫描结果。**暂时没有**（清理 / 还原后的刷新 `refresh_after` 会把扫描结果拿走、重算完再放回去）
///    就等着（每 25 毫秒看一次，期间看取消标记），不算失败——不然刷新那几秒里一连三项都“找不到了”，整批会被
///    “连续 3 项失败”停下，工具栏显示「AI 出错」；等了 `max_wait` 还没回来 → 这一项跳过；
/// 2. `extract` 从扫描结果里取要发的信息，取好马上放开扫描结果（刷新要独占它）；找不到这个路径了（被清理掉了）→ 跳过；
/// 3. 排队拿 `http` 锁（同一时间只发一个请求，手动“问 AI”和测试连接也用它）；**拿到后再看一次取消标记**——排队的时候
///    可能换了服务商、关了 AI、开始了新的扫描，这时一个请求都不发（不然会用旧设置把这一项发给用户刚换掉的服务商）；
/// 4. `recheck()`（拿着锁）：再核对一次设置和缓存（界面壳传 [`recheck_batch`]），不通过就不发；通过时给出现在的设置；
/// 5. `send` 用这份设置发请求（拿着锁）。
pub fn ask_one<R, I, S>(
    cancel: &AtomicBool,
    max_wait: Duration,
    mut current: impl FnMut() -> Option<R>,
    extract: impl FnOnce(&R) -> Option<I>,
    http: &Mutex<()>,
    recheck: impl FnOnce() -> Result<S, AskError>,
    send: impl FnOnce(I, S) -> Result<AiNote, AskError>,
) -> Result<AiNote, AskError> {
    let start = Instant::now();
    let input = loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(AskError::cancelled());
        }
        if let Some(r) = current() {
            let got = extract(&r);
            drop(r);
            break got.ok_or_else(|| AskError::skip(MSG_NOT_FOUND, "扫描结果里已经没有这一项"))?;
        }
        if start.elapsed() >= max_wait {
            return Err(AskError::skip(MSG_NOT_FOUND, "等了很久扫描结果还不在"));
        }
        std::thread::sleep(Duration::from_millis(25).min(max_wait));
    };
    let _one = http.lock().unwrap_or_else(|e| e.into_inner());
    if cancel.load(Ordering::SeqCst) {
        return Err(AskError::cancelled());
    }
    let s = recheck()?;
    send(input, s)
}

/// 自动识别拿到请求锁之后的核对（[`ask_one`] 的 `recheck`）：
/// - 现在的设置 `fresh` 和这一批开始时的 `captured` 不一样（换了服务商 / 模型 / 密钥、关了 AI）或不能用了 → 取消，不发；
///   （换设置时界面壳会先停掉这一批，这里是第二道保险）
/// - 这一项排队的时候已经被问过了（用户手动问的，或者别的批次）→ 跳过（不算失败），不再发第二次；
/// - 否则返回现在的设置，用它发。
pub fn recheck_batch(store: &AiStore, path: &str, captured: &LlmSettings, fresh: &LlmSettings) -> Result<LlmSettings, AskError> {
    if !fresh.usable() || !fresh.same_target(captured) {
        return Err(AskError::cancelled());
    }
    if store.contains(path) {
        return Err(AskError::skip(MSG_ASKED, "排队的时候已经问过了"));
    }
    Ok(fresh.clone())
}

/// 手动“问 AI”的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum ManualAnswer {
    /// 刚问到的（调用方存进缓存、通知界面）。
    Fresh(AiNote),
    /// 排队的时候已经被问过了（自动识别刚好问到它）：缓存里的答案，没发请求。
    Cached(AiNote),
}

/// 手动“问 AI”（界面壳的 `ask_ai`；拆出来好测）。要发的信息调用方已经取好了（`send` 里带着）。
/// 1. 排队拿 `http` 锁（和自动识别、测试连接共用，同一时间只发一个请求）；
/// 2. 拿到后用 `fresh()` 取**现在的**设置核对：不能用（关了 / 没填全 / 云服务没填密钥）→ [`LlmSettings::unusable_error`]
///    （`AI 没打开` / `没填密钥`）；和点按钮时的 `captured` 不一样（换了服务商 / 模型 / 密钥）→ `已取消`。都不发请求——
///    不然排队的这几十秒里用户关了 AI 或换了服务商，这一项还是会用旧设置发出去；
/// 3. `cached()`：排队的时候已经被问过了 → 直接给缓存里的答案，不发第二次；
/// 4. 用现在的设置 `send`。
pub fn ask_manual(
    http: &Mutex<()>,
    captured: &LlmSettings,
    fresh: impl FnOnce() -> LlmSettings,
    cached: impl FnOnce() -> Option<AiNote>,
    send: impl FnOnce(&LlmSettings) -> Result<AiNote, AskError>,
) -> Result<ManualAnswer, AskError> {
    let _one = http.lock().unwrap_or_else(|e| e.into_inner());
    let now = fresh();
    if !now.usable() {
        return Err(now.unusable_error());
    }
    if !now.same_target(captured) {
        return Err(AskError::cancelled());
    }
    if let Some(n) = cached() {
        return Ok(ManualAnswer::Cached(n));
    }
    send(&now).map(ManualAnswer::Fresh)
}

// ───────────────────────── 界面壳用：当前这一批 ─────────────────────────

/// 界面壳里“当前这一批”的登记处（放在 `Mutex` 里）。每次开始 / 停止都换一个代号：旧的那批（还在等网络、
/// 还在排队的）发来的进度直接丢掉，不会盖掉新的状态；停止时把旧的那批的取消标记置上。
#[derive(Debug, Default)]
pub struct BatchSlot {
    gen: u64,
    cancel: Option<Arc<AtomicBool>>,
    status: AiStatus,
}

impl BatchSlot {
    /// 停掉当前这一批（置取消标记），状态回到空闲。返回 (新代号, 状态变了没有——变了要通知界面)。
    pub fn stop(&mut self) -> (u64, bool) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
        self.gen += 1;
        let changed = self.status != AiStatus::default();
        self.status = AiStatus::default();
        (self.gen, changed)
    }

    /// 为代号 `gen` 登记一批新的，返回它的取消标记；这期间又有人开始 / 停止过（代号变了）就返回 None，不该开始。
    pub fn begin(&mut self, gen: u64) -> Option<Arc<AtomicBool>> {
        if self.gen != gen {
            return None;
        }
        let c = Arc::new(AtomicBool::new(false));
        self.cancel = Some(c.clone());
        Some(c)
    }

    /// 代号 `gen` 的那一批报进度：还是当前这一批才记下，返回 true（要通知界面）；旧的那批的进度丢掉。
    pub fn report(&mut self, gen: u64, s: &AiStatus) -> bool {
        if self.gen != gen {
            return false;
        }
        self.status = s.clone();
        true
    }

    pub fn status(&self) -> &AiStatus {
        &self.status
    }

    pub fn gen(&self) -> u64 {
        self.gen
    }
}
