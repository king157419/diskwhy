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
  rect: Rect;
  depth: number;
  /** 有子块画在里面（画成带标题的容器） */
  container: boolean;
  /** “其他”块合并了几个子项 */
  mergedCount: number;
  /** 在展开链上（输入带 boost） */
  chain: boolean;
  ref?: unknown;
  parentKey: string | null;
}

export interface LayoutOptions {
  /** 最短边小于这个像素就合并进“其他” */
  minPx: number;
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
 * - 不在链上的容器被挤得变形（按基础形状铺还够大的子块现在成了细条）：子块整体不画，只剩带名字的块。
 * canon = root 的参照矩形（默认 = rect，即根就是扫描根）；frame = 扫描根的整张图（放大形状的面积归一化用，默认 = rect）。
 */
export function layoutTreemap(root: TmInput, rect: Rect, opt: LayoutOptions = DEFAULT_LAYOUT, canon?: Rect, frame?: Rect): TmItem[] {
  const out: TmItem[] = [];
  place(root, rect, canon ?? rect, 1, null, opt, out, opt.maxDepth, frame ?? rect, true);
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
  inFocus = false,
): boolean {
  const st = structOf(p, canon.w > 0 && canon.h > 0 ? canon : rect);
  let { slots, values, rows } = st;
  if (!slots.length || rect.w <= 0 || rect.h <= 0) return false;
  const plain = fillRows(values, rows, rect).rects;
  let cut = firstThin(slots, rows, plain, opt.minPx);
  // 被挤得变形的容器（链上块的兄弟这类「旁边」的容器）：按基础形状、同样面积铺时还够大的子块，现在成了细条 →
  // 子块整体不画（淡出），只剩带名字的块。正在看的那个文件夹（pk；链为空时是图的根）里的子文件夹不算：
  // 形状变了也照样画（细节正是要看的东西），放不下的行并进「其他」
  if (!top && !((p.boost ?? 0) > 0) && !inFocus) {
    const k = Math.sqrt((rect.w * rect.h) / (st.canon.w * st.canon.h));
    if (Number.isFinite(k) && k > 0) {
      const shaped = { x: 0, y: 0, w: st.canon.w * k, h: st.canon.h * k };
      if (cut < firstThin(slots, rows, fillRows(values, rows, shaped).rects, opt.minPx)) return false;
    }
  }
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
    // 展开链上的那一项永远画出来；并不并只按不放大的布局判断，被它挤成细条的兄弟不算
    cut = Math.max(cut, rowOf(bi) + 1);
  }
  // 图的根 / 链上的块：至少画第一行
  if (cut === 0 && (top || (p.boost ?? 0) > 0)) cut = Math.min(1, rows.length);
  if (cut === 0) return false;
  const w = trimTail(slots, rows, values, rect, orig ? fillRows(values, rows, rect).rects : plain, cut, opt.minPx);
  // 「其他」那一块自己也太小（权重已交给前一格）：不画它，前面那一格吃掉这点空间
  if (cut < rows.length && w[rows[cut].from] === 0) cut = rows.length;
  if (bi >= 0) w[bi] = boostWeight(w[bi], sumExcept(w, bi), slots[bi].node!.boost!);
  const { rects, tail } = fillRows(w, rows, rect, cut);
  /** p 就是正在看的文件夹（链的最后一级；链为空时是图的根）：它的子文件夹不按「被挤变形」藏起来 */
  const focusHere = bi < 0 && (top || (p.boost ?? 0) > 0);

  for (let k = 0; k < slots.length; k++) {
    const r = rects[k];
    // 在「其他」里 / 不画的尾巴 / 被放大的块挤到不足 2px 的兄弟：画不出来，也不占命中
    if (!r || tooSmall(r, 2)) continue;
    const s = slots[k];
    if (!s.node) {
      out.push(otherItem(p, s.value, s.merged, r, depth, parentKey));
      continue;
    }
    const n = s.node;
    const onChain = (n.boost ?? 0) > 0;
    const inner = onChain ? Math.max(opt.boostDepth ?? 2, budget - 1) : budget - 1;
    const canNest = inner >= 1 && !!n.children?.length && r.w >= opt.minContainerPx && r.h >= opt.minContainerPx;
    const item: TmItem = {
      key: n.key,
      kind: "node",
      name: n.name,
      value: n.value,
      rect: r,
      depth,
      container: canNest,
      mergedCount: 0,
      chain: onChain,
      ref: n.ref,
      parentKey,
    };
    out.push(item);
    if (canNest) {
      const box = contentBox(r, opt);
      const cc = canonOf(st, orig ? orig[k] : k, n, opt, frame) ?? box;
      if (!(box.w > opt.minPx && box.h > opt.minPx) || !place(n, box, cc, depth + 1, n.key, opt, out, inner, frame, false, focusHere)) item.container = false;
    }
  }
  if (tail && !tooSmall(tail, 2)) {
    let v = 0;
    let m = 0;
    for (let k = rows[cut].from; k < slots.length; k++) {
      v += slots[k].value;
      m += slots[k].node ? 1 : slots[k].merged;
    }
    out.push(otherItem(p, v, m, tail, depth, parentKey));
  }
  return true;
}

function otherItem(p: TmInput, value: number, merged: number, rect: Rect, depth: number, parentKey: string | null): TmItem {
  return {
    key: `${p.key}|其他|${merged}`,
    kind: "other",
    name: `其他 ${merged} 项`,
    value,
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
