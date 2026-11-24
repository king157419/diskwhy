<script setup lang="ts">
// 看看都是什么（空间分析）：左方块图（颜色 = 三档），右列表（当前展开的那个文件夹的子项）。
// 行 = 色点、图标、名字、灰色类别（≤ 8 字）、细条、大小；混合 / 零散的用空心点，排在最后。
// 点一行在行内展开说明；有子项的文件夹点右端“›”= 在方块图里展开它。右上切「方块图 / 并排 / 列表」。
// 阶段 J：「图例」按钮（悬停 / 点击出现，色块复用方块的配色类）；窄窗口 / 竖屏时方块图在上、列表在下。
// 阶段 K：方块图是一张平面图，单击原地展开（见 store.ts mapClick）；面包屑第 i 级 = 收起到第 i 级；
// Esc = 全部收起，Backspace = 收起一级。
// 阶段 L：说明类的字一律写全（列表标题下的线索长了就折行），不用省略号截断。
// 阶段 M：扫描完后台自动问 AI。认不出来、已经有推测的：类别写「AI 推测」（行的顺序、色点不变），展开行的「这是什么 / 删了会怎样」
// 写推测、前面一个「AI 推测」小标签管这两行（推测没写后果时写中性的兜底，不退回叫人去点「问 AI」的那句），列表标题下的一行同样；
// 同一个文件夹开着时，标题下那一行从线索换成推测（通常短一些）标题不变矮（lhMin），下面的行不会往上跳；
// 工具栏图例旁边一个小标签显示进度 / 出错（AiChip）。没有「问 AI」可点时，后端说法里「或者（点）问 AI」那半句不显示。
// 换了文件夹列表回到最上面。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  AppWindow,
  Box,
  Cpu,
  Download,
  File as FileIcon,
  Folder,
  HardDrive,
  Lightbulb,
  MoreHorizontal,
  ScrollText,
  Sparkles,
  Trash2,
  Wrench,
} from "lucide-vue-next";
import TreemapView from "../components/TreemapView.vue";
import AiChip from "../components/AiChip.vue";
import AiTag from "../components/AiTag.vue";
import { hardBlock, type Category, type Explanation, type NodeView } from "../api";
import { aiLose, noteFor, withoutAskAi } from "../ai";
import { sizeParts, gbNum } from "../capacity";
import { advice, CATEGORY_LABEL, clipWidth, isHardTag, loseTag } from "../labels";
import { samePath } from "../treemap";
import { fitTitle, pathKey } from "../tmtree";
import {
  addPathToCleanup,
  aiNotes,
  askAiNow,
  candidates,
  checked,
  page,
  chain,
  collapseAll,
  collapseOne,
  collapseTo,
  crumbs,
  digTo,
  execPhase,
  exploreView,
  focus,
  listing,
  llmReady,
  clearSel,
  mapClick,
  openRow,
  pkInfo,
  prefetchFor,
  report,
  selKey,
  selSig,
  setExploreView,
  summary,
  tm,
  tmLoading,
  type ExploreView,
  type MapHit,
  type TapInfo,
} from "../store";

const VIEWS: { id: ExploreView; name: string }[] = [
  { id: "map", name: "方块图" },
  { id: "split", name: "并排" },
  { id: "list", name: "列表" },
];

/** 点面包屑第 i 级：展开链截到第 i 级（第 0 级 = 全部收起） */
function crumbGo(i: number) {
  void collapseTo(i);
}
function onHit(h: MapHit, ev: TapInfo) {
  void mapClick(h, ev);
}
/** 鼠标在一个还没取过的文件夹块上停了一会儿：后台预取它的子树（不改任何状态） */
function onPeek(path: string) {
  prefetchFor(path);
}
// 路径很长时面包屑横向滚动，总是露出最后一段；左边被盖住的部分渐隐（不会把「C:」切成「:」）
const crumbEl = ref<HTMLElement | null>(null);
const crumbCut = ref(false);
function crumbFade() {
  const el = crumbEl.value;
  crumbCut.value = !!el && el.scrollLeft > 1;
}
function crumbEnd() {
  if (crumbEl.value) crumbEl.value.scrollLeft = crumbEl.value.scrollWidth;
  crumbFade();
}
watch(crumbs, () => nextTick(crumbEnd));
// 窗口变窄、面包屑放不下了：同样露出最后一段、左边渐隐
let crumbRo: ResizeObserver | null = null;

// ── 图例：悬停出现，点一下钉住（再点 / 点别处 / Esc 收起） ──
const LEGEND: { tone: string; name: string; note: string }[] = [
  { tone: "safe", name: "可放心清理", note: "删了会自动重建" },
  { tone: "review", name: "需你判断", note: "删了有代价" },
  { tone: "keep", name: "先别动", note: "系统 · 程序 · 你的文件" },
  { tone: "mix", name: "认不出来", note: "含“其他 N 项”" },
  { tone: "na", name: "扫不到", note: "无权限 · 系统保留" },
];
const lgHover = ref(false);
const lgPinned = ref(false);
const lgShow = computed(() => lgHover.value || lgPinned.value);
function onDocDown(e: MouseEvent) {
  if (lgPinned.value && !(e.target as Element | null)?.closest?.(".lg-wrap")) lgPinned.value = false;
}

type Tone = "safe" | "review" | "keep" | "mix";
function tone(n: NodeView): Tone {
  if (n.kind === "Rest" || n.pseudo) return "mix";
  const e = n.explanation;
  if (!e || e.source.kind === "unknown") return "mix";
  return e.tier;
}
const ICON: Record<Category, unknown> = {
  cache: Box,
  log: ScrollText,
  installer: Download,
  build: Wrench,
  model: Sparkles,
  userdata: Folder,
  system: Cpu,
  vm: HardDrive,
  trash: Trash2,
  app: AppWindow,
  unknown: Folder,
};
function icon(n: NodeView) {
  if (n.kind === "Rest") return MoreHorizontal;
  // 认不出来的文件用文件图标（ICON.unknown 是文件夹）
  const c = n.explanation?.category;
  if (n.kind === "File" && (!c || c === "unknown")) return FileIcon;
  return ICON[c ?? "unknown"] ?? Folder;
}
/** 认不出来、后台问过 AI 的：它的推测（数据里没带上时用这次运行里收到过的补） */
const aiOf = (n: NodeView | null | undefined) => noteFor(n, aiNotes.value);
function catText(n: NodeView): string {
  if (n.kind === "Rest") return `小文件 ${n.files.toLocaleString("zh-CN")} 个`;
  const e = n.explanation;
  if (aiOf(n)) return "AI 推测";
  if (!e || e.source.kind === "unknown") return n.kind === "Dir" ? `混合 ${n.child_count} 项` : "未知";
  if (e.category === "userdata") return e.tier === "keep" ? "你的文件" : "数据";
  return clipWidth(CATEGORY_LABEL[e.category] || "", 16);
}

const rows = computed(() => {
  const kids = (focus.value?.children ?? []).filter((k) => k.allocated > 0);
  const main = kids.filter((k) => tone(k) !== "mix");
  const tail = kids.filter((k) => tone(k) === "mix");
  return [...main, ...tail];
});
const maxRow = computed(() => Math.max(1, ...rows.value.map((r) => r.allocated)));
/**
 * 换了文件夹：先只画前面一屏的行，方块图的过渡（约 260 ms）结束后再补上其余的——
 * 几百行和方块图的过渡挤在同一帧里渲染，会让方块图的动画卡一下。
 */
const ROWS_FIRST = 24;
const rowLimit = ref(Infinity);
let rowTimer: ReturnType<typeof setTimeout> | undefined;
watch(focus, () => {
  rowLimit.value = ROWS_FIRST;
  clearTimeout(rowTimer);
  rowTimer = setTimeout(() => (rowLimit.value = Infinity), 320);
});
// 换了一个文件夹：列表回到最上面（不然沿用上一个文件夹的滚动位置，最大的那几行被推出视野）。
// 同一个文件夹的数据刷新不动；要展开某一行时，下面那个 watch 会再把它滚进可视区。
watch(
  () => focus.value?.node.path,
  (p, old) => {
    if (p && old && !samePath(p, old) && rowsEl.value) rowsEl.value.scrollTop = 0;
  },
);
const shownRows = computed(() => rows.value.slice(0, rowLimit.value));
/** 列表数据已经是当前展开的那个文件夹的（链一变到列表数据回来之前：旧行淡化，标题先用方块图数据里的名字和大小） */
const listFresh = computed(() => !pkInfo.value || (!!focus.value && samePath(focus.value.node.path, pkInfo.value.path)));
/** 面包屑旁边 / 列表标题的大小：方块图数据里 pk 的大小（链一变立刻跟上，不等列表） */
const focusTotal = computed(() => pkInfo.value?.size ?? focus.value?.node.allocated ?? 0);
const focusName = computed(() => (listFresh.value ? focus.value?.node.name.replace(/\\$/, "") : pkInfo.value?.name) ?? "");

// ── 列表标题放不下：按量出来的字宽截断，省略号用界面字体的「…」（标题字体里的「···」是居中的，和方块图标题栏不一致） ──
const h2El = ref<HTMLElement | null>(null);
const ltEl = ref<HTMLElement | null>(null);
const ltW = ref(0);
const fontTick = ref(0);
let ltRo: ResizeObserver | null = null;
let titleCtx: CanvasRenderingContext2D | null = null;
const titleFit = computed(() => {
  void fontTick.value;
  const name = focusName.value;
  const el = h2El.value;
  if (!el || !ltW.value) return { name, cut: false };
  const cs = getComputedStyle(el);
  titleCtx ??= document.createElement("canvas").getContext("2d");
  if (!titleCtx) return { name, cut: false };
  const ctx = titleCtx;
  const font = `${cs.fontStyle} ${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
  const ui = getComputedStyle(document.documentElement).getPropertyValue("--ui").trim() || "sans-serif";
  const ellFont = `${cs.fontWeight} ${cs.fontSize} ${ui}`;
  const w = (t: string) => {
    ctx.font = font;
    return ctx.measureText(t).width;
  };
  ctx.font = ellFont;
  const ell = ctx.measureText("…").width;
  // 标题最多占这一行的 60%（和 CSS 的 max-width 一致），留 2px 误差
  return fitTitle(name, ltW.value * 0.6 - 2, w, ell);
});
watch(ltEl, (el) => {
  ltRo?.disconnect();
  ltRo = null;
  if (!el || typeof ResizeObserver !== "function") return;
  ltRo = new ResizeObserver(() => (ltW.value = el.clientWidth));
  ltRo.observe(el);
  ltW.value = el.clientWidth;
});

/** 认不出来的东西的第一条线索（列表数据里才有）：方块图的悬停卡片直接写它 */
const firstClue = (n: NodeView | undefined) => (n?.explanation?.source.kind === "unknown" ? n.explanation.clues[0] : undefined);
const clueMap = computed(() => {
  const m = new Map<string, string>();
  for (const l of [listing.value, focus.value]) {
    if (!l) continue;
    for (const n of [l.node, ...l.children]) {
      const c = firstClue(n);
      if (c) m.set(pathKey(n.path), c);
    }
  }
  return m;
});
/**
 * 当前展开的文件夹自己认不出来时，列表标题下面写一行：问过 AI 的写推测（带「AI 推测」标签），没问过写它的第一条线索
 */
const pkLine = computed<{ text: string; ai: boolean } | null>(() => {
  if (!listFresh.value) return null;
  const a = aiOf(focus.value?.node);
  if (a) return { text: a.what.trim(), ai: true };
  const c = firstClue(focus.value?.node);
  return c ? { text: c, ai: false } : null;
});
/**
 * 列表标题的最小高度（px，0 = 按 CSS）：同一个文件夹开着时，标题下那一行从线索换成 AI 推测（通常短一些、少折一行），
 * 标题不跟着变矮——不然下面每一行都往上跳一下。换了文件夹、列表变宽变窄时放开。
 */
const lhEl = ref<HTMLElement | null>(null);
const lhMin = ref(0);
// 默认在 DOM 更新之前跑：这时量到的还是换之前（写着线索）的高度
watch(pkLine, (now, before) => {
  if (now?.ai && before && !before.ai && lhEl.value) lhMin.value = Math.max(lhMin.value, lhEl.value.getBoundingClientRect().height);
});
watch([() => focus.value?.node.path, ltW], () => (lhMin.value = 0));

/** 列表里的大小：GB 一位小数（和设计稿一致），小于 1 GB 用 MB */
function rowSize(b: number): [string, string] {
  const g = b / 1024 ** 3;
  return g >= 1 ? [g >= 100 ? g.toFixed(0) : g.toFixed(1), "GB"] : sizeParts(b);
}

function toggleRow(n: NodeView) {
  openRow.value = openRow.value === n.path + n.kind ? null : n.path + n.kind;
}

// ── 加入清理（展开行里）：什么都能加，除了保护路径 / 系统保留这类硬拦的（显示 ≤ 6 字的原因）；
// keep / 认不出来的加之前二次确认一次，加进去的只进回收站 / 暂存区 ──
/** 路径 → 硬拦原因（null = 能加；"…" = 正在问后端） */
const addWhy = ref<Record<string, string | null>>({});
const canAdd = (n: NodeView) => n.kind !== "Rest" && !n.pseudo;
watch(openRow, async (k) => {
  const n = k ? shownRows.value.find((x) => x.path + x.kind === k) : null;
  if (!n || !canAdd(n) || n.path in addWhy.value) return;
  addWhy.value = { ...addWhy.value, [n.path]: "…" };
  let why: string | null = null;
  try {
    why = await hardBlock(n.path);
  } catch {
    why = null;
  }
  addWhy.value = { ...addWhy.value, [n.path]: why };
});
/** 已经在清理清单里（勾着） */
function inCleanup(n: NodeView) {
  const k = n.path.toLowerCase();
  return candidates.value.some((g) => checked.value.has(g.key) && g.paths.some((p) => p.path.toLowerCase() === k));
}
/** 行里的名字被截断（省略号）时才给原生提示：悬停一会儿能看到全名；没截断不弹 */
function nameTip(e: MouseEvent, name: string) {
  const el = e.currentTarget as HTMLElement;
  if (el.scrollWidth > el.clientWidth + 1) el.title = name;
  else el.removeAttribute("title");
}
/** 列表里的「›」：在方块图里展开这个文件夹（链延长一级） */
function digRow(n: NodeView) {
  if (n.kind === "Dir") void digTo(n.path);
}
// 方块图里选中了一个文件：列表滚到那一行，连同它展开的说明一起进入可视区（说明比列表还高时让行在最上面）
const rowsEl = ref<HTMLElement | null>(null);
watch([openRow, focus, rowLimit], () =>
  nextTick(() => {
    const box = rowsEl.value;
    const row = box?.querySelector<HTMLElement>(".li.open");
    if (!box || !row) return;
    const detail = row.nextElementSibling?.classList.contains("detail") ? (row.nextElementSibling as HTMLElement) : null;
    const c = box.getBoundingClientRect();
    const r = row.getBoundingClientRect();
    // 说明刚出现时带一点位移动画（往上 4px），多留 8px
    const bottom = (detail ?? row).getBoundingClientRect().bottom + (detail ? 8 : 0);
    const top = box.scrollTop + (r.top - c.top);
    const end = box.scrollTop + (bottom - c.top);
    let to = box.scrollTop;
    if (end > box.scrollTop + box.clientHeight) to = Math.min(top, end - box.clientHeight);
    if (top < to) to = top;
    if (Math.abs(to - box.scrollTop) > 1) box.scrollTo({ top: to, behavior: "smooth" });
  }),
);

// 手动「问 AI」（只在认不出来、还没有推测、而且用户在设置里开了 AI 时出现）：问的时候写「…」，出错写后端给的一句短话；
// 问到了答案补进所有视图（store.askAiNow），这一行就换成「这是什么 / 删了会怎样」里的推测
const asking = ref<Record<string, string>>({});
async function doAsk(n: NodeView) {
  const k = pathKey(n.path);
  asking.value = { ...asking.value, [k]: "…" };
  try {
    await askAiNow(n.path);
    const next = { ...asking.value };
    delete next[k];
    asking.value = next;
  } catch (e) {
    asking.value = { ...asking.value, [k]: String(e) };
  }
}
const askState = (n: NodeView) => asking.value[pathKey(n.path)];
/** 展开行里有「问 AI」可点：认不出来、还没有推测、开了 AI */
const canAsk = (n: NodeView) => n.explanation?.source.kind === "unknown" && llmReady.value && !aiOf(n) && n.kind !== "Rest" && !n.pseudo;

/** 展开行的「建议」（阶段 H） */
function adviceOf(n: NodeView, e: Explanation) {
  const a = advice(e.source.kind === "unknown" ? "keep" : e.tier, e.how, e.how_detail, e.source.kind === "rule" ? e.source.group : null);
  // 没有「问 AI」可点（已经有推测了 / 没开 AI）：「……，或者问 AI。」后半句不说
  return canAsk(n) ? a : { ...a, detail: withoutAskAi(a.detail) };
}
/** 展开行的「删了会怎样」（没有推测时）：同样，没有「问 AI」可点就不叫人去点 */
const loseText = (n: NodeView, e: Explanation) => (canAsk(n) ? e.lose : withoutAskAi(e.lose));

function onKey(e: KeyboardEvent) {
  const t = e.target;
  if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || e.isComposing) return;
  // 确认 / 清理弹层开着：页面是 inert 的，键盘不动方块图
  if (report.value || execPhase.value !== "idle") return;
  if (e.key === "Escape") {
    if (lgPinned.value) lgPinned.value = false;
    else void collapseAll();
  } else if (e.key === "Backspace") {
    e.preventDefault();
    void collapseOne(exploreView.value !== "list");
  }
}
onMounted(() => {
  window.addEventListener("keydown", onKey);
  document.addEventListener("mousedown", onDocDown);
  if (crumbEl.value && typeof ResizeObserver === "function") {
    crumbRo = new ResizeObserver(crumbEnd);
    crumbRo.observe(crumbEl.value);
  }
  document.fonts?.ready.then(() => fontTick.value++);
});
onBeforeUnmount(() => {
  clearTimeout(rowTimer);
  crumbRo?.disconnect();
  ltRo?.disconnect();
  window.removeEventListener("keydown", onKey);
  document.removeEventListener("mousedown", onDocDown);
});
</script>

<template>
  <div class="explore" :class="'v-' + exploreView">
    <div class="sub">
      <nav ref="crumbEl" class="crumb" :class="{ cut: crumbCut }" aria-label="位置" @scroll.passive="crumbFade">
        <template v-for="(c, i) in crumbs" :key="c.path + i">
          <span v-if="i > 0" class="sep">/</span>
          <b v-if="i === crumbs.length - 1" class="cur">{{ c.name }}</b>
          <button v-else class="cr" :aria-label="'选中：' + c.name" @click="crumbGo(i)">{{ c.name }}</button>
        </template>
        <span class="sz num">{{ gbNum(focusTotal) }} GB</span>
      </nav>
      <div class="tools">
        <AiChip class="ai-chip" />
        <div class="lg-wrap" @mouseenter="lgHover = true" @mouseleave="lgHover = false">
          <button class="lg-btn" :class="{ on: lgPinned }" :aria-expanded="lgShow" aria-label="图例" @click="lgPinned = !lgPinned">
            <span class="lg-ico" aria-hidden="true"><i class="tm-tone safe"></i><i class="tm-tone review"></i><i class="tm-tone keep"></i></span>图例
          </button>
          <div v-if="lgShow" class="lg-pop" role="tooltip">
            <div v-for="r in LEGEND" :key="r.tone" class="lg-row">
              <i class="sw tm-tone" :class="r.tone"></i><b>{{ r.name }}</b><span>{{ r.note }}</span>
            </div>
          </div>
        </div>
        <div class="seg-t" role="radiogroup" aria-label="视图">
          <button v-for="v in VIEWS" :key="v.id" :class="{ on: exploreView === v.id }" role="radio" :aria-checked="exploreView === v.id" @click="setExploreView(v.id)">
            {{ v.name }}
          </button>
        </div>
      </div>
    </div>

    <div class="body">
      <div v-if="exploreView !== 'list'" class="mapbox">
        <TreemapView
          v-if="summary"
          :data="tm"
          :summary="summary"
          :chain="chain"
          :sel-key="selKey"
          :sel-sig="selSig"
          :loading="tmLoading"
          :pin="exploreView === 'map'"
          :clues="clueMap"
          :notes="aiNotes"
          @hit="onHit"
          @unsel="clearSel"
          @peek="onPeek"
        />
      </div>

      <section v-if="exploreView !== 'map'" class="list">
        <div ref="lhEl" class="lh" :class="{ hasclue: !!pkLine }" :style="lhMin ? { minHeight: lhMin + 'px' } : undefined">
          <div ref="ltEl" class="lt">
            <h2 ref="h2El" :title="titleFit.cut ? focusName : undefined">{{ titleFit.name }}<span v-if="titleFit.cut" class="el">…</span></h2>
            <span class="m"><template v-if="listFresh"><b class="num">{{ rows.length }}</b> 项 · </template><b class="num">{{ gbNum(focusTotal) }}</b> GB</span>
            <span class="sort">按大小</span>
          </div>
          <div v-if="pkLine" class="clue" :class="{ ai: pkLine.ai }">
            <AiTag v-if="pkLine.ai" /><Lightbulb v-else :size="13" :stroke-width="1.8" aria-hidden="true" /><span>{{ pkLine.text }}</span>
          </div>
        </div>
        <div ref="rowsEl" class="rows" :class="{ stale: !listFresh }">
          <template v-for="n in shownRows" :key="n.path + n.kind">
            <div class="li" :class="{ tail: tone(n) === 'mix', open: openRow === n.path + n.kind }">
              <i class="dot" :class="tone(n)"></i>
              <span class="ic"><component :is="icon(n)" :size="16" :stroke-width="1.6" /></span>
              <button class="n" :aria-label="'展开：' + n.name" @mouseenter="nameTip($event, n.name)" @click="toggleRow(n)">{{ n.name }}</button>
              <span class="c" :class="{ ai: !!aiOf(n) }"
                ><svg v-if="aiOf(n)" class="ai-star" viewBox="0 0 12 12" aria-hidden="true"><path d="M6 .6C6.35 3.6 8.4 5.65 11.4 6 8.4 6.35 6.35 8.4 6 11.4 5.65 8.4 3.6 6.35.6 6 3.6 5.65 5.65 3.6 6 .6Z" /></svg
                >{{ catText(n) }}</span
              >
              <span class="br"><i :style="{ width: (n.allocated / maxRow) * 100 + '%' }"></i></span>
              <span class="s num">{{ rowSize(n.allocated)[0] }}<small>{{ rowSize(n.allocated)[1] }}</small></span>
              <button v-if="n.kind === 'Dir' && n.child_count > 0 && !n.pseudo" class="go" :aria-label="'在方块图里展开：' + n.name" @click="digRow(n)">
                <svg class="i" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6" /></svg>
              </button>
              <span v-else></span>
            </div>
            <div v-if="openRow === n.path + n.kind && n.explanation" class="detail">
              <dl>
                <dt>这是什么</dt>
                <dd v-if="aiOf(n)" class="aiw"><AiTag /><span>{{ aiOf(n)!.what.trim() }}</span></dd>
                <dd v-else>{{ n.explanation.what }}</dd>
                <dt>删了会怎样</dt>
                <!-- AI 的说法：同一个「AI 推测」标签管这两行（是猜的，不是确认；档位照旧是认不出来、「建议」照旧是别动）。
                     第二行不再叠一个一样的标签，只留一个看不见的同宽占位：两行推测的字左边对齐 -->
                <dd v-if="aiOf(n)" class="aiw"><AiTag class="ghost" aria-hidden="true" /><span>{{ aiLose(aiOf(n)!) }}</span></dd>
                <dd v-else>
                  <span v-if="loseTag(n.explanation) && n.explanation.tier === 'review'" class="tag" :class="{ hard: isHardTag(n.explanation.lose_kind) }">{{
                    loseTag(n.explanation)
                  }}</span>
                  {{ loseText(n, n.explanation) }}
                </dd>
                <dt>建议</dt>
                <dd>
                  <b class="lead">{{ adviceOf(n, n.explanation).head }}。</b>{{ adviceOf(n, n.explanation).detail }}
                </dd>
                <template v-if="canAdd(n)">
                  <dt>清理</dt>
                  <dd class="add">
                    <span v-if="addWhy[n.path] && addWhy[n.path] !== '…'" class="tag plain">{{ addWhy[n.path] }}</span>
                    <template v-else-if="inCleanup(n)">
                      <span class="added">已加入</span><button class="link small" @click="page = 'cleanup'">去清理</button>
                    </template>
                    <button v-else class="link small" :disabled="addWhy[n.path] === '…'" @click="addPathToCleanup(n)">加入清理</button>
                  </dd>
                </template>
                <template v-if="n.explanation.clues.length">
                  <dt>线索</dt>
                  <dd>{{ n.explanation.clues.join("；") }}</dd>
                </template>
                <template v-if="canAsk(n)">
                  <dt>问 AI</dt>
                  <dd class="ask">
                    <button v-if="!askState(n)" class="link small" @click="doAsk(n)">问 AI</button>
                    <span v-else-if="askState(n) === '…'" class="asking" role="status" aria-label="正在问 AI"><i></i><i></i><i></i></span>
                    <template v-else>
                      <span class="ask-err">{{ askState(n) }}</span><button class="link small" @click="doAsk(n)">重试</button>
                    </template>
                  </dd>
                </template>
              </dl>
              <div class="path mono">{{ n.path }}</div>
            </div>
          </template>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.explore {
  display: flex;
  flex-direction: column;
  padding: 52px 24px 24px;
}
.sub {
  flex: none;
  min-height: 44px;
  display: flex;
  align-items: center;
  gap: 24px;
}
.crumb {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-body);
  color: var(--ink3);
  min-width: 0;
  white-space: nowrap;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: none;
}
.crumb::-webkit-scrollbar {
  display: none;
}
/* 面包屑太长、滚到了最右：左边被盖住的部分渐隐（不是硬切成半个字） */
.crumb.cut {
  -webkit-mask-image: linear-gradient(90deg, transparent 0, #000 36px);
  mask-image: linear-gradient(90deg, transparent 0, #000 36px);
}
.crumb .cr {
  flex: none;
  padding: 2px 4px;
  margin: 0 -4px;
  border-radius: 4px;
}
.crumb .cr:hover {
  background: var(--hover);
}
.crumb b {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.crumb .cr {
  color: var(--ink3);
}
.crumb .cr:hover {
  color: var(--ink);
}
.crumb .sep {
  color: var(--line2);
}
.crumb .sz {
  color: var(--ink3);
  font-size: var(--fs-sub);
  margin-left: 6px;
}
[data-theme="C"] .crumb b {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 18px;
}
.tools {
  margin-left: auto;
  flex: none;
  display: flex;
  align-items: center;
  gap: 16px;
}
/* ── 图例 ── */
.lg-wrap {
  position: relative;
}
.lg-btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 12px 0 10px;
  font-size: var(--fs-sub);
  color: var(--ink2);
  border-radius: var(--r);
  border: var(--bw) solid transparent;
  transition: background-color var(--dur-1) var(--ease);
}
.lg-btn:hover,
.lg-btn.on {
  background: var(--hover);
  color: var(--ink);
}
.lg-ico {
  display: inline-grid;
  grid-template-columns: repeat(3, 6px);
  gap: 2px;
}
.lg-ico i {
  width: 6px;
  height: 12px;
  border-radius: 1.5px;
}
[data-theme="C"] .lg-btn {
  border-color: #141412;
  border-radius: 999px;
  font-family: var(--heading);
  font-weight: 700;
  font-size: 14px;
  color: #141412;
  background: #fff;
}
[data-theme="C"] .lg-btn:hover,
[data-theme="C"] .lg-btn.on {
  background: var(--accent);
}
[data-theme="C"] .lg-ico i {
  border: 1px solid #141412;
}
.lg-pop {
  position: absolute;
  right: 0;
  top: calc(100% + 8px);
  z-index: 20;
  width: 300px;
  padding: 10px 14px;
  background: var(--surface);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-pop);
  display: flex;
  flex-direction: column;
  animation: lg-in var(--dur-2) var(--ease);
}
@keyframes lg-in {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}
[data-theme="C"] .lg-pop {
  border: 2px solid #141412;
  box-shadow: 4px 4px 0 #141412;
}
.lg-row {
  display: grid;
  grid-template-columns: 30px auto 1fr;
  align-items: center;
  column-gap: 10px;
  height: 38px;
  font-size: var(--fs-sub);
}
.lg-row + .lg-row {
  border-top: 1px solid var(--line);
}
.lg-row .sw {
  width: 30px;
  height: 22px;
  border-radius: 4px;
}
[data-theme="C"] .lg-row .sw {
  border: 2px solid #141412;
  border-radius: 6px;
}
.lg-row b {
  font-weight: var(--w-semi);
  color: var(--ink);
  white-space: nowrap;
}
.lg-row span {
  color: var(--ink3);
  font-size: var(--fs-meta);
  text-align: right;
  white-space: nowrap;
}
.seg-t {
  display: flex;
  border: 1px solid var(--line2);
  border-radius: var(--r);
  overflow: hidden;
  font-size: var(--fs-sub);
  flex: none;
}
.seg-t button {
  padding: 6px 14px;
  color: var(--ink2);
}
.seg-t button + button {
  border-left: 1px solid var(--line2);
}
.seg-t button:hover:not(.on) {
  background: var(--hover);
}
.seg-t .on {
  background: var(--ink);
  color: var(--bg);
  font-weight: var(--w-semi);
}
[data-theme="B"] .seg-t .on {
  background: var(--line2);
  color: var(--ink);
}
[data-theme="C"] .seg-t {
  border: 2px solid #141412;
  border-radius: 999px;
}
[data-theme="C"] .seg-t button + button {
  border-left: 2px solid #141412;
}
[data-theme="C"] .seg-t .on {
  background: var(--accent);
  color: #141412;
}

.body {
  flex: 1;
  min-height: 0;
  margin-top: 12px;
  display: flex;
  gap: 24px;
}
.mapbox {
  flex: 1;
  min-width: 0;
}
.list {
  width: 484px;
  flex: none;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.v-list .list {
  width: auto;
  flex: 1;
}
/* 高度至少 64（线索长了折成两三行时往下长，不截断）；底下留 4px：一行线索时总高正好还是 64 */
.lh {
  min-height: 64px;
  flex: none;
  display: flex;
  flex-direction: column;
  padding: 10px 4px 4px;
  border-bottom: 1px solid var(--line2);
  min-width: 0;
}
.lh .lt {
  display: flex;
  align-items: baseline;
  gap: 12px;
  min-width: 0;
}
/* 当前展开的文件夹自己认不出来：标题下面灰色小字写它的第一条线索（阶段 L：写全，长了就折行，不用省略号） */
.lh .clue {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin-top: 1px;
  min-width: 0;
  font-size: var(--fs-meta);
  line-height: 18px;
  color: var(--ink3);
}
.lh .clue svg {
  flex: none;
  margin-top: 2.5px;
}
.lh .clue span {
  min-width: 0;
  overflow-wrap: anywhere;
  user-select: text;
}
/* 问过 AI 的：小标签代替灯泡（标签和第一行字一样高，对齐第一行） */
.lh .clue.ai {
  gap: 7px;
  color: var(--ink2);
}
.lh .clue.ai .tag {
  flex: none;
  min-width: auto;
  user-select: none;
}
[data-theme="C"] .lh .clue.ai > span:not(.tag) {
  padding-top: 1px;
}
.lh h2 {
  font-family: var(--heading);
  font-size: 22px;
  font-weight: var(--heading-w);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: clip;
  max-width: 60%;
}
.lh h2 .el {
  font-family: var(--ui);
}
.lh .m {
  font-size: var(--fs-sub);
  color: var(--ink3);
  white-space: nowrap;
}
.lh .m b {
  color: var(--ink2);
  font-weight: var(--w-med);
}
.lh .sort {
  margin-left: auto;
  font-size: var(--fs-meta);
  color: var(--ink3);
}
.rows {
  transition: opacity var(--dur-1) var(--ease);
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--line2) transparent;
}
/* 链变了、列表数据还没回来：旧的行先淡一点 */
.rows.stale {
  opacity: 0.5;
}
.li {
  height: 44px;
  display: grid;
  grid-template-columns: 16px 26px minmax(0, 1fr) auto 60px 70px 22px;
  align-items: center;
  column-gap: 6px;
  padding: 0 4px;
  border-bottom: 1px solid var(--line);
  transition: background-color var(--dur-1) var(--ease);
}
.li:hover,
.li.open {
  background: var(--hover);
}
.li .ic {
  color: var(--ink3);
  display: grid;
  place-items: center;
}
.li .n {
  font-size: 14px;
  font-weight: var(--w-med);
  color: var(--ink);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  height: 100%;
}
.li .c {
  font-size: var(--fs-meta);
  color: var(--ink3);
  white-space: nowrap;
  padding: 0 10px 0 8px;
}
/* 问过 AI 的：「✦ AI 推测」（中性灰，比「混合 N 项」深一级） */
.li .c.ai {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--ink2);
}
.li .c.ai .ai-star {
  color: var(--ink3);
}
[data-theme="C"] .li .c.ai {
  color: var(--ink2);
}
[data-theme="C"] .li .c.ai .ai-star {
  color: #141412;
}
.li .br {
  height: 4px;
  border-radius: 2px;
  background: var(--sunk);
  overflow: hidden;
}
.li .br i {
  display: block;
  height: 100%;
  background: var(--ink3);
}
[data-theme="B"] .li .br {
  background: #2a241e;
}
[data-theme="B"] .li .br i {
  background: #7e7468;
}
[data-theme="C"] .li .br {
  height: 6px;
  border: 1.5px solid #141412;
  background: #fff;
  border-radius: 4px;
}
[data-theme="C"] .li .br i {
  background: #141412;
}
.li .s {
  text-align: right;
  font-size: 15px;
  font-weight: var(--w-med);
  color: var(--ink);
}
.li .s small {
  font-size: 11px;
  color: var(--ink3);
  margin-left: 2px;
}
.li.tail .n {
  color: var(--ink2);
  font-weight: var(--w-reg);
}
.li .go {
  color: var(--ink3);
  display: grid;
  place-items: center;
  width: 22px;
  height: 28px;
  border-radius: 4px;
}
.li .go:hover {
  color: var(--ink);
  background: var(--sunk);
}
.li .go .i {
  width: 15px;
  height: 15px;
}
.detail {
  padding: 8px 8px 14px 48px;
  border-bottom: 1px solid var(--line);
  user-select: text;
  animation: drop var(--dur-2) var(--ease);
}
@keyframes drop {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}
.detail dl {
  border-left: 2px solid var(--line2);
  padding: 2px 0 2px 14px;
  display: grid;
  grid-template-columns: 64px 1fr;
  row-gap: 6px;
  column-gap: 12px;
  font-size: var(--fs-meta);
  line-height: 1.6;
}
[data-theme="C"] .detail dl {
  border-left: 3px solid #141412;
}
.detail dt {
  color: var(--ink3);
}
.detail dd {
  color: var(--ink2);
}
.detail dd .tag {
  margin-right: 6px;
}
/* 「这是什么」「删了会怎样」是 AI 推测：标签在前，推测的字折行时和第一行对齐（不钻到标签下面） */
.detail dd.aiw {
  display: flex;
  align-items: flex-start;
  gap: 7px;
}
.detail dd.aiw .tag {
  margin: 0.5px 0 0;
}
[data-theme="C"] .detail dd.aiw .tag {
  margin-top: -0.5px;
}
.detail dd.aiw .tag.ghost {
  visibility: hidden;
}
.detail dd.aiw > span:not(.tag) {
  min-width: 0;
  overflow-wrap: anywhere;
}
.detail dd .lead {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.detail .path {
  margin-top: 8px;
  padding-left: 16px;
  word-break: break-all;
}
.link.small {
  font-size: var(--fs-meta);
}
/* 手动问 AI：正在问 = 三个轮流变深的小点；出错 = 后端给的一句短话 + 「重试」 */
.detail dd.add {
  display: flex;
  align-items: center;
  gap: 12px;
}
.detail dd.add .added {
  color: var(--ink2);
}
.detail dd.ask {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  column-gap: 12px;
  row-gap: 2px;
}
.ask-err {
  color: var(--ink2);
  overflow-wrap: anywhere;
}
.asking {
  display: inline-flex;
  gap: 4px;
  height: 19px;
  align-items: center;
}
.asking i {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--ink3);
  opacity: 0.35;
  animation: ask-dot 1.1s var(--ease) infinite;
}
.asking i:nth-child(2) {
  animation-delay: 0.18s;
}
.asking i:nth-child(3) {
  animation-delay: 0.36s;
}
@keyframes ask-dot {
  30% {
    opacity: 1;
  }
}
@media (prefers-reduced-motion: reduce) {
  .asking i {
    animation: none;
    opacity: 0.7;
  }
}

/* ───────── 窄窗口 / 竖屏（阶段 J §3.7）：方块图在上、列表在下。方块图的高度固定（约可用高度的 58%，有下限），
   不随列表行数变（K 第 3 轮 F1：图一变高变矮，整棵树就按新的宽高比重排，和展开的过渡叠在一起就是块乱飞） ───────── */
@media (max-width: 1099px), (max-aspect-ratio: 1/1) {
  .explore {
    padding: 52px var(--gutter) 16px;
  }
  .body {
    flex-direction: column;
    gap: 14px;
  }
  .v-split .mapbox {
    flex: 0 0 auto;
    height: 58%;
    min-height: 280px;
  }
  .list {
    width: auto;
    flex: 1;
  }
  .v-split .list {
    flex: 1 1 0;
    min-height: 0;
  }
  .lh {
    min-height: 48px;
    padding-top: 4px;
  }
  .lh.hasclue {
    min-height: 66px;
  }
}
/* 更窄：副栏换行，面包屑独占一行 */
@media (max-width: 819px) {
  .sub {
    flex-wrap: wrap;
    row-gap: 6px;
    column-gap: 12px;
  }
  .crumb {
    flex: 1 1 100%;
  }
  /* 图例、AI 小标签靠左（小标签在图例右边，出现 / 消失时图例不挪），视图切换靠右 */
  .tools {
    margin-left: 0;
    flex: 1 1 100%;
    justify-content: flex-start;
    gap: 10px;
  }
  .tools .ai-chip {
    order: 1;
  }
  .tools .seg-t {
    order: 2;
    margin-left: auto;
  }
  .lg-pop {
    right: auto;
    left: 0;
  }
}
</style>
