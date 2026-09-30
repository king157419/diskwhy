<script setup lang="ts">
// 看看都是什么（空间分析）：左方块图（两层，颜色 = 三档），右列表（选中的那个文件夹的子项）。
// 行 = 色点、图标、名字、灰色类别（≤ 8 字）、细条、大小；混合 / 零散的用空心点，排在最后。
// 点一行在行内展开说明；有子项的文件夹点右端“›”进去。右上切「方块图 / 并排 / 列表」。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
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
import { CATEGORY_LABEL, clipWidth, isHardTag, loseLabel } from "../labels";
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

/** 面包屑：右边列表那个文件夹的完整路径（最后一段加粗） */
const crumbs = computed(() => {
  const bc = (focus.value ?? listing.value)?.breadcrumb ?? [];
  return bc.map((c, i) => ({ name: i === 0 ? c.name.replace(/\\$/, "") : c.name, path: c.path }));
});
/** 点面包屑：那一段成为右边列表，它的上一层成为方块图 */
async function crumbGo(i: number) {
  const bc = crumbs.value;
  if (i >= bc.length - 1) return;
  if (i === 0) await openDir(bc[0].path);
  else await openDir(bc[i - 1].path, bc[i].path);
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
  if (n.kind === "Rest") return `零散 ${n.files.toLocaleString("zh-CN")} 项`;
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

function onKey(e: KeyboardEvent) {
  if (e.key === "Backspace" && !(e.target instanceof HTMLInputElement)) {
    e.preventDefault();
    void goUp();
  }
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="explore" :class="'v-' + exploreView">
    <div class="sub">
      <div class="crumb">
        <template v-for="(c, i) in crumbs" :key="c.path + i">
          <span v-if="i > 0" class="sep">/</span>
          <b v-if="i === crumbs.length - 1">{{ c.name }}</b>
          <button v-else class="cr" @click="crumbGo(i)">{{ c.name }}</button>
        </template>
        <span class="sz num">{{ gbNum(focusTotal) }} GB</span>
      </div>
      <div class="key">
        <span><i class="dot safe"></i>可放心清理</span><span><i class="dot review"></i>需你判断</span><span><i class="dot keep"></i>先别动</span>
      </div>
      <div class="seg-t" role="radiogroup" aria-label="视图">
        <button v-for="v in VIEWS" :key="v.id" :class="{ on: exploreView === v.id }" role="radio" :aria-checked="exploreView === v.id" @click="setExploreView(v.id)">
          {{ v.name }}
        </button>
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
                  <span v-if="loseLabel(n.explanation.lose_kind) && n.explanation.tier === 'review'" class="tag" :class="{ hard: isHardTag(n.explanation.lose_kind) }">{{
                    loseLabel(n.explanation.lose_kind)
                  }}</span>
                  {{ n.explanation.lose }}
                </dd>
                <dt>怎么清</dt>
                <dd>{{ n.explanation.how_detail || n.explanation.gain_note }}</dd>
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
.sub {
  position: absolute;
  left: 24px;
  right: 24px;
  top: 52px;
  height: 44px;
  display: flex;
  align-items: center;
  gap: 24px;
}
.crumb {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-body);
  color: var(--ink3);
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
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
.key {
  margin-left: auto;
  display: flex;
  gap: 18px;
  font-size: var(--fs-sub);
  color: var(--ink2);
  white-space: nowrap;
}
.key span {
  display: flex;
  align-items: center;
  gap: 7px;
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
  position: absolute;
  left: 24px;
  right: 24px;
  top: 108px;
  bottom: 24px;
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
.detail .path {
  margin-top: 8px;
  padding-left: 16px;
  word-break: break-all;
}
.link.small {
  font-size: var(--fs-meta);
}
</style>
