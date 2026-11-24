<script setup lang="ts">
// 方块图（阶段 F 改成 DOM 绘制，好让三套主题的描边 / 圆角 / 斜条纹全部由 CSS token 决定）。
// 阶段 I：每块的圆角 / 描边 / 投影按块大小收敛（treemap.ts 的 tileBox），间隙等数值来自 tokens.css 的 --tm-*。
// 阶段 J：块的配色类（.tm-tone.safe / review / keep / mix / na）不加 scoped，分析页的「图例」色块直接复用；
// 扫不到的伪块（无权限 / 系统保留 / 未归属）单独一种 na。
// 阶段 K：一张平面图。单击任意深度的文件夹 = 往里挖（展开链 = 根 → 它），它在原位置放大到父块的 60%
// 并显示更深的层级；单击文件 / 其他 / 伪块 = 选中；单击已展开容器的标题栏 = 收起到它的父级。
// - 每个容器的行 / 列结构固定（treemap.ts），块只沿原来的行 / 列伸缩：过渡里没有块横穿别的块；伸缩后太细的行并成「其他」；
// - pk 的内容区不够大（按像素）就把图的根下移一层：镜头推近（每块从旧位置按同一个比例映射过去），反方向是拉远；
//   缩放后图的根画成铺满整张图的容器，顶上一条和链上祖先同样式的标题栏（单击 = 收起这个根，镜头拉远；阶段 L 以前是「收起到这一级」）；
// - 怎么动由 tmmotion.ts 算：离场的块先跟着祖先（或镜头）移动并淡出，压在它要缩进去的那一块上面，约 230 ms 后才删；
// - 图自己变大变小（切视图 / 窗口）不逐块过渡：整张图淡出 → 新布局直接到位 → 淡入；
// - 块上的字：按实际量字决定写法（tmtree.ts fitLabel），过渡开始时字先淡出、过半后用最终的写法淡入；
// - 块按 key 稳定排序、用 z-index 分层（DOM 顺序不变 → 不会被挪动 → CSS 过渡真的生效，块在原地长大）。
// 外观（阶段 K 第 2 / 3 轮）：链上的状态只用中性色（墨黑 / 灰），绝不用三档颜色；悬停 / 选中 / 正在看的文件夹三种状态一眼分得清；
// 键盘：整张图只有一块在 Tab 序列里，方向键按画面位置走，Enter / Space = 单击；链上容器的标题栏 = 「收起」按钮。
// 阶段 L：链上容器标题栏右端画一颗看得见的「收起」按钮（图标 + 字；pk 是反色实心、祖先是描边的，安静一级；窄了只留图标、再窄不画，fitChainHead），
// 过渡里跟着标题栏的字一起淡出 / 淡入（hdcOld）；
// 悬停 / 钉住的卡片内容写全（不截断），挂在 <body> 上量出大小再放（tipplace.ts），靠图边也整张看得见；悬停卡片不压住钉住的卡片。
// 阶段 M：认不出来的块有 AI 推测时，卡片的说明写推测（写全），标签那一行多一个中性的「AI 推测」小标签（虚线框，不用三档颜色）。
// 阶段 M 收尾（用户定的「统一调淡 · 更淡」，分档的方案看上去像出了毛病，否掉了）：展开了东西时，不在展开链上、也不在最深的展开块（pk）
// 里面的块全部同一档调淡——块上盖一层页面底色（::after，A / C 0.62、B 0.5），C 的墨框和硬投影换成铅笔灰。链上的块（祖先容器、
// 它们的标题栏和「收起」按钮、缩放后的根标题条）、pk 里面的、选中的、悬停的、键盘聚焦的从不调淡；悬停调淡的容器的标题栏时，
// 它和它里面画出来的块一起亮起来。哪些块调淡由 tmtree.ts outsideChain 算（纯函数），存在 Tile.dim 上（离场的块沿用最后的值）。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { tmKey, type ScanSummary, type TreemapNode } from "../api";
import { shellNoteFor, tmNoteFor, type AiNotes } from "../ai";
import AiTag from "./AiTag.vue";
import { isHardTag, loseTag } from "../labels";
import { bigEnough, contentBox, pickMapRoot, samePath, tileBox, type HiddenSlot, type LayoutOptions, type Rect, type TileBox, type TileTokens, type TmInput, type TmItem } from "../treemap";
import {
  countLabel,
  fitChainHead,
  fitHead,
  fitLabel,
  headHit,
  hitOf,
  layoutMap,
  navStep,
  navTargets,
  otherSig,
  outsideChain,
  pathKey,
  rootCandidates,
  tileNode,
  tipName,
  type HeadPill,
  type LabelFit,
  type LabelMetrics,
  type MapHit,
  type NavDir,
} from "../tmtree";
import { planMotion, tileZ, type Box, type MotionTile } from "../tmmotion";
import { placeAtPointer, placeBeside, type PlaceBox } from "../tipplace";
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
  /** 阶段 M：这次运行里收到过的 AI 推测（pathKey → 推测）：数据里还没带上时用它补；换了它卡片就重算 */
  notes?: AiNotes;
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
/** 窗口大小（悬停 / 钉住的卡片夹在窗口里） */
const vp = ref({ w: typeof window === "undefined" ? 0 : window.innerWidth, h: typeof window === "undefined" ? 0 : window.innerHeight });
const onWinResize = () => {
  winResizedAt = performance.now();
  vp.value = { w: window.innerWidth, h: window.innerHeight };
};
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
  // 字体加载完以后重新量一次字；后来才按需加载的字体（例如 unicode-range 分出去的拉丁扩展，第一次出现 é 这类字时才下载）加载完也再量一次
  document.fonts?.ready.then(onFontsLoaded);
  document.fonts?.addEventListener("loadingdone", onFontsLoaded);
});
function onFontsLoaded() {
  measured.clear();
  labelCache.clear();
  fontVer.value++;
}
onBeforeUnmount(() => {
  ro?.disconnect();
  clearTimeout(stillTimer);
  clearTimeout(ghostTimer);
  clearTimeout(txTimer);
  clearTimeout(peekTimer);
  fadeAnim?.cancel();
  pinAnim?.cancel();
  window.removeEventListener("resize", onWinResize);
  document.removeEventListener("mousedown", onDocDown, true);
  document.fonts?.removeEventListener("loadingdone", onFontsLoaded);
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
  /** 上一次标题栏的状态（和 txOld 配对）：字先淡出时标题栏的写法（「收起」按钮在不在、给 pk 留的位置）照上一次的算，
   *  淡出的字不会被重新截断，按钮也跟着字一起淡出 / 过半再淡入；null = 没有上一次 */
  hdcOld: "pk" | "on" | "" | null;
  /** 新出现的块从哪里长出来（null = 原地淡入） */
  from: Box | null;
  /** 出现的地方还压着正在让开的兄弟：晚这么多 ms 再淡入（0 = 不晚；子块跟着父块） */
  late: number;
  /** 这块是什么时候出现的（late 从这时算起） */
  born: number;
  /** 正在离场 */
  gone: boolean;
  /** 展开时调淡：不在展开链上、也不在 pk 里面（outsideChain）；离场的块沿用最后的值 */
  dim: boolean;
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
const hdOf = (x: Tile): Tile["hdc"] => (fading(x) && x.hdcOld !== null ? x.hdcOld : x.hdc);

const tiles = computed<Tile[]>(() => {
  void tick.value;
  const L = laid.value;
  if (!L) return [];
  const t = tk.value;
  const leaf: TileTokens = { gap: t.gap, r: t.r, bw: t.bw, shadow: t.shadow, flatPx: FLAT_PX, shadowMinPx: SHADOW_MIN_PX, patternMinPx: PATTERN_MIN_PX };
  const group: TileTokens = { ...leaf, r: t.gR, bw: t.gBw, flatPx: 0 };
  const now = performance.now();
  const instant = still.value || reduced || !prevLive.size;
  // 标题栏的状态和块的类（.pk / .on）同一个判断：pk = pkKey 那一块（pk 画不出来时退到最深的链上的块，也算）
  const pkK = pkKey.value;
  const dims = outsideChain(L.items, pkK);
  // 投影只给顶层块（容器 / 顶层大块），里面的小块只有描边
  const live: Tile[] = L.items.map((it) => {
    const b = tileBox(it.rect, it.container ? group : leaf, it.depth === 1);
    const hdc: Tile["hdc"] = !it.container ? "" : it.key === pkK ? "pk" : it.chain ? "on" : "";
    return { key: it.key, rk: rkeyOf.get(it.key) ?? it.key, it, b, lbOld: null, txOld: null, fd: 0, hd: false, hdc, hdcOld: null, from: null, late: 0, born: 0, gone: false, dim: dims.has(it.key), z: zOf(it), until: 0 };
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
        // 标题栏照上一次画出来的样子淡出（上一次没有标题栏 = 不在链上）
        const shown = s.it.container || s.hd ? s.hdc : "";
        x.hdcOld = old ? (s.hdcOld ?? shown) : shown;
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
/**
 * 链上容器标题栏右端的「收起」按钮（阶段 L）：尺寸和下面 CSS 的 .pill 一致——左内边距 5 + 图标 12 + 间隔 3 + 「收起」+ 右内边距 7，
 * 只有图标的宽 20；和前面的内容隔 6（标题栏的 gap），按钮往右多伸 4（margin-right: -4px，贴近标题栏右边）
 */
const PILL = { padL: 5, icon: 12, gap: 3, padR: 7, iconOnly: 20, before: 6, pull: 4 };
const pillW = computed(() => {
  void fontVer.value;
  const t = tk.value;
  // C：标题字体（霞鹜文楷）700 12px；A / B：界面字体 600 11.5px（和 CSS 一致）
  const font = theme.value === "C" ? `700 12px ${t.heading}` : `600 11.5px ${t.ui}`;
  const full = Math.ceil(PILL.padL + PILL.icon + PILL.gap + textW(font, "收起") + PILL.padR);
  return { full: full + PILL.before - PILL.pull, icon: PILL.iconOnly + PILL.before - PILL.pull };
});
type HeadFit = ReturnType<typeof fitHead> & { pill: HeadPill };
const heads = computed(() => {
  const f = headFonts.value;
  const t = tk.value;
  const pw = pillW.value;
  void txShown.value;
  // 大小是等宽数字（tabular-nums）：canvas 量不出来，数字都当成「0」量（「1」比比例数字宽，按原样量会少算 1–2 px，名字被硬裁掉最后一个字）
  const num = (s: string) => s.replace(/\d/g, "0");
  const m = { name: (s: string) => textW(f.name, s), size: (s: string) => textW(f.size, num(s)), ell: textW(f.ell, "…"), gap: 6 };
  const out = new Map<string, HeadFit>();
  for (const x of tiles.value) {
    if (!x.it.container && !x.hd) continue;
    // 框、内容、链上的状态三样一起取：字正在淡出时全按上一次的（= 上一次最终的写法，一个字都不变），过半以后全按这一次的。
    // 不能混着用——旧的框配新的状态，会在淡出的字上给新出现的按钮让位置（字被重新截断），收起时按钮也会比字先一步消失
    const hs = hdOf(x);
    const isPk = hs === "pk";
    const bw = isPk ? (theme.value === "C" ? 3 : 2) : t.gBw;
    const tx = txIt(x);
    // 标题栏左右内边距 8；pk 留出加载小点的位置（6 + 6）；再留 2 px 量字误差
    let avail = lbOf(x).w - 2 * bw - 16 - 4;
    if (isPk) avail -= 12;
    // 链上的容器（单击标题栏 = 收起）：右端放「收起」按钮，窄了先换成只有图标的、再窄不放；刚变成叶子、标题栏正在淡掉的也照原样放
    out.set(x.rk, hs !== "" ? fitChainHead(tx.name, full(tx.size), avail, m, pw) : { ...fitHead(tx.name, full(tx.size), avail, m), pill: "none" });
  }
  return out;
});
const pillOf = (x: Tile): HeadPill => heads.value.get(x.rk)?.pill ?? "none";

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

// ── 悬停卡片 / 钉住的卡片（阶段 L：内容全部写出来、不截断；挂在 <body> 上、position: fixed，量出大小再放，永远整张看得见） ──
const hover = ref<TmItem | null>(null);
/** 鼠标的窗口坐标（普通变量，不是 ref：鼠标每动一下只挪卡片，不让整张图的模板重跑一遍） */
let mouse = { x: 0, y: 0 };
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
/** 悬停在调淡的容器的标题栏上：这块的 key（它和它里面画出来的块一起亮起来）；null = 没有 */
const litKey = ref<string | null>(null);
const lit = computed(() => {
  const k = litKey.value;
  const s = new Set<string>();
  if (!k) return s;
  const by = byKey.value;
  for (const it of items.value) {
    for (let p: string | null = it.key; p !== null; p = by.get(p)?.parentKey ?? null) {
      if (p === k) {
        s.add(it.key);
        break;
      }
    }
  }
  return s;
});
/** 这块现在调淡（悬停 / 选中 / 键盘聚焦由 CSS 另外排除） */
const dimOn = (x: Tile) => x.dim && !lit.value.has(x.key);
function onMove(e: MouseEvent, x: Tile) {
  peekAt(x);
  const it = x.it;
  const lk = it.container && x.dim && !x.gone && (e.target as Element | null)?.closest?.(".gh") ? x.key : null;
  if (litKey.value !== lk) litKey.value = lk;
  if (it.container || x.gone) {
    hover.value = null;
    return;
  }
  hover.value = it;
  // 只记坐标、直接挪卡片（不读布局）：卡片的大小换了内容才量一次（tipSz）
  mouse = { x: e.clientX, y: e.clientY };
  placeTip();
}
function onLeave() {
  hover.value = null;
  litKey.value = null;
  peekKey = null;
  clearTimeout(peekTimer);
}
// 布局变了（展开 / 收起）：悬停的那块可能已经不在原来的位置，等鼠标再动
watch(items, () => {
  hover.value = null;
  litKey.value = null;
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

/**
 * 卡片内容：名字、大小、一句说明（认不出来的：有 AI 推测写推测、加「AI 推测」标签；没有就写第一条线索，没有线索只写「认不出来」）。
 * 「其他」块不是一个真实的项，不带推测
 */
function tipOf(it: TmItem) {
  const n = tileNode(it);
  const tn = tone(it);
  const pseudo = (it.ref as { pseudo?: string } | undefined)?.pseudo;
  const cnt = countLabel(it);
  // 外壳合并块（key = 外层路径，n = 内层）：内层没问过时借链上问过的上层的推测（ai.ts shellNoteFor）
  const ai = it.kind === "other" || pseudo ? null : n && it.key !== tmKey(n) ? shellNoteFor(props.data, it.key, n, props.notes) : tmNoteFor(n, props.notes);
  let what = "";
  if (it.kind === "other") what = `${it.mergedCount} 个小项合计`;
  else if (pseudo) what = PSEUDO_TIP[pseudo] ?? "";
  else if (n?.kind === "Rest") what = cnt ? `${cnt.num} 个小文件合计` : n.what;
  else if (ai) what = ai.what.trim();
  else if (n && n.tier === "unknown") what = props.clues?.get(pathKey(n.path)) ?? "认不出来";
  else what = n?.what ?? "";
  if (!what && tn === "mix") what = "认不出来";
  return {
    name: tipName(it),
    size: sizeParts(it.size).join(" "),
    tone: tn,
    shell: shellText(it),
    what,
    ai: !!ai,
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
/** 钉住的卡片：内容 + 那块的目标位置（块在图里的坐标） */
const pinTip = computed(() => {
  const k = pinKey.value;
  if (!k || !props.pin || !selOk.value || props.selKey !== k) return null;
  const x = tiles.value.find((t) => t.key === k && !t.gone);
  if (!x) return null;
  return { key: k, b: boxOf(x.b), ...tipOf(x.it) };
});

/** 卡片的大小：内容（或主题 / 字体 / 窗口）变了才量一次——同一张卡片跟着鼠标走时不再读布局 */
type Measured = { of: string; w: number; h: number };
const tipEl = ref<HTMLElement | null>(null);
const pinEl = ref<HTMLElement | null>(null);
const tipSz = ref<Measured | null>(null);
const pinSz = ref<Measured | null>(null);
function sigOf(t: ReturnType<typeof tipOf> | null): string | null {
  if (!t) return null;
  return [t.name, t.size, t.tone, t.shell, t.what, t.ai ? "ai" : "", t.loseText ?? "", theme.value, fontVer.value, vp.value.w].join("\u0000");
}
const measure = (el: HTMLElement | null, of: string | null): Measured | null => (el && of ? { of, w: el.offsetWidth, h: el.offsetHeight } : null);
// flush: post = 新内容已经写进卡片、浏览器还没画：量完马上放好，第一帧就在对的位置（量好之前卡片是 visibility: hidden）
watch(
  () => sigOf(tip.value),
  (s) => {
    tipSz.value = measure(tipEl.value, s);
    placeTip();
  },
  { flush: "post" },
);
watch(() => (pinTip.value ? pinTip.value.key + "\u0000" + sigOf(pinTip.value) : null), (s) => (pinSz.value = measure(pinEl.value, s)), { flush: "post" });

/**
 * 悬停卡片：指针右下，放不下翻到左边 / 上边，夹在窗口里；钉住的卡片在旁边时换到不压住它的那一边。
 * 直接写元素的 left / top（模板里不绑 style）：鼠标每动一下只挪这一张卡片，不重跑整张图的模板。
 */
function placeTip() {
  const el = tipEl.value;
  if (!el) return;
  const s = tipSz.value;
  const ready = !!s && s.of === sigOf(tip.value);
  const pp = pinPlace.value;
  const avoid = pp && pp.ready ? { x: pp.x, y: pp.y, w: pp.w, h: pp.h } : null;
  const p = ready && s ? placeAtPointer(mouse, s, { x: 0, y: 0, w: vp.value.w, h: vp.value.h }, avoid) : { x: mouse.x + 14, y: mouse.y + 16 };
  el.style.left = Math.round(p.x) + "px";
  el.style.top = Math.round(p.y) + "px";
  el.style.visibility = ready ? "visible" : "hidden";
}
/**
 * 钉住的卡片：放在那块的旁边（tipplace.placeBeside），夹在图里；图比卡片还小时夹在窗口里。
 * 图在窗口里的位置在这里读（getBoundingClientRect）：它跟着块（tiles，图一变大小就重排）、卡片大小和窗口大小重算。
 * 图在窗口里挪动而大小不变时不会重算——现在不会发生（图在 flex 布局里，上面多一条横幅 / 换视图都会改它的大小）；
 * 以后要是有这种挪法（比如页面能滚动），在这里加一个依赖（滚动 / ResizeObserver 记下的位置）
 */
const pinPlace = computed(() => {
  const p = pinTip.value;
  const el = wrap.value;
  if (!p || !el) return null;
  const s = pinSz.value;
  const ready = !!s && s.of === p.key + "\u0000" + sigOf(p);
  const o = el.getBoundingClientRect();
  const b: PlaceBox = { x: o.left + p.b.x, y: o.top + p.b.y, w: p.b.w, h: p.b.h };
  const w = s?.w ?? 0;
  const h = s?.h ?? 0;
  const fitsMap = w + 12 <= o.width && h + 12 <= o.height;
  const area: PlaceBox = fitsMap ? { x: o.left, y: o.top, w: o.width, h: o.height } : { x: 0, y: 0, w: vp.value.w, h: vp.value.h };
  const pos = ready && s ? placeBeside(b, { w, h }, area) : { x: b.x + b.w + 10, y: b.y };
  return { key: p.key, ready, x: Math.round(pos.x), y: Math.round(pos.y), w, h };
});
const pinStyle = computed(() => {
  const p = pinPlace.value;
  return p ? { left: p.x + "px", top: p.y + "px", visibility: p.ready ? undefined : ("hidden" as const) } : {};
});
/** 钉住的块在过渡里换了位置：卡片跟着它的目标位置一起滑过去（和块的位置过渡同样的时长 / 缓动） */
let pinAnim: Animation | null = null;
watch(
  pinPlace,
  (now, before) => {
    const el = pinEl.value;
    if (!el || !now || !before || reduced || still.value || typeof el.animate !== "function") return;
    if (!now.ready || !before.ready || now.key !== before.key || now.w !== before.w || now.h !== before.h) return;
    if (Math.abs(now.x - before.x) < 1 && Math.abs(now.y - before.y) < 1) return;
    // 上一段还没滑完：从它现在画在哪里接着滑
    let from = { left: before.x + "px", top: before.y + "px" };
    if (pinAnim && pinAnim.playState === "running") {
      const cs = getComputedStyle(el);
      from = { left: cs.left, top: cs.top };
      pinAnim.cancel();
    }
    pinAnim = el.animate([from, { left: now.x + "px", top: now.y + "px" }], { duration: 260, easing: "cubic-bezier(0.2, 0.7, 0.2, 1)" });
  },
  { flush: "post" },
);
// 钉住的卡片换了位置 / 窗口变了：悬停卡片照新的情况重新放（不压住钉住的卡片）
watch([pinPlace, vp], () => placeTip(), { flush: "post" });

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
        dimOn(x),
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
        { gone: x.gone, txo: fading(x), dim: dimOn(x) },
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
        :aria-label="headTarget(x) && !x.gone ? '收起 ' + x.it.name : undefined"
        :aria-expanded="headTarget(x) && !x.gone ? 'true' : undefined"
        :title="headTitle(x)"
        @click.stop="onHead(x, $event)"
        @keydown.enter.stop.prevent="onHead(x, $event)"
        @keydown.space.stop.prevent="onHead(x, $event)"
      >
        <span v-if="heads.get(x.rk)?.name !== ''" class="gn">{{ heads.get(x.rk)?.name ?? txIt(x).name }}<span v-if="heads.get(x.rk)?.cut" class="el">…</span></span>
        <i v-if="heads.get(x.rk)?.size !== false" class="num">{{ full(txIt(x).size) }}</i>
        <!-- 「收起」按钮（整条标题栏都能点，这里只是让人一眼看出来）：两个往里收的箭头 + 「收起」；窄了只留图标、再窄不画 -->
        <span v-if="pillOf(x) !== 'none'" class="pill" :class="{ ico: pillOf(x) === 'icon' }">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14 4v6h6M14 10l7-7M4 14h6v6M3 21l7-7" /></svg>
          <b v-if="pillOf(x) === 'full'">收起</b>
        </span>
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
    <!-- 卡片挂在 <body> 上（position: fixed）：不被图的 overflow: hidden 裁掉，靠图边的块卡片也整张看得见 -->
    <Teleport to="body">
      <div v-if="tip" ref="tipEl" class="tip">
        <div class="t1"><i class="dot" :class="tip.tone"></i><span class="tn">{{ tip.name }}</span><span class="s num">{{ tip.size }}</span></div>
        <div v-if="tip.shell" class="t2 num">{{ tip.shell }}</div>
        <div v-if="tip.what" class="t2">{{ tip.what }}</div>
        <div v-if="tip.loseText || tip.ai" class="t3">
          <span v-if="tip.loseText" class="tag" :class="{ hard: isHardTag(tip.lose) }">{{ tip.loseText }}</span><AiTag v-if="tip.ai" />
        </div>
      </div>
      <div v-if="pinTip" :key="pinTip.key" ref="pinEl" class="tip pin" :style="pinStyle" role="status">
        <div class="t1"><i class="dot" :class="pinTip.tone"></i><span class="tn">{{ pinTip.name }}</span><span class="s num">{{ pinTip.size }}</span></div>
        <div v-if="pinTip.shell" class="t2 num">{{ pinTip.shell }}</div>
        <div v-if="pinTip.what" class="t2">{{ pinTip.what }}</div>
        <div v-if="pinTip.loseText || pinTip.ai" class="t3">
          <span v-if="pinTip.loseText" class="tag" :class="{ hard: isHardTag(pinTip.lose) }">{{ pinTip.loseText }}</span><AiTag v-if="pinTip.ai" />
        </div>
      </div>
    </Teleport>
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
/* 「收起」按钮（阶段 L）：链上容器标题栏的右端，图标 + 「收起」。整条标题栏都能点，按钮只是让人一眼看出来。
   只用中性色，绝不用三档颜色 / 强调色。分两级：
   - 最深那个（pk）最强：实底标题栏上一颗反色的实心按钮（墨色底 → 浅色按钮；B 的浅色底 → 深色按钮）；
   - 祖先（和缩放后的根条）安静一级：浅底 + 墨线描边的按钮（和「图例」这些按钮同一种样子），悬停 / 键盘聚焦标题栏时反过来变成墨色实心。
     链很深时右上角不会叠出一串一样的黑徽章，一眼看得出哪个是现在这一层。
   描边用 outline（画在按钮里面，不占尺寸）：尺寸和脚本里的 PILL 一致（量名字还剩多少地方用） */
.g .gh .pill,
.gh.out .pill {
  /* 定位 = 画在标题栏的焦点环上面（焦点环从按钮后面穿过去，见下面 :focus-visible） */
  position: relative;
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  height: 18px;
  margin-left: auto;
  margin-right: -4px;
  padding: 0 7px 0 5px;
  border-radius: var(--r-pill);
  background: var(--surface);
  color: var(--ink);
  outline: 1px solid var(--ink);
  outline-offset: -1px;
  font-family: var(--ui);
  font-size: 11.5px;
  font-weight: 600;
  line-height: 1;
  white-space: nowrap;
  /* opacity 和标题栏里别的字一样 110ms（过渡里跟着字一起淡出 / 淡入） */
  transition:
    opacity 110ms var(--ease),
    background-color var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease),
    outline-color var(--dur-1) var(--ease),
    box-shadow var(--dur-1) var(--ease),
    transform var(--dur-1) var(--ease);
}
.g .gh .pill.ico,
.gh.out .pill.ico {
  padding: 0 4px;
}
.gh .pill b {
  font-weight: inherit;
}
.gh .pill svg {
  flex: none;
  width: 12px;
  height: 12px;
  fill: none;
  stroke: currentColor;
  stroke-width: 2.6;
  stroke-linecap: round;
  stroke-linejoin: round;
}
/* 最深那个（pk）：实底标题栏上一颗反色的实心按钮（不要描边） */
.g.pk .gh .pill,
.gh.out.was-pk .pill {
  background: var(--surface);
  color: var(--ink);
  outline-color: transparent;
}
/* 悬停在标题栏任何地方（整条都是按钮）：祖先的按钮变成墨色实心，按钮外面亮一圈，图标往里一捏再弹回来（= 收起这个动作；停下来时图标是原大小，1x 屏上也清楚）。
   亮圈浓一些（标题栏悬停时底色也变深，太淡的圈会和它混在一起） */
.g .gh:hover .pill,
.g .gh:focus-visible .pill {
  background: var(--ink);
  color: var(--surface);
}
.g .gh:hover .pill {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--ink) 34%, transparent);
}
.g.pk .gh:hover .pill,
.g.pk .gh:focus-visible .pill {
  background: var(--surface);
  color: var(--ink);
}
.g.pk .gh:hover .pill {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--surface) 50%, transparent);
}
.g .gh:hover .pill svg {
  animation: pill-pinch 480ms;
}
/* 先收进去（缓入缓出），再带一点回弹弹回原大小 */
@keyframes pill-pinch {
  0% {
    animation-timing-function: cubic-bezier(0.4, 0, 0.6, 1);
  }
  35% {
    transform: scale(0.66);
    animation-timing-function: cubic-bezier(0.3, 1.6, 0.5, 1);
  }
}
.g .gh:active .pill {
  transform: translateY(1px);
}
/* 键盘焦点：标题栏的焦点环照旧在里面一圈（-4px，和别的标题栏一样，不贴着块的描边）；按钮压在环上面，
   外面描一圈它自己的字色（= 和标题栏底色同一侧的颜色），环在按钮两边断开，看上去是从按钮后面穿过去 */
.g .gh:focus-visible .pill,
.g.pk .gh:focus-visible .pill {
  box-shadow: 0 0 0 2px currentColor;
}
/* 减少动画：图标不捏、按下不位移（亮圈照常出现） */
@media (prefers-reduced-motion: reduce) {
  .g .gh:hover .pill svg {
    animation: none;
  }
  .g .gh:active .pill {
    transform: none;
  }
}
.g:hover .gh {
  color: var(--ink);
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
/* 有「收起」按钮时小点放在按钮前面（按钮始终贴着右边） */
.g.wait .gh:has(.pill)::after {
  order: 1;
  margin-left: auto;
}
.g.wait .gh .pill {
  order: 2;
  margin-left: 0;
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
[data-theme="C"] .g.pk .gh:hover {
  background: #33332e;
}
/* C：按钮的字用标题字体（和标题栏的名字、「图例」按钮一样），胶囊形；祖先的白按钮描边粗一点（墨线是 C 的样子） */
[data-theme="C"] .g .gh .pill,
[data-theme="C"] .gh.out .pill {
  font-family: var(--heading);
  font-size: 12px;
  font-weight: 700;
  outline-width: 1.5px;
  outline-offset: -1.5px;
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
/* !important：标题栏里各部分（名字 / 大小 / 「收起」按钮）自己的样式不能盖掉淡出（过渡仍然生效） */
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
/* 悬停 / 钉住的卡片（阶段 L）：挂在 <body> 上、position: fixed（位置由脚本量出大小后算好，写在 left / top 里）。
   内容全部写出来、不截断：名字可以折行（长文件名在任意处断开），说明多长写多长；宽度随内容在 236–300px 之间，高度自动。
   层级 50：压在标题栏（40）/ 图例弹层（20）上面；在提示横幅（60）、确认对话框（80）、A / B 的纸纹（200）下面 */
.tip {
  position: fixed;
  left: 0;
  top: 0;
  z-index: 50;
  width: max-content;
  min-width: 236px;
  max-width: min(300px, calc(100vw - 16px));
  padding: 12px 14px;
  background: var(--surface);
  color: var(--ink);
  border-radius: var(--r);
  box-shadow: 0 16px 40px -12px rgba(0, 0, 0, 0.35), 0 0 0 1px var(--line);
  font-size: var(--fs-sub);
  line-height: 1.4;
  pointer-events: none;
}
/* 悬停卡片：量好、放好之前看不见（位置由 placeTip 直接写在元素上，放好才写 visibility: visible） */
.tip:not(.pin) {
  visibility: hidden;
}
/* 钉住的卡片（整宽方块图里选中一块）：出现时轻轻淡入；块换位置时由脚本让它跟着滑过去 */
.tip.pin {
  z-index: 49;
  animation: tip-in var(--dur-2) var(--ease);
}
@keyframes tip-in {
  from {
    opacity: 0;
    transform: translateY(3px);
  }
}
@media (prefers-reduced-motion: reduce) {
  .tip.pin {
    animation: none;
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
/* 第一行：色点 + 名字（折行）+ 大小（不折行，对齐名字的第一行） */
.tip .t1 {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  font-weight: var(--w-semi);
  font-size: 14px;
  line-height: 20px;
  min-width: 0;
}
.tip .t1 .dot {
  margin-top: 6px;
}
[data-theme="C"] .tip .t1 .dot {
  margin-top: 5px;
}
.tip .t1 .tn {
  flex: 1 1 auto;
  min-width: 0;
  overflow-wrap: anywhere;
}
.tip .t1 .s {
  flex: none;
  margin-left: auto;
  padding-left: 4px;
  white-space: nowrap;
}
.tip .t2 {
  margin-top: 4px;
  color: var(--ink2);
  font-size: var(--fs-meta);
  line-height: 1.55;
  overflow-wrap: anywhere;
}
.tip .t3 {
  margin-top: 10px;
  display: flex;
  align-items: center;
  gap: 8px;
}
/* 后果标签：文字来自规则（用户规则也能写长），在卡片里可以折行、不伸出卡片；一行时高度和别处的标签一样 */
.tip .t3 .tag {
  flex: 0 1 auto;
  min-width: 0;
  height: auto;
  min-height: 20px;
  padding-top: 1px;
  padding-bottom: 1px;
  line-height: 1.35;
  white-space: normal;
  overflow-wrap: anywhere;
}
/* C 的标签是胶囊（999px）：折成几行时改成同样圆的圆角矩形（11px = 一行高的一半，一行时看上去和胶囊一样） */
[data-theme="C"] .tip .t3 .tag {
  min-height: 22px;
  border-radius: 11px;
}
/* 「AI 推测」：固定的四个字，不折行（尺寸见 base.css 的 .tag.ai） */
.tip .t3 .tag.ai {
  flex: none;
  min-height: 0;
  padding-top: 0;
  padding-bottom: 0;
  white-space: nowrap;
}
[data-theme="C"] .tip .t3 .tag.ai {
  min-height: 0;
  border-radius: 999px;
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
/* 展开时调淡（阶段 M 收尾，「统一调淡 · 更淡」）：不在展开链上、也不在 pk 里面的块（.dim，outsideChain）同一档——
   盖一层页面底色，底色、字、标题栏一起淡下去（A / C 0.62、B 0.5）；C 的墨框和硬投影换成铅笔灰（≈ #84827c）。
   层在字（.in / 标题栏）上面、描边下面（paint 包含把它裁在块里）；平时透明度 0，取消调淡时淡出。
   调淡晚 90 ms 再开始（鼠标扫过去不闪），取消立刻开始。选中的、悬停的叶子、键盘聚焦的不调淡（容器的悬停按标题栏算，脚本里另外处理） */
.tm-tile::after {
  content: "";
  position: absolute;
  inset: 0;
  z-index: 9;
  pointer-events: none;
  background: var(--bg);
  opacity: 0;
  transition: opacity var(--dur-2) var(--ease);
}
.tm-tile.dim:not(.sel, :focus-visible, .lf:hover)::after {
  opacity: 0.62;
  transition-delay: 90ms;
}
[data-theme="B"] .tm-tile.dim:not(.sel, :focus-visible, .lf:hover)::after {
  opacity: 0.5;
}
[data-theme="C"] .tm-tile.dim:not(.sel, :focus-visible, .lf:hover) {
  --stroke: color-mix(in srgb, #141412 50%, #f4f0e6);
  border-color: color-mix(in srgb, #141412 50%, #f4f0e6);
}
.map.still .tm-tile::after {
  transition: none;
}
@media (prefers-reduced-motion: reduce) {
  .tm-tile::after {
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
