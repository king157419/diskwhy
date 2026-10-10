<script setup lang="ts">
// 方块图（阶段 F 改成 DOM 绘制，好让三套主题的描边 / 圆角 / 斜条纹全部由 CSS token 决定）。
// 阶段 I：每块的圆角 / 描边 / 投影按块大小收敛（treemap.ts 的 tileBox），间隙等数值来自 tokens.css 的 --tm-*。
// 阶段 J：块的配色类（.tm-tone.safe / review / keep / mix / na）不加 scoped，分析页的「图例」色块直接复用；
// 扫不到的伪块（无权限 / 系统保留 / 未归属）单独一种 na。
// 阶段 K：一张平面图。单击任意深度的文件夹 = 往里挖（展开链 = 根 → 它），它在原位置放大到父块的 60%
// 并显示更深的层级；单击文件 / 其他 / 伪块 = 选中；单击已展开容器的标题栏 = 收起到它的父级。
// - 每个容器的行 / 列结构固定（treemap.ts），块只沿原来的行 / 列伸缩：过渡里没有块横穿别的块；伸缩后太细的行并成「其他」；
// - pk 的内容区不够大（按像素）就把图的根下移一层：镜头推近（每块从旧位置按同一个比例映射过去），反方向是拉远；
//   缩放后图的根画成铺满整张图的容器，顶上一条和链上祖先同样式的标题栏（单击 = 收起到这一级）；
// - 怎么动由 tmmotion.ts 算：离场的块先跟着祖先（或镜头）移动并淡出，压在它要缩进去的那一块上面，约 230 ms 后才删；
// - 图自己变大变小（切视图 / 窗口）不逐块过渡：整张图淡出 → 新布局直接到位 → 淡入；
// - 块上的字：按实际量字决定写法（tmtree.ts fitLabel），过渡开始时字先淡出、过半后用最终的写法淡入；
// - 块按 key 稳定排序、用 z-index 分层（DOM 顺序不变 → 不会被挪动 → CSS 过渡真的生效，块在原地长大）。
// 外观（阶段 K 第 2 / 3 轮）：链上的状态只用中性色（墨黑 / 灰），绝不用三档颜色；悬停 / 选中 / 正在看的文件夹三种状态一眼分得清；
// 键盘：整张图只有一块在 Tab 序列里，方向键按画面位置走，Enter / Space = 单击；链上容器的标题栏 = 「收起」按钮。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { ScanSummary, TreemapNode } from "../api";
import { isHardTag, loseTag } from "../labels";
import { bigEnough, contentBox, pickMapRoot, samePath, tileBox, type HiddenSlot, type LayoutOptions, type Rect, type TileBox, type TileTokens, type TmInput, type TmItem } from "../treemap";
import {
  countLabel,
  fitHead,
  fitLabel,
  headHit,
  hitOf,
  layoutMap,
  navStep,
  navTargets,
  otherSig,
  pathKey,
  rootCandidates,
  tileNode,
  tipName,
  type LabelFit,
  type LabelMetrics,
  type MapHit,
  type NavDir,
} from "../tmtree";
import { planMotion, tileZ, type Box, type MotionTile } from "../tmmotion";
import { theme } from "../theme";
import { sizeParts } from "../capacity";

const props = defineProps<{
  /** 方块图数据：扫描根（拼好的显示树） */
  data: TreemapNode | null;
  summary: ScanSummary;
  /** 展开链 [p1 … pk] */
  chain: string[];
  /** 选中的文件 / 其他 / 伪块（块的 key） */
  selKey: string | null;
  /** 选中的是「其他」时它的内容签名：对不上（被拆开了）就不算选中 */
  selSig?: string | null;
  /** 正在取更深的数据 */
  loading?: boolean;
  /** 没有列表（整宽「方块图」视图）：选中一块时把说明卡片钉在它旁边 */
  pin?: boolean;
  /** 认不出来的文件夹 / 文件的第一条线索（pathKey → 线索；来自列表数据） */
  clues?: ReadonlyMap<string, string>;
}>();
const emit = defineEmits<{ hit: [h: MapHit, ev: { at: number; detail: number }]; unsel: []; peek: [path: string] }>();

const wrap = ref<HTMLDivElement | null>(null);
const probe = ref<HTMLDivElement | null>(null);
const size = ref({ w: 0, h: 0 });
/** 这一次布局直接到位、不做过渡（拖窗口、图自己变大变小） */
const still = ref(false);
let stillTimer: ReturnType<typeof setTimeout> | undefined;
let ghostTimer: ReturnType<typeof setTimeout> | undefined;
let txTimer: ReturnType<typeof setTimeout> | undefined;
let peekTimer: ReturnType<typeof setTimeout> | undefined;
let ro: ResizeObserver | null = null;
/** 上一次窗口大小变化的时间：拖窗口时块直接跟着变（连淡入淡出都不做） */
let winResizedAt = -1e9;
const onWinResize = () => (winResizedAt = performance.now());
/** 系统设置了「减少动画」：不做过渡，离场的块直接删 */
const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
/**
 * 图自己变大变小（F1）：新尺寸**在框变化的同一帧**直接到位（ResizeObserver 的回调在这一帧绘制之前执行，Vue 的更新也在绘制之前做完），
 * 不逐块过渡（宽高比一变整棵树重排，逐块过渡就是乱飞），也不会有一帧旧布局被裁在新框里 / 半边空着；然后整张图轻轻淡入一次
 */
let fadeAnim: Animation | null = null;
const FADE_IN_MS = 140;
/** 新布局第一帧的不透明度（不从全空开始：没有空白帧） */
const FADE_LOW = 0.4;
function commitSize(s: { w: number; h: number }) {
  still.value = true;
  size.value = s;
  clearTimeout(stillTimer);
  stillTimer = setTimeout(() => (still.value = false), 160);
}
function onResized() {
  const el = wrap.value;
  if (!el) return;
  const w = el.clientWidth;
  const h = el.clientHeight;
  if (w === size.value.w && h === size.value.h) return;
  const first = !size.value.w;
  commitSize({ w, h });
  // 第一次 / 减少动画 / 正在拖窗口：直接到位，不淡入
  if (first || reduced || performance.now() - winResizedAt < 400 || typeof el.animate !== "function") return;
  fadeAnim?.cancel();
  fadeAnim = el.animate([{ opacity: FADE_LOW }, { opacity: 1 }], { duration: FADE_IN_MS, easing: "ease-out" });
}
onMounted(() => {
  if (!wrap.value) return;
  onResized();
  readTokens();
  ro = new ResizeObserver(onResized);
  ro.observe(wrap.value);
  window.addEventListener("resize", onWinResize);
  document.addEventListener("mousedown", onDocDown, true);
  // 字体加载完以后重新量一次字
  document.fonts?.ready.then(() => {
    measured.clear();
    labelCache.clear();
    fontVer.value++;
  });
});
onBeforeUnmount(() => {
  ro?.disconnect();
  clearTimeout(stillTimer);
  clearTimeout(ghostTimer);
  clearTimeout(txTimer);
  clearTimeout(peekTimer);
  fadeAnim?.cancel();
  window.removeEventListener("resize", onWinResize);
  document.removeEventListener("mousedown", onDocDown, true);
});

// ── 设计令牌（tokens.css 的 --tm-*）：间隙、圆角上限、描边、投影、最小块都由主题决定，切主题时重读 ──
const tk = ref({ gap: 3, r: 3, bw: 0, shadow: 0, gR: 4, gBw: 1, head: 26, min: 12, min2: 12, heading: "sans-serif", ui: "sans-serif" });
function readTokens() {
  if (!wrap.value) return;
  const cs = getComputedStyle(wrap.value);
  const px = (name: string, d: number) => {
    const v = parseFloat(cs.getPropertyValue(name));
    return Number.isFinite(v) ? v : d;
  };
  const min = px("--tm-min", 12);
  tk.value = {
    gap: px("--tm-gap", 3),
    r: px("--tm-r", 3),
    bw: px("--tm-bw", 0),
    shadow: px("--tm-shadow", 0),
    gR: px("--tm-g-r", 4),
    gBw: px("--tm-g-bw", 1),
    head: px("--tm-head", 26),
    min,
    min2: px("--tm-min2", min),
    heading: cs.getPropertyValue("--heading").trim() || "sans-serif",
    ui: cs.getPropertyValue("--ui").trim() || "sans-serif",
  };
  measured.clear();
  labelCache.clear();
  fontVer.value++;
}
watch(theme, () => nextTick(readTokens));

// ── 量字（canvas）：标题栏、块上的字放不放得下 ──
const fontVer = ref(0);
const measured = new Map<string, number>();
let ctx2d: CanvasRenderingContext2D | null = null;
function textW(font: string, s: string): number {
  const k = font + "\u0000" + s;
  let w = measured.get(k);
  if (w === undefined) {
    ctx2d ??= document.createElement("canvas").getContext("2d");
    if (!ctx2d) return s.length * 8;
    ctx2d.font = font;
    w = ctx2d.measureText(s).width;
    if (measured.size > 6000) measured.clear();
    measured.set(k, w);
  }
  return w;
}
/** 标题栏的字体（和下面 CSS 里的 .gh 一致）：C 主题名字用标题字体，大小 / 省略号用界面字体 */
const headFonts = computed(() => {
  void fontVer.value;
  const t = tk.value;
  return theme.value === "C"
    ? { name: `700 13px ${t.heading}`, size: `600 12px ${t.ui}`, ell: `600 13px ${t.ui}` }
    : { name: `600 12px ${t.ui}`, size: `500 12px ${t.ui}`, ell: `600 12px ${t.ui}` };
});
/**
 * 块上的字（F3）：字体 / 行高从探针元素（和块上的字同样的类）的计算样式读，canvas 量宽度。
 * 数字是等宽数字（tabular-nums），canvas 量不出来：把数字都当成「0」量。
 */
const metrics = computed<LabelMetrics>(() => {
  void fontVer.value;
  const el = probe.value;
  const read = (sel: string) => {
    const e = el?.querySelector<HTMLElement>(sel);
    if (!e) return { font: `500 12px ${tk.value.ui}`, lh: 15.6, px: 12 };
    const cs = getComputedStyle(e);
    const px = parseFloat(cs.fontSize) || 12;
    const lh = parseFloat(cs.lineHeight);
    return { font: `${cs.fontStyle} ${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`, lh: Number.isFinite(lh) ? lh : px * 1.3, px };
  };
  const a = read(".p-n .a");
  const b = read(".p-n .b");
  const A = read(".p-b .a");
  const B = read(".p-b .b");
  const ln = read(".p-n .ln .n");
  const li = read(".p-n .ln i");
  const v = read(".p-n .v");
  const vi = read(".p-n .v i");
  const tz = read(".p-n .b.tz");
  const mu = read(".p-n .b.tz.mu");
  const vu = read(".p-n .v.mu");
  const num = (s: string) => s.replace(/\d/g, "0");
  return {
    name: (s, big) => textW((big ? A : a).font, s),
    size: (s, big) => textW((big ? B : b).font, num(s)),
    lname: (s) => textW(ln.font, s),
    lsize: (s) => textW(li.font, num(s)),
    vname: (s) => textW(v.font, s),
    vsize: (s) => textW(vi.font, num(s)),
    tiny: (s) => textW(tz.font, num(s)),
    micro: (s) => textW(mu.font, num(s)),
    ell: (big) => textW((big ? A : a).font, "…"),
    ellLine: textW(ln.font, "…"),
    ellV: textW(v.font, "…"),
    lh: { a: a.lh, aBig: A.lh, b: b.lh, bBig: B.lh, ln: ln.lh, v: v.lh, s: b.px, t: tz.px, u: mu.px, uv: vu.lh },
  };
});
/** 小于 16px 画平块；小于 48px 不画投影；小于 24px 斜条纹改纯色 */
const FLAT_PX = 16;
const SHADOW_MIN_PX = 48;
const PATTERN_MIN_PX = 24;
/** 离场的块淡出后多久删掉（ms）：淡出（--dur-2 = 180ms）做完就删 */
const GHOST_MS = 230;
/** 位置过渡（--dur-3）的这么多之后，块上的字换成最终的写法淡入（F4） */
const TX_MS = 145;
/** 开发预览：块上带 key / 父块 key（截图脚本量过渡用），正式版不带 */
const DEV = import.meta.env.DEV;

const PSEUDO: Record<string, string> = { inacc: "无权限", sysest: "系统保留", unattr: "未归属", cloud: "云端占位" };
const PSEUDO_TIP: Record<string, string> = {
  inacc: "没有权限读，以管理员身份重启能看到",
  sysest: "Windows 自己占着的，扫不到",
  unattr: "盘上用了、但扫不到在哪",
};

/** 扫描根上的伪块（无权限 / 系统保留 / 未归属）：只有图的根是扫描根时才画 */
const extras = computed<TmInput[]>(() => {
  const b = props.summary.buckets;
  const list: [string, number][] = [
    ["inacc", b.inaccessible],
    ["sysest", b.system_reserved_estimated],
    ["unattr", b.scope_is_volume_root ? b.unattributed : 0],
  ];
  return list.filter(([, v]) => v > 0).map(([k, v]) => ({ key: `pseudo:${k}`, name: PSEUDO[k], value: v, ref: { pseudo: k } }));
});

/** 整张图（多铺一个间隙，最右 / 最下的块扣掉间隙后正好贴边） */
const frame = computed<Rect>(() => ({ x: 0, y: 0, w: size.value.w + tk.value.gap, h: size.value.h + tk.value.gap }));
/**
 * 布局参数。容器内边距：左 = 描边 + 间隙；右 / 下 = 间隙 + 投影 + 描边（容器的块面已经扣掉了间隙和投影），子块四周的空隙都 = 间隙。
 * 两层（根的子项 + 孙项）；展开链上的块下面另外再画两层（子项 + 孙项）。第 2 层起的最小块可以更大（--tm-min2）
 */
const opt = computed<LayoutOptions>(() => {
  const t = tk.value;
  const G = t.gap;
  // 最小块按画出来的块面算（格子 = 块面 + 间隙）
  return { minPx: t.min + G, minPxDeep: t.min2 + G, maxDepth: 2, boostDepth: 2, headerPx: t.head, pad: t.gBw + G, padEnd: G + t.shadow + t.gBw, minContainerPx: 90 };
});

const pk = computed(() => props.chain[props.chain.length - 1] ?? null);
/** 最深的展开块（pk）在图上的那一块 */
function pkTile(items: readonly TmItem[]): TmItem | null {
  const p = pk.value;
  return p ? (items.find((it) => it.chain && it.depth > 0 && samePath(tileNode(it)?.path, p)) ?? null) : null;
}

/**
 * 这一次画什么：从扫描根开始试，pk 的内容区不够大（bigEnough，按像素）就把图的根下移一层（链在图上的下一层），
 * 直到够大或根 = pk 的父级。
 */
const laid = computed<{ root: string; items: TmItem[]; hidden: Map<string, HiddenSlot> } | null>(() => {
  const d = props.data;
  const { w, h } = size.value;
  if (!d || w < 20 || h < 20) return null;
  const cands = props.chain.length ? rootCandidates(d, props.chain) : [d.path];
  const o = opt.value;
  const fr = frame.value;
  const fits = (r: { items: TmItem[] }) => {
    const t = pkTile(r.items);
    return !!t && bigEnough(contentBox(t.rect, o), { w, h });
  };
  return pickMapRoot(cands, (root) => layoutMap(d, props.chain, root, fr, o, extras.value), fits)?.result ?? null;
});
const items = computed<TmItem[]>(() => laid.value?.items ?? []);
const byKey = computed(() => new Map(items.value.map((it) => [it.key, it])));
/** 这一次没有单独画出来的子项结构上的位置，换成和块一样的绘制坐标（像素对齐、扣掉间隙；长度为 0 的线还是 0） */
type SlotBox = { b: Box; parentKey: string | null };
const hiddenBoxes = computed(() => {
  const g = tk.value.gap;
  const m = new Map<string, SlotBox>();
  for (const [k, s] of laid.value?.hidden ?? []) {
    const r = s.rect;
    const x = Math.round(r.x);
    const y = Math.round(r.y);
    m.set(k, { b: { x, y, w: Math.max(0, Math.round(r.x + r.w) - x - g), h: Math.max(0, Math.round(r.y + r.h) - y - g) }, parentKey: s.parentKey });
  }
  return m;
});

/** 最深的展开块（pk 在图上的那一块；合并块也算；pk 画不出来时退到最深的那个链上的块） */
const pkKey = computed(() => {
  const t = pkTile(items.value);
  if (t) return t.key;
  let best: TmItem | null = null;
  for (const it of items.value) if (it.chain && it.depth > 0 && (!best || it.depth > best.depth)) best = it;
  return best?.key ?? null;
});
/** 选中的那一块还在、而且（是「其他」时）内容没变 */
const selOk = computed(() => {
  const k = props.selKey;
  const it = k ? byKey.value.get(k) : undefined;
  if (!it) return false;
  return it.kind !== "other" || !props.selSig || otherSig(it) === props.selSig;
});
/**
 * 描粗边（选中样式）的块：选中的文件 / 其他 / 伪块；没有就是最深的展开块。
 * 最深的展开容器（pk）另有自己的样式（标题栏实底），选中它里面的文件时不变。
 */
const curKey = computed(() => (selOk.value ? props.selKey : pkKey.value));
// 选中的「其他」被拆开了（它所在的文件夹重新取数，内容签名对不上）：不再保留选中
watch([items, () => props.selKey, () => props.selSig], () => {
  const k = props.selKey;
  if (!k || !k.includes("|其他")) return;
  const it = byKey.value.get(k);
  if (!it || (props.selSig && otherSig(it) !== props.selSig)) emit("unsel");
});

// ── 画出来的块：活的块 + 正在离场的块（ghost），按 key 排好 ──
interface Tile {
  key: string;
  /** 元素的身份（v-for 的 key）：平时 = key；某块要换位置又会横穿别的块时换一个新身份（旧的淡出、新的淡入） */
  rk: string;
  it: TmItem;
  /** 这一次的位置（离场的块 = 它要去的位置） */
  b: TileBox;
  /** 上一次的写字框（字先淡出时用它，字不会突然变）；null = 新出现的块 */
  lbOld: TileBox | null;
  /** 上一次写的内容（名字 / 大小 / 项数：「其他」的项数和大小会变）：字先淡出时沿用它，过半以后才换成这一次的 */
  txOld: TmItem | null;
  /** 这一次过渡里字要先淡出、过半再淡入（= 那次过渡的编号；0 = 不用） */
  fd: number;
  /** 容器刚变成叶子：标题栏留着、跟着淡掉（不会一下子变色） */
  hd: boolean;
  /** 容器在链上的状态（标题栏的样子）：pk / 链上的祖先 / 不在链上；刚变成叶子时标题栏按它原来的样子淡掉 */
  hdc: "pk" | "on" | "";
  /** 新出现的块从哪里长出来（null = 原地淡入） */
  from: Box | null;
  /** 出现的地方还压着正在让开的兄弟：晚这么多 ms 再淡入（0 = 不晚；子块跟着父块） */
  late: number;
  /** 这块是什么时候出现的（late 从这时算起） */
  born: number;
  /** 正在离场 */
  gone: boolean;
  z: number;
  /** 离场的块什么时候删 */
  until: number;
}
/** 上一次画出来的：活的块（key → 块）、还在淡出的块、那时图的根 */
let prevLive = new Map<string, Tile>();
let prevGhosts: Tile[] = [];
let prevRoot: string | null = null;
/** 上一次算块时的布局（同一份布局 = 只是离场的块到时间了） */
let prevItems: TmItem[] | null = null;
/** 上一次布局里没有单独画出来的子项结构上的位置（这一次出现的块从这里长出来） */
let prevHidden: Map<string, SlotBox> = new Map();
/** 每个 key 现在用的元素身份 */
const rkeyOf = new Map<string, string>();
let rgen = 0;
/** 离场的块到时间了：重算一次（把它们删掉） */
const tick = ref(0);
/** 字的淡出 / 淡入（F4）：txGen = 最近一次要换字的过渡；txShown = 已经换成最终写法的那一次 */
let txGen = 0;
const txShown = ref(0);
/** txShown 的普通副本：tiles 里读它不建立依赖（不然字一换，整张图又算一次过渡） */
let txShownNow = 0;
/** 元素（按身份）：过渡进行到一半又换了布局时，从屏幕上它们**现在**的位置算起 */
const els = new Map<string, HTMLElement>();
function setEl(rk: string, el: unknown) {
  if (el instanceof HTMLElement) els.set(rk, el);
  else els.delete(rk);
}
function readBoxes(): Map<string, Box> {
  const m = new Map<string, Box>();
  const w = wrap.value;
  if (!w) return m;
  const o = w.getBoundingClientRect();
  for (const [rk, el] of els) {
    const r = el.getBoundingClientRect();
    m.set(rk, { x: r.left - o.left, y: r.top - o.top, w: r.width, h: r.height });
  }
  return m;
}

/** 层次（tmmotion.ts tileZ）：越深越高；同一层里容器 < 叶子 < 「其他」（幕布，比兄弟往下好几层都高） */
const zOf = (it: TmItem) => tileZ(it);
const boxOf = (b: TileBox): Box => ({ x: b.x, y: b.y, w: b.w, h: b.h });
/** 块的内容区（子块铺在这里）：容器扣掉标题栏和内边距；叶子就是它自己 */
function inner(b: Box, container: boolean): Box {
  if (!container) return b;
  const t = tk.value;
  return { x: b.x + t.gBw + t.gap, y: b.y + t.head, w: Math.max(1, b.w - t.gap - 2 * t.gBw), h: Math.max(1, b.h - t.head - t.gBw) };
}
/** 字的框和内容：正在换字（字先淡出）时用上一次的，过半以后用这一次的 */
const fading = (x: Tile) => x.fd > txShown.value;
const lbOf = (x: Tile): TileBox => (fading(x) ? (x.lbOld ?? x.b) : x.b);
const txIt = (x: Tile): TmItem => (fading(x) ? (x.txOld ?? x.it) : x.it);

const tiles = computed<Tile[]>(() => {
  void tick.value;
  const L = laid.value;
  if (!L) return [];
  const t = tk.value;
  const leaf: TileTokens = { gap: t.gap, r: t.r, bw: t.bw, shadow: t.shadow, flatPx: FLAT_PX, shadowMinPx: SHADOW_MIN_PX, patternMinPx: PATTERN_MIN_PX };
  const group: TileTokens = { ...leaf, r: t.gR, bw: t.gBw, flatPx: 0 };
  const now = performance.now();
  const instant = still.value || reduced || !prevLive.size;
  // 投影只给顶层块（容器 / 顶层大块），里面的小块只有描边
  const live: Tile[] = L.items.map((it) => {
    const b = tileBox(it.rect, it.container ? group : leaf, it.depth === 1);
    const hdc: Tile["hdc"] = !it.container ? "" : it.chain && it.depth > 0 && samePath(tileNode(it)?.path, pk.value) ? "pk" : it.chain ? "on" : "";
    return { key: it.key, rk: rkeyOf.get(it.key) ?? it.key, it, b, lbOld: null, txOld: null, fd: 0, hd: false, hdc, from: null, late: 0, born: 0, gone: false, z: zOf(it), until: 0 };
  });
  const done = () => {
    for (const x of live) rkeyOf.set(x.key, x.rk);
  };
  if (instant) {
    done();
    return live.sort(byTile);
  }
  // 布局没变，只是离场的块到时间了：活的块原样保留（不再算一次过渡），刚变成叶子的块的标题栏这时才拿掉
  if (L.items === prevItems) {
    const keep = [...prevLive.values()].map((x) => (x.hd ? { ...x, hd: false } : x));
    return [...keep, ...prevGhosts.filter((gh) => gh.until > now)].sort(byTile);
  }
  const boxes = readBoxes();
  const mt = (x: Tile): MotionTile => ({
    key: x.key,
    rk: x.rk,
    parentKey: x.it.parentKey,
    depth: x.it.depth,
    container: x.it.container,
    path: tileNode(x.it)?.path ?? null,
    z: x.z,
    b: boxOf(x.b),
    merged: x.it.merged,
  });
  const prevM = new Map<string, MotionTile>();
  for (const [k, s] of prevLive) prevM.set(k, mt(s));
  const plan = planMotion({
    prev: prevM,
    cur: (s) => boxes.get(s.rk) ?? s.b,
    live: live.map(mt),
    prevRoot,
    root: L.root,
    inner,
    frame: frame.value,
    head: t.head,
    hidden: prevHidden,
    hiddenNow: hiddenBoxes.value,
    // 上一次出现、还在等着淡入（看不见）的块：过渡被打断时当新块重新算
    pending: new Set([...prevLive.values()].filter((x) => x.late > 0 && now < x.born + x.late + 30).map((x) => x.key)),
  });
  // 开发预览：截图脚本查这一次的动法
  if (DEV) (window as unknown as { __tmPlan: unknown }).__tmPlan = { from: [...plan.from], late: [...plan.late], jump: [...plan.jump], ghosts: plan.ghosts, cam: plan.cam };
  const fresh = new Map(live.map((x) => [x.key, x]));
  for (const k of plan.jump) fresh.get(k)!.rk = `${k}#${++rgen}`;
  const ghostRks = new Set(prevGhosts.map((g) => g.rk));
  const g = plan.moved.size ? ++txGen : 0;
  for (const x of live) {
    const s = prevLive.get(x.key);
    if (plan.moved.has(x.key)) {
      x.fd = g;
      // 同一个元素：字先按旧的写法、旧的内容淡出（不会在旧的块里写出新的、被裁掉的字；「其他」的项数 / 大小不会第 0 帧就跳）
      if (s && s.rk === x.rk) {
        const old = s.fd > txShownNow;
        x.lbOld = old ? (s.lbOld ?? s.b) : s.b;
        x.txOld = old ? (s.txOld ?? s.it) : s.it;
      }
    }
    // 容器变成了叶子：标题栏按原来的样子留着、淡掉
    if (!x.it.container && s && s.rk === x.rk && (s.it.container || s.hd)) {
      x.hd = true;
      x.hdc = s.hdc;
    }
    // 同一个元素（还在、或者是正在淡出又回来的残影）：CSS 过渡从它现在的样子接着走
    if ((s && s.rk === x.rk) || ghostRks.has(x.rk)) continue;
    x.from = plan.from.get(x.key) ?? null;
    x.late = plan.late.get(x.key) ?? 0;
    x.born = now;
  }
  // 离场的块（包括换了身份的旧元素）：去它的祖先 / 镜头那里，同时淡出，过一会儿才删
  const out = live.slice();
  const liveRks = new Set(live.map((x) => x.rk));
  for (const gh of prevGhosts) if (gh.until > now && !liveRks.has(gh.rk)) out.push(gh);
  for (const gh of plan.ghosts) {
    const s = prevLive.get(gh.key)!;
    if (liveRks.has(s.rk)) continue;
    out.push({ ...s, b: { ...s.b, ...gh.to }, from: null, late: 0, born: 0, gone: true, z: gh.z, until: now + GHOST_MS, fd: 0, hd: false });
  }
  done();
  return out.sort(byTile);
});
function byTile(a: Tile, b: Tile) {
  if (a.key !== b.key) return a.key < b.key ? -1 : 1;
  return a.rk < b.rk ? -1 : a.rk > b.rk ? 1 : 0;
}
watch(
  tiles,
  (l) => {
    prevLive = new Map(l.filter((x) => !x.gone).map((x) => [x.key, x]));
    prevGhosts = l.filter((x) => x.gone);
    prevRoot = laid.value?.root ?? null;
    prevItems = laid.value?.items ?? null;
    prevHidden = hiddenBoxes.value;
    clearTimeout(ghostTimer);
    let next = Infinity;
    for (const x of prevGhosts) next = Math.min(next, x.until);
    if (Number.isFinite(next)) ghostTimer = setTimeout(() => tick.value++, Math.max(0, next - performance.now()) + 16);
    // 字：过渡过半后换成最终的写法淡入
    if (txGen > txShown.value && l.some((x) => x.fd === txGen)) {
      const g = txGen;
      clearTimeout(txTimer);
      txTimer = setTimeout(() => (txShown.value = txShownNow = g), TX_MS);
    }
  },
  { flush: "post" },
);

/** 链上的容器（含缩放后图顶上的根标题条）：标题栏是「收起」按钮（键盘的焦点也落在标题栏上） */
const headTarget = (x: Tile) => x.it.container && x.it.chain;
/** 能往里挖的文件夹块（aria-expanded 用） */
const openable = (it: TmItem) => {
  const n = tileNode(it);
  return !!n && n.kind === "Dir" && n.can_open;
};

function boxStyle(x: Tile, sel: boolean) {
  const { it, b, from } = x;
  const s: Record<string, string> = {
    "--x": b.x.toFixed(1) + "px",
    "--y": b.y.toFixed(1) + "px",
    "--w": Math.max(0, b.w).toFixed(1) + "px",
    "--h": Math.max(0, b.h).toFixed(1) + "px",
    borderRadius: b.r + "px",
    // 顶层块的硬投影写在 --sh 里：悬停 / 选中的内圈（box-shadow inset）叠在它上面，不会把投影冲掉
    "--sh": b.shadow ? `${b.shadow}px ${b.shadow}px 0 var(--stroke, var(--ink))` : "0 0 0 0 transparent",
    // 选中的块垫高：它的硬投影压在间隙上，不被旁边的块盖住
    zIndex: String(sel ? x.z + 100 : x.z),
  };
  if (x.late) s["--fd"] = x.late + "ms";
  if (from) {
    s["--x0"] = from.x.toFixed(1) + "px";
    s["--y0"] = from.y.toFixed(1) + "px";
    s["--w0"] = Math.max(0, from.w).toFixed(1) + "px";
    s["--h0"] = Math.max(0, from.h).toFixed(1) + "px";
  }
  if (!it.container) s.borderWidth = b.bw + "px";
  return s;
}

function tone(it: TmItem): string {
  if (it.kind === "other") return "mix";
  const n = tileNode(it);
  if (!n) return "na";
  if (n.kind === "Rest") return "mix";
  return n.tier === "unknown" ? "mix" : n.tier;
}

// ── 容器标题栏：名字露不出 6 个字时不写大小；截断用界面字体的「…」 ──
const heads = computed(() => {
  const f = headFonts.value;
  const t = tk.value;
  void txShown.value;
  // 大小是等宽数字（tabular-nums）：canvas 量不出来，数字都当成「0」量（「1」比比例数字宽，按原样量会少算 1–2 px，名字被硬裁掉最后一个字）
  const num = (s: string) => s.replace(/\d/g, "0");
  const m = { name: (s: string) => textW(f.name, s), size: (s: string) => textW(f.size, num(s)), ell: textW(f.ell, "…"), gap: 6 };
  const out = new Map<string, ReturnType<typeof fitHead>>();
  for (const x of tiles.value) {
    if (!x.it.container && !x.hd) continue;
    const isPk = pkKey.value === x.key;
    const bw = isPk ? (theme.value === "C" ? 3 : 2) : t.gBw;
    const tx = txIt(x);
    // 标题栏左右内边距 8；链上的有收起箭头（14 + 间隔 6）；pk 留出加载小点的位置（6 + 6）；再留 2 px 量字误差
    let avail = lbOf(x).w - 2 * bw - 16 - 4;
    if (x.it.chain) avail -= 20;
    if (isPk) avail -= 12;
    out.set(x.rk, fitHead(tx.name, full(tx.size), avail, m));
  }
  return out;
});

// ── 叶子块上的字（F3）：按量出来的字宽 / 行高决定写法 ──
const NO_LABEL: LabelFit = { mode: "none", big: false, name: "", cut: false, ix: 0, iy: 0 };
const labelCache = new Map<string, LabelFit>();
function labelOf(x: Tile): LabelFit {
  if (x.it.container) return NO_LABEL;
  const b = lbOf(x);
  if (b.flat) return NO_LABEL;
  const tn = tone(x.it);
  const pill = theme.value === "C" && tn === "na" && b.pattern;
  const it = txIt(x);
  const k = `${fontVer.value}|${b.w}|${b.h}|${b.bw}|${it.kind}|${it.name}|${it.size}|${it.mergedCount}|${pill ? 1 : 0}`;
  let f = labelCache.get(k);
  if (!f) {
    f = fitLabel(
      { w: b.w, h: b.h, bw: b.bw, name: it.name, count: countLabel(it), full: full(it.size), short: short(it.size), pill, big: b.w > 150 && b.h > 70 },
      metrics.value,
    );
    if (labelCache.size > 4000) labelCache.clear();
    labelCache.set(k, f);
  }
  return f;
}
const labels = computed(() => {
  void txShown.value;
  void metrics.value;
  const m = new Map<string, LabelFit>();
  for (const x of tiles.value) if (!x.it.container) m.set(x.rk, labelOf(x));
  return m;
});
const lab = (x: Tile) => labels.value.get(x.rk) ?? NO_LABEL;
function inStyle(f: LabelFit) {
  return { "--ix": f.ix + "px", "--iy": f.iy + "px" };
}

/** 单击 / Enter / Space：detail = 鼠标连击次数（系统的双击判定；键盘 = 0） */
function tapInfo(e: Event) {
  return { at: e.timeStamp, detail: e instanceof MouseEvent ? e.detail : 0 };
}
function onTap(x: Tile, e: Event) {
  // 离场的块、缩放后的根（它的身体只是整张图的边框，单击它的标题栏才收起）
  if (x.gone || x.it.depth === 0) return;
  const h = hitOf(x.it, byKey.value, laid.value?.root ?? null);
  const info = tapInfo(e);
  remember(x.key);
  // 没有列表时，选中一块就把说明卡片钉在它旁边；下一次单击（双击的第二下除外）收起
  if (!(info.detail >= 2 && pinKey.value === x.key)) pinKey.value = props.pin && h.kind === "leaf" && pinKey.value !== x.key ? x.key : null;
  emit("hit", h, info);
}
function onHead(x: Tile, e: Event) {
  if (x.gone || !x.it.container) return;
  const h = headHit(x.it);
  remember(x.key);
  pinKey.value = null;
  if (h) emit("hit", h, tapInfo(e));
}

// ── 悬停卡片 / 钉住的卡片 ──
const hover = ref<TmItem | null>(null);
const tipPos = ref({ x: 0, y: 0 });
const TIP_W = 236;
/** 预取（F8）：鼠标在一个还没取过的文件夹块上停 120 ms，就让 store 在后台取它的子树（不改任何状态） */
let peekKey: string | null = null;
const PEEK_MS = 120;
function peekAt(x: Tile) {
  if (x.key === peekKey) return;
  peekKey = x.key;
  clearTimeout(peekTimer);
  const n = tileNode(x.it);
  if (!n || n.kind !== "Dir" || !n.can_open || x.gone || x.it.depth === 0 || pkKey.value === x.key) return;
  const path = n.path;
  peekTimer = setTimeout(() => emit("peek", path), PEEK_MS);
}
function onMove(e: MouseEvent, x: Tile) {
  peekAt(x);
  const it = x.it;
  if (it.container || x.gone) {
    hover.value = null;
    return;
  }
  hover.value = it;
  const r = wrap.value!.getBoundingClientRect();
  let tx = e.clientX - r.left + 14;
  let ty = e.clientY - r.top + 16;
  if (tx + TIP_W + 14 > r.width) tx = e.clientX - r.left - TIP_W - 14;
  if (ty + 110 > r.height) ty = e.clientY - r.top - 110;
  tipPos.value = { x: tx, y: ty };
}
function onLeave() {
  hover.value = null;
  peekKey = null;
  clearTimeout(peekTimer);
}
// 布局变了（展开 / 收起）：悬停的那块可能已经不在原来的位置，等鼠标再动
watch(items, () => {
  hover.value = null;
  peekKey = null;
  clearTimeout(peekTimer);
});

/** 外壳合并块（Users \ king）写全：外层、内层各自的名字和大小（块上只写得下一个数） */
function shellText(it: TmItem): string {
  const n = tileNode(it);
  const parts = it.name.split(" \\ ");
  if (!n || parts.length < 2) return "";
  return `${parts[0]} ${full(it.value)} · ${parts[parts.length - 1]} ${full(n.allocated)}`;
}

/** 卡片内容：名字、大小、一句说明（认不出来的写第一条线索，没有线索就只写「认不出来」） */
function tipOf(it: TmItem) {
  const n = tileNode(it);
  const tn = tone(it);
  const pseudo = (it.ref as { pseudo?: string } | undefined)?.pseudo;
  const cnt = countLabel(it);
  let what = "";
  if (it.kind === "other") what = `${it.mergedCount} 个小项合计`;
  else if (pseudo) what = PSEUDO_TIP[pseudo] ?? "";
  else if (n?.kind === "Rest") what = cnt ? `${cnt.num} 个小文件合计` : n.what;
  else if (n && n.tier === "unknown") what = props.clues?.get(pathKey(n.path)) ?? "认不出来";
  else what = n?.what ?? "";
  if (!what && tn === "mix") what = "认不出来";
  return {
    name: tipName(it),
    size: sizeParts(it.size).join(" "),
    tone: tn,
    shell: shellText(it),
    what,
    lose: n && n.tier === "review" ? n.lose_kind : null,
    loseText: n && n.tier === "review" ? loseTag(n) : null,
  };
}
const pinKey = ref<string | null>(null);
// 选中换了（Esc、点了别处、「其他」被拆开）/ 切到有列表的视图：钉住的卡片收起
watch(
  () => props.selKey,
  (k) => {
    if (k !== pinKey.value) pinKey.value = null;
  },
);
watch(
  () => props.pin,
  (p) => {
    if (!p) pinKey.value = null;
  },
);
const tip = computed(() => {
  const it = hover.value;
  if (!it || it.container || it.key === pinKey.value) return null;
  return tipOf(it);
});
/** 钉住的卡片：放在那块的右边（放不下放左边，都放不下放下面 / 上面），跟着块的目标位置走 */
const pinTip = computed(() => {
  const k = pinKey.value;
  if (!k || !props.pin || !selOk.value || props.selKey !== k) return null;
  const x = tiles.value.find((t) => t.key === k && !t.gone);
  if (!x) return null;
  const { w: W, h: H } = size.value;
  const b = x.b;
  const M = 10;
  const pos: Record<string, string> = {};
  let left = b.x + b.w + M;
  if (left + TIP_W > W - 4) left = b.x - TIP_W - M;
  if (left >= 4) {
    // 靠下的块：卡片底边对齐块的底边（不量卡片多高也不会出界）
    if (b.y + 130 > H) pos.bottom = Math.max(4, H - (b.y + b.h)) + "px";
    else pos.top = Math.max(4, b.y) + "px";
  } else {
    left = Math.min(Math.max(4, b.x + b.w / 2 - TIP_W / 2), W - TIP_W - 4);
    if (b.y + b.h + M + 120 <= H) pos.top = b.y + b.h + M + "px";
    else pos.bottom = Math.max(4, H - b.y + M) + "px";
  }
  pos.left = left + "px";
  return { ...tipOf(x.it), pos };
});

// ── 键盘：只有一块在 Tab 序列里（roving tabindex），方向键按画面位置走 ──
/** 最近一次键盘 / 鼠标落在的块和它的各级父块：那块没了（收起了 / 被并进「其他」）就落到还在的最近的父块上 */
const navPath = ref<string[]>([]);
function remember(key: string) {
  const path: string[] = [];
  let k: string | null = key;
  while (k && path.length < 64) {
    path.push(k);
    k = byKey.value.get(k)?.parentKey ?? null;
  }
  navPath.value = path;
}
const navBoxes = computed(() =>
  navTargets(
    tiles.value.filter((x) => !x.gone).map((x) => ({ key: x.key, parentKey: x.it.parentKey, container: x.it.container, chain: x.it.chain, b: x.b })),
    (t) => t.b,
    tk.value.head,
    (t) => t.b.flat,
  ),
);
const rovingKey = computed(() => {
  const list = navBoxes.value;
  const set = new Set(list.map((b) => b.key));
  for (const k of navPath.value) if (set.has(k)) return k;
  const p = pkKey.value;
  if (p && set.has(p)) return p;
  let best: (typeof list)[number] | null = null;
  for (const b of list) if (!best || b.y < best.y - 1 || (Math.abs(b.y - best.y) <= 1 && b.x < best.x)) best = b;
  return best?.key ?? null;
});
const DIRS: Record<string, NavDir> = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };
function targetEl(key: string | null): HTMLElement | null {
  if (!key) return null;
  const x = tiles.value.find((t) => t.key === key && !t.gone);
  const el = x ? els.get(x.rk) : null;
  if (!x || !el) return null;
  return headTarget(x) ? el.querySelector<HTMLElement>(":scope > .gh") : el;
}
function focusRoving() {
  const el = targetEl(rovingKey.value);
  if (el && document.activeElement !== el) el.focus({ preventScroll: true });
}
function onKeyNav(e: KeyboardEvent) {
  const d = DIRS[e.key];
  if (!d || e.altKey || e.ctrlKey || e.metaKey) return;
  e.preventDefault();
  const next = navStep(navBoxes.value, rovingKey.value ?? "", d);
  if (!next) return;
  hover.value = null;
  remember(next);
  nextTick(focusRoving);
}
/** 焦点是用键盘放进图里的：布局变了（Enter 展开 / Esc 收起）以后，焦点跟着落到当前那一块上 */
let kbd = false;
function onFocusIn() {
  kbd = true;
}
function onFocusOut(e: FocusEvent) {
  const r = e.relatedTarget as Node | null;
  if (r && !wrap.value?.contains(r)) kbd = false;
}
watch(
  [tiles, rovingKey],
  () => {
    if (!kbd) return;
    const a = document.activeElement;
    if (!a || a === document.body || !wrap.value?.contains(a) || a !== targetEl(rovingKey.value)) focusRoving();
  },
  { flush: "post" },
);
/** 鼠标按下：块不拿焦点（焦点环只在键盘 Tab 时出现），之前键盘聚焦的块也放开 */
function onDown(e: MouseEvent) {
  e.preventDefault();
  const a = document.activeElement;
  if (a instanceof HTMLElement && wrap.value?.contains(a)) a.blur();
}
function onDocDown(e: MouseEvent) {
  kbd = false;
  // 点在块以外的地方（图的空隙、工具栏、列表…）：钉住的卡片收起；点在块上由 onTap 处理
  const t = e.target as Element | null;
  if (pinKey.value && !t?.closest?.(".tm-tile")) pinKey.value = null;
}

/** 块上的大小（始终是真实大小）：1 GB 以上一位小数 GB，以下用 MB / KB（不会出现 “0.0 GB”） */
function full(v: number) {
  return v >= 1024 ** 3 ? `${(v / 1024 ** 3).toFixed(1)} GB` : sizeParts(v).join(" ");
}
/** 只写数字的小块：GB 不写单位（和以前一样）；不到 0.1 GB 的写成 “48M” */
function short(v: number) {
  if (v >= 0.095 * 1024 ** 3) return (v / 1024 ** 3).toFixed(1);
  const [n, u] = sizeParts(v);
  return `${Math.round(Number(n))}${u.slice(0, 1)}`;
}
/** 标题栏的提示：名字被截了 / 只写了大小 / 外壳合并块（写全两层的大小） */
function headTitle(x: Tile): string | undefined {
  const h = heads.value.get(x.rk);
  const shell = shellText(x.it);
  if (shell) return `${x.it.name}\n${shell}`;
  return h?.cut || h?.name === "" || h?.size === false ? x.it.name + "  " + full(x.it.size) : undefined;
}
</script>

<template>
  <div
    ref="wrap"
    class="map"
    :class="{ busy: loading, still }"
    @mouseleave="onLeave"
    @mousedown="onDown"
    @keydown="onKeyNav"
    @focusin="onFocusIn"
    @focusout="onFocusOut"
  >
    <!-- 一块 = 一个元素（叶子 / 容器只换类和里面的内容）：叶子变成容器时也是同一个元素在原地长大；离场的块淡出后才删 -->
    <div
      v-for="x in tiles"
      :key="x.rk"
      v-memo="[
        x.b,
        x.from,
        x.late,
        x.gone,
        x.z,
        x.it,
        x.hd,
        fading(x),
        lab(x),
        curKey === x.key,
        pkKey === x.key,
        hover?.key === x.key,
        loading && pkKey === x.key,
        rovingKey === x.key,
        heads.get(x.rk),
      ]"
      :ref="(el) => setEl(x.rk, el)"
      class="tm-tile"
      :class="[
        x.it.container
          ? ['g', { on: x.it.chain && pkKey !== x.key, pk: pkKey === x.key, root: x.it.depth === 0, wait: loading && pkKey === x.key }]
          : [
              'lf',
              'tm-tone',
              tone(x.it),
              {
                big: lab(x).big,
                flat: x.b.flat,
                bare: lab(x).mode === 'none',
                vt: lab(x).mode === 'vert',
                lt: lab(x).mode === 'line',
                sz: lab(x).mode === 'size',
                solid: !x.b.pattern,
                hov: hover?.key === x.key,
                sel: curKey === x.key,
              },
            ],
        { gone: x.gone, txo: fading(x) },
      ]"
      :style="boxStyle(x, !x.it.container && curKey === x.key)"
      :data-k="DEV ? x.key : undefined"
      :data-p="DEV ? (x.it.parentKey ?? '') : undefined"
      :role="x.gone ? undefined : headTarget(x) ? 'group' : 'button'"
      :tabindex="x.gone ? undefined : !headTarget(x) && !x.b.flat && rovingKey === x.key ? 0 : -1"
      :aria-hidden="x.gone ? 'true' : undefined"
      :aria-label="x.gone ? undefined : '方块 ' + x.it.name"
      :aria-expanded="x.gone || headTarget(x) || !openable(x.it) ? undefined : x.it.chain"
      :aria-current="!x.gone && !x.it.container && curKey === x.key && selOk ? 'true' : undefined"
      @click="onTap(x, $event)"
      @keydown.enter.prevent="onTap(x, $event)"
      @keydown.space.prevent="onTap(x, $event)"
      @mousemove="onMove($event, x)"
    >
      <div
        v-if="x.it.container || x.hd"
        class="gh"
        :class="x.it.container ? undefined : ['out', 'was-' + x.hdc]"
        :role="headTarget(x) && !x.gone ? 'button' : undefined"
        :tabindex="headTarget(x) && !x.gone ? (rovingKey === x.key ? 0 : -1) : undefined"
        :aria-label="headTarget(x) && !x.gone ? (x.it.depth === 0 ? '收起到 ' : '收起 ') + x.it.name : undefined"
        :aria-expanded="headTarget(x) && !x.gone ? 'true' : undefined"
        :title="headTitle(x)"
        @click.stop="onHead(x, $event)"
        @keydown.enter.stop.prevent="onHead(x, $event)"
        @keydown.space.stop.prevent="onHead(x, $event)"
      >
        <span v-if="heads.get(x.rk)?.name !== ''" class="gn">{{ heads.get(x.rk)?.name ?? txIt(x).name }}<span v-if="heads.get(x.rk)?.cut" class="el">…</span></span>
        <i v-if="heads.get(x.rk)?.size !== false" class="num">{{ full(txIt(x).size) }}</i>
        <svg v-if="x.it.chain" class="up" viewBox="0 0 24 24" aria-hidden="true"><path d="m6 15 6-6 6 6" /></svg>
      </div>
      <!-- 字放在里面一层（绝对定位、四周留白由 fitLabel 算好）：块自己没有内边距，过渡里块再小也不会被内边距撑大、压到邻块 -->
      <span v-if="!x.it.container && lab(x).mode !== 'none'" class="in" :style="inStyle(lab(x))">
        <template v-if="lab(x).mode === 'full'">
          <span class="a">{{ lab(x).name }}<span v-if="lab(x).cut" class="el">…</span></span>
          <span class="b num">{{ full(txIt(x).size) }}</span>
        </template>
        <span v-else-if="lab(x).mode === 'size'" class="b num" :class="{ tz: lab(x).tiny, mu: lab(x).micro }">{{ short(txIt(x).size) }}</span>
        <span v-else-if="lab(x).mode === 'vert'" class="v" :class="{ only: !lab(x).name, mu: lab(x).micro }">{{ lab(x).name }}{{ lab(x).cut ? "…" : "" }}<i class="num">{{ short(txIt(x).size) }}</i></span>
        <span v-else class="ln"
          ><span class="n">{{ lab(x).name }}<span v-if="lab(x).cut" class="el">…</span></span><i class="num">{{ full(txIt(x).size) }}</i></span
        >
      </span>
    </div>
    <!-- 量字的探针：和块上的字同样的类（读计算出来的字体 / 行高） -->
    <div ref="probe" class="probe" aria-hidden="true">
      <div class="lf p-n">
        <span class="in"><span class="a">名</span><span class="b num">0</span><span class="b num tz">0</span><span class="b num tz mu">0</span><span class="ln"><span class="n">名</span><i class="num">0</i></span><span class="v">名<i class="num">0</i></span><span class="v mu"><i class="num">0</i></span></span>
      </div>
      <div class="lf big p-b">
        <span class="in"><span class="a">名</span><span class="b num">0</span></span>
      </div>
    </div>
    <div v-if="tip" class="tip" :style="{ left: tipPos.x + 'px', top: tipPos.y + 'px' }">
      <div class="t1"><i class="dot" :class="tip.tone"></i><span class="tn">{{ tip.name }}</span><span class="s num">{{ tip.size }}</span></div>
      <div v-if="tip.shell" class="t2 num">{{ tip.shell }}</div>
      <div v-if="tip.what" class="t2">{{ tip.what }}</div>
      <div v-if="tip.loseText" class="t3"><span class="tag" :class="{ hard: isHardTag(tip.lose) }">{{ tip.loseText }}</span></div>
    </div>
    <div v-if="pinTip" class="tip pin" :style="pinTip.pos" role="status">
      <div class="t1"><i class="dot" :class="pinTip.tone"></i><span class="tn">{{ pinTip.name }}</span><span class="s num">{{ pinTip.size }}</span></div>
      <div v-if="pinTip.shell" class="t2 num">{{ pinTip.shell }}</div>
      <div v-if="pinTip.what" class="t2">{{ pinTip.what }}</div>
      <div v-if="pinTip.loseText" class="t3"><span class="tag" :class="{ hard: isHardTag(pinTip.lose) }">{{ pinTip.loseText }}</span></div>
    </div>
  </div>
</template>

<style scoped>
.map {
  position: relative;
  width: 100%;
  height: 100%;
  /* 镜头推近 / 拉远时移出画面的块在图的边上被裁掉，不压到旁边的列表 */
  overflow: hidden;
  isolation: isolate;
}
.map.busy {
  cursor: progress;
}
.tm-tile {
  position: absolute;
  cursor: pointer;
}
/* 缩放后图的根（铺满整张图）：身体只是边框，单击它的标题条才收起 */
.tm-tile.root {
  cursor: default;
}
.tm-tile.root > .gh {
  cursor: pointer;
}
/* 键盘焦点（只在 Tab / 方向键时出现，鼠标点不会拿到焦点）：块里面一圈，和选中的粗边、悬停的内圈都不一样，也不压到旁边的块 */
.tm-tile:focus-visible {
  outline: 2px solid var(--ink);
  outline-offset: -6px;
}
.g {
  background: var(--surface);
  border: var(--tm-g-bw) solid var(--line);
}
.g:hover {
  border-color: var(--line2);
}
/* 标题栏：整条都能点（在链上 = 收起；不在 = 往里挖） */
.g .gh,
.gh.out {
  position: absolute;
  left: 0;
  right: 0;
  top: 0;
  /* 标题栏底边 = 子块开始的地方（描边加粗时跟着扣掉） */
  height: calc(var(--tm-head) - var(--gb, var(--tm-g-bw)));
  padding: 0 8px;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--ink2);
  font-weight: var(--w-med);
  white-space: nowrap;
  overflow: hidden;
  border-radius: inherit;
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
  transition:
    background-color var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease),
    opacity 150ms var(--ease);
}
/* 容器刚变成叶子：标题栏（连同它的底色）跟着淡掉，叶子的颜色从下面露出来（不会一下子从墨黑变色） */
.tm-tile .gh.out {
  opacity: 0;
  pointer-events: none;
}
/* 叶子刚变成容器：标题栏淡入（不会在块上「啪」地出现一条黑条） */
@starting-style {
  .g .gh {
    opacity: 0;
  }
}
.map.still .gh {
  transition: none;
}
.g .gh:focus-visible {
  outline: 2px solid currentColor;
  outline-offset: -4px;
}
/* 名字的截断由脚本按宽度算好（名字露不出 6 个字时不写大小）；省略号用界面字体的「…」，不用标题字体里居中的「···」 */
.g .gh .gn,
.gh.out .gn {
  min-width: 0;
  overflow: hidden;
  text-overflow: clip;
}
.g .gh .gn .el,
.gh.out .gn .el {
  font-family: var(--ui);
}
.g .gh i,
.gh.out i {
  flex: none;
  font-style: normal;
  color: var(--ink3);
  /* 字色和标题栏底色一起过渡（不会白字先变、底色后变，中间一瞬白字压白底） */
  transition:
    color var(--dur-1) var(--ease),
    opacity var(--dur-1) var(--ease);
}
.g .gh .up,
.gh.out .up {
  flex: none;
  width: 14px;
  height: 14px;
  margin-left: auto;
  fill: none;
  stroke: currentColor;
  stroke-width: 2.2;
  stroke-linecap: round;
  stroke-linejoin: round;
  opacity: 0.55;
  transition: opacity var(--dur-1) var(--ease);
}
.g:hover .gh {
  color: var(--ink);
}
.g .gh:hover .up {
  opacity: 1;
}
/* 展开链上的容器：只用中性色（绝不用三档颜色）。祖先：标题字加深加粗、描边加深一级；最深那个（pk）再强一级 */
.g.on {
  border-color: var(--line2);
}
.g.on .gh,
.g.pk .gh,
.gh.out.was-on,
.gh.out.was-pk {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.g.on .gh i,
.g.pk .gh i,
.gh.out.was-on i,
.gh.out.was-pk i {
  color: var(--ink2);
}
.g.pk {
  --gb: 2px;
  border: 2px solid var(--ink);
}
/* 正在取更深的数据：标题栏右侧一个小点轻轻闪（150ms 之后才出现，快的时候看不到） */
.g.wait .gh::after {
  content: "";
  flex: none;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
  opacity: 0;
  animation: tm-wait 0.9s var(--ease) 0.15s infinite;
}
@keyframes tm-wait {
  50% {
    opacity: 0.6;
  }
}
/* A 纸本：祖先的标题栏垫一层浅灰；悬停只加深字和描边（和祖先的灰底分得开） */
[data-theme="A"] .g.on .gh,
[data-theme="A"] .gh.out.was-on {
  background: var(--sunk);
}
[data-theme="A"] .g.on .gh:hover {
  background: #e2ded5;
}
/* A：正在看的文件夹 = 墨色实底标题栏、白字（和选中的 3px 描边是两种形式） */
[data-theme="A"] .g.pk .gh,
[data-theme="A"] .gh.out.was-pk {
  background: var(--ink);
  color: #fbfaf7;
}
[data-theme="A"] .g.pk .gh i,
[data-theme="A"] .gh.out.was-pk i {
  color: rgba(251, 250, 247, 0.72);
}
[data-theme="A"] .g.pk .gh:hover {
  background: #36342f;
}
/* B 暖炭：链上用浅色字 + 更亮的中性描边（金色只留给「可放心清理」） */
[data-theme="B"] .g {
  background: #1a1612;
  border-color: #2a241e;
}
[data-theme="B"] .g:hover {
  border-color: #4a4037;
}
[data-theme="B"] .g.on {
  border-color: #6e6358;
}
[data-theme="B"] .g.on .gh,
[data-theme="B"] .gh.out.was-on {
  background: #221d18;
}
/* B：正在看的文件夹 = 浅暖灰实底标题栏、深色字，描边中性灰（选中是浅色 3px 描边，悬停只提亮：三种形式） */
[data-theme="B"] .g.pk {
  border-color: #9a8f82;
}
[data-theme="B"] .g.pk .gh,
[data-theme="B"] .gh.out.was-pk {
  background: #d9cfc1;
  color: #1b1713;
}
[data-theme="B"] .g.pk .gh i,
[data-theme="B"] .gh.out.was-pk i {
  color: rgba(27, 23, 19, 0.7);
}
[data-theme="B"] .g.on .gh:hover {
  background: #342d26;
}
[data-theme="B"] .g.pk .gh:hover {
  background: #e6ddd0;
}
/* C 卡通：墨线容器。悬停：标题栏垫米色；链上的祖先：8% 墨色浅灰底 + 下面一道墨线；最深那个（pk）：墨黑实底白字 + 3px 墨框 */
[data-theme="C"] .g {
  border-color: #141412;
  background: #fff;
}
[data-theme="C"] .g .gh,
[data-theme="C"] .gh.out {
  font-family: var(--heading);
  font-weight: 700;
  color: #141412;
  font-size: 13px;
}
/* 大小的浅色用字色的透明度（不占 opacity：过渡里字淡出 / 残影的字立刻隐藏都靠 opacity，不能被这里盖掉） */
[data-theme="C"] .g .gh i,
[data-theme="C"] .gh.out i {
  font-family: var(--ui);
  font-size: 12px;
  font-weight: 600;
  color: rgba(20, 20, 18, 0.6);
}
[data-theme="C"] .g:not(.on):not(.pk):hover .gh {
  background: var(--bg);
}
[data-theme="C"] .g.on .gh,
[data-theme="C"] .gh.out.was-on {
  background: rgba(20, 20, 18, 0.08);
  box-shadow: inset 0 -2px 0 #141412;
}
[data-theme="C"] .g.on .gh:hover {
  background: rgba(20, 20, 18, 0.15);
}
[data-theme="C"] .g.pk {
  --gb: 3px;
  border: 3px solid #141412;
}
[data-theme="C"] .g.pk .gh,
[data-theme="C"] .gh.out.was-pk {
  background: #141412;
  color: #fff;
}
[data-theme="C"] .g.pk .gh i,
[data-theme="C"] .gh.out.was-pk i {
  color: rgba(255, 255, 255, 0.72);
}
[data-theme="C"] .g.pk .gh .up,
[data-theme="C"] .gh.out.was-pk .up {
  opacity: 0.9;
}
[data-theme="C"] .g.pk .gh:hover {
  background: #33332e;
}
/* 块上的字（F4）：过渡开始时，位置要变的块上的字约 60 ms 淡出，过渡过半后换成最终的写法再淡入；
   离场的块（残影）字立刻隐藏，只留色块淡出——不会出现新旧两层字叠在一起 */
.tm-tile .in,
.tm-tile > .gh > * {
  transition: opacity 110ms var(--ease);
}
/* 换成最终写法时新建的字（写法变了：一行 → 两行、标题栏放得下大小了…）也从 0 淡入，不会「啪」地先冒出一个数字 */
@starting-style {
  .tm-tile .in,
  .tm-tile > .gh > * {
    opacity: 0;
  }
}
/* !important：标题栏里各部分自己的不透明度（收起箭头 .55 / pk .9 / 悬停 1）不能盖掉淡出（过渡仍然生效） */
.tm-tile.txo .in,
.tm-tile.txo > .gh > * {
  opacity: 0 !important;
  transition-duration: 60ms;
}
.tm-tile.gone .in,
.tm-tile.gone > .gh > * {
  opacity: 0 !important;
  transition: none;
}
.lf {
  overflow: hidden;
  font-size: 12px;
  line-height: 1.3;
}
/* 字的那一层：四周留白用 inset（不是块的 padding），块的大小永远 = 布局给的格子。
   写法和留白由脚本按量出来的字宽 / 行高算好（fitLabel）：放得下才写，不会被裁掉半行 */
.lf .in {
  position: absolute;
  inset: var(--iy, 7px) var(--ix, 8px);
  overflow: hidden;
}
.lf .a {
  display: block;
  font-weight: var(--w-med);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: clip;
}
/* 截断用的「…」：界面字体（不用标题字体里居中的「···」） */
.lf .el {
  font-family: var(--ui);
}
.lf .b {
  display: block;
  opacity: 0.8;
  white-space: nowrap;
}
/* 只写大小的小块：一行，行高 = 字号（留白已经算进去了）；更小的块用小一号的字 */
.lf.sz .b {
  line-height: 1;
}
.lf .b.tz {
  font-size: 11px;
  line-height: 1;
}
.lf .b.tz.mu {
  font-size: 10px;
}
/* 平块（最短边 < 16px）连描边也没有 */
.lf.flat {
  border-width: 0 !important;
}
/* 细高条：竖排名字 + 大小（中文竖排自然，西文转 90°） */
.lf .v {
  display: block;
  writing-mode: vertical-rl;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: clip;
  max-height: 100%;
  font-weight: var(--w-med);
  line-height: 1.25;
}
.lf .v i {
  font-style: normal;
  font-weight: var(--w-reg);
  opacity: 0.8;
  margin-top: 6px;
}
/* 只竖着写大小 */
.lf .v.only i {
  margin-top: 0;
}
/* 20 px 宽的细高条：10px 竖着写大小 */
.lf .v.mu {
  font-size: 10px;
}
/* 扁长条：一行字，垂直居中 */
.lf.lt .in {
  inset: 0 var(--ix, 8px);
  display: flex;
  align-items: center;
}
.lf .ln {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
  white-space: nowrap;
  font-size: 11.5px;
  line-height: 1;
}
.lf .ln .n {
  min-width: 0;
  overflow: hidden;
  text-overflow: clip;
  font-weight: var(--w-med);
}
.lf .ln i {
  flex: none;
  font-style: normal;
  opacity: 0.8;
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
/* 三种状态各用一种表现形式（F9），同屏一眼分得清：悬停 = 只提亮（A 再加一圈很淡的白内圈）；
   选中 = 3px 实线描边 + 内圈；正在看的文件夹（pk）= 标题栏实底。都不用三档颜色（金色只留给「可放心清理」） */
.lf.hov {
  filter: brightness(1.07) saturate(1.05);
  box-shadow:
    inset 0 0 0 1.5px rgba(255, 255, 255, 0.75),
    var(--sh);
}
[data-theme="B"] .lf.hov {
  filter: brightness(1.22);
  box-shadow: var(--sh);
}
[data-theme="C"] .lf.hov {
  outline: none;
  filter: none;
  /* 白圈里面再压一道很淡的墨线：浅色 / 斜纹的块上白圈也看得出来 */
  box-shadow:
    inset 0 0 0 2px #fff,
    inset 0 0 0 3px rgba(20, 20, 18, 0.2),
    var(--sh);
}
/* 选中：A / B 3px 实线描边；C 墨框里面再加 3px 墨线（看上去边框变粗）+ 右下 3px 硬投影，块垫高 */
.lf.sel {
  outline: 3px solid var(--ink);
  outline-offset: -3px;
  filter: none;
  box-shadow:
    inset 0 0 0 5px rgba(255, 255, 255, 0.55),
    var(--sh);
}
/* B：浅色 3px 实线 + 里面一圈暗线 + 轻微内发光、底色提亮 */
[data-theme="B"] .lf.sel {
  outline-color: #f3ece2;
  filter: brightness(1.12);
  box-shadow:
    inset 0 0 0 4px rgba(12, 10, 8, 0.7),
    inset 0 0 18px 4px rgba(255, 240, 220, 0.22);
}
[data-theme="C"] .lf.sel {
  outline: none;
  box-shadow:
    inset 0 0 0 3px #141412,
    3px 3px 0 #141412,
    var(--sh);
}
/* C：扫不到的伪块（点阵底纹）上的字放在白底小标签里，不和点混在一起 */
[data-theme="C"] .lf.na:not(.solid) .a,
[data-theme="C"] .lf.na:not(.solid) .b,
[data-theme="C"] .lf.na:not(.solid) .ln,
[data-theme="C"] .lf.na:not(.solid) .v {
  width: fit-content;
  max-width: 100%;
  box-sizing: border-box;
  padding: 1px 5px;
  border-radius: 5px;
  background: #fff;
  box-shadow: 0 0 0 1.5px #141412;
}
[data-theme="C"] .lf.na:not(.solid) .b {
  margin-top: 4px;
  opacity: 1;
}
[data-theme="C"] .lf.na:not(.solid) .in {
  inset: 8px 9px;
}
[data-theme="C"] .lf.na:not(.solid).lt .in {
  inset: 0 9px;
}

/* 量字的探针：不显示，只读它的计算样式 */
.probe {
  position: absolute;
  left: -9999px;
  top: 0;
  visibility: hidden;
  pointer-events: none;
}
.probe .lf {
  position: relative;
  width: 300px;
  height: 120px;
}
.tip {
  position: absolute;
  z-index: 1000;
  width: 236px;
  padding: 12px 14px;
  background: var(--surface);
  border-radius: var(--r);
  box-shadow: 0 16px 40px -12px rgba(0, 0, 0, 0.35), 0 0 0 1px var(--line);
  font-size: var(--fs-sub);
  pointer-events: none;
}
/* 钉住的卡片（整宽方块图里选中一块）：跟着块的目标位置走 */
.tip.pin {
  z-index: 999;
  transition:
    left var(--dur-3) var(--ease),
    top var(--dur-3) var(--ease),
    bottom var(--dur-3) var(--ease);
  animation: tip-in var(--dur-2) var(--ease);
}
@keyframes tip-in {
  from {
    opacity: 0;
    transform: translateY(3px);
  }
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
.tip .t1 .tn {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
/* 阶段 K：块在原地长大 / 缩小（位置、大小过渡）。位置写在 --x / --y / --w / --h 里（行内样式），
   新出现的块从 --x0 …（它最近的祖先原来的框里按比例映射的位置）淡入并跟着父块一起长出来 */
.tm-tile {
  left: var(--x);
  top: var(--y);
  width: var(--w);
  height: var(--h);
  box-shadow: var(--sh, none);
  /* 块是绝对定位、尺寸写死的：每一帧的重排 / 重绘只在块自己里面（几百块同时过渡也不拖慢） */
  contain: size layout paint;
  /* 背景色不过渡：容器 ↔ 叶子切换时直接换色（过渡会先闪一下白板） */
  transition:
    left var(--dur-3) var(--ease),
    top var(--dur-3) var(--ease),
    width var(--dur-3) var(--ease),
    height var(--dur-3) var(--ease),
    border-radius var(--dur-3) var(--ease),
    border-color var(--dur-1) var(--ease),
    box-shadow var(--dur-1) var(--ease),
    filter var(--dur-1) var(--ease),
    opacity var(--dur-2) var(--ease) var(--fd, 0ms);
}
/* 叶子变成容器：底色从叶子的颜色淡到容器的白底（同时它的子块淡入），不会先闪一块空白板；
   容器变成叶子走上面的规则，直接换色 */
.tm-tile.g:not(.gone) {
  transition:
    left var(--dur-3) var(--ease),
    top var(--dur-3) var(--ease),
    width var(--dur-3) var(--ease),
    height var(--dur-3) var(--ease),
    border-radius var(--dur-3) var(--ease),
    background-color var(--dur-2) var(--ease),
    border-color var(--dur-1) var(--ease),
    filter var(--dur-1) var(--ease),
    opacity var(--dur-2) var(--ease) var(--fd, 0ms);
}
/* 离场的块：跟着祖先 / 镜头移动的同时淡出，过一会儿才删 */
.tm-tile.gone {
  opacity: 0;
  pointer-events: none;
  transition:
    left var(--dur-3) var(--ease),
    top var(--dur-3) var(--ease),
    width var(--dur-3) var(--ease),
    height var(--dur-3) var(--ease),
    opacity var(--dur-2) var(--ease) var(--fd, 0ms);
}
@starting-style {
  .tm-tile {
    opacity: 0;
    left: var(--x0, var(--x));
    top: var(--y0, var(--y));
    width: var(--w0, var(--w));
    height: var(--h0, var(--h));
  }
}
.map.still .tm-tile {
  transition: none !important;
}
@media (prefers-reduced-motion: reduce) {
  .tm-tile {
    transition: none !important;
  }
}
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
/* 「认不出来」：字用 --ink2（以前 --ink3 压在斜纹上几乎看不清），斜纹对比也收一点 */
[data-theme="B"] .tm-tone.mix {
  background: #2e2823;
  color: var(--ink2);
}
/* 「认不出来」和「先别动」在 A / B 里只差一点明度，看不出来：和 C 一样给细斜条纹（只给 ≥ 24px 的块和图例色块）。
   条纹下面垫一层平均色：叶子变成容器时条纹（背景图）没法过渡，底色从这个颜色过渡到容器的底色，不会第 0 帧透出一块白板 */
[data-theme="A"] .tm-tone.mix:not(.solid) {
  background: repeating-linear-gradient(135deg, #e2ddd3 0 5px, #cdc6b9 5px 7px) #dcd7cc;
}
[data-theme="B"] .tm-tone.mix:not(.solid) {
  background: repeating-linear-gradient(135deg, #2a241f 0 5px, #3e362e 5px 7px) #302924;
}
[data-theme="B"] .tm-tone.na {
  color: var(--ink2);
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
  background: repeating-linear-gradient(135deg, #f4f0e6 0 5px, #e6e1d5 5px 10px) #ede8dd;
}
[data-theme="C"] .tm-tone.na:not(.solid) {
  background:
    radial-gradient(#141412 1px, transparent 1.3px) 0 0 / 8px 8px,
    #fff;
}
[data-theme="C"] .tm-tone.na.solid {
  background: #fff;
}
/* 平块（没有描边）：白色在米色底上像一道缝，改成中灰 */
[data-theme="C"] .tm-tone.na.flat {
  background: var(--keep);
}
</style>
