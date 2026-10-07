// 全局界面状态 + 动作（阶段 F：首页 / 清理 / 看看都是什么 / 确认弹层 / 暂存区 / 设置）。
// 只调用 api.ts 里的后端命令；清理候选由后端算好（ScanSummary.candidates），这里只管勾选和预检。
import { computed, ref, watch } from "vue";
import {
  actionEnv,
  cancelScan,
  getEnv,
  getTreemap,
  listDir,
  listDrives,
  openSnapshot,
  previewPlan,
  cancelPreview,
  PREVIEW_CANCELLED,
  actionStatus,
  cancelExecute,
  CLOSE_BLOCKED_EVENT,
  dismissInterrupted,
  elevateQuarantine,
  executePlan,
  execProgress,
  lastExecReport,
  lastInterrupted,
  prepareQuarantine,
  type DestStatus,
  type ExecProgress,
  type ExecReport,
  type Interrupted,
  relaunchAsAdmin,
  saveSnapshot,
  startScan,
  startupScanDir,
  startupSnapshot,
  tmKey,
  type ActionEnv,
  type CandidateGroup,
  type DirListing,
  type DriveInfo,
  type EnvInfo,
  type ExecOutcome,
  type LlmSettings,
  type NodeView,
  type PreflightReport,
  type PreviewProgress,
  type ScanProgress,
  type ScanSummary,
  type Strategy,
  type TreemapNode,
} from "./api";
import { cleanCat, type CleanCat } from "./labels";
import { collapseChain, samePath } from "./treemap";

export type Phase = "idle" | "scanning" | "done";
export type Page = "home" | "cleanup" | "explore" | "quarantine" | "settings";
export type ExploreView = "map" | "split" | "list";

// ───── 基本状态 ─────
export const page = ref<Page>("home");
export const env = ref<EnvInfo | null>(null);
export const drives = ref<DriveInfo[]>([]);
export const scanRoot = ref("");
export const phase = ref<Phase>("idle");
export const progress = ref<ScanProgress | null>(null);
export const summary = ref<ScanSummary | null>(null);
export const scanFinishedAt = ref<number | null>(null);
export const message = ref("");
export const notice = ref("");
export const cancelling = ref(false);
export const relaunching = ref(false);
export const snapBusy = ref(false);
export const llmReady = computed(() => env.value?.llm_ready ?? false);

/** 扫描的盘（C:）的信息（容量读数用） */
export const drive = computed(() => {
  const r = (summary.value?.root ?? scanRoot.value).toUpperCase();
  return drives.value.find((d) => r.startsWith(d.root.toUpperCase())) ?? null;
});

// ───── 清理候选 / 勾选 ─────
export const candidates = computed<CandidateGroup[]>(() => summary.value?.candidates ?? []);
/** 勾选的候选组（key） */
export const checked = ref<Set<string>>(new Set());

export function canCheck(g: CandidateGroup): boolean {
  return g.deletable_bytes > 0;
}

/** 默认勾选：可放心清理且能直接删的全选，需你判断的一律不选 */
export function resetSelection() {
  checked.value = new Set(candidates.value.filter((g) => g.tier === "safe" && canCheck(g)).map((g) => g.key));
}

export function toggleGroup(key: string) {
  const g = candidates.value.find((x) => x.key === key);
  if (!g || !canCheck(g)) return;
  const s = new Set(checked.value);
  if (s.has(key)) s.delete(key);
  else s.add(key);
  checked.value = s;
}

export function setGroups(keys: string[], on: boolean) {
  const s = new Set(checked.value);
  for (const k of keys) {
    const g = candidates.value.find((x) => x.key === k);
    if (!g || !canCheck(g)) continue;
    if (on) s.add(k);
    else s.delete(k);
  }
  checked.value = s;
}

export const checkedGroups = computed(() => candidates.value.filter((g) => checked.value.has(g.key) && canCheck(g)));
export const checkedBytes = computed(() => checkedGroups.value.reduce((a, g) => a + g.deletable_bytes, 0));

/** 首页：能直接删的“可放心清理”合计（= 主按钮清理的量） */
export const safeGroups = computed(() => candidates.value.filter((g) => g.tier === "safe" && canCheck(g)));
export const safeBytes = computed(() => safeGroups.value.reduce((a, g) => a + g.deletable_bytes, 0));
/** 首页：“需你判断”合计（清理页里需你判断的全部行） */
export const reviewBytes = computed(() => candidates.value.filter((g) => g.tier === "review").reduce((a, g) => a + g.bytes, 0));

export function groupCat(g: CandidateGroup): CleanCat {
  return cleanCat(g.group, g.category);
}

/** 首页一行小字：可放心清理按分类拆开（大到小） */
export const safeByCat = computed(() => {
  const m = new Map<CleanCat, number>();
  for (const g of safeGroups.value) m.set(groupCat(g), (m.get(groupCat(g)) ?? 0) + g.deletable_bytes);
  return [...m.entries()].sort((a, b) => b[1] - a[1]);
});

// ───── 预检 / 确认 / 执行（阶段 D 的护栏逻辑原样保留，后端重新判断每一项） ─────
export const actEnv = ref<ActionEnv | null>(null);
export const previewBusy = ref(false);
export const report = ref<PreflightReport | null>(null);
/** 确认弹层这一次打开的序号（App 用它当 key：换去向重新预检时弹层不重建） */
export const dialogSeq = ref(0);

export async function refreshActionEnv() {
  try {
    actEnv.value = await actionEnv();
  } catch (e) {
    message.value = `读取暂存区信息失败：${e}`;
  }
}

// ── 删到哪里（阶段 J §3.1 / §3.2）：可放心清理默认直接删；下拉只管「需你判断」的去向 ──
export type DestMode = "stash" | "recycle" | "permanent" | "folder";
export interface DestState {
  mode: DestMode;
  /** stash：哪个盘（"D:\\"） */
  drive: string;
  /** stash / folder：能用的暂存区位置（Strategy.quarantine.dest_root）；stash 还没解析时为 "" */
  dest: string;
  /** stash 的解析状态：ok / 要授权 / 建不了 / 正在问后端；"" = 还没解析（清单里没有需你判断的，不用解析） */
  status: "" | "ok" | "needs_elevation" | "failed" | "pending";
  failMsg: string;
  /** 用户在下拉里自己选的（明确选「回收站」时可放心清理的也进回收站） */
  explicit: boolean;
}
export const dest = ref<DestState>({ mode: "recycle", drive: "", dest: "", status: "", failMsg: "", explicit: false });
/** 正在等 UAC 授权 */
export const elevating = ref(false);

/** 暂存区要在目的盘上留的余量 */
const STASH_SLACK = 64 * 1024 ** 2;

/** 要清理的东西所在的盘（"C:\\"） */
function sourceDrive(): string {
  const m = /^([A-Za-z]):/.exec(summary.value?.root ?? scanRoot.value);
  return m ? `${m[1].toUpperCase()}:\\` : "";
}

/** 可以当暂存区的盘：源盘以外的固定盘，剩余多的在前 */
export const stashDrives = computed(() => {
  const src = sourceDrive().toUpperCase();
  return (actEnv.value?.drives ?? drives.value).filter((d) => d.root.toUpperCase() !== src).sort((a, b) => b.free_bytes - a.free_bytes);
});

/** 勾选里有没有需你判断的、需你判断的有多大（决定要不要解析暂存区、空间够不够） */
function reviewNeed(): { has: boolean; bytes: number } {
  const g = checkedGroups.value.filter((x) => x.tier === "review");
  return { has: g.length > 0, bytes: g.reduce((a, x) => a + x.deletable_bytes, 0) };
}

/** 问后端某个盘上的暂存区能不能用（①已有 ②试建 ③以前用过的位置）；结果写进 dest */
async function resolveStash(drive: string): Promise<DestState["status"]> {
  dest.value = { ...dest.value, mode: "stash", drive, dest: "", status: "pending", failMsg: "" };
  let st: DestStatus;
  try {
    st = await prepareQuarantine(drive);
  } catch (e) {
    st = { status: "failed", message: String(e) };
  }
  if (dest.value.mode !== "stash" || dest.value.drive !== drive) return dest.value.status;
  if (st.status === "ok") dest.value = { ...dest.value, dest: st.dest_root, status: "ok" };
  else if (st.status === "needs_elevation") dest.value = { ...dest.value, status: "needs_elevation" };
  else dest.value = { ...dest.value, status: "failed", failMsg: st.message };
  return dest.value.status;
}

/**
 * 能当**默认**暂存区的盘：不放 Windows 所在的盘（扫的是 D 盘时，不能把东西默认挪去通常最满的
 * C 盘）；这些盘仍在下拉里，用户可以自己选。
 */
export function defaultStashDrives(needBytes: number) {
  const sys = (env.value?.system_drive || "C:\\").toUpperCase();
  return stashDrives.value.filter(
    (d) => d.root.toUpperCase() !== sys && d.free_bytes > needBytes + STASH_SLACK,
  );
}

/** 清理页底栏「剩余 x → y」：按默认去向真能腾出的（可放心清理直接删；需你判断只有默认去别的盘暂存时才算） */
export const freesByDefault = computed(() => {
  let safe = 0;
  let review = 0;
  for (const g of checkedGroups.value) {
    if (g.tier === "review") review += g.deletable_bytes;
    else safe += g.deletable_bytes;
  }
  return safe + (review > 0 && defaultStashDrives(review).length ? review : 0);
});

/**
 * 这次运行里用户在下拉里**明确**选过的去向，按源盘记（"D:\\" → 回收站 …）。下次打开弹层时，只要还能用就照它来，
 * 不再每次都退回默认（扫 D 盘时默认是回收站，用户每次都得重新选）。不落盘：重开程序回到默认。
 */
type DestMemo = Pick<DestState, "mode" | "drive" | "dest">;
const destMemo = new Map<string, DestMemo>();
/** 某个源盘记住的去向（测试 / 调试用） */
export function rememberedDest(src = sourceDrive()): DestMemo | null {
  return destMemo.get(src.toUpperCase()) ?? null;
}
export function forgetDestChoices() {
  destMemo.clear();
}
function rememberDest(mode: DestMode, arg?: string) {
  const src = sourceDrive().toUpperCase();
  if (!src) return;
  // 只有可放心清理时下拉里的「直接删除」就是默认（可放心清理本来就直接删）：不记，免得下次把需你判断的也变成直接删
  if (mode === "permanent" && !reviewNeed().has) {
    destMemo.delete(src);
    return;
  }
  destMemo.set(src, { mode, drive: mode === "stash" ? (arg ?? "") : "", dest: mode === "folder" ? (arg ?? "") : "" });
}

/** 用记住的去向（还能用时）：回收站 / 直接删除总能用；暂存区要那个盘还在、空间够、暂存区能用或能授权；文件夹交给预检去判断 */
async function useRemembered(need: { has: boolean; bytes: number }): Promise<boolean> {
  const m = destMemo.get(sourceDrive().toUpperCase());
  if (!m) return false;
  const base: DestState = { mode: m.mode, drive: "", dest: "", status: "", failMsg: "", explicit: true };
  switch (m.mode) {
    case "recycle":
    case "permanent":
      dest.value = base;
      return true;
    case "folder":
      if (!m.dest) return false;
      dest.value = { ...base, dest: m.dest, status: "ok" };
      return true;
    case "stash": {
      const d = stashDrives.value.find((x) => x.root.toUpperCase() === m.drive.toUpperCase());
      if (!d || (need.has && d.free_bytes <= need.bytes + STASH_SLACK)) return false;
      dest.value = { ...base, drive: d.root };
      if (!need.has) return true;
      const st = await resolveStash(d.root);
      return st === "ok" || st === "needs_elevation";
    }
  }
}

/**
 * 默认去向（§3.1）：有别的固定盘、空间够、暂存区能用或能授权 → 「D 盘暂存区」；否则 → 「回收站」。
 * 清单里没有需你判断的时不去解析（不在 D:\ 白建文件夹），去向只影响需你判断的那部分。
 */
async function pickDefaultDest() {
  const need = reviewNeed();
  if (await useRemembered(need)) return;
  const cands = defaultStashDrives(need.bytes);
  const recycle: DestState = { mode: "recycle", drive: "", dest: "", status: "", failMsg: "", explicit: false };
  if (!cands.length) {
    dest.value = recycle;
    return;
  }
  if (!need.has) {
    dest.value = { mode: "stash", drive: cands[0].root, dest: "", status: "", failMsg: "", explicit: false };
    return;
  }
  for (const d of cands) {
    dest.value = { ...recycle, mode: "stash", drive: d.root };
    const st = await resolveStash(d.root);
    if (st === "ok" || st === "needs_elevation") return;
  }
  dest.value = recycle;
}

/** 当前去向 → 预检用的 strategy + safeDirect */
export function currentPlanArgs(): { strategy: Strategy; safeDirect: boolean } {
  const d = dest.value;
  switch (d.mode) {
    case "permanent":
      return { strategy: { kind: "permanent" }, safeDirect: true };
    case "folder":
      return { strategy: { kind: "quarantine", dest_root: d.dest }, safeDirect: true };
    case "recycle":
      return { strategy: { kind: "recycle_bin" }, safeDirect: !d.explicit };
    case "stash":
      if (d.status === "ok") return { strategy: { kind: "quarantine", dest_root: d.dest }, safeDirect: true };
      // 要授权 / 建不了：照样按这个盘预检（数字、预计时间都对），主按钮由弹层拦住，先授权或选文件夹
      if (d.status === "needs_elevation" || d.status === "failed") return { strategy: { kind: "quarantine", dest_root: d.drive }, safeDirect: true };
      // 还没解析（清单里没有需你判断的）：可放心清理照样直接删；万一后端判成需你判断的，也只会进回收站
      return { strategy: { kind: "recycle_bin" }, safeDirect: true };
  }
}

/** 后台逐个文件检查的进度（弹层里显示）；检查做完为 null */
export const checkProgress = ref<PreviewProgress | null>(null);
/** 当前这次预检的序号：关掉弹层 / 换了删到哪里之后，旧的结果回来也不用 */
let previewSeq = 0;

/** 清理页「清理」：每次打开弹层都重新挑去向（这个源盘上次明确选过的优先，否则默认），然后预检 */
export async function doPreview() {
  // 清理进行中 / 完成页还开着：不再开新的预检（后端也会拒绝）
  if (!checkedGroups.value.length || execPhase.value !== "idle") return;
  dialogSeq.value++;
  previewBusy.value = true;
  try {
    await pickDefaultDest();
  } finally {
    previewBusy.value = false;
  }
  await runPreview();
}

/**
 * 预检：把勾选组里能直接删的路径交给后端（带着规则 id，后端会核对说明有没有变）。
 * 阶段 G：后端先发 quote（按扫描时的大小），弹层立刻出现、按钮处于“正在检查”；
 * 逐个文件的检查在后台进行（progress），做完 promise 返回完整报告，按钮才可用。
 */
export async function runPreview() {
  const items = checkedGroups.value.flatMap((g) => g.paths.filter((p) => p.deletable).map((p) => ({ path: p.path, expect_rule: g.rule_id })));
  if (!items.length) return;
  const seq = ++previewSeq;
  const { strategy, safeDirect } = currentPlanArgs();
  previewBusy.value = true;
  message.value = "";
  // 换去向重新预检：旧的报告先留着（弹层不闪），但标成“正在检查”，主按钮不可用
  if (report.value) report.value = { ...report.value, pending: true, est_secs: 0 };
  checkProgress.value = { phase: "plan", dirs: 0, entries: 0, checked: 0, to_check: 0, locked: 0, rm_done: 0, rm_total: 0 };
  try {
    const final = await previewPlan(
      items,
      strategy,
      (e) => {
        if (seq !== previewSeq) return;
        if (e.kind === "quote") {
          report.value = e.report;
          previewBusy.value = false;
        } else checkProgress.value = e.progress;
      },
      safeDirect,
    );
    if (seq !== previewSeq) return;
    report.value = final;
  } catch (e) {
    if (seq !== previewSeq) return;
    if (String(e) !== PREVIEW_CANCELLED) {
      message.value = `预检失败：${e}`;
      report.value = null;
    }
  } finally {
    if (seq === previewSeq) {
      previewBusy.value = false;
      checkProgress.value = null;
    }
  }
}

/** 关掉确认弹层：还在检查就让后端停下 */
export function closeReport() {
  const checking = report.value?.pending || checkProgress.value !== null;
  previewSeq++;
  report.value = null;
  checkProgress.value = null;
  previewBusy.value = false;
  if (checking) void cancelPreview().catch(() => {});
}

/** 弹层下拉里选了去向：暂存区（某个盘）/ 回收站 / 直接删除 / 其他文件夹（已选好的路径）→ 重新预检 */
export async function chooseDest(mode: DestMode, arg?: string) {
  if (mode !== "folder" || arg) rememberDest(mode, arg);
  if (mode === "stash") {
    dest.value = { mode, drive: arg ?? "", dest: "", status: "pending", failMsg: "", explicit: true };
    if (report.value) report.value = { ...report.value, pending: true, est_secs: 0 };
    await resolveStash(arg ?? "");
  } else if (mode === "folder") {
    if (!arg) return;
    dest.value = { mode, drive: "", dest: arg, status: "ok", failMsg: "", explicit: true };
  } else {
    dest.value = { mode, drive: "", dest: "", status: "", failMsg: "", explicit: true };
  }
  if (report.value) await runPreview();
}

/** 「授权」：UAC 一次，提权建 X:\DiskWhy暂存区；成功就重新预检，失败 / 取消 → 请用户选文件夹 */
export async function authorizeStash() {
  const d = dest.value;
  if (d.mode !== "stash" || !d.drive || elevating.value) return;
  elevating.value = true;
  let st: DestStatus;
  try {
    st = await elevateQuarantine(d.drive);
  } catch (e) {
    st = { status: "failed", message: String(e) };
  } finally {
    elevating.value = false;
  }
  if (dest.value.mode !== "stash" || dest.value.drive !== d.drive) return;
  if (st.status === "ok") {
    dest.value = { ...dest.value, dest: st.dest_root, status: "ok", failMsg: "" };
    if (report.value) await runPreview();
  } else if (st.status === "failed") dest.value = { ...dest.value, status: "failed", failMsg: st.message };
}

// ── 执行（阶段 J §3.4 / §3.5）：状态放在这里，界面重载后也能重新附着 ──
export type ExecPhase = "idle" | "running" | "done";
export const execPhase = ref<ExecPhase>("idle");
export const execProg = ref<ExecProgress | null>(null);
export const execResult = ref<ExecReport | null>(null);
export const execStopping = ref(false);
export const execError = ref("");
/** 最近一次被拦下关窗的时间（ms）；弹层据此闪一下「停止」 */
export const closeBlockedAt = ref(0);

/** 开始执行（确认弹层主按钮）。失败（动任何文件之前）回到确认状态并显示原因 */
export async function runExec(rep: PreflightReport) {
  if (execPhase.value === "running") return;
  execPhase.value = "running";
  execError.value = "";
  execStopping.value = false;
  execResult.value = null;
  execProg.value = {
    done: 0,
    total: rep.total_entries,
    bytes_done: 0,
    total_bytes: rep.total_bytes,
    cost_done: 0,
    cost_total: 0,
    elapsed_ms: 0,
    current: "",
    cancelling: false,
  };
  try {
    let shownEarly = false;
    const o = await executePlan(rep.plan_id, rep.locked_count > 0, (p) => {
      if (p.finished) {
        // 后端做完了、正在重扫受影响的文件夹（大盘要几秒）：先把完成页亮出来
        shownEarly = true;
        void showFinished();
        return;
      }
      execProg.value = p;
      if (p.cancelling) execStopping.value = true;
    });
    // 完成页已经提前亮出、用户也已经点了「完成」：不要再弹回来
    if (!shownEarly || (execPhase.value as ExecPhase) !== "idle") {
      execResult.value = o.report;
      execPhase.value = "done";
    }
    await onExecuted(o);
  } catch (e) {
    execError.value = String(e);
    execPhase.value = "idle";
  } finally {
    execStopping.value = false;
  }
}

async function showFinished() {
  const r = await lastExecReport().catch(() => null);
  if (r && execPhase.value === "running") {
    execResult.value = r;
    execPhase.value = "done";
    execStopping.value = false;
  }
}

/** 「停止」：已处理的照常记账，剩下的原样不动 */
export async function stopExec() {
  if (execPhase.value !== "running" || execStopping.value) return;
  execStopping.value = true;
  try {
    const ok = await cancelExecute();
    if (!ok && execPhase.value !== "running") execStopping.value = false;
  } catch (e) {
    execStopping.value = false;
    message.value = `停止失败：${e}`;
  }
}

/** 完成页「完成」 */
export function finishExec() {
  execPhase.value = "idle";
  execResult.value = null;
  execProg.value = null;
  closeReport();
}

/** 执行中关窗被拦下（后端事件 close-blocked，或标题栏 ✕） */
export function onCloseBlocked() {
  if (execPhase.value === "running") closeBlockedAt.value = Date.now();
  else notice.value = "正在处理，稍后再关";
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/**
 * 界面重载 / 窗口重建后重新附着到正在进行的执行（§3 决定 0 / 崩溃文档 3）：轮询 exec_progress 直到结束，
 * 然后显示 last_exec_report。正在进行的是还原 / 清空（没有执行进度）时只等它结束。
 */
export async function attachExec() {
  let seen = false;
  const t0 = Date.now();
  for (;;) {
    // 2 秒还没有执行进度：在跑的是还原 / 清空（或者后端卡在“执行中”），不再等它，照常往下走
    if (!seen && Date.now() - t0 > 2000) return;
    const p = await execProgress().catch(() => null);
    if (p) {
      seen = true;
      execProg.value = p;
      if (execPhase.value === "idle") execPhase.value = "running";
      if (p.cancelling) execStopping.value = true;
    }
    if (!(await actionStatus().catch(() => false))) break;
    await sleep(300);
  }
  if (!seen) return;
  const r = await lastExecReport().catch(() => null);
  execStopping.value = false;
  if (r) {
    execResult.value = r;
    execPhase.value = "done";
  } else execPhase.value = "idle";
}

// ── 上次清理被打断（§3 决定 0）──
export const interrupted = ref<Interrupted | null>(null);
export async function loadInterrupted() {
  interrupted.value = await lastInterrupted().catch(() => null);
}
export async function doDismissInterrupted() {
  interrupted.value = null;
  await dismissInterrupted().catch(() => {});
}

export async function onExecuted(o: ExecOutcome) {
  if (o.summary) {
    summary.value = o.summary;
    resetSelection();
  }
  await reloadViews();
  await refreshActionEnv();
}

export async function onQuarantineChanged(s: ScanSummary | null) {
  if (s) {
    summary.value = s;
    resetSelection();
    await reloadViews();
  }
  await refreshActionEnv();
}

// ───── 扫描 ─────
let closeListening = false;
async function listenCloseBlocked() {
  if (closeListening) return;
  closeListening = true;
  try {
    const { listen } = await import("@tauri-apps/api/event");
    await listen(CLOSE_BLOCKED_EVENT, () => onCloseBlocked());
  } catch {
    closeListening = false;
  }
}

export async function init() {
  try {
    env.value = await getEnv();
    drives.value = await listDrives();
    const c = drives.value.find((d) => d.root.toUpperCase() === "C:\\");
    scanRoot.value = (c ?? drives.value[0])?.root ?? "C:\\";
    void refreshActionEnv();
    void listenCloseBlocked();
    // 界面重载 / 窗口重建时清理还在跑：先重新附着、等它做完再扫（不和清理抢盘）
    if (await actionStatus().catch(() => false)) await attachExec();
    void loadInterrupted();
    // 用快照启动（diskwhy.exe x.dwsnap）或 --scan <文件夹> 时按参数来；否则一打开就扫 C 盘
    const snap = await startupSnapshot();
    if (snap) {
      await doOpenSnapshot(snap);
      return;
    }
    const dir = await startupScanDir();
    if (dir) scanRoot.value = dir;
    await doScan();
  } catch (e) {
    message.value = `初始化失败：${e}`;
  }
}

export async function doScan() {
  if (!scanRoot.value || phase.value === "scanning") return;
  message.value = "";
  phase.value = "scanning";
  progress.value = null;
  summary.value = null;
  listing.value = null;
  tm.value = null;
  focus.value = null;
  cancelling.value = false;
  checked.value = new Set();
  try {
    const s = await startScan(scanRoot.value, (p) => (progress.value = p));
    summary.value = s;
    scanFinishedAt.value = Date.now();
    phase.value = "done";
    resetSelection();
  } catch (e) {
    phase.value = "idle";
    message.value = String(e);
  } finally {
    cancelling.value = false;
  }
}

export async function doCancel() {
  cancelling.value = true;
  await cancelScan();
}

// ───── 看看都是什么（方块图 + 列表） ─────
/** 方块图的根（当前文件夹） */
export const listing = ref<DirListing | null>(null);
export const tm = ref<TreemapNode | null>(null);
export const tmLoading = ref(false);
/** 右边列表展示的文件夹（当前文件夹里选中的那一块；默认最大的子文件夹） */
export const focus = ref<DirListing | null>(null);
export const focusKey = ref<string | null>(null);
/** 列表里展开说明的那一行 */
export const openRow = ref<string | null>(null);
export const exploreView = ref<ExploreView>(loadView());

function loadView(): ExploreView {
  try {
    const v = localStorage.getItem("diskwhy.explore");
    if (v === "map" || v === "split" || v === "list") return v;
  } catch {
    /* 用默认 */
  }
  return "split";
}
export function setExploreView(v: ExploreView) {
  exploreView.value = v;
  try {
    localStorage.setItem("diskwhy.explore", v);
  } catch {
    /* 忽略 */
  }
}

/**
 * 打开一个文件夹作为方块图的根；focusPath 给了就把它当右边列表（可以就是根自己）。
 * 没给 focusPath 时：第一次进入分析页（listing 还是 null）自动挑最大的子文件夹；之后一律列根自己（阶段 J §3.9）。
 */
export async function openDir(path: string, focusPath?: string) {
  const first = listing.value === null;
  try {
    tmLoading.value = true;
    const [l, t] = await Promise.all([listDir(path), getTreemap(path, 3)]);
    listing.value = l;
    tm.value = t;
    openRow.value = null;
    const target = focusPath ?? (first ? defaultFocus(t) : undefined);
    await setFocus(target ?? null);
  } catch (e) {
    message.value = String(e);
  } finally {
    tmLoading.value = false;
  }
}

/** 第一次进入时的右边列表：最大的子文件夹；它如果只是个“外壳”（Users 里几乎全是 king），就再往里走一层 */
function defaultFocus(t: TreemapNode): string | undefined {
  const dirs = t.children.filter((c) => c.kind === "Dir" && c.children.length > 0);
  if (!dirs.length) return undefined;
  const big = dirs.reduce((a, b) => (b.allocated > a.allocated ? b : a));
  return collapseChain(big).node.path;
}

/** 右边列表换成 path；path 为空或就是方块图的根 → 列根自己（方块图里不框任何块） */
export async function setFocus(path: string | null) {
  openRow.value = null;
  if (!path || (listing.value && samePath(path, listing.value.node.path))) {
    focus.value = listing.value;
    focusKey.value = null;
    return;
  }
  try {
    focus.value = await listDir(path, 200);
    focusKey.value = path;
  } catch {
    focus.value = listing.value;
    focusKey.value = null;
  }
}

/** 进入列表里的某个文件夹：它的父文件夹成为方块图的根，它自己成为右边列表 */
export async function enterRow(n: NodeView) {
  if (n.kind !== "Dir" || !focus.value) return;
  await openDir(focus.value.node.path, n.path);
}

/** 进入方块图里的某一块（双击）：它成为方块图的根，右边列表也是它 */
export async function enterBlock(path: string) {
  await openDir(path, path);
}

/** 换盘（阶段 J §3.6）：首页盘名下拉选了另一个盘 → 扫那个盘；正在扫就先停下当前这次 */
export async function switchDrive(root: string) {
  if (!root) return;
  if (phase.value === "scanning") {
    if (samePath(root, scanRoot.value)) return;
    await doCancel();
    await new Promise<void>((resolve) => {
      const stop = watch(phase, (p) => {
        if (p !== "scanning") {
          stop();
          resolve();
        }
      });
      if (phase.value !== "scanning") {
        stop();
        resolve();
      }
    });
  }
  scanRoot.value = root;
  message.value = "";
  page.value = "home";
  await doScan();
}

export async function goUp() {
  const bc = listing.value?.breadcrumb;
  if (bc && bc.length > 1) await openDir(bc[bc.length - 2].path, listing.value!.node.path);
}

export async function goExplore() {
  page.value = "explore";
  if (!listing.value && summary.value) await openDir(summary.value.root);
}

export function selectKeyOf(n: { kind: NodeView["kind"]; path: string }): string {
  return tmKey(n);
}

/** 动作之后重新读当前视图（后端已经只重扫了受影响的子树） */
async function reloadViews() {
  if (listing.value) await openDir(listing.value.node.path, focusKey.value ?? undefined);
}

// ───── 其他 ─────
export async function doRelaunch() {
  relaunching.value = true;
  try {
    await relaunchAsAdmin();
  } catch (e) {
    message.value = `没有以管理员身份重启：${e}`;
  } finally {
    relaunching.value = false;
  }
}

export async function doSaveSnapshot() {
  snapBusy.value = true;
  try {
    const p = await saveSnapshot();
    if (p) notice.value = "已保存";
  } catch (e) {
    message.value = `没保存上：${e}`;
  } finally {
    snapBusy.value = false;
  }
}

export async function doOpenSnapshot(path?: string) {
  snapBusy.value = true;
  message.value = "";
  try {
    const s = await openSnapshot(path);
    if (!s) return;
    summary.value = s;
    scanRoot.value = s.root;
    scanFinishedAt.value = s.snapshot ? s.snapshot.meta.saved_at * 1000 : Date.now();
    phase.value = "done";
    listing.value = null;
    focus.value = null;
    resetSelection();
    if (s.snapshot) notice.value = "在看保存的扫描结果";
    page.value = "home";
  } catch (e) {
    message.value = `打不开：${e}`;
  } finally {
    snapBusy.value = false;
  }
}

export function onSettingsSaved(s: LlmSettings) {
  if (env.value) env.value.llm_ready = s.enabled && !!s.base_url && !!s.model;
}
