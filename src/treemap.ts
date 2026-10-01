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

/**
 * squarified：把按值加权的项目铺满 rect，面积与值成正比，尽量接近正方形。
 * 值 <= 0 的项目被忽略。输出顺序 = 按值降序。
 */
export function squarify<T>(items: Weighted<T>[], rect: Rect): Placed<T>[] {
  const list = items.filter((i) => i.value > 0).sort((a, b) => b.value - a.value);
  const out: Placed<T>[] = [];
  const total = list.reduce((s, i) => s + i.value, 0);
  if (!list.length || rect.w <= 0 || rect.h <= 0 || total <= 0) return out;
  const scale = (rect.w * rect.h) / total;
  const areas = list.map((i) => i.value * scale);

  let { x, y, w, h } = rect;
  let i = 0;
  while (i < list.length) {
    const side = Math.min(w, h);
    const row: number[] = [areas[i]];
    let j = i + 1;
    while (j < list.length) {
      const next = [...row, areas[j]];
      if (worst(next, side) <= worst(row, side)) {
        row.push(areas[j]);
        j++;
      } else break;
    }
    const rowSum = row.reduce((s, a) => s + a, 0);
    // 最后一行吃掉剩余空间，避免浮点累积误差留下缝
    const last = j >= list.length;
    if (w >= h) {
      // 竖着放一列，宽 = rowSum / h
      const cw = last ? w : rowSum / h;
      let cy = y;
      row.forEach((a, k) => {
        const ch = k === row.length - 1 ? y + h - cy : a / cw;
        out.push({ rect: { x, y: cy, w: cw, h: ch }, data: list[i + k].data, value: list[i + k].value });
        cy += ch;
      });
      x += cw;
      w -= cw;
    } else {
      const rh = last ? h : rowSum / w;
      let cx = x;
      row.forEach((a, k) => {
        const cw = k === row.length - 1 ? x + w - cx : a / rh;
        out.push({ rect: { x: cx, y, w: cw, h: rh }, data: list[i + k].data, value: list[i + k].value });
        cx += cw;
      });
      y += rh;
      h -= rh;
    }
    i = j;
  }
  return out;
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
  ref?: unknown;
  parentKey: string | null;
}

export interface LayoutOptions {
  /** 最短边小于这个像素就合并进“其他” */
  minPx: number;
  /** 最多嵌套几层（1 = 只画根的子项） */
  maxDepth: number;
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

/**
 * 嵌套布局：root 自己不出块，它的子项铺满 rect；子项里够大的继续往里铺（最多 maxDepth 层）。
 * 最短边 < minPx 的块和后端没发来的小项合并成一个“其他”块；“其他”自己也太小就不画
 * （留作父块背景色）。返回按绘制顺序（先父后子）排好的扁平列表。
 */
export function layoutTreemap(root: TmInput, rect: Rect, opt: LayoutOptions = DEFAULT_LAYOUT): TmItem[] {
  const out: TmItem[] = [];
  place(root, rect, 1, null, opt, out);
  return out;
}

function tooSmall(r: Rect, minPx: number): boolean {
  return Math.min(r.w, r.h) < minPx;
}

interface Slot {
  node: TmInput | null; // null = 其他
  merged: number;
}

function place(parent: TmInput, rect: Rect, depth: number, parentKey: string | null, opt: LayoutOptions, out: TmItem[]) {
  const kids = (parent.children ?? []).filter((c) => c.value > 0);
  let otherValue = Math.max(0, parent.hidden ?? 0);
  let otherCount = parent.hiddenCount ?? 0;
  // 子项合计比父小（数据不一致或后端截断）时，差额也算进“其他”
  const sumKids = kids.reduce((s, c) => s + c.value, 0) + otherValue;
  if (parent.value > sumKids) otherValue += parent.value - sumKids;

  let keep = kids.slice();
  let placed: Placed<Slot>[] = [];
  let dropOther = false;
  // 反复：铺 → 有块太小就把“尾巴”（值 ≤ 最大的那个太小块的全部子项）整段并进一个“其他” → 再铺，直到稳定。
  // “其他”自己也太小时不留空洞：把它从布局里拿掉重铺（它的面积不到 minPx²，兄弟块吃掉这点空间）。
  for (let round = 0; round < 12; round++) {
    const items: Weighted<Slot>[] = keep.map((c) => ({ value: c.value, data: { node: c, merged: 0 } }));
    if (otherValue > 0 && !dropOther) items.push({ value: otherValue, data: { node: null, merged: otherCount } });
    placed = squarify(items, rect);
    const small = placed.filter((p) => p.data.node && tooSmall(p.rect, opt.minPx));
    if (small.length) {
      const cut = Math.max(...small.map((p) => p.value));
      for (const c of keep) {
        if (c.value <= cut) {
          otherValue += c.value;
          otherCount += 1;
        }
      }
      keep = keep.filter((c) => c.value > cut);
      dropOther = false;
      continue;
    }
    const other = placed.find((p) => !p.data.node);
    if (other && tooSmall(other.rect, opt.minPx) && keep.length) {
      dropOther = true;
      continue;
    }
    break;
  }

  for (const p of placed) {
    const r = p.rect;
    if (!p.data.node) {
      if (tooSmall(r, opt.minPx)) continue;
      out.push({
        key: `${parent.key}|其他`,
        kind: "other",
        name: `其他 ${p.data.merged} 项`,
        value: p.value,
        rect: r,
        depth,
        container: false,
        mergedCount: p.data.merged,
        parentKey,
      });
      continue;
    }
    const n = p.data.node;
    const canNest =
      depth < opt.maxDepth &&
      !!n.children?.length &&
      r.w >= opt.minContainerPx &&
      r.h >= opt.minContainerPx;
    const item: TmItem = {
      key: n.key,
      kind: "node",
      name: n.name,
      value: n.value,
      rect: r,
      depth,
      container: canNest,
      mergedCount: 0,
      ref: n.ref,
      parentKey,
    };
    out.push(item);
    if (canNest) {
      const inner: Rect = {
        x: r.x + opt.pad,
        y: r.y + opt.headerPx,
        w: r.w - opt.pad - (opt.padEnd ?? opt.pad),
        h: r.h - opt.headerPx - (opt.padEnd ?? opt.pad),
      };
      if (inner.w > opt.minPx && inner.h > opt.minPx) place(n, inner, depth + 1, n.key, opt, out);
      else item.container = false;
    }
  }
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
export function collapseChain<T extends ChainNode>(n: T, share = 0.95, maxHops = 3): { node: T; name: string } {
  let node = n;
  let name = n.name;
  for (let hop = 0; hop < maxHops; hop++) {
    const dirs = node.children.filter((c) => c.kind === "Dir");
    if (!dirs.length || node.allocated <= 0) break;
    const big = dirs.reduce((a, b) => (b.allocated > a.allocated ? b : a));
    if (big.allocated < node.allocated * share || !big.children.length) break;
    node = big as T;
    name = `${name} \\ ${big.name}`;
  }
  return { node, name };
}
