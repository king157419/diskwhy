<script setup lang="ts">
// 方块图（阶段 F 改成 DOM 绘制，好让三套主题的描边 / 圆角 / 斜条纹全部由 CSS token 决定）。
// 布局仍是 src/treemap.ts 的 squarified + 嵌套：两层（当前文件夹的子项 → 它们的子项），颜色 = 三档，
// 认不出来 / 合并的“其他”画成混合色。单击选中一个顶层块（右边列表跟着变），双击进入。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { tmKey, type ScanSummary, type TreemapNode } from "../api";
import { isHardTag, loseTag } from "../labels";
import { collapseChain, layoutTreemap, type TmInput, type TmItem } from "../treemap";
import { sizeParts } from "../capacity";

const props = defineProps<{
  data: TreemapNode | null;
  summary: ScanSummary;
  isScanRoot: boolean;
  focusPath: string | null;
}>();
const emit = defineEmits<{ select: [path: string]; open: [path: string] }>();

const wrap = ref<HTMLDivElement | null>(null);
const size = ref({ w: 0, h: 0 });
let ro: ResizeObserver | null = null;
onMounted(() => {
  if (!wrap.value) return;
  const upd = () => (size.value = { w: wrap.value!.clientWidth, h: wrap.value!.clientHeight });
  upd();
  ro = new ResizeObserver(upd);
  ro.observe(wrap.value);
});
onBeforeUnmount(() => ro?.disconnect());

const PSEUDO: Record<string, string> = { inacc: "无权限", sysest: "系统保留", unattr: "未归属", cloud: "云端占位" };

function toInput(n: TreemapNode): TmInput {
  // 几乎只装着一个文件夹的“外壳”（Users → king）合成一块，名字写成 “Users \ king”，里面直接铺内层的子项
  const { node, name } = collapseChain(n);
  return {
    key: tmKey(node),
    name,
    value: node.allocated,
    children: node.children.map(toInput),
    hidden: node.hidden_bytes,
    hiddenCount: node.hidden_count,
    ref: node,
  };
}

const root = computed<TmInput | null>(() => {
  const d = props.data;
  if (!d) return null;
  const r = toInput(d);
  if (props.isScanRoot) {
    const b = props.summary.buckets;
    const extra: [string, number][] = [
      ["inacc", b.inaccessible],
      ["sysest", b.system_reserved_estimated],
      ["unattr", b.scope_is_volume_root ? b.unattributed : 0],
    ];
    for (const [k, v] of extra) {
      if (v <= 0) continue;
      r.children!.push({ key: `pseudo:${k}`, name: PSEUDO[k], value: v, ref: { pseudo: k } });
      r.value += v;
    }
  }
  return r;
});

const G = 3;
const items = computed<TmItem[]>(() => {
  const r = root.value;
  const { w, h } = size.value;
  if (!r || w < 20 || h < 20) return [];
  return layoutTreemap(r, { x: 0, y: 0, w: w + G, h: h + G }, { minPx: 8, maxDepth: 2, headerPx: 26, pad: 4, minContainerPx: 90 });
});

function nodeOf(it: TmItem): TreemapNode | null {
  const x = it.ref as TreemapNode | { pseudo: string } | undefined;
  return x && "path" in x ? x : null;
}
function tone(it: TmItem): string {
  if (it.kind === "other") return "mix";
  const n = nodeOf(it);
  if (!n) return "keep";
  if (n.kind === "Rest") return "mix";
  return n.tier === "unknown" ? "mix" : n.tier;
}
function topNode(it: TmItem): TreemapNode | null {
  if (it.depth === 1) return nodeOf(it);
  const p = items.value.find((x) => x.key === it.parentKey);
  return p ? nodeOf(p) : null;
}

function onClick(it: TmItem) {
  const n = topNode(it);
  if (n && n.kind === "Dir") emit("select", n.path);
}
function onDbl(it: TmItem) {
  const n = nodeOf(it);
  if (n && n.can_open) emit("open", n.path);
  else {
    const t = topNode(it);
    if (t && t.can_open) emit("open", t.path);
  }
}

// ── 悬停卡片 ──
const hover = ref<TmItem | null>(null);
const tipPos = ref({ x: 0, y: 0 });
function onMove(e: MouseEvent, it: TmItem) {
  hover.value = it;
  const r = wrap.value!.getBoundingClientRect();
  let x = e.clientX - r.left + 14;
  let y = e.clientY - r.top + 16;
  if (x + 250 > r.width) x = e.clientX - r.left - 250;
  if (y + 110 > r.height) y = e.clientY - r.top - 110;
  tipPos.value = { x, y };
}
const tip = computed(() => {
  const it = hover.value;
  if (!it || it.container) return null;
  const n = nodeOf(it);
  return {
    name: it.name,
    size: sizeParts(it.value).join(" "),
    tone: tone(it),
    what: n?.what ?? (it.kind === "other" ? `${it.mergedCount} 个小项合计` : ""),
    lose: n && n.tier === "review" ? n.lose_kind : null,
    loseText: n && n.tier === "review" ? loseTag(n) : null,
  };
});

function label(it: TmItem): "full" | "size" | "none" {
  const w = it.rect.w - G;
  const h = it.rect.h - G;
  if (w > 62 && h > 40) return "full";
  if (w > 40 && h > 22) return "size";
  return "none";
}
function gb(v: number) {
  return (v / 1024 ** 3).toFixed(1);
}
</script>

<template>
  <div ref="wrap" class="map" @mouseleave="hover = null">
    <template v-for="it in items" :key="it.key">
      <div
        v-if="it.container"
        class="g"
        :class="{ sel: nodeOf(it)?.path === focusPath }"
        :style="{ left: it.rect.x + 'px', top: it.rect.y + 'px', width: it.rect.w - G + 'px', height: it.rect.h - G + 'px' }"
        role="button"
        tabindex="0"
        :aria-label="'方块 ' + it.name"
        @click="onClick(it)"
        @dblclick="onDbl(it)"
        @keydown.enter="onDbl(it)"
        @keydown.space.prevent="onClick(it)"
      >
        <div class="gh">{{ it.name }}<i class="num">{{ gb(it.value) }} GB</i></div>
      </div>
      <div
        v-else
        class="lf"
        :class="[tone(it), { big: it.rect.w > 150 && it.rect.h > 70, hov: hover?.key === it.key, sel: it.depth === 1 && nodeOf(it)?.path === focusPath }]"
        :style="{ left: it.rect.x + 'px', top: it.rect.y + 'px', width: it.rect.w - G + 'px', height: it.rect.h - G + 'px' }"
        role="button"
        tabindex="-1"
        :aria-label="'方块 ' + it.name"
        @click="onClick(it)"
        @dblclick="onDbl(it)"
        @mousemove="onMove($event, it)"
      >
        <template v-if="label(it) === 'full'">
          <span class="a">{{ it.name }}</span><span class="b num">{{ gb(it.value) }} GB</span>
        </template>
        <span v-else-if="label(it) === 'size'" class="b num">{{ gb(it.value) }}</span>
      </div>
    </template>
    <div v-if="tip" class="tip" :style="{ left: tipPos.x + 'px', top: tipPos.y + 'px' }">
      <div class="t1"><i class="dot" :class="tip.tone"></i>{{ tip.name }}<span class="s num">{{ tip.size }}</span></div>
      <div v-if="tip.what" class="t2">{{ tip.what }}</div>
      <div v-if="tip.loseText" class="t3"><span class="tag" :class="{ hard: isHardTag(tip.lose) }">{{ tip.loseText }}</span></div>
    </div>
  </div>
</template>

<style scoped>
.map {
  position: relative;
  width: 100%;
  height: 100%;
}
.g {
  position: absolute;
  background: var(--surface);
  border: 1px solid var(--line);
  cursor: pointer;
  transition: border-color var(--dur-1) var(--ease);
}
.g:hover {
  border-color: var(--line2);
}
.g .gh {
  position: absolute;
  left: 8px;
  right: 8px;
  top: 4px;
  height: 18px;
  display: flex;
  gap: 6px;
  font-size: 12px;
  color: var(--ink2);
  font-weight: var(--w-med);
  white-space: nowrap;
  overflow: hidden;
}
.g .gh i {
  font-style: normal;
  color: var(--ink3);
}
.g.sel {
  border: 2px solid var(--ink);
  z-index: 2;
}
[data-theme="A"] .g {
  border-radius: 4px;
}
[data-theme="B"] .g {
  background: #1a1612;
  border-color: #2a241e;
  border-radius: 6px;
}
[data-theme="B"] .g.sel {
  border-color: var(--accent);
}
[data-theme="C"] .g {
  border: 2px solid #141412;
  border-radius: 10px;
  background: #fff;
}
[data-theme="C"] .g.sel {
  border-width: 3px;
  box-shadow: 5px 5px 0 #141412;
}
[data-theme="C"] .g .gh {
  font-family: var(--heading);
  font-weight: 700;
  color: #141412;
  font-size: 13px;
}
.lf {
  position: absolute;
  overflow: hidden;
  padding: 7px 8px;
  font-size: 12px;
  line-height: 1.3;
  cursor: pointer;
  z-index: 3;
}
.lf .a {
  display: block;
  font-weight: var(--w-med);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.lf .b {
  display: block;
  opacity: 0.8;
}
.lf.big .a {
  font-size: 15px;
}
.lf.big .b {
  font-size: 13px;
}
.lf.safe {
  background: var(--safe);
  color: #fff;
}
.lf.review {
  background: var(--review);
  color: #fff;
}
.lf.keep {
  background: var(--keep);
  color: var(--ink);
}
.lf.mix {
  background: var(--keep2);
  color: var(--ink2);
}
[data-theme="A"] .lf {
  border-radius: 3px;
}
[data-theme="B"] .lf {
  border-radius: 4px;
}
[data-theme="B"] .lf.safe {
  background: #e4bc70;
  color: #241a0d;
}
[data-theme="B"] .lf.review {
  background: #d97d62;
  color: #21120c;
}
[data-theme="B"] .lf.keep {
  background: #40372f;
  color: var(--ink2);
}
[data-theme="B"] .lf.mix {
  background: #2e2823;
  color: var(--ink3);
}
[data-theme="C"] .lf {
  border: 2px solid #141412;
  border-radius: 8px;
  color: #141412;
}
[data-theme="C"] .lf.mix {
  background: repeating-linear-gradient(135deg, #f4f0e6 0 5px, #e6e1d5 5px 10px);
}
.lf.hov {
  outline: 2px solid var(--ink);
  outline-offset: -2px;
  z-index: 4;
}
[data-theme="B"] .lf.hov {
  outline-color: #f3ece2;
}
.lf.sel {
  outline: 2px solid var(--ink);
  outline-offset: -2px;
}

.tip {
  position: absolute;
  z-index: 10;
  width: 236px;
  padding: 12px 14px;
  background: var(--surface);
  border-radius: var(--r);
  box-shadow: 0 16px 40px -12px rgba(0, 0, 0, 0.35), 0 0 0 1px var(--line);
  font-size: var(--fs-sub);
  pointer-events: none;
}
[data-theme="B"] .tip {
  background: #221d18;
  box-shadow: 0 20px 50px -10px rgba(0, 0, 0, 0.8), 0 0 0 1px #3b332b;
}
[data-theme="C"] .tip {
  border: 2px solid #141412;
  box-shadow: 4px 4px 0 #141412;
  border-radius: 12px;
}
.tip .t1 {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: var(--w-semi);
  font-size: 14px;
  min-width: 0;
}
.tip .t1 .s {
  margin-left: auto;
  white-space: nowrap;
}
.tip .t2 {
  margin-top: 4px;
  color: var(--ink3);
  font-size: var(--fs-meta);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.tip .t3 {
  margin-top: 10px;
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
