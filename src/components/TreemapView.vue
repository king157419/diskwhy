<script setup lang="ts">
// 方块图：Canvas 2D 画 squarified treemap（布局算法在 src/treemap.ts）。
// - 面积 = 实际占用；颜色 = 档位（与列表色标一致）；根层额外画“无权限 / 系统保留（推算）/ 未归属”斜纹块。
// - 悬停显示路径、大小、这是什么；单击选中（右侧详情联动）；双击进入；返回键 / Backspace / “上一级”上钻。
// - 键盘：Tab 聚焦后 ←→ 在根层块之间移动（同时显示提示），Enter 进入，空格选中，Backspace 上一级。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { tmKey, type Crumb, type PseudoKey, type ScanSummary, type TierKey, type TmSelect, type TreemapNode } from "../api";
import { TIER_SHORT, humanBytes, pct } from "../format";
import { hitTest, layoutTreemap, type TmInput, type TmItem } from "../treemap";

const props = defineProps<{
  data: TreemapNode | null;
  summary: ScanSummary;
  isScanRoot: boolean;
  selectedKey: string | null;
  crumbs: Crumb[];
  loading: boolean;
  /** 只把“可清理”画成彩色，其余变淡 */
  highlightSafe?: boolean;
}>();
const emit = defineEmits<{ select: [s: TmSelect]; open: [path: string]; up: [] }>();

const wrap = ref<HTMLDivElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const items = ref<TmItem[]>([]);
const hover = ref<TmItem | null>(null);
const focusIdx = ref(-1);
const tipPos = ref({ x: 0, y: 0 });
const renderMs = ref(0);
const topCount = ref(0);
let ro: ResizeObserver | null = null;
let hatch: CanvasPattern | null = null;
let hatchLight: CanvasPattern | null = null;

const PSEUDO_LABEL: Record<PseudoKey, string> = {
  inacc: "无权限（推算）",
  sysest: "系统保留（推算）",
  unattr: "未归属",
  cloud: "云端占位",
};

function toInput(n: TreemapNode): TmInput {
  return {
    key: tmKey(n),
    name: n.name,
    value: n.allocated,
    children: n.children.map(toInput),
    hidden: n.hidden_bytes,
    hiddenCount: n.hidden_count,
    ref: n,
  };
}

const pseudoBlocks = computed(() => {
  if (!props.isScanRoot) return [] as { key: PseudoKey; bytes: number }[];
  const b = props.summary.buckets;
  const list: { key: PseudoKey; bytes: number }[] = [
    { key: "inacc", bytes: b.inaccessible },
    { key: "sysest", bytes: b.system_reserved_estimated },
    { key: "unattr", bytes: b.unattributed },
    { key: "cloud", bytes: b.cloud_placeholder },
  ];
  return list.filter((x) => x.bytes > 0);
});

const rootInput = computed<TmInput | null>(() => {
  const d = props.data;
  if (!d) return null;
  const r = toInput(d);
  for (const p of pseudoBlocks.value) {
    r.children!.push({ key: `pseudo:${p.key}`, name: PSEUDO_LABEL[p.key], value: p.bytes, ref: { pseudo: p.key } });
    r.value += p.bytes;
  }
  return r;
});

const total = computed(() => rootInput.value?.value ?? 0);

function cssVar(name: string, fallback: string): string {
  const el = wrap.value;
  if (!el) return fallback;
  const v = getComputedStyle(el).getPropertyValue(name).trim();
  return v || fallback;
}

function makeHatch(ctx: CanvasRenderingContext2D, bg: string, line: string): CanvasPattern | null {
  const c = document.createElement("canvas");
  c.width = 8;
  c.height = 8;
  const g = c.getContext("2d");
  if (!g) return null;
  if (bg) {
    g.fillStyle = bg;
    g.fillRect(0, 0, 8, 8);
  }
  g.strokeStyle = line;
  g.lineWidth = 2;
  g.beginPath();
  g.moveTo(-2, 10);
  g.lineTo(10, -2);
  g.moveTo(-2, 2);
  g.lineTo(2, -2);
  g.moveTo(6, 10);
  g.lineTo(10, 6);
  g.stroke();
  return ctx.createPattern(c, "repeat");
}

interface Palette {
  tier: Record<TierKey, string>;
  text: Record<TierKey, string>;
  other: string;
  otherText: string;
  pseudoBg: string;
  pseudoLine: string;
  border: string;
  accent: string;
  hover: string;
}

function palette(): Palette {
  return {
    tier: {
      safe: cssVar("--t-safe", "#1f9d55"),
      review: cssVar("--t-review", "#e0a100"),
      keep: cssVar("--t-keep", "#5a7394"),
      unknown: cssVar("--t-unknown", "#9ea2d8"),
      zero: cssVar("--tm-other", "#dde1e7"),
    },
    text: { safe: "#ffffff", review: "#2b1d00", keep: "#ffffff", unknown: "#23264f", zero: "#3b4252" },
    other: cssVar("--tm-other", "#c9ced8"),
    otherText: cssVar("--tm-other-text", "#3b4252"),
    pseudoBg: cssVar("--tm-pseudo", "#4a5160"),
    pseudoLine: cssVar("--tm-pseudo-line", "#6c7486"),
    border: cssVar("--card", "#ffffff"),
    accent: cssVar("--accent", "#2f6fed"),
    hover: cssVar("--text", "#1d2330"),
  };
}

function tierOf(it: TmItem): TierKey {
  const n = it.ref as TreemapNode | { pseudo: PseudoKey } | undefined;
  if (!n || "pseudo" in n) return "unknown";
  return (n as TreemapNode).tier;
}

function pseudoOf(it: TmItem): PseudoKey | null {
  const n = it.ref as { pseudo?: PseudoKey } | undefined;
  return n && n.pseudo ? n.pseudo : null;
}

function nodeOf(it: TmItem): TreemapNode | null {
  const n = it.ref as TreemapNode | { pseudo: PseudoKey } | undefined;
  if (!n || "pseudo" in n) return null;
  return n as TreemapNode;
}

/** 深层块稍微提亮，让嵌套层次看得出来 */
function shade(hex: string, depth: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m || depth <= 1) return hex;
  const n = parseInt(m[1], 16);
  const k = Math.min(0.12 * (depth - 1), 0.3);
  const f = (c: number) => Math.round(c + (255 - c) * k);
  return `rgb(${f((n >> 16) & 255)},${f((n >> 8) & 255)},${f(n & 255)})`;
}

function fitText(ctx: CanvasRenderingContext2D, s: string, maxW: number): string {
  if (ctx.measureText(s).width <= maxW) return s;
  let lo = 0;
  let hi = s.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (ctx.measureText(s.slice(0, mid) + "…").width <= maxW) lo = mid;
    else hi = mid - 1;
  }
  return lo > 0 ? s.slice(0, lo) + "…" : "";
}

function draw() {
  const cv = canvas.value;
  const el = wrap.value;
  const root = rootInput.value;
  if (!cv || !el) return;
  const t0 = performance.now();
  const w = el.clientWidth;
  const h = el.clientHeight;
  const dpr = window.devicePixelRatio || 1;
  if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) {
    cv.width = Math.round(w * dpr);
    cv.height = Math.round(h * dpr);
    cv.style.width = w + "px";
    cv.style.height = h + "px";
  }
  const ctx = cv.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  if (!root || w < 20 || h < 20) {
    items.value = [];
    return;
  }
  const pal = palette();
  // 无权限 / 推算的块：白底斜纹（不再用深灰，免得和“保留”“合并的小块”三种灰分不开）
  hatch = makeHatch(ctx, "#ffffff", pal.pseudoLine);
  hatchLight = makeHatch(ctx, "", "rgba(255,255,255,0.35)");
  const laid = layoutTreemap(root, { x: 0, y: 0, w, h });
  items.value = laid;
  topCount.value = laid.filter((i) => i.depth === 1).length;

  ctx.textBaseline = "top";
  for (const it of laid) {
    const r = it.rect;
    const pseudo = pseudoOf(it);
    const node = nodeOf(it);
    const tk = tierOf(it);
    let fill: string | CanvasPattern = pal.other;
    let textColor = pal.otherText;
    if (pseudo) {
      fill = hatch ?? pal.pseudoBg;
      textColor = "#3b4252";
    } else if (it.kind === "node") {
      fill = shade(pal.tier[tk], it.depth);
      textColor = pal.text[tk];
      if (props.highlightSafe && tk !== "safe") {
        fill = "#eceef1";
        textColor = "#8a909c";
      }
    }
    ctx.fillStyle = fill;
    ctx.fillRect(r.x, r.y, r.w, r.h);
    // 保留档加一层淡斜纹（“锁纹”），和其他颜色一眼分开
    if ((node?.system_reserved || (tk === "keep" && !props.highlightSafe && it.kind === "node")) && hatchLight) {
      ctx.fillStyle = hatchLight;
      ctx.fillRect(r.x, r.y, r.w, r.h);
    }
    if (it.container) {
      // 容器：内部铺一层淡色底，标题栏保留档位色
      ctx.fillStyle = "rgba(0,0,0,0.18)";
      ctx.fillRect(r.x + 2, r.y + 17, r.w - 4, r.h - 19);
    }
    ctx.strokeStyle = pal.border;
    ctx.lineWidth = 1;
    ctx.strokeRect(r.x + 0.5, r.y + 0.5, Math.max(0, r.w - 1), Math.max(0, r.h - 1));

    // 文字
    const pad = 4;
    if (it.container) {
      if (r.w > 40) {
        ctx.fillStyle = textColor;
        ctx.font = "600 11.5px 'Segoe UI', 'Microsoft YaHei UI', sans-serif";
        ctx.fillText(fitText(ctx, `${it.name}  ${humanBytes(it.value)}`, r.w - pad * 2), r.x + pad, r.y + 3);
      }
    } else if (r.w > 36 && r.h > 16) {
      ctx.fillStyle = textColor;
      ctx.font = "600 12px 'Segoe UI', 'Microsoft YaHei UI', sans-serif";
      ctx.fillText(fitText(ctx, it.name, r.w - pad * 2), r.x + pad, r.y + 3);
      if (r.h > 32) {
        ctx.font = "11px 'Segoe UI', 'Microsoft YaHei UI', sans-serif";
        ctx.globalAlpha = 0.9;
        ctx.fillText(fitText(ctx, humanBytes(it.value), r.w - pad * 2), r.x + pad, r.y + 18);
        ctx.globalAlpha = 1;
      }
    }
  }
  // 选中 / 悬停 / 键盘焦点描边
  const outline = (it: TmItem | null | undefined, color: string, lw: number) => {
    if (!it) return;
    const r = it.rect;
    ctx.strokeStyle = color;
    ctx.lineWidth = lw;
    ctx.strokeRect(r.x + lw / 2, r.y + lw / 2, Math.max(0, r.w - lw), Math.max(0, r.h - lw));
  };
  outline(laid.find((i) => i.key === props.selectedKey), pal.accent, 3);
  outline(hover.value ? laid.find((i) => i.key === hover.value!.key) : null, pal.hover, 2);
  renderMs.value = performance.now() - t0;
}

let raf = 0;
function schedule() {
  cancelAnimationFrame(raf);
  raf = requestAnimationFrame(draw);
}

onMounted(() => {
  ro = new ResizeObserver(() => schedule());
  if (wrap.value) ro.observe(wrap.value);
  window.addEventListener("keydown", onGlobalKey);
  schedule();
});
onBeforeUnmount(() => {
  ro?.disconnect();
  window.removeEventListener("keydown", onGlobalKey);
  cancelAnimationFrame(raf);
});
// 换了目录 / 换了扫描结果：清掉悬停和键盘焦点；只是换了选中项：保留（键盘选中时提示要继续显示）
watch(
  () => [props.data, props.summary],
  () => {
    hover.value = null;
    focusIdx.value = -1;
    schedule();
  },
);
watch(
  () => [props.selectedKey, props.highlightSafe],
  () => schedule(),
);
watch(hover, () => schedule());

function localXY(e: MouseEvent) {
  const r = canvas.value!.getBoundingClientRect();
  return { x: e.clientX - r.left, y: e.clientY - r.top };
}

function onMove(e: MouseEvent) {
  const { x, y } = localXY(e);
  const it = hitTest(items.value, x, y);
  if (it?.key !== hover.value?.key) hover.value = it;
  tipPos.value = { x, y };
}

function onLeave() {
  hover.value = null;
}

function emitSelect(it: TmItem) {
  const node = nodeOf(it);
  const pseudo = pseudoOf(it);
  const parent = it.parentKey ? items.value.find((i) => i.key === it.parentKey) : null;
  const parentNode = parent ? nodeOf(parent) : props.data;
  emit("select", { key: it.key, node, pseudo, parentPath: parentNode?.path ?? null });
}

function onClick(e: MouseEvent) {
  const { x, y } = localXY(e);
  const it = hitTest(items.value, x, y);
  if (it) {
    emitSelect(it);
    canvas.value?.focus();
  }
}

function onDbl(e: MouseEvent) {
  const { x, y } = localXY(e);
  const it = hitTest(items.value, x, y);
  if (!it) return;
  // 双击深层小块时，进入它所在的最近一个能进的目录
  let cur: TmItem | null | undefined = it;
  while (cur) {
    const n = nodeOf(cur);
    if (n?.can_open) {
      emit("open", n.path);
      return;
    }
    cur = cur.parentKey ? items.value.find((i) => i.key === cur!.parentKey) : null;
  }
  emitSelect(it);
}

const topItems = computed(() => items.value.filter((i) => i.depth === 1));

function focusTop(i: number) {
  const list = topItems.value;
  if (!list.length) return;
  focusIdx.value = ((i % list.length) + list.length) % list.length;
  const it = list[focusIdx.value];
  hover.value = it;
  tipPos.value = { x: it.rect.x + Math.min(it.rect.w / 2, 160), y: it.rect.y + Math.min(it.rect.h / 2, 60) };
}

function onKey(e: KeyboardEvent) {
  const list = topItems.value;
  if (e.key === "ArrowRight" || e.key === "ArrowDown") {
    focusTop(focusIdx.value + 1);
    e.preventDefault();
  } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
    focusTop(focusIdx.value <= 0 ? list.length - 1 : focusIdx.value - 1);
    e.preventDefault();
  } else if (e.key === "Enter") {
    const it = list[focusIdx.value];
    const n = it ? nodeOf(it) : null;
    if (n?.can_open) emit("open", n.path);
    else if (it) emitSelect(it);
    e.preventDefault();
  } else if (e.key === " ") {
    const it = list[focusIdx.value];
    if (it) emitSelect(it);
    e.preventDefault();
  } else if (e.key === "Backspace") {
    emit("up");
    e.preventDefault();
  }
}

/** 鼠标“后退”键（XButton1）/ Alt+← 也能上一级 */
function onGlobalKey(e: KeyboardEvent) {
  if (e.altKey && e.key === "ArrowLeft") {
    emit("up");
    e.preventDefault();
  }
}
function onMouseUp(e: MouseEvent) {
  if (e.button === 3) {
    emit("up");
    e.preventDefault();
  }
}

// 供读屏 / 自动化用的隐藏按钮：选中某块、进入某块
function srSelect(i: number) {
  focusTop(i);
  const it = topItems.value[i];
  if (it) emitSelect(it);
  nextTick(() => canvas.value?.focus());
}
function srOpen(i: number) {
  const n = nodeOf(topItems.value[i]);
  if (n?.can_open) emit("open", n.path);
}

const tip = computed(() => {
  const it = hover.value;
  if (!it) return null;
  const node = nodeOf(it);
  const pseudo = pseudoOf(it);
  const share = pct(it.value, total.value);
  if (pseudo) {
    const notes: Record<PseudoKey, string> = {
      inacc: "当前权限读不了的文件夹里的东西，大小是用卷已用空间减去量到的部分推算的（含 NTFS 元数据）。点一下看说明。",
      sysest: "系统保留目录里读不到的部分（卷影副本等），按差额推算。",
      unattr: "说不清归属的部分（主要是 NTFS 元数据）。",
      cloud: "只在云端的文件，本地不占空间。",
    };
    return { title: PSEUDO_LABEL[pseudo], path: "", size: humanBytes(it.value), share, chip: "unknown" as TierKey, chipText: "归属", what: notes[pseudo], hint: "单击查看解释" };
  }
  if (it.kind === "other") {
    return {
      title: it.name,
      path: "",
      size: humanBytes(it.value),
      share,
      chip: "unknown" as TierKey,
      chipText: "合并",
      what: "这些项太小，画出来不到 6 像素，合并显示。进入上一层文件夹的列表可以逐个看。",
      hint: "",
    };
  }
  return {
    title: it.name,
    path: node?.path ?? "",
    size: humanBytes(it.value),
    share,
    chip: (node?.tier ?? "unknown") as TierKey,
    chipText: TIER_SHORT[(node?.tier ?? "unknown") as TierKey],
    what: [node?.what, node?.owner ? `属于：${node.owner}` : ""].filter(Boolean).join(" · "),
    hint: node?.can_open ? "单击看详情 · 双击进入" : "单击看详情",
  };
});

const tipStyle = computed(() => {
  const el = wrap.value;
  const w = el?.clientWidth ?? 800;
  const h = el?.clientHeight ?? 400;
  const x = tipPos.value.x + 16;
  const y = tipPos.value.y + 16;
  // 提示框约 170px 高：靠近底部时往上放，始终留在方块图区域里
  const tipH = 170;
  const top = y + tipH > h ? Math.max(4, Math.min(tipPos.value.y - tipH - 8, h - tipH)) : y;
  return {
    left: Math.max(4, Math.min(x, w - 330)) + "px",
    top: top + "px",
  };
});

defineExpose({ redraw: schedule });
</script>

<template>
  <section class="tm card" aria-label="方块图">
    <div class="tm-head">
      <button class="btn tm-up" :disabled="crumbs.length <= 1" title="上一级（Backspace / Alt+← / 鼠标后退键）" @click="emit('up')">
        ↑ 上一级
      </button>
      <nav class="crumbs tm-crumbs" aria-label="方块图路径">
        <template v-for="(c, i) in crumbs" :key="c.path">
          <span v-if="i > 0" class="sep">›</span>
          <button class="crumb" :class="{ current: i === crumbs.length - 1 }" @click="emit('open', c.path)">{{ c.name }}</button>
        </template>
      </nav>
      <ul class="tm-legend small">
        <li><i class="dot dot-safe"></i>可清理</li>
        <li><i class="dot dot-review"></i>需判断</li>
        <li><i class="dot dot-keep"></i>保留</li>
        <li><i class="dot dot-unknown"></i>未认出</li>
        <li><i class="dot dot-pseudo"></i>读不了（斜纹）</li>
      </ul>
    </div>
    <div ref="wrap" class="tm-canvas-wrap" @mouseleave="onLeave">
      <canvas
        ref="canvas"
        tabindex="0"
        aria-label="方块图：面积表示实际占用，颜色表示能不能清理。方向键切换、回车进入、退格返回上一级"
        @mousemove="onMove"
        @click="onClick"
        @dblclick="onDbl"
        @keydown="onKey"
        @mouseup="onMouseUp"
      ></canvas>
      <div v-if="tip" class="tm-tip" :style="tipStyle" role="tooltip">
        <div class="tm-tip-title">
          <span class="chip" :class="'chip-' + tip.chip">{{ tip.chipText }}</span>
          <b>{{ tip.title }}</b>
        </div>
        <div v-if="tip.path" class="tm-tip-path">{{ tip.path }}</div>
        <div class="tm-tip-size"><b>{{ tip.size }}</b> <span class="muted">· 占当前视图 {{ tip.share }}</span></div>
        <div v-if="tip.what" class="tm-tip-what">{{ tip.what }}</div>
        <div v-if="tip.hint" class="muted small">{{ tip.hint }}</div>
      </div>
      <div v-if="loading" class="tm-loading muted">正在准备方块图…</div>
      <div v-else-if="!data" class="tm-loading muted">没有数据</div>
    </div>
    <div class="tm-foot muted small">
      <span>{{ topCount }} 个根层块 · 布局+绘制 {{ renderMs.toFixed(1) }} ms</span>
      <span v-if="pseudoBlocks.length">斜纹块 = 归属统计里量不到的部分（无权限 / 推算），点一下看解释</span>
      <span v-if="summary.buckets.dedup_enabled && summary.buckets.hardlink_dedup + summary.buckets.mapped_dedup > 0">
        已去重 {{ humanBytes(summary.buckets.hardlink_dedup + summary.buckets.mapped_dedup) }}（硬链接 / 商店应用目录映射，只算一次）
      </span>
    </div>
    <ul class="sr-only" aria-label="方块图区块">
      <li v-for="(it, i) in topItems" :key="it.key">
        <button type="button" @click="srSelect(i)">方块 {{ it.name }} {{ humanBytes(it.value) }}</button>
        <button v-if="nodeOf(it)?.can_open" type="button" @click="srOpen(i)">进入 {{ it.name }}</button>
      </li>
    </ul>
  </section>
</template>
