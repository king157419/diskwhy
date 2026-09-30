// 全局界面状态 + 动作（阶段 F：首页 / 清理 / 看看都是什么 / 确认弹层 / 隔离区 / 设置）。
// 只调用 api.ts 里的后端命令；清理候选由后端算好（ScanSummary.candidates），这里只管勾选和预检。
import { computed, ref } from "vue";
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
import { collapseChain } from "./treemap";

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
/** 删到哪里：回收站（默认）/ 隔离到其他盘 */
export const cleanupMode = ref<"recycle" | "quarantine">("recycle");
export const qDest = ref("");

export async function refreshActionEnv() {
  try {
    actEnv.value = await actionEnv();
  } catch (e) {
    message.value = `读取隔离区信息失败：${e}`;
  }
}

export function currentStrategy(): Strategy {
  if (cleanupMode.value === "quarantine" && qDest.value) return { kind: "quarantine", dest_root: qDest.value };
  return { kind: "recycle_bin" };
}

/** 后台逐个文件检查的进度（弹层里显示）；检查做完为 null */
export const checkProgress = ref<PreviewProgress | null>(null);
/** 当前这次预检的序号：关掉弹层 / 换了删到哪里之后，旧的结果回来也不用 */
let previewSeq = 0;

/**
 * 预检：把勾选组里能直接删的路径交给后端（带着规则 id，后端会核对说明有没有变）。
 * 阶段 G：后端先发 quote（按扫描时的大小），弹层立刻出现、按钮处于“正在检查”；
 * 逐个文件的检查在后台进行（progress），做完 promise 返回完整报告，按钮才可用。
 */
export async function doPreview(strategy: Strategy = currentStrategy()) {
  const items = checkedGroups.value.flatMap((g) => g.paths.filter((p) => p.deletable).map((p) => ({ path: p.path, expect_rule: g.rule_id })));
  if (!items.length) return;
  const seq = ++previewSeq;
  previewBusy.value = true;
  message.value = "";
  checkProgress.value = { phase: "plan", dirs: 0, entries: 0, checked: 0, to_check: 0, locked: 0, rm_done: 0, rm_total: 0 };
  try {
    const final = await previewPlan(items, strategy, (e) => {
      if (seq !== previewSeq) return;
      if (e.kind === "quote") {
        report.value = e.report;
        previewBusy.value = false;
      } else checkProgress.value = e.progress;
    });
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

/** 改删到哪里（回收站 ↔ 隔离到某个盘）：重新预检 */
export async function requote(s: Strategy) {
  if (s.kind === "quarantine") {
    cleanupMode.value = "quarantine";
    qDest.value = s.dest_root;
  } else cleanupMode.value = "recycle";
  await doPreview(s);
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
export async function init() {
  try {
    env.value = await getEnv();
    drives.value = await listDrives();
    const c = drives.value.find((d) => d.root.toUpperCase() === "C:\\");
    scanRoot.value = (c ?? drives.value[0])?.root ?? "C:\\";
    void refreshActionEnv();
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

/** 打开一个文件夹作为方块图的根；focusPath 给了就把它当右边列表，否则挑最大的子文件夹 */
export async function openDir(path: string, focusPath?: string) {
  try {
    tmLoading.value = true;
    const [l, t] = await Promise.all([listDir(path), getTreemap(path, 3)]);
    listing.value = l;
    tm.value = t;
    openRow.value = null;
    const target = focusPath ?? defaultFocus(t);
    await setFocus(target ?? null);
  } catch (e) {
    message.value = String(e);
  } finally {
    tmLoading.value = false;
  }
}

/** 默认的右边列表：最大的子文件夹；它如果只是个“外壳”（Users 里几乎全是 king），就再往里走一层 */
function defaultFocus(t: TreemapNode): string | undefined {
  const dirs = t.children.filter((c) => c.kind === "Dir" && c.children.length > 0);
  if (!dirs.length) return undefined;
  const big = dirs.reduce((a, b) => (b.allocated > a.allocated ? b : a));
  return collapseChain(big).node.path;
}

export async function setFocus(path: string | null) {
  openRow.value = null;
  if (!path) {
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

/** 进入方块图里的某一块（双击） */
export async function enterBlock(path: string) {
  await openDir(path);
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
    if (p) notice.value = `快照已保存：${p}`;
  } catch (e) {
    message.value = `保存快照失败：${e}`;
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
    if (s.snapshot) notice.value = `正在查看快照：${s.snapshot.path}`;
    page.value = "home";
  } catch (e) {
    message.value = `打开快照失败：${e}`;
  } finally {
    snapBusy.value = false;
  }
}

export function onSettingsSaved(s: LlmSettings) {
  if (env.value) env.value.llm_ready = s.enabled && !!s.base_url && !!s.model;
}
