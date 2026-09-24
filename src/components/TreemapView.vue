<script setup lang="ts">
// 方块图：Canvas 2D 画 squarified treemap（布局算法在 src/treemap.ts）。
// - 面积 = 实际占用；颜色 = 档位（与列表色标一致）；根层额外画“无权限 / 系统保留（推算）/ 未归属”斜纹块。
// - 阶段 E：大圆角色块 + 块间留缝；块内有图标、名称、大小，右下角百分比；选中有强调色描边；悬停是浮动卡片。
// - 悬停显示路径、大小、这是什么；单击选中（右侧详情联动）；双击进入；Backspace / Alt+← / 鼠标后退键上钻。
// - 键盘：Tab 聚焦后 ←→ 在根层块之间移动（同时显示提示），Enter 进入，空格选中，Backspace 上一级。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { tmKey, type PseudoKey, type ScanSummary, type TierKey, type TmSelect, type TreemapNode } from "../api";
import { TIER_SHORT, humanBytes, pct } from "../format";
import { DEFAULT_LAYOUT, hitTest, layoutTreemap, type TmInput, type TmItem } from "../treemap";
import { isDark, themePref } from "../theme";
import TierChip from "./ui/TierChip.vue";

const props = defineProps<{
  data: TreemapNode | null;
  summary: ScanSummary;
  isScanRoot: boolean;
  selectedKey: string | null;
  loading: boolean;
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

/** 方块图专用的布局参数：容器标题栏更高（放图标 + 名字 + 大小），内边距更大 */
const LAYOUT = { ...DEFAULT_LAYOUT, headerPx: 28, pad: 5, minContainerPx: 96 };

const PSEUDO_LABEL: Record<PseudoKey, string> = {
  inacc: "无权限（推算）",
  sysest: "系统保留（推算）",
  unattr: "未归属",
  cloud: "云端占位",
};

// lucide 图标的路径数据（ISC 许可），用 Path2D 画在 Canvas 上
const ICON_PATHS: Record<string, string[]> = {
  folder: ["M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"],
  file: [
    "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z",
    "M14 2v5a1 1 0 0 0 1 1h5",
  ],
  lock: ["M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z", "M7 11V7a5 5 0 0 1 10 0v4"],
  cloud: ["M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"],
  layers: [
    "M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z",
    "M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12",
    "M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17",
  ],
  shield: [
    "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z",
    "M12 8v4",
    "M12 16h.01",
  ],
};
const iconCache = new Map<string, Path2D[]>();
function iconPaths(name: string): Path2D[] {
  let p = iconCache.get(name);
  if (!p) {
    p = (ICON_PATHS[name] ?? []).map((d) => new Path2D(d));
    iconCache.set(name, p);
  }
  return p;
}

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
    // 扫描的只是一个文件夹时，“未归属”= 扫描范围之外的整个盘，不画进这个文件夹的方块图
    { key: "unattr", bytes: b.scope_is_volume_root ? b.unattributed : 0 },
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

function makeHatch(ctx: CanvasRenderingContext2D, bg: string, line: string, size = 10): CanvasPattern | null {
  const c = document.createElement("canvas");
  c.width = size;
  c.height = size;
  const g = c.getContext("2d");
  if (!g) return null;
  if (bg) {
    g.fillStyle = bg;
    g.fillRect(0, 0, size, size);
  }
  g.strokeStyle = line;
  g.lineWidth = 1.5;
  g.beginPath();
  g.moveTo(-2, size + 2);
  g.lineTo(size + 2, -2);
  g.moveTo(-2, 2);
  g.lineTo(2, -2);
  g.moveTo(size - 2, size + 2);
  g.lineTo(size + 2, size - 2);
  g.stroke();
  return ctx.createPattern(c, "repeat");
}

interface Palette {
  dark: boolean;
  tier: Record<TierKey, string>;
  other: string;
  pseudoBg: string;
  pseudoLine: string;
  text: string;
  accent: string;
  font: string;
  mono: string;
}

function palette(): Palette {
  return {
    dark: isDark(),
    tier: {
      safe: cssVar("--tm-safe", "#2c6650"),
      review: cssVar("--tm-review", "#715a2c"),
      keep: cssVar("--tm-keep", "#773840"),
      unknown: cssVar("--tm-unknown", "#3b3e4b"),
    },
    other: cssVar("--tm-other", "#2a2b32"),
    pseudoBg: cssVar("--tm-pseudo", "#22232a"),
    pseudoLine: cssVar("--tm-pseudo-line", "#363843"),
    text: cssVar("--tm-text", "#f4f4f7"),
    accent: cssVar("--accent", "#8b8cff"),
    font: cssVar("--font-sans", "'Microsoft YaHei UI', sans-serif"),
    mono: cssVar("--font-mono", "Consolas, monospace"),
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

function parseColor(c: string): [number, number, number] | null {
  const m = /^#([0-9a-f]{6})$/i.exec(c.trim());
  if (m) {
    const n = parseInt(m[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  return null;
}

/** 嵌套层次：深色主题往亮里调、浅色主题往深里调一点 */
function shade(hex: string, depth: number, dark: boolean): string {
  const rgb = parseColor(hex);
  if (!rgb || depth <= 1) return hex;
  const k = Math.min((dark ? 0.075 : 0.05) * (depth - 1), 0.22);
  const target = dark ? 255 : 0;
  const f = (c: number) => Math.round(c + (target - c) * k);
  return `rgb(${f(rgb[0])},${f(rgb[1])},${f(rgb[2])})`;
}

function fitText(ctx: CanvasRenderingContext2D, s: string, maxW: number): string {
  if (maxW <= 0) return "";
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

function rr(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  const rad = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, rad);
}

function drawIcon(ctx: CanvasRenderingContext2D, name: string, x: number, y: number, size: number, color: string, alpha = 1) {
  ctx.save();
  ctx.translate(x, y);
  ctx.scale(size / 24, size / 24);
  ctx.strokeStyle = color;
  ctx.globalAlpha = alpha;
  ctx.lineWidth = 1.9;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  for (const p of iconPaths(name)) ctx.stroke(p);
  ctx.restore();
}

function iconName(it: TmItem): string {
  if (pseudoOf(it)) return "shield";
  if (it.kind === "other") return "layers";
  const n = nodeOf(it);
  if (!n) return "folder";
  if (n.kind === "Rest") return "layers";
  if (n.inaccessible) return "lock";
  if (n.cloud_placeholder) return "cloud";
  return n.kind === "File" ? "file" : "folder";
}

/** 每层块之间留出的缝（向内收的像素） */
function insetOf(depth: number): number {
  return depth === 1 ? 2 : 1.5;
}
function radiusOf(depth: number): number {
  return depth === 1 ? 10 : depth === 2 ? 7 : 5;
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
  const hatch = makeHatch(ctx, pal.pseudoBg, pal.pseudoLine);
  const hatchSys = makeHatch(ctx, "", pal.dark ? "rgba(255,255,255,0.13)" : "rgba(0,0,0,0.09)", 8);
  const laid = layoutTreemap(root, { x: 0, y: 0, w, h }, LAYOUT);
  items.value = laid;
  topCount.value = laid.filter((i) => i.depth === 1).length;
  const hoverKey = hover.value?.key ?? null;

  ctx.textBaseline = "alphabetic";
  for (const it of laid) {
    const ins = insetOf(it.depth);
    const x = it.rect.x + ins;
    const y = it.rect.y + ins;
    const bw = it.rect.w - ins * 2;
    const bh = it.rect.h - ins * 2;
    if (bw < 2 || bh < 2) continue;
    const rad = radiusOf(it.depth);
    const pseudo = pseudoOf(it);
    const node = nodeOf(it);
    const tk = tierOf(it);

    let fill: string | CanvasPattern = pal.other;
    if (pseudo) fill = hatch ?? pal.pseudoBg;
    else if (it.kind === "node") fill = shade(pal.tier[tk], it.depth, pal.dark);

    rr(ctx, x, y, bw, bh, rad);
    ctx.fillStyle = fill;
    ctx.fill();
    if (node?.system_reserved && hatchSys) {
      ctx.fillStyle = hatchSys;
      ctx.fill();
    }
    // 细亮边：让色块有一点“卡片”质感
    ctx.strokeStyle = pal.dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.05)";
    ctx.lineWidth = 1;
    rr(ctx, x + 0.5, y + 0.5, bw - 1, bh - 1, rad);
    ctx.stroke();

    if (it.container) {
      // 容器：内部铺一层暗底，子块画在上面；标题栏保留档位色
      const top = it.rect.y + LAYOUT.headerPx - 1;
      rr(ctx, x + 2, top, bw - 4, it.rect.y + it.rect.h - ins - 2 - top, Math.max(3, rad - 3));
      ctx.fillStyle = pal.dark ? "rgba(0,0,0,0.26)" : "rgba(255,255,255,0.5)";
      ctx.fill();
    }
    if (hoverKey === it.key) {
      rr(ctx, x, y, bw, bh, rad);
      ctx.fillStyle = pal.dark ? "rgba(255,255,255,0.08)" : "rgba(255,255,255,0.35)";
      ctx.fill();
      ctx.strokeStyle = pal.dark ? "rgba(255,255,255,0.55)" : "rgba(0,0,0,0.35)";
      ctx.lineWidth = 1.5;
      rr(ctx, x + 0.75, y + 0.75, bw - 1.5, bh - 1.5, rad);
      ctx.stroke();
    }

    drawLabel(ctx, it, x, y, bw, bh, pal, !!pseudo);
  }

  // 选中：强调色描边 + 柔光
  const sel = laid.find((i) => i.key === props.selectedKey);
  if (sel) {
    const ins = insetOf(sel.depth);
    const x = sel.rect.x + ins;
    const y = sel.rect.y + ins;
    const bw = sel.rect.w - ins * 2;
    const bh = sel.rect.h - ins * 2;
    ctx.save();
    ctx.shadowColor = pal.accent;
    ctx.shadowBlur = 14;
    ctx.strokeStyle = pal.accent;
    ctx.lineWidth = 2.5;
    rr(ctx, x + 1.25, y + 1.25, bw - 2.5, bh - 2.5, radiusOf(sel.depth));
    ctx.stroke();
    ctx.restore();
  }
  renderMs.value = performance.now() - t0;
}

function drawLabel(ctx: CanvasRenderingContext2D, it: TmItem, x: number, y: number, bw: number, bh: number, pal: Palette, pseudo: boolean) {
  const color = pal.text;
  const icon = iconName(it);
  if (it.container) {
    // 标题栏：图标 + 名字 + 大小（一行）
    if (bw < 44) return;
    const pad = 9;
    const cy = y + 16;
    let tx = x + pad;
    if (bw > 90) {
      drawIcon(ctx, icon, tx, cy - 11, 14, color, 0.85);
      tx += 20;
    }
    ctx.fillStyle = color;
    ctx.font = `600 12.5px ${pal.font}`;
    const sizeText = humanBytes(it.value);
    ctx.font = `500 11.5px ${pal.font}`;
    const sw = ctx.measureText(sizeText).width;
    ctx.font = `600 12.5px ${pal.font}`;
    const room = x + bw - pad - tx;
    const showSize = room > sw + 50;
    const name = fitText(ctx, it.name, showSize ? room - sw - 10 : room);
    ctx.fillText(name, tx, cy);
    if (showSize) {
      ctx.font = `500 11.5px ${pal.font}`;
      ctx.globalAlpha = 0.72;
      ctx.fillText(sizeText, x + bw - pad - sw, cy);
      ctx.globalAlpha = 1;
    }
    return;
  }
  const big = bw > 220 && bh > 130;
  if (bw > 64 && bh > 44) {
    const pad = big ? 14 : 10;
    let cy = y + pad;
    const iconSize = big ? 20 : 15;
    drawIcon(ctx, icon, x + pad, cy, iconSize, color, pseudo ? 0.7 : 0.85);
    cy += iconSize + (big ? 10 : 7);
    const nameSize = big ? 15 : 12.5;
    ctx.fillStyle = color;
    ctx.font = `600 ${nameSize}px ${pal.font}`;
    if (cy + nameSize <= y + bh - 4) {
      ctx.fillText(fitText(ctx, it.name, bw - pad * 2), x + pad, cy + nameSize);
      cy += nameSize + (big ? 7 : 5);
    }
    const sz = big ? 13 : 11.5;
    if (cy + sz <= y + bh - 4) {
      ctx.font = `500 ${sz}px ${pal.font}`;
      ctx.globalAlpha = 0.75;
      ctx.fillText(fitText(ctx, humanBytes(it.value), bw - pad * 2), x + pad, cy + sz);
      ctx.globalAlpha = 1;
      cy += sz;
    }
    // 很大、还能进去的块：左下角淡淡提示“双击进入”
    const nd = nodeOf(it);
    if (big && nd?.can_open && bh > 170) {
      ctx.font = `500 11.5px ${pal.font}`;
      ctx.globalAlpha = 0.45;
      ctx.fillText("双击进入 →", x + pad, y + bh - pad);
      ctx.globalAlpha = 1;
    }
    // 右下角百分比（占当前视图）
    if (bw > 96 && bh > 84 && total.value > 0) {
      const p = pct(it.value, total.value);
      ctx.font = `600 ${big ? 13 : 11.5}px ${pal.font}`;
      const pw = ctx.measureText(p).width;
      if (y + bh - pad > cy + 14) {
        ctx.globalAlpha = 0.6;
        ctx.fillText(p, x + bw - pad - pw, y + bh - pad);
        ctx.globalAlpha = 1;
      }
    }
  } else if (bw > 40 && bh > 20) {
    ctx.fillStyle = color;
    ctx.font = `600 11px ${pal.font}`;
    ctx.fillText(fitText(ctx, it.name, bw - 12), x + 6, y + Math.min(bh - 6, 16));
  }
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
  // 网页字体加载完再画一次，Canvas 里的文字才会用上 Geist
  document.fonts?.ready.then(() => schedule());
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
  () => props.selectedKey,
  () => schedule(),
);
watch(hover, () => schedule());
watch(themePref, () => nextTick(schedule));
// 跟随系统时，系统主题变了也要重画
const mq = window.matchMedia("(prefers-color-scheme: light)");
onMounted(() => mq.addEventListener?.("change", schedule));
onBeforeUnmount(() => mq.removeEventListener?.("change", schedule));

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

/** Alt+← 也能上一级 */
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
      inacc: "当前权限读不了的文件夹里的东西，大小是用卷已用空间减去量到的部分推算的（含 NTFS 元数据）。",
      sysest: "系统保留目录里读不到的部分（卷影副本等），按差额推算。",
      unattr: "说不清归属的部分（主要是 NTFS 元数据）。",
      cloud: "只在云端的文件，本地不占空间。",
    };
    return { title: PSEUDO_LABEL[pseudo], path: "", size: humanBytes(it.value), share, chip: "unknown" as TierKey, chipText: "归属统计", what: notes[pseudo], owner: "", hint: "单击查看解释" };
  }
  if (it.kind === "other") {
    return {
      title: it.name,
      path: "",
      size: humanBytes(it.value),
      share,
      chip: "unknown" as TierKey,
      chipText: "合并",
      what: "这些项太小，画出来不到 6 像素，合并显示。切到列表可以逐个看。",
      owner: "",
      hint: "",
    };
  }
  const tk = (node?.tier ?? "unknown") as TierKey;
  return {
    title: it.name,
    path: node?.path ?? "",
    size: humanBytes(it.value),
    share,
    chip: tk,
    chipText: TIER_SHORT[tk],
    what: node?.what ?? "",
    owner: node?.owner ?? "",
    hint: node?.can_open ? "单击看详情 · 双击进入" : "单击看详情",
  };
});

const TIP_W = 300;
const tipStyle = computed(() => {
  const el = wrap.value;
  const w = el?.clientWidth ?? 800;
  const h = el?.clientHeight ?? 400;
  const tipH = 180;
  let left = tipPos.value.x + 18;
  if (left + TIP_W > w - 8) left = tipPos.value.x - TIP_W - 18;
  let top = tipPos.value.y + 18;
  if (top + tipH > h - 8) top = Math.max(8, tipPos.value.y - tipH - 12);
  return { left: Math.max(8, left) + "px", top: top + "px", width: TIP_W + "px" };
});

defineExpose({ redraw: schedule });
</script>

<template>
  <section class="tm card" aria-label="方块图">
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
      <Transition name="tip">
        <div v-if="tip" class="tm-tip" :style="tipStyle" role="tooltip">
          <div class="tip-head">
            <TierChip :tier="tip.chip" :label="tip.chipText" />
            <span class="tip-share num">{{ tip.share }}</span>
          </div>
          <div class="tip-title">{{ tip.title }}</div>
          <div v-if="tip.path" class="tip-path mono">{{ tip.path }}</div>
          <div class="tip-size num">{{ tip.size }}</div>
          <div v-if="tip.what" class="tip-what">{{ tip.what }}</div>
          <div v-if="tip.owner" class="tip-owner muted small">属于 {{ tip.owner }}</div>
          <div v-if="tip.hint" class="tip-hint">{{ tip.hint }}</div>
        </div>
      </Transition>
      <div v-if="loading && !data" class="tm-loading muted">正在准备方块图…</div>
      <div v-else-if="!data && !loading" class="tm-loading muted">没有数据</div>
    </div>
    <div class="tm-foot">
      <ul class="legend">
        <li><i class="dot dot-safe"></i>可清理</li>
        <li><i class="dot dot-review"></i>需判断</li>
        <li><i class="dot dot-keep"></i>保留</li>
        <li><i class="dot dot-unknown"></i>未知</li>
        <li v-if="pseudoBlocks.length"><i class="dot hatch"></i>无权限 / 推算（点一下看解释）</li>
      </ul>
      <span class="spacer"></span>
      <span class="stat num">{{ topCount }} 块 · {{ renderMs.toFixed(1) }} ms</span>
    </div>
    <ul class="sr-only" aria-label="方块图区块">
      <li v-for="(it, i) in topItems" :key="it.key">
        <button type="button" @click="srSelect(i)">方块 {{ it.name }} {{ humanBytes(it.value) }}</button>
        <button v-if="nodeOf(it)?.can_open" type="button" @click="srOpen(i)">进入 {{ it.name }}</button>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.tm {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 8px;
  overflow: hidden;
}
.tm-canvas-wrap {
  position: relative;
  flex: 1;
  min-height: 0;
  border-radius: var(--r-md);
  overflow: hidden;
}
canvas {
  display: block;
  outline: none;
  cursor: pointer;
}
canvas:focus-visible {
  box-shadow: inset 0 0 0 2px var(--accent-ring);
}
.tm-loading {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.tm-tip {
  position: absolute;
  z-index: 5;
  pointer-events: none;
  padding: 14px 16px 13px;
  border-radius: var(--r-lg);
  background: color-mix(in srgb, var(--surface-1) 92%, transparent);
  backdrop-filter: blur(12px);
  box-shadow: var(--shadow-pop);
  font-size: var(--fs-sm);
}
.tip-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.tip-share {
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.tip-title {
  margin-top: 9px;
  font-size: var(--fs-lg);
  font-weight: 650;
  letter-spacing: -0.01em;
  line-height: 1.3;
  word-break: break-all;
}
.tip-path {
  margin-top: 2px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  word-break: break-all;
}
.tip-size {
  margin-top: 8px;
  font-size: var(--fs-xl);
  font-weight: 600;
  letter-spacing: -0.02em;
}
.tip-what {
  margin-top: 6px;
  color: var(--text-1);
  line-height: 1.5;
}
.tip-owner {
  margin-top: 2px;
}
.tip-hint {
  margin-top: 10px;
  padding-top: 9px;
  border-top: 1px solid var(--border-1);
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.tip-enter-active,
.tip-leave-active {
  transition: opacity var(--dur-1) var(--ease);
}
.tip-enter-from,
.tip-leave-to {
  opacity: 0;
}
.tm-foot {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 8px 2px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.legend {
  display: flex;
  flex-wrap: wrap;
  gap: 14px;
  list-style: none;
  margin: 0;
  padding: 0;
}
.legend li {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-2);
}
.dot.hatch {
  background: repeating-linear-gradient(135deg, var(--text-3) 0 2px, transparent 2px 4px);
}
.stat {
  white-space: nowrap;
}
</style>
