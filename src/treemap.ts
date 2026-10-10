// 方块图（treemap）布局：squarified 算法（Bruls, Huizing, van Wijk 2000）+ 嵌套 + 小块合并。
// 纯函数、不碰 DOM，方便单元测试（src/treemap.test.ts）。

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Weighted<T> {
  value: number;
  data: T;
}

export interface Placed<T> {
  rect: Rect;
  data: T;
  value: number;
}

/** 一行里最差的长宽比（越接近 1 越好）。areas 已按面积换算。 */
function worst(row: number[], side: number): number {
  let sum = 0;
  let max = 0;
  let min = Infinity;
  for (const a of row) {
    sum += a;
    if (a > max) max = a;
    if (a < min) min = a;
  }
  if (sum <= 0 || side <= 0) return Infinity;
  const s2 = side * side;
  const sum2 = sum * sum;
  return Math.max((s2 * max) / sum2, sum2 / (s2 * min));
}

/** squarify 分出来的一行：哪几项（在列表里的下标），竖着一列（true）还是横着一行 */
interface Strip {
  from: number;
  to: number;
  column: boolean;
}

/** squarify 的分行：按面积依次贪心，行里最差长宽比不再变好就换行。只和 rect 的长宽比有关（等比缩放结果不变） */
function strips(values: number[], rect: Rect): Strip[] {
  const total = values.reduce((s, v) => s + v, 0);
  const out: Strip[] = [];
  if (!values.length || rect.w <= 0 || rect.h <= 0 || total <= 0) return out;
  const scale = (rect.w * rect.h) / total;
  const areas = values.map((v) => v * scale);
  let { w, h } = rect;
  let i = 0;
  while (i < values.length) {
    const side = Math.min(w, h);
    const row: number[] = [areas[i]];
    let j = i + 1;
    while (j < values.length) {
      const next = [...row, areas[j]];
      if (worst(next, side) <= worst(row, side)) {
        row.push(areas[j]);
        j++;
      } else break;
    }
    const rowSum = row.reduce((s, a) => s + a, 0);
    const column = w >= h;
    out.push({ from: i, to: j, column });
    if (column) w -= rowSum / h;
    else h -= rowSum / w;
    i = j;
  }
  return out;
}

/**
 * 按给定的分行把 values 铺进 rect（面积与 values 成正比）。分行固定、只换权重 / 换框时，每块都留在原来的行 / 列里，
 * 而且是仿射等变的：框按轴缩放，结果跟着同样缩放。
 * cut < rows.length：第 cut 行起不铺，剩下的那个矩形（正好是这些行原来占的地方）作为 tail 返回（画成「其他」）。
 */
function fillRows(values: number[], rows: Strip[], rect: Rect, cut = rows.length): { rects: (Rect | null)[]; tail: Rect | null } {
  const rects: (Rect | null)[] = values.map(() => null);
  let { x, y, w, h } = rect;
  let rest = values.reduce((s, v) => s + v, 0);
  for (let ri = 0; ri < Math.min(cut, rows.length); ri++) {
    const r = rows[ri];
    let sum = 0;
    for (let k = r.from; k < r.to; k++) sum += values[k];
    // 最后一行吃掉剩余空间，避免浮点累积误差留下缝
    const last = ri === rows.length - 1;
    const f = last || rest <= 0 ? 1 : sum / rest;
    if (r.column) {
      // 竖着放一列
      const cw = w * f;
      let cy = y;
      for (let k = r.from; k < r.to; k++) {
        const ch = k === r.to - 1 ? y + h - cy : sum > 0 ? (h * values[k]) / sum : 0;
        rects[k] = { x, y: cy, w: cw, h: ch };
        cy += ch;
      }
      x += cw;
      w -= cw;
    } else {
      const rh = h * f;
      let cx = x;
      for (let k = r.from; k < r.to; k++) {
        const cw = k === r.to - 1 ? x + w - cx : sum > 0 ? (w * values[k]) / sum : 0;
        rects[k] = { x: cx, y, w: cw, h: rh };
        cx += cw;
      }
      y += rh;
      h -= rh;
    }
    rest -= sum;
  }
  return { rects, tail: cut < rows.length ? { x, y, w, h } : null };
}

function placeList<T>(list: Weighted<T>[], values: number[], rows: Strip[], rect: Rect): Placed<T>[] {
  const { rects } = fillRows(values, rows, rect);
  const out: Placed<T>[] = [];
  list.forEach((it, k) => {
    const r = rects[k];
    if (r) out.push({ rect: r, data: it.data, value: it.value });
  });
  return out;
}

/**
 * squarified：把按值加权的项目铺满 rect，面积与值成正比，尽量接近正方形。
 * 值 <= 0 的项目被忽略。输出顺序 = 按值降序；keepOrder 时按调用方给的顺序铺。
 */
export function squarify<T>(items: Weighted<T>[], rect: Rect, keepOrder = false): Placed<T>[] {
  const list = items.filter((i) => i.value > 0);
  if (!keepOrder) list.sort((a, b) => b.value - a.value);
  const values = list.map((i) => i.value);
  return placeList(list, values, strips(values, rect), rect);
}

// ───── 嵌套布局 ─────

/** 布局输入：通用的树节点。 */
export interface TmInput {
  key: string;
  name: string;
  value: number;
  /** 块上写的大小（默认 = value）。外壳合并块：链走进内层时写内层的大小，面积仍按外层 */
  size?: number;
  children?: TmInput[];
  /** 没有展开发过来的子项合计（画进“其他”） */
  hidden?: number;
  hiddenCount?: number;
  /**
   * 阶段 K：展开链上的块。目标最小面积占比（占父块内容区，例如 0.6）；本来就更大就不动。
   * 每个父块最多一个这样的子项。它下面另外允许画 boostDepth 层。
   */
  boost?: number;
  /**
   * 外壳合并块（Users \ king）：原始的每一层（外层 → 内层；每层带全部子项的 key / 值即可）。
   * 算「基础结构」时沿原始的层走，所以合并不合并，内层子项的行 / 列结构都一样（不会因为合并方式变了而横穿）。
   */
  via?: TmInput[];
  /** 调用方挂的任意数据（原始节点等） */
  ref?: unknown;
}

export type TmItemKind = "node" | "other";

export interface TmItem {
  key: string;
  kind: TmItemKind;
  name: string;
  value: number;
  /** 块上写的大小（外壳合并块见 TmInput.size） */
  size: number;
  rect: Rect;
  /** 1 = 图的根下面那一层；0 = 自动缩放后图顶上的根标题条（整张图就是它的内容区） */
  depth: number;
  /** 有子块画在里面（画成带标题的容器） */
  container: boolean;
  /** “其他”块合并了几个子项 */
  mergedCount: number;
  /** “其他”块里合并了哪些子项（只有画得出来的那些有 key 的；过渡时它们从这块里分出来 / 缩进这块里） */
  merged?: string[];
  /** 在展开链上（输入带 boost） */
  chain: boolean;
  ref?: unknown;
  parentKey: string | null;
}

export interface LayoutOptions {
  /** 最短边小于这个像素就合并进“其他” */
  minPx: number;
  /** 第 2 层及更深的块用的最小边（默认 = minPx；C 主题墨框粗，小块写不下字，提高一点） */
  minPxDeep?: number;
  /** 最多嵌套几层（1 = 只画根的子项）；展开链上的块下面另算（boostDepth） */
  maxDepth: number;
  /** 展开链上的块（带 boost）下面最多再画几层（默认 2 = 子项和孙项） */
  boostDepth?: number;
  /** 容器标题栏高度 */
  headerPx: number;
  /** 容器内边距（左 / 上方标题栏之外的左边）：= 容器描边 + 间隙 */
  pad: number;
  /** 容器右 / 下内边距（默认 = pad）：= 间隙 + 容器描边 + 容器硬投影，让子块的外描边不被裁、投影不压子块 */
  padEnd?: number;
  /** 容器至少多大才展开子块 */
  minContainerPx: number;
}

export const DEFAULT_LAYOUT: LayoutOptions = { minPx: 6, maxDepth: 3, headerPx: 17, pad: 2, minContainerPx: 44 };

/** 展开链上的文件夹至少占父块内容区的这么多 */
export const BOOST = 0.6;

/** 深度为 depth 的块（1 = 图的根下面那一层）至少要这么大，小了就并进「其他」 */
export function minPxAt(opt: LayoutOptions, depth: number): number {
  return depth >= 2 && opt.minPxDeep ? Math.max(opt.minPx, opt.minPxDeep) : opt.minPx;
}

/** 容器的内容区（子块铺在这里）：扣掉标题栏和内边距 */
export function contentBox(r: Rect, opt: LayoutOptions): Rect {
  const pe = opt.padEnd ?? opt.pad;
  return { x: r.x + opt.pad, y: r.y + opt.headerPx, w: r.w - opt.pad - pe, h: r.h - opt.headerPx - pe };
}

/**
 * 嵌套布局：root 自己不出块，它的子项铺满 rect；子项里够大的继续往里铺（最多 maxDepth 层；
 * 展开链上带 boost 的块下面另外再画 boostDepth 层）。返回按绘制顺序（先父后子）排好的扁平列表。
 *
 * 阶段 K（结构稳定）：每个容器的行 / 列结构（squarify 的分行和顺序）只按数据和它的「基础参照矩形」算一次
 * （canonical：展开链为空时它的内容区；那时太小、只有放大以后才会画出子项的，用它放大时的形状），
 * 之后这个容器被放大、被挤小、被镜头推近，都只沿原来的行 / 列按比例伸缩，不重新 squarify → 过渡里子块不会横穿。
 * - 链上的块在它的容器里用有效权重 eff（≥ BOOST 的面积），兄弟块按原来的比例挤在剩下的地方；
 * - 「其他」= 从第一个出现细条（最短边 < minPx，按**不放大**的布局判断）的那一行起的所有行，正好是一个矩形：
 *   容器变大就多露几行，变小就少露几行，大块的位置不受影响；
 * - 伸缩以后（放大的块把兄弟挤小、容器被挤变形）只改画法、不改结构（squeeze）：厚度 < minPx 的行 / 列
 *   （小的在后面，是连续的尾部）合成一块「其他」铺满这几行合起来的矩形；一行里沿行方向 < minPx 的尾部几块
 *   在这一行的末端合成一小块「其他」。画出来的块（包括「其他」）最短边都 ≥ minPx，不再有细条。
 * canon = root 的参照矩形（默认 = rect，即根就是扫描根）；frame = 扫描根的整张图（放大形状的面积归一化用，默认 = rect）。
 * head：自动缩放后图的根不是扫描根——根自己画成一块铺满整张图的容器（深度 0，顶上一条标题栏），子块铺在它的内容区里；
 * 它的 key / 名字和它在上一级图里那一块一样（外壳合并块用外层的 key），镜头推近 / 拉远时就是同一块在长大 / 缩小。
 */
export function layoutTreemap(
  root: TmInput,
  rect: Rect,
  opt: LayoutOptions = DEFAULT_LAYOUT,
  canon?: Rect,
  frame?: Rect,
  head?: { key: string; name: string; size: number },
  hid?: Map<string, HiddenSlot>,
): TmItem[] {
  const out: TmItem[] = [];
  if (head) {
    out.push({
      key: head.key,
      kind: "node",
      name: head.name,
      value: root.value,
      size: head.size,
      rect,
      depth: 0,
      container: true,
      mergedCount: 0,
      chain: true,
      ref: root.ref,
      parentKey: null,
    });
    const box = contentBox(rect, opt);
    if (box.w > 0 && box.h > 0) place(root, box, canon ?? box, 1, head.key, opt, out, opt.maxDepth, frame ?? rect, true, hid);
    return out;
  }
  place(root, rect, canon ?? rect, 1, null, opt, out, opt.maxDepth, frame ?? rect, true, hid);
  return out;
}

function tooSmall(r: Rect, minPx: number): boolean {
  return Math.min(r.w, r.h) < minPx;
}

interface Slot {
  node: TmInput | null; // null = 尾巴（后端没发来的 + 数据对不上的差额）
  /** 真实值 */
  value: number;
  /** 尾巴合并了几个子项 */
  merged: number;
}

const byValue = (a: TmInput, b: TmInput) => b.value - a.value || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);

/** 一个容器的格子：子项按值降序（同值按 key），后端没发来的那部分永远排在最后（它本来就是最小的那些） */
function slotsOf(p: TmInput): Slot[] {
  const kids = (p.children ?? []).filter((c) => c.value > 0).sort(byValue);
  let tail = Math.max(0, p.hidden ?? 0);
  const sum = kids.reduce((s, c) => s + c.value, 0) + tail;
  // 子项合计比父小（数据不一致或后端截断）时，差额也算进尾巴
  if (p.value > sum) tail += p.value - sum;
  const out: Slot[] = kids.map((c) => ({ node: c, value: c.value, merged: 0 }));
  if (tail > 0) out.push({ node: null, value: tail, merged: p.hiddenCount ?? 0 });
  return out;
}

const sumExcept = (v: number[], i: number) => v.reduce((s, x, k) => (k === i ? s : s + x), 0);

/** 第一个有细条（子项的最短边 < minPx）的行；没有 = rows.length。尾巴格子不算（它太小就直接不画） */
function firstThin(slots: Slot[], rows: Strip[], rects: (Rect | null)[], minPx: number): number {
  for (let ri = 0; ri < rows.length; ri++) {
    for (let k = rows[ri].from; k < rows[ri].to; k++) {
      const r = rects[k];
      if (slots[k].node && r && tooSmall(r, minPx)) return ri;
    }
  }
  return rows.length;
}

/**
 * 不放大时这个容器在 rect 里怎么铺：从哪一行起并成「其他」（cut），以及实际用的权重
 * （「其他」/ 尾巴格子自己也太小就不画：权重设 0，前面的块吃掉这点空间，不留空洞）。
 */
function plainPlan(slots: Slot[], rows: Strip[], values: number[], rect: Rect, minPx: number): { cut: number; w: number[] } {
  const plain = fillRows(values, rows, rect).rects;
  const cut = firstThin(slots, rows, plain, minPx);
  return { cut, w: trimTail(slots, rows, values, rect, plain, cut, minPx) };
}

/**
 * 「其他」/ 尾巴格子自己也太小：不画它，把它的权重交给它前面那一格（只改最后几格，前面各行的位置一点不动——
 * 数据变细、尾巴时有时无时，大块不会跟着挪）。返回实际用的权重。
 */
function trimTail(slots: Slot[], rows: Strip[], values: number[], rect: Rect, plain: (Rect | null)[], cut: number, minPx: number): number[] {
  const w = values.slice();
  if (cut < rows.length) {
    const from = rows[cut].from;
    const tail = fillRows(values, rows, rect, cut).tail;
    if (from > 0 && tail && tooSmall(tail, minPx)) {
      for (let k = from; k < slots.length; k++) {
        w[from - 1] += w[k];
        w[k] = 0;
      }
    }
  } else {
    const ti = slots.length - 1;
    if (ti > 0 && !slots[ti].node && plain[ti] && tooSmall(plain[ti]!, minPx)) {
      w[ti - 1] += w[ti];
      w[ti] = 0;
    }
  }
  return w;
}

/** 一个容器在它的参照矩形里的结构：格子、分行（和参照矩形的长宽比有关），以及不放大时的铺法（按需算） */
interface Struct {
  slots: Slot[];
  values: number[];
  rows: Strip[];
  canon: Rect;
  plan?: { cut: number; w: number[] };
}
function structOf(p: TmInput, canon: Rect): Struct {
  const slots = slotsOf(p);
  const values = slots.map((s) => s.value);
  return { slots, values, rows: strips(values, canon), canon };
}

/**
 * 第 i 个格子的基础参照矩形（只看数据，和展开链、浏览历史无关）：父块在它自己的参照矩形里、不放大地铺开时，
 * 这个子项够大能当容器 → 用它的内容区；不够大（只有放大以后才画得出子项）→ 用它在父块里放大到 BOOST 时的形状，
 * 面积归一化到整张图的 BOOST 倍（只有长宽比影响分行，归一化让「够不够当容器」的判断有意义）。
 */
function canonAt(st: Struct, i: number, opt: LayoutOptions, frame: Rect): Rect | null {
  const { slots, values, rows, canon } = st;
  if (i < 0 || canon.w <= 0 || canon.h <= 0) return null;
  st.plan ??= plainPlan(slots, rows, values, canon, opt.minPx);
  const base = fillRows(st.plan.w, rows, canon, st.plan.cut).rects[i];
  if (base && base.w >= opt.minContainerPx && base.h >= opt.minContainerPx) {
    const c = contentBox(base, opt);
    if (c.w > 0 && c.h > 0) return c;
  }
  const w2 = values.slice();
  w2[i] = boostWeight(values[i], sumExcept(values, i), BOOST);
  const pr = fillRows(w2, rows, canon).rects[i];
  if (!pr || pr.w <= 0 || pr.h <= 0) return null;
  const k = Math.sqrt((BOOST * frame.w * frame.h) / (pr.w * pr.h));
  const big = { x: 0, y: 0, w: pr.w * k, h: pr.h * k };
  const c = contentBox(big, opt);
  return c.w > 0 && c.h > 0 ? c : big;
}

function canonChild(p: TmInput, canonP: Rect, key: string, opt: LayoutOptions, frame: Rect): Rect | null {
  const st = structOf(p, canonP);
  return canonAt(st, st.slots.findIndex((s) => s.node?.key === key), opt, frame);
}

/** 第 i 个格子（子块 c）的参照矩形；外壳合并块沿原始的每一层走到内层 */
function canonOf(st: Struct, i: number, c: TmInput, opt: LayoutOptions, frame: Rect): Rect | null {
  let cv = canonAt(st, i, opt, frame);
  const via = c.via ?? [];
  for (let k = 1; k < via.length && cv; k++) cv = canonChild(via[k - 1], cv, via[k].key, opt, frame);
  return cv;
}

/**
 * 从扫描根（nodes[0]，参照矩形 = frame）沿 nodes 往下走到最后一个节点，返回它的参照矩形。
 * 自动缩放后图的根不是扫描根：根的子块沿用同一套结构，镜头推近 / 拉远时块正好按比例移动。
 */
export function canonPath(nodes: readonly TmInput[], frame: Rect, opt: LayoutOptions): Rect {
  let cv: Rect = frame;
  for (let i = 1; i < nodes.length; i++) {
    const next = canonChild(nodes[i - 1], cv, nodes[i].key, opt, frame);
    if (!next) break;
    cv = next;
  }
  return cv;
}

/**
 * 阶段 K：放大后的有效权重。让 boost 的那一项至少占 boost 的面积：
 * eff / (eff + rest) ≥ boost ⇔ eff ≥ boost / (1 − boost) × rest。本来就更大就用真实值。
 */
export function boostWeight(value: number, rest: number, boost: number): number {
  const b = Math.min(0.95, Math.max(0, boost));
  if (b <= 0 || rest <= 0) return value;
  return Math.max(value, (b / (1 - b)) * rest);
}

/**
 * 铺一层子项；其中 boosted 那一项至少占 boost 的面积，而且**在原地长大**：
 * 分行（哪几块在同一行 / 列、行是横是竖）按真实值的 squarify 定下来不变，只把它的权重换成有效权重 eff 再按比例铺。
 * 这样每一块都留在原来的行 / 列里：放大的块向四周撑开，兄弟块按原来的比例被挤小，动画里没有块横穿别的块。
 * 本来就够大（eff = 真实值）就和不放大一模一样。
 */
export function squarifyBoosted<T>(items: Weighted<T>[], rect: Rect, isBoosted: (d: T) => boolean, boost: number): Placed<T>[] {
  const list = items.filter((i) => i.value > 0);
  const values = list.map((i) => i.value);
  const rows = strips(values, rect);
  const bi = list.findIndex((i) => isBoosted(i.data));
  if (bi < 0) return placeList(list, values, rows, rect);
  const eff = boostWeight(values[bi], sumExcept(values, bi), boost);
  if (eff <= values[bi]) return placeList(list, values, rows, rect);
  const w2 = values.slice();
  w2[bi] = eff;
  return placeList(list, w2, rows, rect);
}

/**
 * 伸缩以后怎么画（结构不变，只改画法）。
 * w0 = 实际权重（已经放大、已经扣掉不画的尾巴）；cut0 = 按不放大的布局并成「其他」的那一行。
 * - 厚度（这一行 / 列的最短边）< minPx 的行：在放大的那一项（bi）后面 → 从它起的所有行并成尾部的「其他」；
 *   在 bi 前面（放大的块把前面的兄弟挤细了，不是连续的尾部）→ 这一行整个不画，权重挪进尾部的「其他」
 *   （没有尾部就挪给 bi）。面积和值成正比的性质不变：「其他」的面积 = 它合并的那些项的大小。
 * - 尾部的「其他」自己太薄：接到最后画出来的那一行的末尾，合成行末的「其他」（前面各行的位置一点不动）；合起来还是太短才交给前一格。
 * - 画出来的每一行里：沿行方向 < minPx 的尾部几块合成这一行末端的一小块「其他」（合起来也太短就交给前一块）；
 *   bi 所在那一行里排在 bi 前面、被挤短的块交给 bi（同一行里挪，行的厚度不变）。
 * 任何权重都只在固定的行 / 列结构里分配：两块之间的先后（左右 / 上下）只由结构决定，过渡里不会横穿。
 */
interface Squeeze {
  w: number[];
  /** 实际用的分行：尾部太薄、接到了最后一行末尾时，那一行延长到最后一格（其余和输入一样） */
  rows: Strip[];
  cut: number;
  /** 每一行从哪一格起并成行末的「其他」（= row.to 表示没有） */
  ends: number[];
  /** 整行不画的行（权重挪走了） */
  drop: Set<number>;
  /** 被挪走的行去了哪里：尾部「其他」（或接到最后一行末尾的「其他」）/ 放大的那一项 / 尾部太小交给前一格 */
  movedTo: "tail" | "boost" | "none";
  /** 尾部太薄、接到了这一行的末尾（-1 = 没有） */
  appended: number;
  /** 接上来的尾部原来的那条细框（结构上它被挤成了这个框远端的一条线）和它是从哪一格开始的 */
  thin: { rect: Rect; from: number; column: boolean } | null;
  rects: (Rect | null)[];
  tail: Rect | null;
}

/** 一行 / 列（strip）在布局里占的矩形：它里面各格的外框 */
function spanOf(rects: (Rect | null)[], r: Strip): Rect | null {
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (let k = r.from; k < r.to; k++) {
    const q = rects[k];
    if (!q) continue;
    x0 = Math.min(x0, q.x);
    y0 = Math.min(y0, q.y);
    x1 = Math.max(x1, q.x + q.w);
    y1 = Math.max(y1, q.y + q.h);
  }
  return x1 > x0 && y1 > y0 ? { x: x0, y: y0, w: x1 - x0, h: y1 - y0 } : null;
}

function squeeze(rows: Strip[], w0: number[], rect: Rect, cut0: number, bi: number, minPx: number): Squeeze {
  const bRow = bi >= 0 ? rows.findIndex((r) => bi >= r.from && bi < r.to) : -1;
  let cut = cut0;
  const drop = new Set<number>();
  const mv: { to: Squeeze["movedTo"] } = { to: "none" };
  const weights = () => {
    const w = w0.slice();
    let moved = 0;
    for (const r of drop)
      for (let k = rows[r].from; k < rows[r].to; k++) {
        moved += w[k];
        w[k] = 0;
      }
    mv.to = "none";
    if (moved > 0) {
      if (cut < rows.length) {
        w[rows[cut].from] += moved;
        mv.to = "tail";
      } else if (bi >= 0) {
        w[bi] += moved;
        mv.to = "boost";
      }
    }
    return w;
  };
  let w = weights();
  for (let iter = 0; iter < 2 * rows.length + 2; iter++) {
    const rects = fillRows(w, rows, rect, cut).rects;
    let changed = false;
    for (let r = 0; r < cut; r++) {
      if (r === bRow || drop.has(r)) continue;
      const s = spanOf(rects, rows[r]);
      if (s && Math.min(s.w, s.h) >= minPx) continue;
      if (r < bRow) drop.add(r);
      else cut = r;
      changed = true;
      break;
    }
    if (!changed) break;
    w = weights();
  }
  // 尾部的「其他」自己太薄（画不出来）：接到前面那一行（最后画出来的一行）的末尾，合成行末的「其他」——那一行吃掉剩下的全部空间，
  // 前面各行一点不动；被挤小的项（回收站之类）还在图上，不会悄悄算进前一块、把它撑大（F2）。合起来还是太短才交给前一格
  let { tail } = fillRows(w, rows, rect, cut);
  let rowsOut = rows;
  let appended = -1;
  let tailFrom = -1;
  let thin: Squeeze["thin"] = null;
  if (cut < rows.length && tail && tooSmall(tail, minPx)) {
    let r0 = cut - 1;
    while (r0 >= 0 && drop.has(r0)) r0--;
    if (r0 >= 0 && r0 === cut - 1) {
      tailFrom = rows[cut].from;
      thin = { rect: tail, from: tailFrom, column: rows[r0].column };
      rowsOut = rows.slice(0, cut);
      rowsOut[r0] = { ...rows[r0], to: w.length };
      appended = r0;
      cut = rowsOut.length;
    } else if (r0 >= 0) {
      const to = rows[r0].to - 1;
      for (let k = rows[cut].from; k < w.length; k++) {
        w[to] += w[k];
        w[k] = 0;
      }
      if (mv.to === "tail") mv.to = "none";
      cut = rows.length;
    }
  }
  let rects = fillRows(w, rowsOut, rect, cut).rects;
  // 每一行的行末：沿行方向太短的尾部几块
  const ends = rowsOut.map((r) => r.to);
  for (let r = 0; r < Math.min(cut, rowsOut.length); r++) {
    if (drop.has(r)) continue;
    const row = rowsOut[r];
    const len = (k: number) => {
      const q = rects[k];
      return q && w[k] > 0 ? (row.column ? q.h : q.w) : 0;
    };
    const L = Array.from({ length: row.to - row.from }, (_, i) => len(row.from + i));
    const short = (k: number) => k !== bi && L[k - row.from] < minPx;
    let k0 = row.to;
    while (k0 - 1 >= row.from && short(k0 - 1)) k0--;
    // 接上来的尾部整个并成一块（被挤掉的行挪过来的权重也在里面）
    if (r === appended) k0 = Math.min(k0, tailFrom);
    if (k0 < row.to) {
      let tot = 0;
      let any = false;
      for (let k = k0; k < row.to; k++) {
        tot += L[k - row.from];
        if (w[k] > 0) any = true;
      }
      if (any && (tot >= minPx || k0 === row.from)) ends[r] = k0;
      else if (any) {
        for (let k = k0; k < row.to; k++) {
          w[k0 - 1] += w[k];
          w[k] = 0;
        }
        if (r === appended && mv.to === "tail") mv.to = "none";
      }
    }
    // 不在行尾、却被挤短的块（bi 前面的兄弟；或者后一格吃了尾巴变长了）：交给同一行里离它最近的够长的块
    // （同一行里挪，行的厚度不变，别的块的长短也不变）
    for (let k = row.from; k < k0; k++) {
      if (w[k] <= 0 || !short(k)) continue;
      let j = -1;
      for (let d = 1; d < row.to - row.from && j < 0; d++) {
        if (k - d >= row.from && !short(k - d) && w[k - d] > 0) j = k - d;
        else if (k + d < k0 && !short(k + d) && w[k + d] > 0) j = k + d;
      }
      if (j < 0) continue;
      w[j] += w[k];
      w[k] = 0;
    }
  }
  ({ rects, tail } = fillRows(w, rowsOut, rect, cut));
  return { w, rows: rowsOut, cut, ends, drop, movedTo: mv.to, appended, thin, rects, tail };
}

/**
 * 每一格在这次布局里「结构上」的位置（过渡用：上一次没有单独画出来的块，这一次出现时从这里长出来，沿原来的行 / 列撑开，不横穿）：
 * - 画出来的行：按实际权重沿行铺（权重 0 的格子 = 它原来位置上一条长度为 0 的线）；整行权重为 0（被挤掉的行）= 厚度为 0 的一条，
 *   沿行按真实值分开；
 * - 并成尾部「其他」的行：在尾部「其他」的框里按真实值铺（它们就在那块「其他」里面）。
 */
function structRects(sq: Squeeze, values: number[], rect: Rect): (Rect | null)[] {
  const { rows, w, cut, tail } = sq;
  const out = fillRows(w, rows, rect, cut).rects;
  for (let r = 0; r < Math.min(cut, rows.length); r++) {
    const row = rows[r];
    let s = 0;
    let vs = 0;
    for (let k = row.from; k < row.to; k++) {
      s += w[k];
      vs += values[k];
    }
    if (s > 0) continue;
    const a = out[row.from];
    const b = out[row.to - 1];
    if (!a || !b) continue;
    const len = row.column ? b.y + b.h - a.y : b.x + b.w - a.x;
    let pos = 0;
    for (let k = row.from; k < row.to; k++) {
      const l = vs > 0 ? (len * values[k]) / vs : len / (row.to - row.from);
      out[k] = row.column ? { x: a.x, y: a.y + pos, w: 0, h: l } : { x: a.x + pos, y: a.y, w: l, h: 0 };
      pos += l;
    }
  }
  if (cut < rows.length && tail) {
    const from = rows[cut].from;
    const sub = rows.slice(cut).map((r) => ({ from: r.from - from, to: r.to - from, column: r.column }));
    fillRows(values.slice(from), sub, tail).rects.forEach((q, i) => (out[from + i] = q));
  }
  if (sq.thin) {
    // 太薄、接到最后一行末尾的尾部：结构上它们被挤成了原来那条细框远端的一条线（最后一行把那条细框也占了），沿着线按真实值分开
    const { rect: t, from, column } = sq.thin;
    let vs = 0;
    for (let k = from; k < values.length; k++) vs += values[k];
    const len = column ? t.h : t.w;
    let pos = 0;
    for (let k = from; k < values.length; k++) {
      const l = vs > 0 ? (len * values[k]) / vs : 0;
      out[k] = column ? { x: t.x + t.w, y: t.y + pos, w: 0, h: l } : { x: t.x + pos, y: t.y + t.h, w: l, h: 0 };
      pos += l;
    }
  }
  return out;
}

/** 这次布局里没有单独画出来的子项（并进了「其他」、被挤掉、权重交给了邻居）：结构上的位置（布局坐标）和父块 */
export interface HiddenSlot {
  rect: Rect;
  parentKey: string | null;
}

/**
 * 铺 p 的子项到 rect（canon = p 的参照矩形）。budget = 这一层往下还能画几层（≥ 1 才铺子项）；
 * 子项是展开链上的块时重新给 boostDepth 层。返回 false = 一个子块都不画（调用方把 p 画成叶子）。
 */
function place(
  p: TmInput,
  rect: Rect,
  canon: Rect,
  depth: number,
  parentKey: string | null,
  opt: LayoutOptions,
  out: TmItem[],
  budget: number,
  frame: Rect,
  top: boolean,
  hid?: Map<string, HiddenSlot>,
): boolean {
  const st = structOf(p, canon.w > 0 && canon.h > 0 ? canon : rect);
  let { slots, values, rows } = st;
  if (!slots.length || rect.w <= 0 || rect.h <= 0) return false;
  const minPx = minPxAt(opt, depth);
  const boosted = (p.boost ?? 0) > 0;
  const plain = fillRows(values, rows, rect).rects;
  let cut = firstThin(slots, rows, plain, minPx);
  /** 新顺序里的第 k 格原来是第几格（算子块的参照矩形用） */
  let orig: number[] | null = null;
  let bi = slots.findIndex((s) => (s.node?.boost ?? 0) > 0);
  const rowOf = (k: number) => rows.findIndex((r) => k >= r.from && k < r.to);
  if (bi >= 0 && rowOf(bi) >= cut) {
    // 展开链上的那一项本来在「其他」里（太小）：把它单独拿出来，排在留下的行之后、自成一行，其余的照旧并成「其他」。
    // 留下的行不变（它们的位置只被按比例挤小），不会为了露出它而把一串细条画出来
    const keepN = rows[cut].from;
    const order = [...Array.from({ length: keepN }, (_, k) => k), bi, ...Array.from({ length: slots.length - keepN }, (_, k) => keepN + k).filter((k) => k !== bi)];
    const rest = fillRows(values, rows, st.canon, cut).tail!;
    rows = [...rows.slice(0, cut), { from: keepN, to: keepN + 1, column: rest.w >= rest.h }];
    if (slots.length > keepN + 1) rows.push({ from: keepN + 1, to: slots.length, column: true });
    slots = order.map((k) => slots[k]);
    values = order.map((k) => values[k]);
    orig = order;
    bi = keepN;
    cut += 1;
  } else if (bi >= 0) {
    // 展开链上的那一项永远画出来
    cut = Math.max(cut, rowOf(bi) + 1);
  }
  // 图的根 / 链上的块：至少试着画第一行（放不下的块在行末并成「其他」）
  if (cut === 0 && (top || boosted)) cut = Math.min(1, rows.length);
  if (cut === 0) return false;
  const w0 = trimTail(slots, rows, values, rect, orig ? fillRows(values, rows, rect).rects : plain, cut, minPx);
  // 「其他」那一块自己也太小（权重已交给前一格）：不画它，前面那一格吃掉这点空间
  if (cut < rows.length && w0[rows[cut].from] === 0) cut = rows.length;
  if (bi >= 0) w0[bi] = boostWeight(w0[bi], sumExcept(w0, bi), slots[bi].node!.boost!);
  const sq = squeeze(rows, w0, rect, cut, bi, minPx);
  cut = sq.cut;
  // 一行都画不出来：不在链上的容器画成叶子（链上的 / 图的根：整个内容区是一块「其他」）
  if (cut === 0 && !(top || boosted)) return false;
  const { rects, tail, ends } = sq;
  rows = sq.rows;
  const last = slots.length - 1;
  if (hid) {
    // 没有单独画出来的子项：记下它们结构上的位置（下一次它们出现时从这里长出来）
    const sr = structRects(sq, values, rect);
    const drawn = (k: number) => {
      const r = rows.findIndex((x) => k >= x.from && k < x.to);
      const q = rects[k];
      return r >= 0 && r < cut && k < ends[r] && !!q && sq.w[k] > 0 && !tooSmall(q, 2);
    };
    for (let k = 0; k < slots.length; k++) {
      const n = slots[k].node;
      const q = sr[k];
      if (n && q && !drawn(k)) hid.set(n.key, { rect: q, parentKey });
    }
  }
  /** 「其他」的 key：含最后一格的那块（尾部 / 最后一行的行末）= 父 key|其他；别的行末 = 父 key|其他|行号。不带项数（项数变了还是同一块） */
  const okey = (r: number, lastIn: boolean) => (lastIn && cut >= rows.length ? "" : `|${r}`);
  const count = (k: number) => (slots[k].node ? 1 : slots[k].merged);
  const keyOf = (k: number) => slots[k].node?.key;

  for (let r = 0; r < Math.min(cut, rows.length); r++) {
    const row = rows[r];
    for (let k = row.from; k < ends[r]; k++) {
      const q = rects[k];
      // 并掉的 / 不画的尾巴 / 被挪走权重的：画不出来，也不占命中
      if (!q || sq.w[k] <= 0 || tooSmall(q, 2)) continue;
      const s = slots[k];
      if (!s.node) {
        out.push(otherItem(p, s.value, s.merged, q, depth, parentKey, okey(r, k === last), []));
        continue;
      }
      const n = s.node;
      const onChain = (n.boost ?? 0) > 0;
      const inner = onChain ? Math.max(opt.boostDepth ?? 2, budget - 1) : budget - 1;
      const canNest = inner >= 1 && !!n.children?.length && q.w >= opt.minContainerPx && q.h >= opt.minContainerPx;
      const item: TmItem = {
        key: n.key,
        kind: "node",
        name: n.name,
        value: n.value,
        size: n.size ?? n.value,
        rect: q,
        depth,
        container: canNest,
        mergedCount: 0,
        chain: onChain,
        ref: n.ref,
        parentKey,
      };
      out.push(item);
      if (canNest) {
        const box = contentBox(q, opt);
        const cc = canonOf(st, orig ? orig[k] : k, n, opt, frame) ?? box;
        if (!(box.w > opt.minPx && box.h > opt.minPx) || !place(n, box, cc, depth + 1, n.key, opt, out, inner, frame, false, hid)) item.container = false;
      }
    }
    if (ends[r] < row.to) {
      // 行末的「其他」：这几格合起来的矩形
      let x0 = Infinity;
      let y0 = Infinity;
      let x1 = -Infinity;
      let y1 = -Infinity;
      let v = 0;
      let m = 0;
      const ks: string[] = [];
      for (let k = ends[r]; k < row.to; k++) {
        v += slots[k].value;
        m += count(k);
        const nk = keyOf(k);
        if (nk) ks.push(nk);
        const q = rects[k];
        if (!q || sq.w[k] <= 0) continue;
        x0 = Math.min(x0, q.x);
        y0 = Math.min(y0, q.y);
        x1 = Math.max(x1, q.x + q.w);
        y1 = Math.max(y1, q.y + q.h);
      }
      // 接到这一行末尾的尾部：被挤掉的前面几行（权重挪进了尾部）也算进来
      if (r === sq.appended && sq.movedTo === "tail")
        for (const d of sq.drop)
          for (let k = rows[d].from; k < rows[d].to; k++) {
            v += slots[k].value;
            m += count(k);
            const nk = keyOf(k);
            if (nk) ks.push(nk);
          }
      if (x1 > x0 && y1 > y0) out.push(otherItem(p, v, m, { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, depth, parentKey, okey(r, row.to - 1 === last), ks));
    }
  }
  if (cut < rows.length && tail && !tooSmall(tail, minPx)) {
    let v = 0;
    let m = 0;
    const ks: string[] = [];
    const add = (k: number) => {
      v += slots[k].value;
      m += count(k);
      const nk = keyOf(k);
      if (nk) ks.push(nk);
    };
    for (let k = rows[cut].from; k < slots.length; k++) add(k);
    // 被挤掉的前面几行：权重挪进了尾部，项数和大小也算进来
    if (sq.movedTo === "tail") for (const r of sq.drop) for (let k = rows[r].from; k < rows[r].to; k++) add(k);
    out.push(otherItem(p, v, m, tail, depth, parentKey, "", ks));
  }
  return true;
}

function otherItem(p: TmInput, value: number, merged: number, rect: Rect, depth: number, parentKey: string | null, suffix: string, keys: string[]): TmItem {
  return {
    merged: keys,
    key: `${p.key}|其他${suffix}`,
    kind: "other",
    name: `其他 ${merged} 项`,
    value,
    size: value,
    rect,
    depth,
    container: false,
    mergedCount: merged,
    chain: false,
    parentKey,
  };
}

// ───── 单块的绘制尺寸（阶段 I）：圆角 / 描边 / 投影按块大小收敛 ─────

/** 方块图的设计令牌（来自 tokens.css 的 --tm-*，三主题各一套）。单位 px。 */
export interface TileTokens {
  /** 块与块之间的固定间隙 */
  gap: number;
  /** 圆角上限（方块图专用，比卡片圆角小） */
  r: number;
  /** 描边宽度 */
  bw: number;
  /** 硬投影偏移（0 = 不画） */
  shadow: number;
  /** 最短边小于这个就画成平块：只填色，没有描边 / 圆角 / 投影 */
  flatPx: number;
  /** 最短边小于这个就不画投影 */
  shadowMinPx: number;
  /** 最短边小于这个，斜条纹块改成纯浅色 */
  patternMinPx: number;
}

export interface TileBox {
  /** 像素对齐后的左上角 */
  x: number;
  y: number;
  /** 块面（不含投影）的宽高；块面 + 投影 + 间隙 恰好 = 分到的格子 */
  w: number;
  h: number;
  r: number;
  bw: number;
  shadow: number;
  flat: boolean;
  pattern: boolean;
}

/**
 * 把布局给的格子换成实际绘制的块：先像素对齐（间隙处处一样宽），右 / 下扣掉固定间隙；
 * 够大才有描边、圆角（不超过 min(r, w/4, h/4)）和投影，投影画在格子里面（块面相应缩小），绝不压到邻块。
 */
export function tileBox(rect: Rect, t: TileTokens, withShadow = true): TileBox {
  const x = Math.round(rect.x);
  const y = Math.round(rect.y);
  const w = Math.max(0, Math.round(rect.x + rect.w) - x - t.gap);
  const h = Math.max(0, Math.round(rect.y + rect.h) - y - t.gap);
  const m = Math.min(w, h);
  if (m < t.flatPx) return { x, y, w, h, r: 0, bw: 0, shadow: 0, flat: true, pattern: false };
  const shadow = withShadow && t.shadow > 0 && m >= t.shadowMinPx ? t.shadow : 0;
  const fw = w - shadow;
  const fh = h - shadow;
  return {
    x,
    y,
    w: fw,
    h: fh,
    r: Math.max(0, Math.floor(Math.min(t.r, fw / 4, fh / 4))),
    bw: t.bw,
    shadow,
    flat: false,
    pattern: Math.min(fw, fh) >= t.patternMinPx,
  };
}

/** 命中测试：返回最深的包含该点的块（列表是先父后子的绘制顺序）。 */
export function hitTest(items: TmItem[], x: number, y: number): TmItem | null {
  for (let i = items.length - 1; i >= 0; i--) {
    const r = items[i].rect;
    if (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) return items[i];
  }
  return null;
}

// ───── 单子项链合并（阶段 F） ─────

export interface ChainNode {
  name: string;
  kind: string;
  allocated: number;
  children: ChainNode[];
}

/**
 * “外壳”文件夹合并：一个文件夹里 ≥ share 的空间都在唯一一个最大的子文件夹里（比如 C:\Users 里几乎全是 king），
 * 就把两层合成一块、名字写成 “Users \ king”，最多合并 maxHops 层。内层的子项还没发过来（children 为空）时不合并。
 */
export function collapseChain<T extends ChainNode>(
  n: T,
  share = 0.95,
  maxHops = 3,
  stop?: (from: T, to: T) => boolean,
): { node: T; name: string; hops: T[] } {
  let node = n;
  let name = n.name;
  const hops: T[] = [n];
  for (let hop = 0; hop < maxHops; hop++) {
    const dirs = node.children.filter((c) => c.kind === "Dir");
    if (!dirs.length || node.allocated <= 0) break;
    const big = dirs.reduce((a, b) => (b.allocated > a.allocated ? b : a)) as T;
    if (big.allocated < node.allocated * share || !big.children.length) break;
    // 阶段 K：展开链从这里拐进了别的子文件夹（比如 Users → Public），就不能把它合并掉
    if (stop?.(node, big)) break;
    node = big;
    hops.push(big);
    name = `${name} \\ ${big.name}`;
  }
  return { node, name, hops };
}


// ───── 空间分析的导航（阶段 K：一张平面图，单击原地展开） ─────

/** 路径比较：大小写不敏感，忽略末尾的反斜杠（“C:\” 和 “c:” 视为同一个） */
export function samePath(a: string | null | undefined, b: string | null | undefined): boolean {
  if (a == null || b == null) return false;
  const n = (s: string) => s.replace(/[\\/]+$/, "").toLowerCase();
  return n(a) === n(b);
}

/**
 * 自动缩放的门槛（按像素，不按层数）：最深的展开块（pk）的内容区够大，才画得下它的子项和孙项。
 * 一般要 ≥ 360×260；图本身矮于 500 px 时改成 ≥ 图面积的 30%。
 */
export function bigEnough(c: Rect, map: { w: number; h: number }): boolean {
  if (c.w <= 0 || c.h <= 0) return false;
  if (map.h < 500) return c.w * c.h >= 0.3 * map.w * map.h;
  return c.w >= 360 && c.h >= 260;
}

/**
 * 自动缩放：从扫描根开始，依次试链在图上每一层的根（cands 由浅到深，最后一个 = pk 的父级），选第一个让 pk 够大的；
 * 都不够就用最深的那个。attempt 返回 null = 这个根画不出来，跳过。
 */
export function pickMapRoot<T>(cands: readonly string[], attempt: (root: string) => T | null, fits: (r: T) => boolean): { root: string; result: T } | null {
  let last: { root: string; result: T } | null = null;
  for (const root of cands) {
    const result = attempt(root);
    if (result === null) continue;
    last = { root, result };
    if (fits(result)) break;
  }
  return last;
}

/** 把 r 在 from 框里的相对位置按比例映射到 to 框里（新块从祖先原来的框里长出来 / 镜头推近拉远） */
export function mapBox(r: Rect, from: Rect, to: Rect): Rect {
  const sx = from.w > 0 ? to.w / from.w : 1;
  const sy = from.h > 0 ? to.h / from.h : 1;
  return { x: to.x + (r.x - from.x) * sx, y: to.y + (r.y - from.y) * sy, w: r.w * sx, h: r.h * sy };
}
