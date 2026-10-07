<script setup lang="ts">
// 方块图（阶段 F 改成 DOM 绘制，好让三套主题的描边 / 圆角 / 斜条纹全部由 CSS token 决定）。
// 阶段 I：每块的圆角 / 描边 / 投影按块大小收敛（treemap.ts 的 tileBox），间隙等数值来自 tokens.css 的 --tm-*。
// 布局仍是 src/treemap.ts 的 squarified + 嵌套：两层（当前文件夹的子项 → 它们的子项），颜色 = 三档，
// 认不出来 / 合并的“其他”画成混合色。单击选中一个顶层块（右边列表跟着变），双击进入。
// 阶段 J：块的配色类（.tm-tone.safe / review / keep / mix / na）不加 scoped，分析页的「图例」色块直接复用；
// 扫不到的伪块（无权限 / 系统保留 / 未归属）单独一种 na；“外壳”合并块（Users \ king）单击选中的是外层 Users。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { tmKey, type ScanSummary, type TreemapNode } from "../api";
import { isHardTag, loseTag } from "../labels";
import { collapseChain, layoutTreemap, samePath, tileBox, type TileBox, type TileTokens, type TmInput, type TmItem } from "../treemap";
import { theme } from "../theme";
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
  readTokens();
  ro = new ResizeObserver(upd);
  ro.observe(wrap.value);
});
onBeforeUnmount(() => ro?.disconnect());

// ── 设计令牌（tokens.css 的 --tm-*）：间隙、圆角上限、描边、投影都由主题决定，切主题时重读 ──
const tk = ref({ gap: 3, r: 3, bw: 0, shadow: 0, gR: 4, gBw: 1, head: 26 });
function readTokens() {
  if (!wrap.value) return;
  const cs = getComputedStyle(wrap.value);
  const px = (name: string, d: number) => {
    const v = parseFloat(cs.getPropertyValue(name));
    return Number.isFinite(v) ? v : d;
  };
  tk.value = {
    gap: px("--tm-gap", 3),
    r: px("--tm-r", 3),
    bw: px("--tm-bw", 0),
    shadow: px("--tm-shadow", 0),
    gR: px("--tm-g-r", 4),
    gBw: px("--tm-g-bw", 1),
    head: px("--tm-head", 26),
  };
}
watch(theme, () => nextTick(readTokens));
/** 小于 16px 画平块；小于 48px 不画投影；小于 24px 斜条纹改纯色 */
const FLAT_PX = 16;
const SHADOW_MIN_PX = 48;
const PATTERN_MIN_PX = 24;

const PSEUDO: Record<string, string> = { inacc: "无权限", sysest: "系统保留", unattr: "未归属", cloud: "云端占位" };
const PSEUDO_TIP: Record<string, string> = {
  inacc: "没有权限读，以管理员身份重启能看到",
  sysest: "Windows 自己占着的，扫不到",
  unattr: "盘上用了、但扫不到在哪",
};

/** 合并块的 key → 外层文件夹的路径（单击选中外层，比如 Users，而不是里面的 king） */
let outerOf = new Map<string, string>();

function toInput(n: TreemapNode): TmInput {
  // 几乎只装着一个文件夹的“外壳”（Users → king）合成一块，名字写成 “Users \ king”，里面直接铺内层的子项
  const { node, name } = collapseChain(n);
  if (node !== n) outerOf.set(tmKey(node), n.path);
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
  outerOf = new Map();
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

const items = computed<TmItem[]>(() => {
  const r = root.value;
  const { w, h } = size.value;
  const t = tk.value;
  const G = t.gap;
  if (!r || w < 20 || h < 20) return [];
  // 整张图多铺一个间隙，最右 / 最下的块扣掉间隙后正好贴边。
  // 容器内边距：左 = 描边 + 间隙；右 / 下 = 间隙 + 投影 + 描边（容器的块面已经扣掉了间隙和投影），子块四周的空隙都 = 间隙。
  return layoutTreemap(r, { x: 0, y: 0, w: w + G, h: h + G }, {
    minPx: 8,
    maxDepth: 2,
    headerPx: t.head,
    pad: t.gBw + G,
    padEnd: G + t.shadow + t.gBw,
    minContainerPx: 90,
  });
});

const tiles = computed<{ it: TmItem; b: TileBox }[]>(() => {
  const t = tk.value;
  const leaf: TileTokens = { gap: t.gap, r: t.r, bw: t.bw, shadow: t.shadow, flatPx: FLAT_PX, shadowMinPx: SHADOW_MIN_PX, patternMinPx: PATTERN_MIN_PX };
  const group: TileTokens = { ...leaf, r: t.gR, bw: t.gBw, flatPx: 0 };
  // 投影只给顶层块（容器 / 顶层大块），里面的小块只有描边
  return items.value.map((it) => ({ it, b: tileBox(it.rect, it.container ? group : leaf, it.depth === 1) }));
});

function boxStyle(b: TileBox, group: boolean) {
  const s: Record<string, string> = {
    left: b.x + "px",
    top: b.y + "px",
    width: b.w + "px",
    height: b.h + "px",
    borderRadius: b.r + "px",
    boxShadow: b.shadow ? `${b.shadow}px ${b.shadow}px 0 var(--stroke, var(--ink))` : "none",
  };
  if (!group) s.borderWidth = b.bw + "px";
  return s;
}

function nodeOf(it: TmItem): TreemapNode | null {
  const x = it.ref as TreemapNode | { pseudo: string } | undefined;
  return x && "path" in x ? x : null;
}
function tone(it: TmItem): string {
  if (it.kind === "other") return "mix";
  const n = nodeOf(it);
  if (!n) return "na";
  if (n.kind === "Rest") return "mix";
  return n.tier === "unknown" ? "mix" : n.tier;
}
function topItem(it: TmItem): TmItem | null {
  if (it.depth === 1) return it;
  return items.value.find((x) => x.key === it.parentKey) ?? null;
}
function topNode(it: TmItem): TreemapNode | null {
  const t = topItem(it);
  return t ? nodeOf(t) : null;
}
/** 单击一个顶层块时选中的路径：合并块选外层（Users），其余就是它自己 */
function selPath(it: TmItem): string | null {
  const n = nodeOf(it);
  if (!n) return null;
  return outerOf.get(it.key) ?? n.path;
}
function isSel(it: TmItem): boolean {
  return it.depth === 1 && samePath(selPath(it), props.focusPath);
}

function onClick(it: TmItem) {
  const t = topItem(it);
  const n = t ? nodeOf(t) : null;
  if (t && n && n.kind === "Dir") emit("select", selPath(t)!);
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
  const tn = tone(it);
  const pseudo = (it.ref as { pseudo?: string } | undefined)?.pseudo;
  let what = n?.what ?? (it.kind === "other" ? `${it.mergedCount} 个小项合计` : "");
  if (!what && pseudo) what = PSEUDO_TIP[pseudo] ?? "";
  // 认不出来的块没有说明时的兜底（阶段 J §3.9）
  if (!what && tn === "mix") what = "认不出来，点开看线索";
  return {
    name: it.name,
    size: sizeParts(it.value).join(" "),
    tone: tn,
    what,
    lose: n && n.tier === "review" ? n.lose_kind : null,
    loseText: n && n.tier === "review" ? loseTag(n) : null,
  };
});

function label(b: TileBox): "full" | "size" | "none" {
  if (b.flat) return "none";
  const { w, h } = b;
  if (w > 62 && h > 40) return "full";
  // 内边距 7 + 描边 2 + 一行字约 16px：低于 30px 的块写数字会被截掉半行，干脆不写
  if (w > 40 && h >= 30) return "size";
  return "none";
}
function gb(v: number) {
  return (v / 1024 ** 3).toFixed(1);
}
</script>

<template>
  <div ref="wrap" class="map" @mouseleave="hover = null">
    <template v-for="{ it, b } in tiles" :key="it.key">
      <div
        v-if="it.container"
        class="g"
        :class="{ sel: isSel(it) }"
        :style="boxStyle(b, true)"
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
        class="lf tm-tone"
        :class="[
          tone(it),
          {
            big: b.w > 150 && b.h > 70,
            flat: b.flat,
            bare: label(b) === 'none',
            solid: !b.pattern,
            hov: hover?.key === it.key,
            sel: isSel(it),
          },
        ]"
        :style="boxStyle(b, false)"
        role="button"
        tabindex="-1"
        :aria-label="'方块 ' + it.name"
        @click="onClick(it)"
        @dblclick="onDbl(it)"
        @mousemove="onMove($event, it)"
      >
        <template v-if="label(b) === 'full'">
          <span class="a">{{ it.name }}</span><span class="b num">{{ gb(it.value) }} GB</span>
        </template>
        <span v-else-if="label(b) === 'size'" class="b num">{{ gb(it.value) }}</span>
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
  border: var(--tm-g-bw) solid var(--line);
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
[data-theme="B"] .g {
  background: #1a1612;
  border-color: #2a241e;
}
[data-theme="B"] .g.sel {
  border-color: var(--accent);
}
[data-theme="C"] .g {
  border-color: #141412;
  background: #fff;
}
/* 卡通：顶层块本来就带硬投影（画在格子里面），选中 = 加粗描边 + 标题栏涂亮色 */
[data-theme="C"] .g.sel {
  border-width: 3px;
  background: linear-gradient(var(--accent) 0 calc(var(--tm-head) - 3px), #fff 0);
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
/* 没有文字的块不留内边距；平块（最短边 < 16px）连描边也没有。
   不然 border-box 下 padding + 描边会把小块撑成至少 20×18，配上圆角就成了圆 / 药丸，还压到邻块 */
.lf.bare {
  padding: 0;
}
.lf.flat {
  border-width: 0 !important;
}
.lf.big .a {
  font-size: 15px;
}
.lf.big .b {
  font-size: 13px;
}
[data-theme="C"] .lf {
  border-style: solid;
  border-color: #141412;
  color: #141412;
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

<!-- 方块的配色（不加 scoped：分析页「图例」的色块直接用同一组类，C 主题的斜条纹就是真斜条纹） -->
<style>
.tm-tone.safe {
  background: var(--safe);
  color: #fff;
}
.tm-tone.review {
  background: var(--review);
  color: #fff;
}
.tm-tone.keep {
  background: var(--keep);
  color: var(--ink);
}
.tm-tone.mix {
  background: var(--keep2);
  color: var(--ink2);
}
/* 扫不到（无权限 / 系统保留 / 未归属）：细网格 */
.tm-tone.na {
  color: var(--ink2);
  background:
    repeating-linear-gradient(45deg, transparent 0 5px, rgba(0, 0, 0, 0.07) 5px 6px),
    repeating-linear-gradient(-45deg, transparent 0 5px, rgba(0, 0, 0, 0.07) 5px 6px),
    var(--keep2);
}
[data-theme="B"] .tm-tone.safe {
  background: #e4bc70;
  color: #241a0d;
}
[data-theme="B"] .tm-tone.review {
  background: #d97d62;
  color: #21120c;
}
[data-theme="B"] .tm-tone.keep {
  background: #40372f;
  color: var(--ink2);
}
[data-theme="B"] .tm-tone.mix {
  background: #2e2823;
  color: var(--ink3);
}
/* 「认不出来」和「先别动」在 A / B 里只差一点明度，看不出来：和 C 一样给细斜条纹（只给 ≥ 24px 的块和图例色块） */
[data-theme="A"] .tm-tone.mix:not(.solid) {
  background: repeating-linear-gradient(135deg, #e2ddd3 0 5px, #cdc6b9 5px 7px);
}
[data-theme="B"] .tm-tone.mix:not(.solid) {
  background: repeating-linear-gradient(135deg, #2a241f 0 5px, #4a4037 5px 7px);
}
[data-theme="B"] .tm-tone.na {
  color: var(--ink3);
  background:
    repeating-linear-gradient(45deg, transparent 0 5px, rgba(255, 255, 255, 0.06) 5px 6px),
    repeating-linear-gradient(-45deg, transparent 0 5px, rgba(255, 255, 255, 0.06) 5px 6px),
    #221d18;
}
[data-theme="C"] .tm-tone {
  color: #141412;
}
/* 斜条纹只给 ≥ 24px 的块，小块用纯浅色，免得一片噪点 */
[data-theme="C"] .tm-tone.mix:not(.solid) {
  background: repeating-linear-gradient(135deg, #f4f0e6 0 5px, #e6e1d5 5px 10px);
}
[data-theme="C"] .tm-tone.na:not(.solid) {
  background:
    radial-gradient(#141412 1px, transparent 1.3px) 0 0 / 8px 8px,
    #fff;
}
[data-theme="C"] .tm-tone.na.solid {
  background: #fff;
}
</style>
