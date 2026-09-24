// 全局界面状态 + 动作（阶段 E 从 App.vue 挪出来，让侧栏的各个页面共用）。
// 这里只调用 api.ts 里已有的后端命令，没有新增 / 修改任何 IPC 接口。
import { computed, ref, watch } from "vue";
import {
  actionEnv,
  cancelScan,
  getEnv,
  getTreemap,
  listDir,
  listDrives,
  nodeInfo,
  openSnapshot,
  pickScanFolder,
  previewPlan,
  relaunchAsAdmin,
  saveSnapshot,
  startScan,
  startupScanDir,
  startupSnapshot,
  tmKey,
  type ActionEnv,
  type CartItem,
  type DirListing,
  type DriveInfo,
  type EnvInfo,
  type ExecOutcome,
  type LlmSettings,
  type NodeView,
  type PreflightReport,
  type PseudoKey,
  type ScanProgress,
  type ScanSummary,
  type Strategy,
  type TmSelect,
  type TreemapNode,
} from "./api";
import { fmtDuration, fmtTime } from "./format";
import { pseudoView } from "./pseudo";

export type Phase = "idle" | "scanning" | "done";
export type Page = "overview" | "analysis" | "cleanup" | "quarantine" | "settings";
export type ViewMode = "list" | "map";

// ───── 基本状态 ─────
export const page = ref<Page>("overview");
export const env = ref<EnvInfo | null>(null);
export const drives = ref<DriveInfo[]>([]);
export const selectedDrive = ref("");
/** 用户选的文件夹（不是整个盘） */
export const customRoot = ref("");
export const phase = ref<Phase>("idle");
export const progress = ref<ScanProgress | null>(null);
export const summary = ref<ScanSummary | null>(null);
/** 空间分析页当前打开的目录 */
export const listing = ref<DirListing | null>(null);
/** 扫描根目录的列表（概览页“最大的目录”用，不随下钻变化） */
export const rootListing = ref<DirListing | null>(null);
export const tm = ref<TreemapNode | null>(null);
export const tmLoading = ref(false);
export const selected = ref<NodeView | null>(null);
export const selectedKey = ref<string | null>(null);
export const message = ref("");
export const notice = ref("");
export const toast = ref<{ text: string; action?: { label: string; page: Page } } | null>(null);
export const cancelling = ref(false);
export const relaunching = ref(false);
export const snapBusy = ref(false);
export const llmReady = computed(() => env.value?.llm_ready ?? false);

let toastTimer: ReturnType<typeof setTimeout> | undefined;
export function showToast(text: string, action?: { label: string; page: Page }) {
  toast.value = { text, action };
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = null), 4200);
}

function loadViewMode(): ViewMode {
  try {
    const v = localStorage.getItem("diskwhy.view2");
    if (v === "list" || v === "map") return v;
  } catch {
    /* 存储不可用时用默认 */
  }
  return "list";
}
export const viewMode = ref<ViewMode>(loadViewMode());
watch(viewMode, (v) => {
  try {
    localStorage.setItem("diskwhy.view2", v);
  } catch {
    /* 忽略 */
  }
});

export const detailNode = computed(() => selected.value ?? listing.value?.node ?? null);
export const isScanRoot = computed(() => !!summary.value && !!listing.value && listing.value.node.path === summary.value.root);

// ───── 清理清单 / 预览 / 隔离区（阶段 D） ─────
export const actEnv = ref<ActionEnv | null>(null);
export const cart = ref<CartItem[]>([]);
/** 清单里被取消勾选的项（小写路径）：预览时不带上 */
export const unchecked = ref<Set<string>>(new Set());
export const previewBusy = ref(false);
export const report = ref<PreflightReport | null>(null);
export const cartPaths = computed(() => new Set(cart.value.map((c) => c.path.toLowerCase())));
export const checkedCart = computed(() => cart.value.filter((c) => !unchecked.value.has(c.path.toLowerCase())));

/** 删到哪里：回收站 / 隔离到其他盘（清理清单页的选择；预览里“改为隔离到 X”会改它） */
export const cleanupMode = ref<"recycle" | "quarantine">("recycle");
export const CUSTOM_DEST = "__custom__";
export const qDest = ref("");
export const qCustom = ref("");

export async function refreshActionEnv() {
  try {
    actEnv.value = await actionEnv();
  } catch (e) {
    message.value = `读取隔离区信息失败：${e}`;
  }
}

export function addToCart(n: NodeView) {
  const e = n.explanation;
  if (!e || cartPaths.value.has(n.path.toLowerCase())) return;
  cart.value.push({
    path: n.path,
    name: n.name,
    kind: n.kind,
    allocated: n.allocated,
    tier: e.tier,
    rule_id: e.source.kind === "rule" ? e.source.id : null,
    what: e.what,
    lose: e.lose,
  });
  showToast(`已把「${n.name}」加入清理清单（共 ${cart.value.length} 项）。加入不会马上删除。`, { label: "去清理清单", page: "cleanup" });
}

export function removeFromCart(path: string) {
  const k = path.toLowerCase();
  cart.value = cart.value.filter((c) => c.path.toLowerCase() !== k);
  if (unchecked.value.has(k)) {
    const s = new Set(unchecked.value);
    s.delete(k);
    unchecked.value = s;
  }
}

export function toggleChecked(path: string) {
  const k = path.toLowerCase();
  const s = new Set(unchecked.value);
  if (s.has(k)) s.delete(k);
  else s.add(k);
  unchecked.value = s;
}

export function clearCart() {
  cart.value = [];
  unchecked.value = new Set();
}

export async function doPreview(strategy: Strategy) {
  const items = checkedCart.value;
  if (!items.length) return;
  previewBusy.value = true;
  message.value = "";
  try {
    report.value = await previewPlan(
      items.map((c) => ({ path: c.path, expect_rule: c.rule_id })),
      strategy,
    );
  } catch (e) {
    message.value = `预览失败：${e}`;
  } finally {
    previewBusy.value = false;
  }
}

/** 预览里点“改为隔离到 X”：切换策略并重新预览 */
export async function onRequote(s: Strategy) {
  if (s.kind === "quarantine") {
    cleanupMode.value = "quarantine";
    const d = (actEnv.value?.drives ?? drives.value).find((x) => x.root === s.dest_root);
    if (d) qDest.value = d.root;
    else {
      qDest.value = CUSTOM_DEST;
      qCustom.value = s.dest_root;
    }
  }
  report.value = null;
  await doPreview(s);
}

export async function onExecuted(o: ExecOutcome) {
  if (o.summary) summary.value = o.summary;
  const done = new Set(o.report.items.filter((i) => !i.blocked && i.failed === 0).map((i) => i.path.toLowerCase()));
  cart.value = cart.value.filter((c) => !done.has(c.path.toLowerCase()));
  await reloadViews();
  await refreshActionEnv();
}

export async function onQuarantineChanged(s: ScanSummary | null) {
  if (s) summary.value = s;
  if (s) await reloadViews();
  await refreshActionEnv();
}

/** 动作之后：重新读取当前目录和根目录（后端已经只重扫了受影响的子树） */
async function reloadViews() {
  if (listing.value) await open(listing.value.node.path, true);
  if (summary.value) {
    try {
      rootListing.value = await listDir(summary.value.root, 12);
    } catch {
      /* 忽略 */
    }
  }
}

// ───── 扫描 / 浏览 ─────
export async function init() {
  try {
    env.value = await getEnv();
    drives.value = await listDrives();
    const c = drives.value.find((d) => d.root.toUpperCase() === "C:\\");
    selectedDrive.value = (c ?? drives.value[0])?.root ?? "";
    await refreshActionEnv();
    // 用 “diskwhy.exe 某个快照.dwsnap” 启动（或双击关联的快照文件）时直接打开
    const snap = await startupSnapshot();
    if (snap) await doOpenSnapshot(snap);
    const dir = await startupScanDir();
    if (dir && !snap) {
      customRoot.value = dir;
      selectedDrive.value = dir;
      await doScan();
    }
  } catch (e) {
    message.value = `初始化失败：${e}`;
  }
}

export async function doPickFolder() {
  try {
    const p = await pickScanFolder();
    if (!p) return;
    customRoot.value = p;
    selectedDrive.value = p;
    await doScan();
  } catch (e) {
    message.value = `选择文件夹失败：${e}`;
  }
}

export async function doScan() {
  if (!selectedDrive.value || phase.value === "scanning") return;
  message.value = "";
  notice.value = "";
  phase.value = "scanning";
  if (page.value !== "overview" && page.value !== "analysis") page.value = "overview";
  progress.value = null;
  summary.value = null;
  listing.value = null;
  rootListing.value = null;
  tm.value = null;
  selected.value = null;
  cancelling.value = false;
  clearCart();
  try {
    const s = await startScan(selectedDrive.value, (p) => (progress.value = p));
    summary.value = s;
    phase.value = "done";
    await afterResult(s);
  } catch (e) {
    phase.value = "idle";
    message.value = String(e);
  } finally {
    cancelling.value = false;
  }
}

async function afterResult(s: ScanSummary) {
  await open(s.root);
  rootListing.value = listing.value;
}

export async function doCancel() {
  cancelling.value = true;
  await cancelScan();
}

export async function open(path: string, keepSelection = false) {
  try {
    tmLoading.value = true;
    const [l, t] = await Promise.all([listDir(path), getTreemap(path, 3)]);
    listing.value = l;
    tm.value = t;
    if (!keepSelection) {
      selected.value = null;
      selectedKey.value = null;
    }
  } catch (e) {
    message.value = String(e);
  } finally {
    tmLoading.value = false;
  }
}

/** 从概览等页面跳到空间分析并打开某个目录 */
export async function openInAnalysis(path: string, selectNode?: NodeView) {
  page.value = "analysis";
  await open(path);
  if (selectNode) select(selectNode);
}

export function goUp() {
  const bc = listing.value?.breadcrumb;
  if (bc && bc.length > 1) open(bc[bc.length - 2].path);
}

export function select(n: NodeView) {
  selected.value = n;
  selectedKey.value = tmKey(n);
}

export async function onTmSelect(s: TmSelect) {
  selectedKey.value = s.key;
  if (s.pseudo) {
    selected.value = pseudoView(s.pseudo as PseudoKey, summary.value!);
    return;
  }
  if (!s.node) return; // “其他”块：只高亮
  const direct = listing.value?.children.find((c) => tmKey(c) === s.key);
  if (direct) {
    selected.value = direct;
    return;
  }
  try {
    selected.value = await nodeInfo(s.node.path, s.node.kind);
  } catch (e) {
    message.value = String(e);
  }
}

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
    phase.value = "done";
    clearCart();
    const sn = s.snapshot;
    notice.value = sn
      ? `正在查看快照：${sn.path}（${sn.meta.kind === "snapshot" ? `保存于 ${fmtTime(sn.meta.saved_at)}` : "CLI --json 输出，目录树可能只有几层"}，读取用时 ${fmtDuration(sn.load_ms)}）。说明是按当前规则库现算的。`
      : "";
    await afterResult(s);
  } catch (e) {
    message.value = `打开快照失败：${e}`;
  } finally {
    snapBusy.value = false;
  }
}

export function onSettingsSaved(s: LlmSettings) {
  if (env.value) env.value.llm_ready = s.enabled && !!s.base_url && !!s.model;
}
