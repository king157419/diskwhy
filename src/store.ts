// 全局界面状态 + 动作（阶段 F：首页 / 清理 / 看看都是什么 / 确认弹层 / 暂存区 / 设置）。
// 只调用 api.ts 里的后端命令；清理候选由后端算好（ScanSummary.candidates），这里只管勾选和预检。
import { computed, ref, shallowRef, watch } from "vue";
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
import { samePath } from "./treemap";
import { ancestorsIn, buildDisplay, chainLevelStarts, collapseFor, findTm, isUnder, pathKey, type MapHit, type Stub } from "./tmtree";

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
  resetExplore();
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

// ───── 看看都是什么（阶段 K：一张平面图，单击原地展开） ─────
// 方块图的根是扫描根；单击一个文件夹 = 展开链变成「根 → 它」，它在原位置放大（BOOST）并显示更深的层级。
// pk 太小时图的根自动下移（按像素，TreemapView 决定）；面包屑仍从盘根开始。右边列表 = 链的最后一级（空链 = 盘根）。
// 数据：链上每一级都按它自己做根取一次子树（getTreemap(path, 3)，每个文件夹只取一次），显示树 = 盘根的数据 +
// 链上每一级自己的子树依次拼上去（buildDisplay：只看链和缓存，同一条链永远是同一张图，展开后的细节和浏览历史无关）。

/** 扫描根的列表（盘根） */
export const listing = ref<DirListing | null>(null);
/** 扫描根自己取回来的方块图数据 */
const rootTm = shallowRef<TreemapNode | null>(null);
/** 展开链：从根往下被单击挖开的文件夹路径 [p1 … pk]（p1 是根的子文件夹） */
export const chain = ref<string[]>([]);
/** 正在取方块图数据（方块图上最深的展开块标题栏有轻微的 loading 态，不挡操作） */
export const tmLoading = ref(false);
/** 右边列表展示的文件夹（= 链的最后一级；空链 = 盘根） */
export const focus = ref<DirListing | null>(null);
/** 方块图里单击选中的文件 / 其余小文件 / 「其他」/ 伪块（块的 key） */
export const selKey = ref<string | null>(null);
/** 选中的是「其他」时它的内容签名（项数 | 大小）：它所在的文件夹重新取数、原来的「其他」被拆开后对不上，就不再算选中 */
export const selSig = ref<string | null>(null);
/** 列表里展开说明的那一行 */
export const openRow = ref<string | null>(null);
export const exploreView = ref<ExploreView>(loadView());

// ── 数据：按路径缓存「以它自己为根」取回来的子树（大小写不敏感），新扫描 / 清理之后整个作废 ──
const tmCache = new Map<string, TreemapNode>();
/** 父级数据里没有的文件夹（超过每层个数上限）：用列表信息补的节点 */
const tmStubs = new Map<string, Stub>();
const tmInflight = new Map<string, Promise<void>>();
/** 缓存 / 补节点变了 +1（显示树重新拼） */
const tmVer = ref(0);

const rootPath = () => rootTm.value?.path ?? listing.value?.node.path ?? summary.value?.root ?? "";
/** 当前展开的文件夹 pk（空链 = null） */
export const focusKey = computed(() => chain.value[chain.value.length - 1] ?? null);
/** 方块图画的树（扫描根）：盘根的数据 + 链上每一级自己的子树 */
export const tm = computed<TreemapNode | null>(() => {
  void tmVer.value;
  return buildDisplay(rootTm.value, chain.value, tmCache, tmStubs);
});
/** 当前展开的文件夹（空链 = 盘根）在方块图数据里的名字和大小：链一变立刻跟上，不等列表 */
export const pkInfo = computed(() => {
  const p = focusKey.value ?? rootPath();
  const n = p ? findTm(tm.value, p) : null;
  return n ? { path: n.path, name: n.name.replace(/[\\/]+$/, "") || n.path, size: n.allocated } : null;
});
/** 面包屑：盘根 → … → pk（名字取自方块图数据，链一变立刻跟上，不等列表） */
export const crumbs = computed(() => {
  const root = rootPath();
  if (!root) return [];
  const name = (p: string) => findTm(tm.value, p)?.name ?? p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? p;
  return [{ name: root.replace(/[\\/]+$/, ""), path: root }, ...chain.value.map((p) => ({ name: name(p), path: p }))];
});

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

/** 数据的代数：新扫描 / 清理之后 +1，之前发出去的请求回来一律丢掉 */
let tmGen = 0;
/** 导航的序号：快速连点时，只有最后一次的列表结果才用 */
let navSeq = 0;
/** 正在取的方块图数据个数（tmLoading = 有没有） */
let loads = 0;
async function loading<T>(p: Promise<T>): Promise<T> {
  loads++;
  tmLoading.value = true;
  try {
    return await p;
  } finally {
    loads--;
    tmLoading.value = loads > 0;
    // 用户要的数据都回来了：轮到后台预取
    if (!loads) pump();
  }
}

function resetExplore() {
  tmGen++;
  navSeq++;
  tmCache.clear();
  tmStubs.clear();
  tmInflight.clear();
  clearPrefetch();
  tmVer.value++;
  listing.value = null;
  rootTm.value = null;
  focus.value = null;
  chain.value = [];
  selKey.value = null;
  selSig.value = null;
  openRow.value = null;
}

/** 以 path 为根取它的子树（深 3 层）放进缓存；取过的不再取，同一个路径同时只发一次 */
function fetchOwn(path: string, gen: number): Promise<void> {
  const k = pathKey(path);
  if (tmCache.has(k)) return Promise.resolve();
  let p = tmInflight.get(k);
  // 正在后台预取：接着等它（这时算「正在取」，标题栏上有 loading 小点，别的预取先停一停）
  if (p && pfReqs.has(p)) return loading(p);
  if (!p) {
    const req: Promise<void> = loading(getTreemap(path, 3))
      .then((sub) => {
        if (gen !== tmGen) return;
        tmCache.set(k, sub);
        tmVer.value++;
      })
      .finally(() => {
        if (tmInflight.get(k) === req) tmInflight.delete(k);
      });
    tmInflight.set(k, req);
    p = req;
  }
  return p;
}

// ── 预取（K 第 3 轮 F8）：鼠标在一个还没取过的文件夹块上停 120 ms / 挖开一个文件夹后空闲时，在后台把子树取进缓存，
// 下一次单击不用等数据（常见路径上单击到画面稳定约 300 ms）。只往缓存里放、不改任何状态（显示树只拼链上的，
// 缓存里多了链外的子树，图一点不变）。同时最多 2 个；用户自己要的数据在取的时候先停；新扫描 / 换盘 / 清理之后清空。
const PREFETCH_MAX = 2;
const PREFETCH_QUEUE = 8;
const pfQueue: string[] = [];
/** 正在飞的预取请求（fetchOwn 碰到它时接着等，并算进 loading） */
const pfReqs = new Set<Promise<void>>();
let pfActive = 0;
let pfIdle: { cancel: () => void } | null = null;
/** 测试可以关掉「空闲时预取」（它用计时器，测别的东西时会多出请求） */
export const prefetchCfg = { idle: true };

const queued = (k: string) => pfQueue.findIndex((q) => pathKey(q) === k);
function enqueue(path: string, urgent: boolean) {
  const k = pathKey(path);
  if (tmCache.has(k) || tmInflight.has(k)) return;
  const i = queued(k);
  if (i >= 0) {
    if (!urgent) return;
    pfQueue.splice(i, 1);
  }
  if (urgent) pfQueue.unshift(path);
  else pfQueue.push(path);
  if (pfQueue.length > PREFETCH_QUEUE) pfQueue.length = PREFETCH_QUEUE;
}
function pump() {
  if (!rootTm.value) return;
  while (pfActive < PREFETCH_MAX && !loads && pfQueue.length) {
    const path = pfQueue.shift()!;
    const k = pathKey(path);
    if (tmCache.has(k) || tmInflight.has(k)) continue;
    const gen = tmGen;
    pfActive++;
    const req: Promise<void> = getTreemap(path, 3)
      .then((sub) => {
        if (gen !== tmGen) return;
        tmCache.set(k, sub);
        // 取回来的时候用户已经挖进去了（它在链上）：显示树重拼；不在链上就什么都不变
        if (chain.value.some((c) => pathKey(c) === k)) tmVer.value++;
      })
      .catch(() => {})
      .finally(() => {
        pfActive--;
        pfReqs.delete(req);
        if (tmInflight.get(k) === req) tmInflight.delete(k);
        pump();
      });
    pfReqs.add(req);
    tmInflight.set(k, req);
  }
}
function clearPrefetch() {
  pfQueue.length = 0;
  pfIdle?.cancel();
  pfIdle = null;
}
/** 鼠标停在方块图上某个文件夹块上：从图上的链往下到它，还没取过的每一级都排到队首（外壳合并块两层都要） */
export function prefetchFor(path: string) {
  if (!rootTm.value) return;
  const anc = ancestorsIn(tm.value, path);
  if (!anc) return;
  const todo = anc.filter((p) => !tmCache.has(pathKey(p)));
  for (let i = todo.length - 1; i >= 0; i--) enqueue(todo[i], true);
  pump();
}
/** 挖开一个文件夹（或者回到盘根）以后，空闲时预取它里面最大的 3 个子文件夹 */
function prefetchKids() {
  pfIdle?.cancel();
  pfIdle = null;
  if (!prefetchCfg.idle || exploreView.value === "list") return;
  const gen = tmGen;
  const run = () => {
    pfIdle = null;
    if (gen !== tmGen) return;
    const n = findTm(tm.value, focusKey.value ?? rootPath());
    if (!n) return;
    const dirs = n.children
      .filter((c) => c.kind === "Dir" && c.can_open)
      .sort((a, b) => b.allocated - a.allocated)
      .slice(0, 3);
    for (const d of dirs) for (const h of collapseFor(d, chain.value).hops) enqueue(h.path, false);
    pump();
  };
  const ric = (globalThis as { requestIdleCallback?: (f: () => void, o?: { timeout: number }) => number }).requestIdleCallback;
  const cic = (globalThis as { cancelIdleCallback?: (id: number) => void }).cancelIdleCallback;
  if (ric && cic) {
    const id = ric(run, { timeout: 1500 });
    pfIdle = { cancel: () => cic(id) };
  } else {
    const id = setTimeout(run, 300);
    pfIdle = { cancel: () => clearTimeout(id) };
  }
}
/** 测试 / 调试：这个文件夹的子树取过了吗 */
export function tmCached(path: string): boolean {
  return tmCache.has(pathKey(path));
}

/** 往里挖时最多等数据这么久再动（超过就先用手上的数据动，数据回来再补一次） */
const DATA_WAIT_MS = 90;

/** 链上每一级都按它自己做根取一次（后台、并行；显示树按链的顺序拼，到达的先后不影响结果） */
async function loadOwn(ch: readonly string[], gen: number) {
  await Promise.all(ch.map((p) => fetchOwn(p, gen).catch(() => {})));
}

/**
 * 保证链上每一级都在显示树里：父级的数据里没有就先取父级自己的子树；还没有（超过每层的个数上限）就用列表数据补一个节点
 * （补节点单独记着，祖先的数据重拼以后照样补上）；连列表都读不到 = 已经不存在了，链在这一级之前截断。返回还有效的那一段链。
 */
async function ensureChain(ch: string[], gen: number, alive: () => boolean = () => true): Promise<string[]> {
  const root = rootPath();
  for (let i = 0; i < ch.length; i++) {
    const p = ch[i];
    const parent = i ? ch[i - 1] : root;
    const shown = () => findTm(buildDisplay(rootTm.value, ch.slice(0, i + 1), tmCache, tmStubs), p);
    if (shown()) continue;
    await fetchOwn(parent, gen).catch(() => {});
    // 用户已经点了别处（或者数据换代了）：不再为这条链取数据
    if (gen !== tmGen || !alive()) return ch;
    if (shown()) continue;
    const l = await listDir(p, 1).catch(() => null);
    if (gen !== tmGen || !alive()) return ch;
    if (!l) return ch.slice(0, i);
    tmStubs.set(pathKey(p), { parent, node: stubNode(l) });
    tmVer.value++;
  }
  return ch;
}

/** 方块图数据里没有的文件夹：用列表里它自己的信息补一个（子项等它被取回来时再拼） */
function stubNode(l: DirListing): TreemapNode {
  const n = l.node;
  const e = n.explanation;
  return {
    name: n.name,
    path: n.path,
    kind: n.kind,
    allocated: n.allocated,
    tier: !e || e.source.kind === "unknown" ? "unknown" : e.tier,
    what: e?.what ?? "",
    owner: e?.owner ?? null,
    lose_kind: e?.lose_kind ?? null,
    category: e?.category ?? "unknown",
    inaccessible: n.inaccessible,
    reparse_skipped: n.reparse_skipped,
    system_reserved: n.system_reserved,
    cloud_placeholder: n.cloud_placeholder,
    hardlink_dup: !!n.hardlink_dup,
    mapped: !!n.mapped,
    can_open: n.kind === "Dir" && n.child_count > 0 && !n.reparse_skipped,
    children: [],
    hidden_bytes: 0,
    hidden_count: 0,
  };
}

/** 右边列表换成 path（null = 盘根）；过期的结果丢掉 */
async function loadFocus(path: string | null, seq: number, pre?: DirListing) {
  const root = rootPath();
  if (!path || samePath(path, root)) {
    if (listing.value) focus.value = listing.value;
    return;
  }
  try {
    const l = pre ?? (await listDir(path, 200));
    if (seq === navSeq) focus.value = l;
  } catch {
    if (seq === navSeq) focus.value = listing.value;
  }
}

/** 取盘根的列表和方块图数据（第一次进分析页 / 清理之后） */
async function fetchRoot(root: string, gen: number): Promise<boolean> {
  const [l, t] = await loading(Promise.all([listDir(root), getTreemap(root, 3)]));
  if (gen !== tmGen) return false;
  listing.value = l;
  tmCache.set(pathKey(t.path), t);
  rootTm.value = t;
  tmVer.value++;
  return true;
}

/** 第一次进分析页：什么都不展开，方块图按真实比例显示整个盘，右边列表 = 盘根 */
async function openRoot() {
  resetExplore();
  const root = summary.value?.root;
  if (!root) return;
  const gen = tmGen;
  try {
    if (await fetchRoot(root, gen)) {
      focus.value = listing.value;
      prefetchKids();
    }
  } catch (e) {
    message.value = String(e);
  }
}

/**
 * 往里挖到 path：展开链 = 「根 → path」（祖先从方块图数据里找，找不到用列表的面包屑），
 * 链一变方块图立刻重排（块在原地长大），链上每一级自己的子树在后台取回来拼上；右边列表 = path。
 * path 为空或就是根 = 全部收起。opts.sel / opts.row / opts.sig：同时选中方块图里的一块、展开列表的一行。
 */
export async function digTo(path: string | null, opts: { sel?: string | null; row?: string | null; sig?: string | null } = {}) {
  const root = rootPath();
  if (!root) return;
  const seq = ++navSeq;
  const gen = tmGen;
  const pick = () => {
    selKey.value = opts.sel ?? null;
    selSig.value = opts.sig ?? null;
    openRow.value = opts.row ?? null;
  };
  if (!path || samePath(path, root)) {
    pick();
    chain.value = [];
    await loadFocus(null, seq);
    if (seq === navSeq) prefetchKids();
    return;
  }
  let anc = ancestorsIn(tm.value, path);
  let pre: DirListing | undefined;
  if (!anc) {
    try {
      pre = await listDir(path, 200);
    } catch (e) {
      if (seq === navSeq) message.value = String(e);
      return;
    }
    if (seq !== navSeq) return;
    anc = pre.breadcrumb.slice(1).map((c) => c.path);
  }
  const alive = () => seq === navSeq;
  const data = ensureChain(anc, gen, alive).then(async (ok) => {
    if (gen === tmGen && alive()) await loadOwn(ok, gen);
    return ok;
  });
  const list = loadFocus(path, seq, pre);
  // 链上有还没取过的数据：先等一小会儿（一般很快就回来），块的放大和更细的子块一次到位，不用先动一遍、数据回来再动一遍
  // （只有列表、看不见方块图时不用等）
  if (exploreView.value !== "list" && anc.some((p) => !tmCache.has(pathKey(p)))) await Promise.race([data, sleep(DATA_WAIT_MS)]);
  if (seq !== navSeq) return;
  pick();
  chain.value = anc;
  const [ok] = await Promise.all([data, list]);
  // 链上有一级已经不存在了（被清理掉）：截到它之前
  if (seq === navSeq && gen === tmGen && ok.length < anc.length) {
    chain.value = ok;
    await loadFocus(ok[ok.length - 1] ?? null, seq);
  }
  if (seq === navSeq && gen === tmGen) prefetchKids();
}

/** 面包屑第 i 级 / 收起：链截到前 i 级（0 = 全部收起） */
export async function collapseTo(i: number) {
  const ch = chain.value;
  const n = Math.max(0, Math.min(i, ch.length));
  await digTo(n === 0 ? null : ch[n - 1]);
}
/**
 * Backspace：收起一级。看得见方块图时按图上的一层（合并成一块的外壳 Users \ king 一次收起）；
 * 只有列表时（byMap = false）按文件夹一级一级往上。
 */
export async function collapseOne(byMap = true) {
  if (chain.value.length) {
    const starts = byMap ? chainLevelStarts(tm.value, chain.value) : [];
    await collapseTo(starts[starts.length - 1] ?? chain.value.length - 1);
  } else if (selKey.value) {
    clearSel();
    openRow.value = null;
  }
}
/** Esc：全部收起 */
export async function collapseAll() {
  if (chain.value.length || selKey.value || openRow.value) await collapseTo(0);
}

/** 不再选中方块图里的任何块（「其他」被拆开了等） */
export function clearSel() {
  selKey.value = null;
  selSig.value = null;
}

/**
 * 单击文件 / 其余小文件 / 「其他 N 项」/ 伪块：选中它。展开链设为它的父文件夹（已经是就不动），
 * 这块描粗边；row 给了就在右边列表展开那一行的说明（伪块 / 其他不给）。
 */
export async function selectLeaf(key: string, parent: string | null, row: string | null = null, sig: string | null = null) {
  const pk = focusKey.value ?? rootPath();
  const par = parent ?? rootPath();
  if (samePath(par, pk)) {
    selKey.value = key;
    selSig.value = sig;
    openRow.value = row;
    return;
  }
  await digTo(par, { sel: key, row, sig });
}

export type { MapHit };

/** 键盘（Enter / Space）的双击防抖：第二次落在这么多毫秒内、且第一次刚改变了展开链，就忽略 */
export const TAP_GUARD_MS = 300;
/** 上一次改变了展开链的单击：落在哪一块（路径）、什么时候 */
let lastTap: { path: string; at: number } | null = null;

/** 单击的目标在图上的位置（路径）：文件夹 / 标题栏 = 那一块（合并块按外层）；文件 = 它自己；其他 / 伪块 = 所在的文件夹 */
function hitPath(h: MapHit, root: string): string {
  if (h.kind === "dir") return h.path;
  if (h.kind === "head") return h.outer;
  return h.row ? h.key.replace(/\|.*$/, "") : (h.parent ?? root);
}
const related = (a: string, b: string) => samePath(a, b) || isUnder(a, b) || isUnder(b, a);

/** 单击的事件信息：at = 时间戳；detail = 鼠标连击次数（系统的双击判定；键盘触发 = 0） */
export interface TapInfo {
  at?: number;
  detail?: number;
}

/**
 * 方块图的单击（阶段 K §2）。双击 = 两次单击：第二下（鼠标按 detail ≥ 2，跟系统的双击设置走；键盘按 300 ms）
 * 只有落在第一下那块、或它的祖先 / 后代上，而且第一下刚改变了展开链，才忽略（防止一次双击连挖两层 / 挖了又收）；点别的块照常处理。
 */
export async function mapClick(hit: MapHit, ev: TapInfo = {}) {
  const at = ev.at ?? performance.now();
  const detail = ev.detail ?? 0;
  const root = rootPath();
  const target = hitPath(hit, root);
  if (lastTap && related(target, lastTap.path)) {
    const dt = at - lastTap.at;
    if (detail >= 2 || (detail === 0 && dt >= 0 && dt < TAP_GUARD_MS)) return;
  }
  const pk = focusKey.value ?? root;
  // 单击当前展开块（pk）自己的空白处：什么都不做（不重取列表、不收起列表里展开的行）
  if (hit.kind === "dir" && samePath(hit.path, pk)) {
    lastTap = null;
    return;
  }
  let changes = false;
  let run: () => Promise<void>;
  if (hit.kind === "dir") {
    changes = true;
    run = () => digTo(hit.path);
  } else if (hit.kind === "head") {
    const i = chain.value.findIndex((c) => samePath(c, hit.outer));
    changes = i >= 0 || !samePath(hit.path, pk);
    run = i >= 0 ? () => collapseTo(i) : () => digTo(hit.path);
  } else {
    changes = !samePath(hit.parent ?? root, pk);
    run = () => selectLeaf(hit.key, hit.parent, hit.row, hit.sig ?? null);
  }
  lastTap = changes ? { path: target, at } : null;
  await run();
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

export async function goExplore() {
  page.value = "explore";
  if (!listing.value && summary.value) await openRoot();
}

export function selectKeyOf(n: { kind: NodeView["kind"]; path: string }): string {
  return tmKey(n);
}

/**
 * 动作之后重新读当前视图（后端已经只重扫了受影响的子树）：清缓存、重取盘根，
 * 按原来的链重新取数据；链上不存在了的路径截掉。
 */
export async function reloadViews() {
  if (!listing.value) return;
  const root = listing.value.node.path;
  const keep = chain.value.slice();
  tmGen++;
  navSeq++;
  tmCache.clear();
  tmStubs.clear();
  tmInflight.clear();
  clearPrefetch();
  const gen = tmGen;
  const seq = navSeq;
  clearSel();
  openRow.value = null;
  try {
    if (!(await fetchRoot(root, gen))) return;
    const ok = await ensureChain(keep, gen);
    // 重新读的时候用户已经点了别处：听用户的
    if (gen !== tmGen || seq !== navSeq) return;
    chain.value = ok;
    await Promise.all([loadOwn(ok, gen), loadFocus(ok[ok.length - 1] ?? null, seq)]);
  } catch (e) {
    message.value = String(e);
  }
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
    resetExplore();
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
