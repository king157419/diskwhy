<script setup lang="ts">
// 看看都是什么（空间分析）：左方块图（两层，颜色 = 三档），右列表（选中的那个文件夹的子项）。
// 行 = 色点、图标、名字、灰色类别（≤ 8 字）、细条、大小；混合 / 零散的用空心点，排在最后。
// 点一行在行内展开说明；有子项的文件夹点右端“›”进去。右上切「方块图 / 并排 / 列表」。
// 阶段 J：面包屑每一级都能点（盘根 → 方块图和列表都是盘根；其余 → 上一级做方块图、它自己做列表）；
// 「图例」按钮（悬停 / 点击出现，色块复用方块的配色类）；窄窗口 / 竖屏时方块图在上、列表在下。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  AppWindow,
  Box,
  Cpu,
  Download,
  File as FileIcon,
  Folder,
  HardDrive,
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
import { crumbTarget } from "../treemap";
import {
  enterBlock,
  enterRow,
  exploreView,
  focus,
  focusKey,
  goUp,
  listing,
  llmReady,
  openDir,
  openRow,
  setExploreView,
  setFocus,
  summary,
  tm,
  tmLoading,
  type ExploreView,
} from "../store";

const VIEWS: { id: ExploreView; name: string }[] = [
  { id: "map", name: "方块图" },
  { id: "split", name: "并排" },
  { id: "list", name: "列表" },
];

const isScanRoot = computed(() => !!summary.value && !!listing.value && listing.value.node.path === summary.value.root);

/** 面包屑：右边列表那个文件夹的完整路径（最后一段加粗 = 当前选中） */
const crumbs = computed(() => {
  const bc = (focus.value ?? listing.value)?.breadcrumb ?? [];
  return bc.map((c, i) => ({ name: i === 0 ? c.name.replace(/\\$/, "") : c.name, path: c.path }));
});
/** 点面包屑第 i 级：见 treemap.ts crumbTarget（盘根也能选中，列在右边） */
async function crumbGo(i: number) {
  const t = crumbTarget(crumbs.value, i);
  if (t) await openDir(t.open, t.focus);
}
// 路径很长时面包屑横向滚动，总是露出最后一段
const crumbEl = ref<HTMLElement | null>(null);
watch(crumbs, () => nextTick(() => crumbEl.value && (crumbEl.value.scrollLeft = crumbEl.value.scrollWidth)));

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
const focusTotal = computed(() => focus.value?.node.allocated ?? 0);

/** 列表里的大小：GB 一位小数（和设计稿一致），小于 1 GB 用 MB */
function rowSize(b: number): [string, string] {
  const g = b / 1024 ** 3;
  return g >= 1 ? [g >= 100 ? g.toFixed(0) : g.toFixed(1), "GB"] : sizeParts(b);
}

function toggleRow(n: NodeView) {
  openRow.value = openRow.value === n.path + n.kind ? null : n.path + n.kind;
}

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
  if (e.key === "Escape") lgPinned.value = false;
  if (e.key === "Backspace" && !(e.target instanceof HTMLInputElement)) {
    e.preventDefault();
    void goUp();
  }
}
onMounted(() => {
  window.addEventListener("keydown", onKey);
  document.addEventListener("mousedown", onDocDown);
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKey);
  document.removeEventListener("mousedown", onDocDown);
});
</script>

<template>
  <div class="explore" :class="'v-' + exploreView">
    <div class="sub">
      <nav ref="crumbEl" class="crumb" aria-label="位置">
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
      <div v-if="exploreView !== 'list'" class="mapbox" :class="{ loading: tmLoading }">
        <TreemapView
          v-if="summary"
          :data="tm"
          :summary="summary"
          :is-scan-root="isScanRoot"
          :focus-path="focusKey"
          @select="setFocus"
          @open="enterBlock"
        />
      </div>

      <section v-if="exploreView !== 'map'" class="list">
        <div class="lh">
          <h2>{{ focus?.node.name.replace(/\\$/, "") ?? "" }}</h2>
          <span class="m"><b class="num">{{ rows.length }}</b> 项 · <b class="num">{{ gbNum(focusTotal) }}</b> GB</span>
          <span class="sort">按大小</span>
        </div>
        <div class="rows">
          <template v-for="n in rows" :key="n.path + n.kind">
            <div class="li" :class="{ tail: tone(n) === 'mix', open: openRow === n.path + n.kind }">
              <i class="dot" :class="tone(n)"></i>
              <span class="ic"><component :is="icon(n)" :size="16" :stroke-width="1.6" /></span>
              <button class="n" :aria-label="'展开：' + n.name" @click="toggleRow(n)">{{ n.name }}</button>
              <span class="c">{{ catText(n) }}</span>
              <span class="br"><i :style="{ width: (n.allocated / maxRow) * 100 + '%' }"></i></span>
              <span class="s num">{{ rowSize(n.allocated)[0] }}<small>{{ rowSize(n.allocated)[1] }}</small></span>
              <button v-if="n.kind === 'Dir' && n.child_count > 0 && !n.pseudo" class="go" :aria-label="'进入：' + n.name" @click="enterRow(n)">
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
  transition: opacity var(--dur-2) var(--ease);
}
.mapbox.loading {
  opacity: 0.55;
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
  align-items: baseline;
  gap: 12px;
  padding: 10px 4px 0;
  border-bottom: 1px solid var(--line2);
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
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--line2) transparent;
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

/* ───────── 窄窗口 / 竖屏（阶段 J §3.7）：方块图约 45% 高在上，列表在下 ───────── */
@media (max-width: 1099px), (max-aspect-ratio: 1/1) {
  .explore {
    padding: 52px var(--gutter) 16px;
  }
  .body {
    flex-direction: column;
    gap: 14px;
  }
  .v-split .mapbox {
    flex: 0 0 45%;
    min-height: 0;
  }
  .list {
    width: auto;
    flex: 1;
  }
  .lh {
    height: 48px;
    padding-top: 4px;
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
