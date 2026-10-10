<script setup lang="ts">
// 方块图（阶段 F 改成 DOM 绘制，好让三套主题的描边 / 圆角 / 斜条纹全部由 CSS token 决定）。
// 阶段 I：每块的圆角 / 描边 / 投影按块大小收敛（treemap.ts 的 tileBox），间隙等数值来自 tokens.css 的 --tm-*。
// 阶段 J：块的配色类（.tm-tone.safe / review / keep / mix / na）不加 scoped，分析页的「图例」色块直接复用；
// 扫不到的伪块（无权限 / 系统保留 / 未归属）单独一种 na。
// 阶段 K：一张平面图。单击任意深度的文件夹 = 往里挖（展开链 = 根 → 它），它在原位置放大到父块的 60%
// 并显示更深的层级；单击文件 / 其他 / 伪块 = 选中；单击已展开容器的标题栏 = 收起到它的父级。
// - 每个容器的行 / 列结构固定（treemap.ts），块只沿原来的行 / 列伸缩：过渡里没有块横穿别的块；
// - pk 的内容区不够大（按像素）就把图的根下移一层：镜头推近（每块从旧位置按同一个比例映射过去），反方向是拉远；
// - 离场的块先跟着祖先（或镜头）移动并淡出，约 260 ms 后才删掉，不会一下子留出空白；
// - 块按 key 稳定排序、用 z-index 分层（DOM 顺序不变 → 不会被挪动 → CSS 过渡真的生效，块在原地长大）。
// 外观（阶段 K 第 2 轮）：链上的状态只用中性色（墨黑 / 灰），绝不用三档颜色；悬停 / 选中在三套主题里都一眼看得出；
// 键盘：整张图只有一块在 Tab 序列里，方向键按画面位置走，Enter / Space = 单击；链上容器的标题栏 = 「收起」按钮。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { ScanSummary, TreemapNode } from "../api";
import { isHardTag, loseTag } from "../labels";
import {
  bigEnough,
  canonPath,
  contentBox,
  layoutTreemap,
  mapBox,
  pickMapRoot,
  samePath,
  tileBox,
  type LayoutOptions,
  type Rect,
  type TileBox,
  type TileTokens,
  type TmInput,
  type TmItem,
} from "../treemap";
import {
  countLabel,
  countText,
  findTm,
  fitHead,
  headHit,
  hitOf,
  isUnder,
  navStep,
  navTargets,
  otherSig,
  pathInputs,
  pathKey,
  rootCandidates,
  tileNode,
  tipName,
  toInput,
  type MapHit,
  type NavDir,
} from "../tmtree";
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
const emit = defineEmits<{ hit: [h: MapHit, ev: { at: number; detail: number }]; unsel: [] }>();

const wrap = ref<HTMLDivElement | null>(null);
const size = ref({ w: 0, h: 0 });
/** 正在拖窗口大小：块直接跟着变，不做过渡（不然整张图拖着尾巴追窗口边） */
const still = ref(false);
let stillTimer: ReturnType<typeof setTimeout> | undefined;
let ghostTimer: ReturnType<typeof setTimeout> | undefined;
let ro: ResizeObserver | null = null;
/** 上一次窗口大小变化的时间：只有拖窗口时才不做过渡；图自己变大变小（窄屏下列表变短、切视图）照常过渡 */
let winResizedAt = -1e9;
const onWinResize = () => (winResizedAt = performance.now());
/** 系统设置了「减少动画」：不做过渡，离场的块直接删 */
const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
onMounted(() => {
  if (!wrap.value) return;
  const upd = () => {
    const w = wrap.value!.clientWidth;
    const h = wrap.value!.clientHeight;
    if (w === size.value.w && h === size.value.h) return;
    if (size.value.w && performance.now() - winResizedAt < 400) {
      still.value = true;
      clearTimeout(stillTimer);
      stillTimer = setTimeout(() => (still.value = false), 200);
    }
    size.value = { w, h };
  };
  upd();
  readTokens();
  ro = new ResizeObserver(upd);
  ro.observe(wrap.value);
  window.addEventListener("resize", onWinResize);
  document.addEventListener("mousedown", onDocDown, true);
  // 标题字体加载完以后重新量一次标题栏的字
  document.fonts?.ready.then(() => fontVer.value++);
});
onBeforeUnmount(() => {
  ro?.disconnect();
  clearTimeout(stillTimer);
  clearTimeout(ghostTimer);
  window.removeEventListener("resize", onWinResize);
  document.removeEventListener("mousedown", onDocDown, true);
});

// ── 设计令牌（tokens.css 的 --tm-*）：间隙、圆角上限、描边、投影、最小块都由主题决定，切主题时重读 ──
const tk = ref({ gap: 3, r: 3, bw: 0, shadow: 0, gR: 4, gBw: 1, head: 26, min: 12, heading: "sans-serif", ui: "sans-serif" });
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
    min: px("--tm-min", 12),
    heading: cs.getPropertyValue("--heading").trim() || "sans-serif",
    ui: cs.getPropertyValue("--ui").trim() || "sans-serif",
  };
  measured.clear();
  fontVer.value++;
}
watch(theme, () => nextTick(readTokens));

// ── 量字（canvas）：标题栏放不放得下名字 + 大小 ──
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
    if (measured.size > 4000) measured.clear();
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
/** 小于 16px 画平块；小于 48px 不画投影；小于 24px 斜条纹改纯色 */
const FLAT_PX = 16;
const SHADOW_MIN_PX = 48;
const PATTERN_MIN_PX = 24;
/** 离场的块淡出后多久删掉（ms）：比位置过渡（--dur-3 = 260ms）稍长 */
const GHOST_MS = 280;
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
 * 两层（根的子项 + 孙项）；展开链上的块下面另外再画两层（子项 + 孙项）
 */
const opt = computed<LayoutOptions>(() => {
  const t = tk.value;
  const G = t.gap;
  return { minPx: t.min, maxDepth: 2, boostDepth: 2, headerPx: t.head, pad: t.gBw + G, padEnd: G + t.shadow + t.gBw, minContainerPx: 90 };
});

/** 以 rootPath 为图的根铺一次（参照矩形从扫描根沿原始的层走下来：缩放前后同一套行 / 列结构） */
function layoutFor(rootPath: string): { root: string; items: TmItem[] } | null {
  const d = props.data;
  if (!d) return null;
  const node = findTm(d, rootPath);
  if (!node) return null;
  const scan = samePath(node.path, d.path);
  const input = toInput(node, props.chain, { maxDepth: 2, boostDepth: 2, extra: scan ? extras.value : [] });
  const fr = frame.value;
  let canon = fr;
  if (!scan) {
    const nodes = pathInputs(d, node.path, extras.value);
    if (nodes) canon = canonPath(nodes, fr, opt.value);
  }
  return { root: node.path, items: layoutTreemap(input, fr, opt.value, canon, fr) };
}

const pk = computed(() => props.chain[props.chain.length - 1] ?? null);
/** 最深的展开块（pk）在图上的那一块 */
function pkTile(items: readonly TmItem[]): TmItem | null {
  const p = pk.value;
  return p ? (items.find((it) => it.chain && samePath(tileNode(it)?.path, p)) ?? null) : null;
}

/**
 * 这一次画什么：从扫描根开始试，pk 的内容区不够大（bigEnough，按像素）就把图的根下移一层（链在图上的下一层），
 * 直到够大或根 = pk 的父级。
 */
const laid = computed<{ root: string; items: TmItem[] } | null>(() => {
  const d = props.data;
  const { w, h } = size.value;
  if (!d || w < 20 || h < 20) return null;
  const cands = props.chain.length ? rootCandidates(d, props.chain) : [d.path];
  const o = opt.value;
  const fits = (r: { items: TmItem[] }) => {
    const t = pkTile(r.items);
    return !!t && bigEnough(contentBox(t.rect, o), { w, h });
  };
  return pickMapRoot(cands, layoutFor, fits)?.result ?? null;
});
const items = computed<TmItem[]>(() => laid.value?.items ?? []);
const byKey = computed(() => new Map(items.value.map((it) => [it.key, it])));

/** 最深的展开块（pk 在图上的那一块；合并块也算；pk 画不出来时退到最深的那个链上的块） */
const pkKey = computed(() => {
  const t = pkTile(items.value);
  if (t) return t.key;
  let best: TmItem | null = null;
  for (const it of items.value) if (it.chain && (!best || it.depth > best.depth)) best = it;
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
 * 最深的展开容器（pk）另有自己的样式（标题栏墨黑实底），选中它里面的文件时不变。
 */
const curKey = computed(() => (selOk.value ? props.selKey : pkKey.value));
// 选中的「其他」被拆开了（它所在的文件夹重新取数 / 放大后多露出几块，项数一变它就是另一块了）：不再保留选中
watch([items, () => props.selKey, () => props.selSig], () => {
  const k = props.selKey;
  if (!k || !k.includes("|其他")) return;
  const it = byKey.value.get(k);
  if (!it || (props.selSig && otherSig(it) !== props.selSig)) emit("unsel");
});

// ── 画出来的块：活的块 + 正在离场的块（ghost），按 key 排好 ──
type Box = { x: number; y: number; w: number; h: number };
interface Tile {
  key: string;
  /** 元素的身份（v-for 的 key）：平时 = key；某块要换位置又会横穿别的块时换一个新身份（旧的淡出、新的淡入） */
  rk: string;
  it: TmItem;
  /** 这一次的位置（离场的块 = 它要去的位置） */
  b: TileBox;
  /** 决定写什么字的框（离场的块沿用离场前的，字不会突然变） */
  lb: TileBox;
  /** 新出现的块从哪里长出来（null = 原地淡入） */
  from: Box | null;
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
/** 每个 key 现在用的元素身份 */
const rkeyOf = new Map<string, string>();
let rgen = 0;
/** 离场的块到时间了：重算一次（把它们删掉） */
const tick = ref(0);
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

const zOf = (it: TmItem) => it.depth * 2 + (it.container ? 0 : 1);
const boxOf = (b: TileBox): Box => ({ x: b.x, y: b.y, w: b.w, h: b.h });
const nodePath = (it: TmItem) => tileNode(it)?.path ?? null;
/** 块的内容区（子块铺在这里）：容器扣掉标题栏和内边距；叶子就是它自己 */
function inner(b: Box, container: boolean): Box {
  if (!container) return b;
  const t = tk.value;
  return { x: b.x + t.gBw + t.gap, y: b.y + t.head, w: Math.max(1, b.w - t.gap - 2 * t.gBw), h: Math.max(1, b.h - t.head - t.gBw) };
}
/** 两块之间有没有一个分隔方向（左右 / 上下）在前后两次都成立：有 → 线性过渡的每一帧都不重叠 */
function keepsApart(a0: Box, b0: Box, a1: Box, b1: Box): boolean {
  const s = (a: Box, b: Box) => [a.x + a.w <= b.x + 1, b.x + b.w <= a.x + 1, a.y + a.h <= b.y + 1, b.y + b.h <= a.y + 1];
  const p = s(a0, b0);
  const q = s(a1, b1);
  return p.some((v, i) => v && q[i]);
}

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
    return { key: it.key, rk: rkeyOf.get(it.key) ?? it.key, it, b, lb: b, from: null, gone: false, z: zOf(it), until: 0 };
  });
  const done = () => {
    for (const x of live) rkeyOf.set(x.key, x.rk);
  };
  if (instant) {
    done();
    return live.sort(byTile);
  }
  const fresh = new Map(live.map((x) => [x.key, x]));
  const boxes = readBoxes();
  /** 上一次的块现在在屏幕上的位置（过渡进行到一半时是中间的位置） */
  const cur = (s: Tile) => boxes.get(s.rk) ?? boxOf(s.b);
  const was = (k: string | null) => (k ? (prevLive.get(k) ?? null) : null);

  // 镜头：图的根变了。cam = 同一块区域在旧图 / 新图里的框（推近：新根原来的内容区 ↔ 整张图；拉远反过来）
  let cam: { old: Box; now: Box } | null = null;
  const fr = frame.value;
  if (prevRoot && !samePath(prevRoot, L.root)) {
    // 另一边的图上那一块没画出来（太深）时，用画出来的最深的那个祖先（例如 FileRepository 没画出来就用 System32）
    const deepest = (list: Iterable<Tile>, target: string, above: string) => {
      let best: Tile | null = null;
      let bestLen = -1;
      for (const x of list) {
        const p = nodePath(x.it);
        if (!p || !(samePath(p, target) || isUnder(target, p)) || !isUnder(p, above)) continue;
        if (p.length > bestLen) {
          best = x;
          bestLen = p.length;
        }
      }
      return best;
    };
    if (isUnder(L.root, prevRoot)) {
      const o = deepest(prevLive.values(), L.root, prevRoot);
      if (o) cam = { old: inner(cur(o), o.it.container && samePath(nodePath(o.it), L.root)), now: fr };
    } else if (isUnder(prevRoot, L.root)) {
      const n = deepest(live, prevRoot, L.root);
      if (n) cam = { old: fr, now: inner(boxOf(n.b), n.it.container && samePath(nodePath(n.it), prevRoot)) };
    }
  }

  // 安全网：同一个父块下、前后都在的两块，必须有一个分隔方向前后都成立，不然过渡里它们会横穿
  // （数据变细、某块换了行等）。这时小的那块换一个新身份：旧的跟着父块淡出，新的从父块里淡入。
  const groups = new Map<string, Tile[]>();
  for (const x of live) {
    const s = was(x.key);
    if (!s || s.it.parentKey !== x.it.parentKey) continue;
    const g = x.it.parentKey ?? "";
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g)!.push(x);
  }
  const jump = new Set<string>();
  for (const g of groups.values()) {
    for (let i = 0; i < g.length; i++) {
      for (let j = i + 1; j < g.length; j++) {
        const a = g[i];
        const b = g[j];
        if (jump.has(a.key) || jump.has(b.key)) continue;
        if (!keepsApart(cur(was(a.key)!), cur(was(b.key)!), boxOf(a.b), boxOf(b.b))) jump.add(a.b.w * a.b.h < b.b.w * b.b.h ? a.key : b.key);
      }
    }
  }
  for (const k of jump) fresh.get(k)!.rk = `${k}#${++rgen}`;
  /** 前后都在、而且没换身份的块 */
  const stays = (k: string | null) => !!k && !!was(k) && fresh.has(k) && !jump.has(k);

  // 新出现的块：找一个前后都在的祖先，按比例映射进那个祖先现在的内容区，从那里跟着它一起长到现在的位置；
  // 没有这样的祖先（图的根变了）就按镜头映射（推近时从新根原来的那一小块里长出来，拉远时从画面外移进来）
  const ghostRks = new Set(prevGhosts.map((g) => g.rk));
  for (const x of live) {
    if (stays(x.key) || ghostRks.has(x.rk)) continue; // 同一个元素：CSS 过渡从它现在的样子接着走
    let pk = x.it.parentKey;
    while (pk && !stays(pk)) pk = fresh.get(pk)?.it.parentKey ?? null;
    if (pk) {
      const s = was(pk)!;
      const n = fresh.get(pk)!;
      x.from = mapBox(boxOf(x.b), inner(boxOf(n.b), n.it.container), inner(cur(s), s.it.container));
    } else if (cam) x.from = mapBox(boxOf(x.b), cam.now, cam.old);
  }

  // 离场的块（包括换了身份的旧元素）：跟着它前后都在的最近祖先移动（容器变成叶子时它的子块跟着缩回去），
  // 没有就跟着镜头移出画面；同时淡出，过一会儿才删
  const out = live.slice();
  const liveRks = new Set(live.map((x) => x.rk));
  for (const g of prevGhosts) if (g.until > now && !liveRks.has(g.rk)) out.push(g);
  for (const [k, s] of prevLive) {
    if (liveRks.has(s.rk)) continue;
    let pk = s.it.parentKey;
    while (pk && !stays(pk)) pk = prevLive.get(pk)?.it.parentKey ?? null;
    const c = cur(s);
    let to: Box = c;
    if (pk) {
      const a = was(pk)!;
      const n = fresh.get(pk)!;
      to = mapBox(c, inner(cur(a), a.it.container), inner(boxOf(n.b), n.it.container));
    } else if (cam) to = mapBox(c, cam.old, cam.now);
    void k;
    // 跟着镜头移出画面的块（新根以外的）垫在最底下；跟着祖先走的保持原来的层次（不会被父块的底色盖住）
    out.push({ ...s, b: { ...s.b, ...to }, from: null, gone: true, z: !pk && cam ? Math.min(s.z, 1) : s.z, until: now + GHOST_MS });
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
    clearTimeout(ghostTimer);
    let next = Infinity;
    for (const x of prevGhosts) next = Math.min(next, x.until);
    if (Number.isFinite(next)) ghostTimer = setTimeout(() => tick.value++, Math.max(0, next - performance.now()) + 16);
  },
  { flush: "post" },
);

/** 链上的容器：标题栏是「收起」按钮（键盘的焦点也落在标题栏上） */
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
  const m = { name: (s: string) => textW(f.name, s), size: (s: string) => textW(f.size, s), ell: textW(f.ell, "…"), gap: 6 };
  const out = new Map<string, ReturnType<typeof fitHead>>();
  for (const x of tiles.value) {
    if (!x.it.container) continue;
    const isPk = pkKey.value === x.key;
    const bw = isPk ? (theme.value === "C" ? 3 : 2) : t.gBw;
    // 标题栏左右内边距 8；链上的有收起箭头（14 + 间隔 6）；pk 留出加载小点的位置（6 + 6）
    let avail = x.lb.w - 2 * bw - 16 - 2;
    if (x.it.chain) avail -= 20;
    if (isPk) avail -= 12;
    out.set(x.rk, fitHead(x.it.name, full(x.it.value), avail, m));
  }
  return out;
});

/** 单击 / Enter / Space：detail = 鼠标连击次数（系统的双击判定；键盘 = 0） */
function tapInfo(e: Event) {
  return { at: e.timeStamp, detail: e instanceof MouseEvent ? e.detail : 0 };
}
function onTap(x: Tile, e: Event) {
  if (x.gone) return;
  const h = hitOf(x.it, byKey.value, laid.value?.root ?? null);
  const info = tapInfo(e);
  remember(x.key);
  // 没有列表时，选中一块就把说明卡片钉在它旁边；下一次单击（双击的第二下除外）收起
  if (!(info.detail >= 2 && pinKey.value === x.key)) pinKey.value = props.pin && h.kind === "leaf" && pinKey.value !== x.key ? x.key : null;
  emit("hit", h, info);
}
function onHead(x: Tile, e: Event) {
  if (x.gone) return;
  const h = headHit(x.it);
  remember(x.key);
  pinKey.value = null;
  if (h) emit("hit", h, tapInfo(e));
}

// ── 悬停卡片 / 钉住的卡片 ──
const hover = ref<TmItem | null>(null);
const tipPos = ref({ x: 0, y: 0 });
const TIP_W = 236;
function onMove(e: MouseEvent, x: Tile) {
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
// 布局变了（展开 / 收起）：悬停的那块可能已经不在原来的位置，等鼠标再动
watch(items, () => (hover.value = null));

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
    size: sizeParts(it.value).join(" "),
    tone: tn,
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

function label(b: TileBox): "full" | "size" | "vert" | "line" | "none" {
  if (b.flat) return "none";
  const { w, h } = b;
  if (w > 62 && h > 40) return "full";
  // 被挤成细高条的块（展开旁边的块时常见）：名字竖着写
  if (w >= 24 && h >= 96 && h >= w * 2.5) return "vert";
  // 被挤成扁长条的块：名字和大小写成一行
  if (w >= 96 && h >= 17) return "line";
  // 内边距 7 + 描边 2 + 一行字约 16px：低于 30px 的块写数字会被截掉半行，干脆不写
  if (w > 40 && h >= 30) return "size";
  return "none";
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
</script>

<template>
  <div
    ref="wrap"
    class="map"
    :class="{ busy: loading, still }"
    @mouseleave="hover = null"
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
        x.lb,
        x.from,
        x.gone,
        x.z,
        x.it,
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
          ? ['g', { on: x.it.chain && pkKey !== x.key, pk: pkKey === x.key, wait: loading && pkKey === x.key }]
          : [
              'lf',
              'tm-tone',
              tone(x.it),
              {
                big: x.lb.w > 150 && x.lb.h > 70,
                flat: x.lb.flat,
                bare: label(x.lb) === 'none',
                vt: label(x.lb) === 'vert',
                lt: label(x.lb) === 'line',
                solid: !x.lb.pattern,
                hov: hover?.key === x.key,
                sel: curKey === x.key,
              },
            ],
        { gone: x.gone },
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
        v-if="x.it.container"
        class="gh"
        :role="headTarget(x) && !x.gone ? 'button' : undefined"
        :tabindex="headTarget(x) && !x.gone ? (rovingKey === x.key ? 0 : -1) : undefined"
        :aria-label="headTarget(x) && !x.gone ? '收起 ' + x.it.name : undefined"
        :aria-expanded="headTarget(x) && !x.gone ? 'true' : undefined"
        :title="heads.get(x.rk)?.cut || heads.get(x.rk)?.name === '' ? x.it.name + '  ' + full(x.it.value) : undefined"
        @click.stop="onHead(x, $event)"
        @keydown.enter.stop.prevent="onHead(x, $event)"
        @keydown.space.stop.prevent="onHead(x, $event)"
      >
        <span v-if="heads.get(x.rk)?.name !== ''" class="gn">{{ heads.get(x.rk)?.name ?? x.it.name }}<span v-if="heads.get(x.rk)?.cut" class="el">…</span></span>
        <i v-if="heads.get(x.rk)?.size !== false" class="num">{{ full(x.it.value) }}</i>
        <svg v-if="x.it.chain" class="up" viewBox="0 0 24 24" aria-hidden="true"><path d="m6 15 6-6 6 6" /></svg>
      </div>
      <!-- 字放在里面一层（绝对定位、四周留白）：块自己没有内边距，过渡里块再小也不会被内边距撑大、压到邻块 -->
      <span v-else-if="label(x.lb) !== 'none'" class="in">
        <template v-if="label(x.lb) === 'full'">
          <span v-if="countLabel(x.it)" class="a cnt"
            ><span class="c">{{ countLabel(x.it)!.num + " " + countLabel(x.it)!.post }}</span><span class="p">{{ countLabel(x.it)!.pre + " " }}</span></span
          >
          <span v-else class="a">{{ x.it.name }}</span>
          <span class="b num">{{ full(x.it.value) }}</span>
        </template>
        <span v-else-if="label(x.lb) === 'size'" class="b num">{{ short(x.it.value) }}</span>
        <span v-else-if="label(x.lb) === 'vert'" class="v">{{ countLabel(x.it) ? countText(countLabel(x.it)!) : x.it.name }}<i class="num">{{ short(x.it.value) }}</i></span>
        <span v-else class="ln"
          ><span v-if="countLabel(x.it)" class="n cnt"
            ><span class="c">{{ countLabel(x.it)!.num + " " + countLabel(x.it)!.post }}</span><span class="p">{{ countLabel(x.it)!.pre + " " }}</span></span
          ><span v-else class="n">{{ x.it.name }}</span><i class="num">{{ full(x.it.value) }}</i></span
        >
      </span>
    </div>
    <div v-if="tip" class="tip" :style="{ left: tipPos.x + 'px', top: tipPos.y + 'px' }">
      <div class="t1"><i class="dot" :class="tip.tone"></i><span class="tn">{{ tip.name }}</span><span class="s num">{{ tip.size }}</span></div>
      <div v-if="tip.what" class="t2">{{ tip.what }}</div>
      <div v-if="tip.loseText" class="t3"><span class="tag" :class="{ hard: isHardTag(tip.lose) }">{{ tip.loseText }}</span></div>
    </div>
    <div v-if="pinTip" class="tip pin" :style="pinTip.pos" role="status">
      <div class="t1"><i class="dot" :class="pinTip.tone"></i><span class="tn">{{ pinTip.name }}</span><span class="s num">{{ pinTip.size }}</span></div>
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
.g .gh {
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
    color var(--dur-1) var(--ease);
}
.g .gh:focus-visible {
  outline: 2px solid currentColor;
  outline-offset: -4px;
}
/* 名字的截断由脚本按宽度算好（名字露不出 6 个字时不写大小）；省略号用界面字体的「…」，不用标题字体里居中的「···」 */
.g .gh .gn {
  min-width: 0;
  overflow: hidden;
  text-overflow: clip;
}
.g .gh .gn .el {
  font-family: var(--ui);
}
.g .gh i {
  flex: none;
  font-style: normal;
  color: var(--ink3);
}
.g .gh .up {
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
.g.pk .gh {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.g.on .gh i,
.g.pk .gh i {
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
[data-theme="A"] .g.on .gh {
  background: var(--sunk);
}
[data-theme="A"] .g.on .gh:hover {
  background: #e2ded5;
}
[data-theme="A"] .g.pk .gh:hover {
  background: var(--hover);
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
[data-theme="B"] .g.on .gh {
  background: #221d18;
}
[data-theme="B"] .g.pk {
  border-color: #d9cfc1;
}
[data-theme="B"] .g.pk .gh {
  background: #2b251f;
}
[data-theme="B"] .g.on .gh:hover,
[data-theme="B"] .g.pk .gh:hover {
  background: #342d26;
}
/* C 卡通：墨线容器。悬停：标题栏垫米色；链上的祖先：8% 墨色浅灰底 + 下面一道墨线；最深那个（pk）：墨黑实底白字 + 3px 墨框 */
[data-theme="C"] .g {
  border-color: #141412;
  background: #fff;
}
[data-theme="C"] .g .gh {
  font-family: var(--heading);
  font-weight: 700;
  color: #141412;
  font-size: 13px;
}
[data-theme="C"] .g .gh i {
  font-family: var(--ui);
  font-size: 12px;
  font-weight: 600;
  color: #141412;
  opacity: 0.6;
}
[data-theme="C"] .g:not(.on):not(.pk):hover .gh {
  background: var(--bg);
}
[data-theme="C"] .g.on .gh {
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
[data-theme="C"] .g.pk .gh {
  background: #141412;
  color: #fff;
}
[data-theme="C"] .g.pk .gh i {
  color: #fff;
  opacity: 0.72;
}
[data-theme="C"] .g.pk .gh .up {
  opacity: 0.9;
}
[data-theme="C"] .g.pk .gh:hover {
  background: #33332e;
}
.lf {
  overflow: hidden;
  font-size: 12px;
  line-height: 1.3;
}
/* 字的那一层：四周留白用 inset（不是块的 padding），块的大小永远 = 布局给的格子 */
.lf .in {
  position: absolute;
  inset: 7px 8px;
  overflow: hidden;
}
.lf .a {
  display: block;
  font-weight: var(--w-med);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 「其他 410 项」「其余 2380 个」：放不下时整个前缀不写（不会截成「其 58 项」），数字保住。
   DOM 里数字在前、前缀在后，从右往左排、放不下的前缀换到第二行（被裁掉），整行靠左 */
.lf .cnt {
  display: flex;
  flex-direction: row-reverse;
  flex-wrap: wrap;
  justify-content: flex-end;
  height: 1lh;
  overflow: hidden;
}
.lf .cnt > span {
  flex: none;
  white-space: pre;
}
.lf .b {
  display: block;
  opacity: 0.8;
  white-space: nowrap;
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
  text-overflow: ellipsis;
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
.lf.vt .in {
  inset: 8px 4px;
}
/* 扁长条：一行字，垂直居中 */
.lf.lt .in {
  inset: 0 8px;
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
  text-overflow: ellipsis;
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
/* 悬停：A / B 一圈描边 + 轻微提亮；C 在墨框里面加一圈白（卡通的“高光”） */
.lf.hov {
  outline: 2px solid rgba(28, 27, 24, 0.55);
  outline-offset: -2px;
  filter: brightness(1.04);
}
[data-theme="B"] .lf.hov {
  outline-color: rgba(243, 236, 226, 0.6);
  filter: brightness(1.14);
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
}
[data-theme="B"] .lf.sel {
  outline-color: #f3ece2;
  box-shadow: 0 0 0 1px #0c0a08;
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
    opacity var(--dur-2) var(--ease);
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
    opacity var(--dur-2) var(--ease);
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
    opacity var(--dur-2) var(--ease);
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
/* 「认不出来」和「先别动」在 A / B 里只差一点明度，看不出来：和 C 一样给细斜条纹（只给 ≥ 24px 的块和图例色块） */
[data-theme="A"] .tm-tone.mix:not(.solid) {
  background: repeating-linear-gradient(135deg, #e2ddd3 0 5px, #cdc6b9 5px 7px);
}
[data-theme="B"] .tm-tone.mix:not(.solid) {
  background: repeating-linear-gradient(135deg, #2a241f 0 5px, #3e362e 5px 7px);
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
/* 平块（没有描边）：白色在米色底上像一道缝，改成中灰 */
[data-theme="C"] .tm-tone.na.flat {
  background: var(--keep);
}
</style>
