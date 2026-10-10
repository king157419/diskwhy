<script setup lang="ts">
// 看看都是什么（空间分析）：左方块图（颜色 = 三档），右列表（当前展开的那个文件夹的子项）。
// 行 = 色点、图标、名字、灰色类别（≤ 8 字）、细条、大小；混合 / 零散的用空心点，排在最后。
// 点一行在行内展开说明；有子项的文件夹点右端“›”= 在方块图里展开它。右上切「方块图 / 并排 / 列表」。
// 阶段 J：「图例」按钮（悬停 / 点击出现，色块复用方块的配色类）；窄窗口 / 竖屏时方块图在上、列表在下。
// 阶段 K：方块图是一张平面图，单击原地展开（见 store.ts mapClick）；面包屑第 i 级 = 收起到第 i 级；
// Esc = 全部收起，Backspace = 收起一级。
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
import { askAi, type Category, type Explanation, type NodeView } from "../api";
import { sizeParts, gbNum } from "../capacity";
import { advice, CATEGORY_LABEL, clipWidth, isHardTag, loseTag } from "../labels";
import { samePath } from "../treemap";
import { pathKey } from "../tmtree";
import {
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
  if (n.kind === "File" && !n.explanation?.category) return FileIcon;
  return ICON[n.explanation?.category ?? "unknown"] ?? Folder;
}
function catText(n: NodeView): string {
  if (n.kind === "Rest") return `小文件 ${n.files.toLocaleString("zh-CN")} 个`;
  const e = n.explanation;
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
const shownRows = computed(() => rows.value.slice(0, rowLimit.value));
/** 列表数据已经是当前展开的那个文件夹的（链一变到列表数据回来之前：旧行淡化，标题先用方块图数据里的名字和大小） */
const listFresh = computed(() => !pkInfo.value || (!!focus.value && samePath(focus.value.node.path, pkInfo.value.path)));
/** 面包屑旁边 / 列表标题的大小：方块图数据里 pk 的大小（链一变立刻跟上，不等列表） */
const focusTotal = computed(() => pkInfo.value?.size ?? focus.value?.node.allocated ?? 0);
const focusName = computed(() => (listFresh.value ? focus.value?.node.name.replace(/\\$/, "") : pkInfo.value?.name) ?? "");

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
/** 当前展开的文件夹自己认不出来时，列表标题下面写它的第一条线索 */
const pkClue = computed(() => (listFresh.value ? firstClue(focus.value?.node) : undefined));

/** 列表里的大小：GB 一位小数（和设计稿一致），小于 1 GB 用 MB */
function rowSize(b: number): [string, string] {
  const g = b / 1024 ** 3;
  return g >= 1 ? [g >= 100 ? g.toFixed(0) : g.toFixed(1), "GB"] : sizeParts(b);
}

function toggleRow(n: NodeView) {
  openRow.value = openRow.value === n.path + n.kind ? null : n.path + n.kind;
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

// 问 AI（只在认不出来、而且用户在设置里开了 AI 时出现）
const ai = ref<Record<string, Explanation | string>>({});
async function doAsk(n: NodeView) {
  ai.value = { ...ai.value, [n.path]: "…" };
  try {
    ai.value = { ...ai.value, [n.path]: await askAi(n.path) };
  } catch (e) {
    ai.value = { ...ai.value, [n.path]: String(e) };
  }
}
const aiOf = (p: string) => ai.value[p];

/** 展开行的「建议」（阶段 H） */
function adviceOf(e: Explanation) {
  return advice(e.source.kind === "unknown" ? "keep" : e.tier, e.how, e.how_detail, e.source.kind === "rule" ? e.source.group : null);
}

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
});
onBeforeUnmount(() => {
  clearTimeout(rowTimer);
  crumbRo?.disconnect();
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
          @hit="onHit"
          @unsel="clearSel"
        />
      </div>

      <section v-if="exploreView !== 'map'" class="list">
        <div class="lh" :class="{ hasclue: !!pkClue }">
          <div class="lt">
            <h2>{{ focusName }}</h2>
            <span class="m"><template v-if="listFresh"><b class="num">{{ rows.length }}</b> 项 · </template><b class="num">{{ gbNum(focusTotal) }}</b> GB</span>
            <span class="sort">按大小</span>
          </div>
          <div v-if="pkClue" class="clue" :title="pkClue"><Lightbulb :size="13" :stroke-width="1.8" aria-hidden="true" /><span>{{ pkClue }}</span></div>
        </div>
        <div ref="rowsEl" class="rows" :class="{ stale: !listFresh }">
          <template v-for="n in shownRows" :key="n.path + n.kind">
            <div class="li" :class="{ tail: tone(n) === 'mix', open: openRow === n.path + n.kind }">
              <i class="dot" :class="tone(n)"></i>
              <span class="ic"><component :is="icon(n)" :size="16" :stroke-width="1.6" /></span>
              <button class="n" :aria-label="'展开：' + n.name" @click="toggleRow(n)">{{ n.name }}</button>
              <span class="c">{{ catText(n) }}</span>
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
                <dd>{{ n.explanation.what }}</dd>
                <dt>删了会怎样</dt>
                <dd>
                  <span v-if="loseTag(n.explanation) && n.explanation.tier === 'review'" class="tag" :class="{ hard: isHardTag(n.explanation.lose_kind) }">{{
                    loseTag(n.explanation)
                  }}</span>
                  {{ n.explanation.lose }}
                </dd>
                <dt>建议</dt>
                <dd>
                  <b class="lead">{{ adviceOf(n.explanation).head }}。</b>{{ adviceOf(n.explanation).detail }}
                </dd>
                <template v-if="n.explanation.clues.length">
                  <dt>线索</dt>
                  <dd>{{ n.explanation.clues.join("；") }}</dd>
                </template>
                <template v-if="n.explanation.source.kind === 'unknown' && llmReady">
                  <dt>问 AI</dt>
                  <dd>
                    <button v-if="!aiOf(n.path)" class="link small" @click="doAsk(n)">问 AI</button>
                    <template v-else-if="typeof aiOf(n.path) === 'string'">{{ aiOf(n.path) }}</template>
                    <template v-else>{{ (aiOf(n.path) as Explanation).what }} · {{ (aiOf(n.path) as Explanation).lose }}</template>
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
.lh {
  height: 64px;
  flex: none;
  display: flex;
  flex-direction: column;
  padding: 10px 4px 0;
  border-bottom: 1px solid var(--line2);
  min-width: 0;
}
.lh .lt {
  display: flex;
  align-items: baseline;
  gap: 12px;
  min-width: 0;
}
/* 当前展开的文件夹自己认不出来：标题下面一行灰色小字写它的第一条线索 */
.lh .clue {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 1px;
  min-width: 0;
  font-size: var(--fs-meta);
  line-height: 18px;
  color: var(--ink3);
  white-space: nowrap;
}
.lh .clue svg {
  flex: none;
}
.lh .clue span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.lh h2 {
  font-family: var(--heading);
  font-size: 22px;
  font-weight: var(--heading-w);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 60%;
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

/* ───────── 窄窗口 / 竖屏（阶段 J §3.7）：方块图至少约 58% 高在上，列表在下；列表行少时列表变矮，空出来的给方块图 ───────── */
@media (max-width: 1099px), (max-aspect-ratio: 1/1) {
  .explore {
    padding: 52px var(--gutter) 16px;
  }
  .body {
    flex-direction: column;
    gap: 14px;
  }
  .v-split .mapbox {
    flex: 1 1 0;
    min-height: 58%;
  }
  .list {
    width: auto;
    flex: 1;
  }
  .v-split .list {
    flex: 0 1 auto;
    max-height: calc(42% - 14px);
  }
  .v-split .rows {
    flex: 0 1 auto;
  }
  .lh {
    height: 48px;
    padding-top: 4px;
  }
  .lh.hasclue {
    height: 66px;
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
  .tools {
    margin-left: 0;
    flex: 1 1 100%;
    justify-content: space-between;
  }
  .lg-pop {
    right: auto;
    left: 0;
  }
}
</style>
