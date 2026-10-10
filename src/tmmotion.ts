// 阶段 K：方块图两次布局之间怎么动（纯函数，不碰 DOM；TreemapView 只负责量出块现在在屏幕上的位置）。
// - 前后都在、同一个元素的块：CSS 过渡从它现在的样子接着走；
// - 同一个父块下前后都在的两块，没有一个分隔方向前后都成立（过渡里会横穿）：小的那块换一个新身份（旧的淡出、新的淡入）；
// - 新出现的块（第 4 轮）：上一次布局里它没有单独画出来（并进了「其他」、被挤掉的行、权重交给了邻居），就从它**结构上的位置**
//   长出来——被挤掉的行是原地一条厚度为 0 的线，并在「其他」里的是「其他」框里它那一格；行 / 列结构前后一样，沿原来的行 / 列撑开，
//   不会从「其他」那里一路飞回原位、横穿留下的兄弟。其余的从「前后都在的最近的祖先」旧的内容区里按比例映射出来；图的根变了按镜头；
// - 安全网（第 4 轮）：同一个父块下，新块的起点框和每个兄弟（留下的、或者已经定好的新块）都要有一个分隔方向前后都成立；
//   「其他」（幕布，层次高）盖着从它下面出来的新块不算。不成立就换父块映射再试，还不行就原地淡入，并且晚到挡路的兄弟让开（按缓动曲线算出时刻）；
//   新块的子块跟着它一起出现（同一个起点映射、同一个延迟），不会先露出一个白底空框；
// - 离场的块（残影）：从它现在在屏幕上的位置出发，跟着前后都在的最近的祖先（或者镜头）移动并淡出。
//   层次：那个祖先还是容器 → 垫在它的子块下面（不会夹在容器和它的子块中间）；那个祖先变成了叶子 → 压在它上面、缩进它里面（F5：一次退回好几级，
//   旧的子块跟着缩小淡出，不露出一整块空板）；并进「其他」的块垫在「其他」下面、跟着它走。
import { isUnder } from "./tmtree";
import { mapBox, samePath } from "./treemap";

export type Box = { x: number; y: number; w: number; h: number };

export interface MotionTile {
  key: string;
  /** 元素的身份 */
  rk: string;
  parentKey: string | null;
  depth: number;
  container: boolean;
  /** 块对应的文件夹 / 文件路径（伪块、「其他」为 null） */
  path: string | null;
  z: number;
  /** 目标位置（上一次的块 = 它上一次的目标位置） */
  b: Box;
  /** 「其他」块合并了哪些子项（key）；只有「其他」有（可以是空数组） */
  merged?: readonly string[];
}

export interface MotionIn {
  /** 上一次画出来的活块（key → 块） */
  prev: ReadonlyMap<string, MotionTile>;
  /** 上一次的块现在在屏幕上的位置（过渡进行到一半时是中间的位置）；量不到就用它的目标位置 */
  cur: (t: MotionTile) => Box;
  /** 这一次的活块（先父后子） */
  live: readonly MotionTile[];
  prevRoot: string | null;
  root: string;
  /** 块的内容区（容器扣掉标题栏和内边距） */
  inner: (b: Box, container: boolean) => Box;
  frame: Box;
  /** 容器标题栏高（叶子块太矮时残影直接映射进整块） */
  head: number;
  /** 上一次布局里没有单独画出来的子项：结构上的位置（和上一次的块同一个坐标系）、父块 */
  hidden?: ReadonlyMap<string, { b: Box; parentKey: string | null }>;
  /** 这一次布局里没有单独画出来的子项（离场的块缩回这里） */
  hiddenNow?: ReadonlyMap<string, { b: Box; parentKey: string | null }>;
  /** 上一次新出现、还在等兄弟让开（还没淡入）的块：过渡被打断时它们还看不见，这一次当新块重新算（不从它等着的地方接着走） */
  pending?: ReadonlySet<string>;
  /** 位置过渡的时长（ms）和缓动（进度 = ease(时间比例)）：算「等兄弟让开」的延迟用 */
  dur?: number;
  ease?: (t: number) => number;
}

export interface Ghost {
  key: string;
  /** 要去的位置 */
  to: Box;
  z: number;
}

export interface MotionOut {
  /** 换了新身份的块（key） */
  jump: Set<string>;
  /** 新出现的块从哪里长出来（key → 框；= 它自己的框 → 原地淡入） */
  from: Map<string, Box>;
  /** 离场的块（上一次的 key）：去哪、垫在哪一层 */
  ghosts: Ghost[];
  /** 这一次位置 / 大小有变化、或者新出现的块（字先淡出，过渡过半后用最终的写法淡入） */
  moved: Set<string>;
  /** 新出现的块晚多少 ms 再淡入（出现的地方还压着正在让开的兄弟；子块跟着父块） */
  late: Map<string, number>;
  /** 镜头（图的根变了）：同一块区域在旧图 / 新图里的框 */
  cam: { old: Box; now: Box; out: boolean } | null;
}

/** 两块之间有没有一个分隔方向（左右 / 上下）在前后两次都成立：有 → 线性过渡的每一帧都不重叠 */
export function keepsApart(a0: Box, b0: Box, a1: Box, b1: Box): boolean {
  const s = (a: Box, b: Box) => [a.x + a.w <= b.x + 1, b.x + b.w <= a.x + 1, a.y + a.h <= b.y + 1, b.y + b.h <= a.y + 1];
  const p = s(a0, b0);
  const q = s(a1, b1);
  return p.some((v, i) => v && q[i]);
}

/** CSS cubic-bezier(x1, y1, x2, y2)：时间比例 → 进度 */
export function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const at = (a: number, b: number, s: number) => 3 * a * s * (1 - s) * (1 - s) + 3 * b * s * s * (1 - s) + s * s * s;
  return (t: number) => {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    let lo = 0;
    let hi = 1;
    for (let i = 0; i < 30; i++) {
      const mid = (lo + hi) / 2;
      if (at(x1, x2, mid) < t) lo = mid;
      else hi = mid;
    }
    return at(y1, y2, (lo + hi) / 2);
  };
}
/** tokens.css 的 --ease */
export const EASE = cubicBezier(0.2, 0.7, 0.2, 1);
/** tokens.css 的 --dur-3（位置过渡） */
export const DUR = 260;

/** 进度 p 是在什么时间比例达到的（ease 的反函数） */
function timeOf(p: number, ease: (t: number) => number): number {
  if (p <= 0) return 0;
  if (p >= 1) return 1;
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 30; i++) {
    const mid = (lo + hi) / 2;
    if (ease(mid) < p) lo = mid;
    else hi = mid;
  }
  return (lo + hi) / 2;
}

/** 线性量 c0 → c1（进度 p ∈ [0, 1]）满足 「< bound」（lt）或「> bound」的进度区间；空 = null */
function linRange(c0: number, c1: number, lt: boolean, bound: number): [number, number] | null {
  const d = c1 - c0;
  const ok = (c: number) => (lt ? c < bound : c > bound);
  if (Math.abs(d) < 1e-9) return ok(c0) ? [0, 1] : null;
  // c(p) = c0 + d·p 在 p* 处等于 bound：「<」且在变大 / 「>」且在变小 → p* 之前满足；反过来 → p* 之后满足
  const p = (bound - c0) / d;
  const below = lt === d > 0;
  if (below) return p <= 0 ? null : [0, Math.min(1, p)];
  return p >= 1 ? null : [Math.max(0, p), 1];
}
function meet(a: [number, number] | null, b: [number, number] | null): [number, number] | null {
  if (!a || !b) return null;
  const lo = Math.max(a[0], b[0]);
  const hi = Math.min(a[1], b[1]);
  return lo <= hi ? [lo, hi] : null;
}

/** 从 a0 线性移动到 a1 的框和静止的框 s 重叠（两个方向都多于 m px）的进度区间；不重叠 = null */
export function overlapSpan(a0: Box, a1: Box, s: Box, m = 2): [number, number] | null {
  let r: [number, number] | null = [0, 1];
  r = meet(r, linRange(a0.x, a1.x, true, s.x + s.w - m));
  r = meet(r, linRange(a0.x + a0.w, a1.x + a1.w, false, s.x + m));
  r = meet(r, linRange(a0.y, a1.y, true, s.y + s.h - m));
  r = meet(r, linRange(a0.y + a0.h, a1.y + a1.h, false, s.y + m));
  return r;
}

/** 从 a0 线性移动到 a1 的框，从什么进度起一直包住静止的框 s（误差 e px）；最后也包不住 = 1 */
export function containFrom(a0: Box, a1: Box, s: Box, e = 1): number {
  let r: [number, number] | null = [0, 1];
  r = meet(r, linRange(a0.x, a1.x, true, s.x + e + 1e-6));
  r = meet(r, linRange(a0.x + a0.w, a1.x + a1.w, false, s.x + s.w - e - 1e-6));
  r = meet(r, linRange(a0.y, a1.y, true, s.y + e + 1e-6));
  r = meet(r, linRange(a0.y + a0.h, a1.y + a1.h, false, s.y + s.h - e - 1e-6));
  return r ? r[0] : 1;
}

/**
 * 块的层次：越深越高；同一层里容器 < 叶子 < 「其他」。「其他」像幕布：比兄弟和兄弟往下好几层的子块都高
 * （从它下面出来的新容器连同里面的子块一起被它盖着，它缩回去时一起露出来；静止时它不和任何块重叠，高一点没有影响）
 */
export const OTHER_LIFT = 24;
export function tileZ(it: { depth: number; container: boolean; kind: string }): number {
  return it.depth * 4 + (it.container ? 0 : it.kind === "other" ? 3 + OTHER_LIFT : 2);
}

const differs = (a: Box, b: Box) => Math.abs(a.x - b.x) > 1.5 || Math.abs(a.y - b.y) > 1.5 || Math.abs(a.w - b.w) > 1.5 || Math.abs(a.h - b.h) > 1.5;
const area = (b: Box) => Math.max(0, b.w) * Math.max(0, b.h);
/** 淡入比挡路的兄弟让开早这么多 ms（淡入开头那几帧几乎是透明的） */
const LEAD_MS = 12;
/** 「等兄弟让开」最多等这么久 */
const MAX_LATE_MS = 230;

export function planMotion(m: MotionIn): MotionOut {
  const { prev, cur, live, inner, frame } = m;
  const dur = m.dur ?? DUR;
  const ease = m.ease ?? EASE;
  const fresh = new Map(live.map((x) => [x.key, x]));
  const was = (k: string | null) => (k ? (prev.get(k) ?? null) : null);
  const isOther = (t: MotionTile) => t.merged !== undefined;

  // ── 镜头：图的根变了（推近：新根原来的内容区 ↔ 新根铺满整张图的内容区；拉远反过来） ──
  let cam: MotionOut["cam"] = null;
  if (m.prevRoot && !samePath(m.prevRoot, m.root)) {
    /** 另一边的图上那一块没画出来（太深）时，用画出来的最深的那个祖先 */
    const deepest = (list: Iterable<MotionTile>, target: string, above: string) => {
      let best: MotionTile | null = null;
      for (const x of list) {
        const p = x.path;
        if (!p || x.depth === 0 || !(samePath(p, target) || isUnder(target, p)) || !isUnder(p, above)) continue;
        if (!best || p.length > best.path!.length) best = x;
      }
      return best;
    };
    /** 一张图的根框：缩放后有根标题条（深度 0）= 它的内容区（full：整块）；没有 = 整张图 */
    const rootBox = (list: Iterable<MotionTile>, box: (t: MotionTile) => Box, full = false) => {
      for (const x of list) if (x.depth === 0) return full ? box(x) : inner(box(x), true);
      return frame;
    };
    // 两边要对应：另一边那一块是容器 → 内容区对内容区；是叶子 / 更浅的祖先 → 整块对整块（不然旁边的块会从根标题条那一条里冒出来）
    if (isUnder(m.root, m.prevRoot)) {
      const o = deepest(prev.values(), m.root, m.prevRoot);
      const inn = !!o && o.container && samePath(o.path, m.root);
      if (o) cam = { old: inner(cur(o), inn), now: rootBox(live, (x) => x.b, !inn), out: false };
    } else if (isUnder(m.prevRoot, m.root)) {
      const n = deepest(live, m.prevRoot, m.root);
      const inn = !!n && n.container && samePath(n.path, m.prevRoot);
      if (n) cam = { old: rootBox(prev.values(), cur, !inn), now: inner(n.b, inn), out: true };
    }
  }

  // ── 安全网：同一个父块下、前后都在的两块必须有一个分隔方向前后都成立，不然小的那块换一个新身份。
  //    按这一次的父块分组（外壳合并块拆开 / 合上时子块换了父块，也要查：它们照样从旧位置走到新位置） ──
  const groups = new Map<string, MotionTile[]>();
  for (const x of live) {
    const s = was(x.key);
    if (!s) continue;
    const g = x.parentKey ?? "";
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g)!.push(x);
  }
  const jump = new Set<string>();
  for (const k of m.pending ?? []) if (fresh.has(k) && prev.has(k)) jump.add(k);
  for (const g of groups.values()) {
    for (let i = 0; i < g.length; i++) {
      for (let j = i + 1; j < g.length; j++) {
        const a = g[i];
        const b = g[j];
        if (jump.has(a.key) || jump.has(b.key)) continue;
        if (!keepsApart(cur(was(a.key)!), cur(was(b.key)!), a.b, b.b)) jump.add(a.b.w * a.b.h < b.b.w * b.b.h ? a.key : b.key);
      }
    }
  }
  /** 前后都在（不管换没换身份）：子块 / 残影跟着它走 */
  const both = (k: string | null) => !!k && !!was(k) && fresh.has(k);
  /** 前后都在、而且是同一个元素 */
  const stays = (k: string | null) => !!k && both(k) && !jump.has(k);

  // ── 「其他」吸收 / 分出：上一次单独画的块这一次并进了哪块「其他」；这一次单独画的块上一次在哪块「其他」里 ──
  const union = (bs: Box[]): Box | null => {
    if (!bs.length) return null;
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -Infinity;
    let y1 = -Infinity;
    for (const b of bs) {
      x0 = Math.min(x0, b.x);
      y0 = Math.min(y0, b.y);
      x1 = Math.max(x1, b.x + b.w);
      y1 = Math.max(y1, b.y + b.h);
    }
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  };
  /** 上一次的 key → 这一次把它并进去的「其他」 */
  const absorbInto = new Map<string, MotionTile>();
  for (const x of live) for (const k of x.merged ?? []) if (prev.has(k) && !fresh.has(k)) absorbInto.set(k, x);
  /** 这一次的 key → 上一次装着它的「其他」（没有结构位置时的退路） */
  const splitOut = new Map<string, MotionTile>();
  for (const o of prev.values()) for (const k of o.merged ?? []) if (fresh.has(k) && !prev.has(k)) splitOut.set(k, o);
  const splitNew = new Map<string, Box | null>();
  const newUnion = (o: MotionTile) => {
    if (!splitNew.has(o.key)) {
      const bs: Box[] = [];
      for (const [k, src] of splitOut) if (src.key === o.key) bs.push(fresh.get(k)!.b);
      splitNew.set(o.key, union(bs));
    }
    return splitNew.get(o.key)!;
  };
  /**
   * 上一次的「其他」这一次换了 key（比如外壳合并块拆开了，父块变了；行末的小「其他」并进了尾部）：
   * 按合并的子项找它的接班人（重合最多的那块新「其他」）——旧的变成接班人的样子淡出，接班人从旧的那里长出来
   */
  const successor = new Map<string, MotionTile>();
  for (const o of prev.values()) {
    if (!o.merged?.length || fresh.has(o.key)) continue;
    let best: MotionTile | null = null;
    let bestN = 0;
    for (const x of live) {
      if (!x.merged?.length || was(x.key)) continue;
      const n = o.merged.filter((k) => x.merged!.includes(k)).length;
      if (n > bestN) {
        best = x;
        bestN = n;
      }
    }
    if (best) successor.set(o.key, best);
  }
  const absorbedOld = new Map<string, Box | null>();
  const oldUnion = (x: MotionTile) => {
    if (!absorbedOld.has(x.key)) {
      const bs: Box[] = [];
      for (const [k, o] of absorbInto) if (o.key === x.key) bs.push(cur(prev.get(k)!));
      for (const [k, o] of successor) if (o.key === x.key) bs.push(cur(prev.get(k)!));
      let u = union(bs);
      // 它们原来不挨在一起（中间隔着留下来的兄弟）：合起来的框会压住兄弟，不用它
      if (u)
        for (const o of live) {
          if (o.parentKey !== x.parentKey || o.key === x.key || !stays(o.key)) continue;
          const b = cur(was(o.key)!);
          const ix = Math.min(b.x + b.w, u.x + u.w) - Math.max(b.x, u.x);
          const iy = Math.min(b.y + b.h, u.y + u.h) - Math.max(b.y, u.y);
          if (ix > 2 && iy > 2) {
            u = null;
            break;
          }
        }
      absorbedOld.set(x.key, u);
    }
    return absorbedOld.get(x.key)!;
  };

  /** 父块前后都在、是同一个元素：新块可以按它的旧内容区映射 */
  const viaParent = (x: MotionTile): Box | null => {
    let a = x.parentKey;
    while (a && !both(a)) a = fresh.get(a)?.parentKey ?? null;
    if (!a) return null;
    const s = was(a)!;
    const n = fresh.get(a)!;
    // 从父块旧的内容区（扣掉标题栏：父块现在是容器，它的子块就从标题栏下面长出来，不压在标题字上）映射出来
    return mapBox(x.b, inner(n.b, n.container), inner(cur(s), n.container));
  };
  /** 上一次布局里它结构上的位置（同一个父块）：按父块从上一次的目标位置到现在屏幕上的位置映射 */
  const viaSlot = (x: MotionTile): Box | null => {
    const h = m.hidden?.get(x.key);
    if (!h || h.parentKey !== x.parentKey) return null;
    if (h.parentKey === null) return cam ? null : h.b;
    if (!stays(h.parentKey)) return null;
    const p = was(h.parentKey)!;
    return mapBox(h.b, p.b, cur(p));
  };

  // ── 新出现的块：先按规则定起点（父块也是新的块之后跟着父块） ──
  const from = new Map<string, Box>();
  const moved = new Set<string>();
  /** 每个新块可以试的起点（依次） */
  const tries = new Map<string, Box[]>();
  for (const x of live) {
    if (stays(x.key)) {
      if (differs(cur(was(x.key)!), x.b)) moved.add(x.key);
      continue;
    }
    moved.add(x.key);
    const pk = x.parentKey;
    if (pk && fresh.has(pk) && !stays(pk)) continue; // 父块也是新的：之后跟着父块
    const t: Box[] = [];
    const add = (b: Box | null) => {
      if (b) t.push(b);
    };
    // 新的「其他」：从它吸收的那些块原来合起来的地方长出来
    if (x.merged && !was(x.key)) add(oldUnion(x));
    add(viaSlot(x));
    // 没有结构位置（比如上一次的图里没记）时的退路：从装着它的那块旧「其他」里按比例长出来
    const src = splitOut.get(x.key);
    const su = src ? newUnion(src) : null;
    if (src && su && !m.hidden) add(mapBox(x.b, su, cur(src)));
    add(viaParent(x));
    if (!t.length && cam) add(mapBox(x.b, cam.now, cam.old));
    tries.set(x.key, t);
  }

  // ── 安全网（新块）：同一个父块下，起点框和每个兄弟都要有一个分隔方向前后都成立；不然换下一个起点，都不行就原地淡入、等兄弟让开。
  //    父块也是新的：先试跟着父块一起长出来（同一个映射、同一个延迟，不会先露出一个白底空框）；分组按出现的先后（父块所在的组先定） ──
  const late = new Map<string, number>();
  const start = (t: MotionTile) => (stays(t.key) ? cur(was(t.key)!) : (from.get(t.key) ?? t.b));
  const byParent = new Map<string, MotionTile[]>();
  for (const x of live) {
    const g = x.parentKey ?? "";
    if (!byParent.has(g)) byParent.set(g, []);
    byParent.get(g)!.push(x);
  }
  /** a 从 a0 动到 a.b、b 从 b0 动到 b.b 时会不会压在一起（幕布盖住从它下面出来的新块不算） */
  const clash = (a: MotionTile, a0: Box, b: MotionTile, b0: Box) => {
    if (keepsApart(a0, b0, a.b, b.b)) return false;
    const top = a.z > b.z ? a : b.z > a.z ? b : null;
    const low = top === a ? b : a;
    return !(top && isOther(top) && !stays(low.key));
  };
  const decided = new Set<string>();
  for (const [g, list] of byParent) {
    const par = g ? fresh.get(g) : undefined;
    /** 父块也是新出现的：它的起点、延迟已经定了（它所在的组先处理） */
    const parNew = !!par && !stays(par.key);
    const parLate = parNew ? (late.get(par.key) ?? 0) : 0;
    const fixed = list.filter((t) => stays(t.key));
    for (const t of fixed) decided.add(t.key);
    const fresh1 = list.filter((t) => !stays(t.key)).sort((a, b) => area(b.b) - area(a.b));
    for (const x of fresh1) {
      const others = list.filter((o) => o !== x && decided.has(o.key));
      const cands = parNew ? [mapBox(x.b, par.b, from.get(par.key) ?? par.b)] : (tries.get(x.key) ?? []);
      let pick: Box | null = null;
      for (const f of cands) {
        if (others.every((o) => !clash(x, f, o, start(o)))) {
          pick = f;
          break;
        }
      }
      if (pick) {
        from.set(x.key, pick);
        decided.add(x.key);
        if (parLate) late.set(x.key, parLate);
        continue;
      }
      // 原地淡入：等挡路的兄弟让开、父块长到包住它再出现
      from.set(x.key, x.b);
      decided.add(x.key);
      let p = 0;
      for (const o of others) {
        const o0 = start(o);
        if (!clash(x, x.b, o, o0)) continue;
        const s = overlapSpan(o0, o.b, x.b);
        if (s) p = Math.max(p, s[1]);
      }
      if (par) p = Math.max(p, containFrom(inner(start(par), par.container), inner(par.b, par.container), x.b, 2));
      const ms = Math.min(MAX_LATE_MS, Math.max(parLate, dur * timeOf(p, ease) - LEAD_MS));
      if (ms > 0) late.set(x.key, Math.round(ms));
    }
  }

  // ── 离场的块 ──
  const ghosts: Ghost[] = [];
  const liveRks = new Set(live.map((x) => x.rk));
  for (const [k, s] of prev) {
    if (liveRks.has(s.rk) && !jump.has(k)) continue;
    const c = cur(s);
    // 换了 key 的旧「其他」：变成接班人的样子淡出（和长出来的接班人叠在同一个地方交叉淡入淡出）
    const heir = successor.get(k);
    if (heir) {
      ghosts.push({ key: k, to: heir.b, z: Math.max(s.z, heir.z) });
      continue;
    }
    // 并进了「其他」：垫在那块「其他」下面；它是从这些块原来合起来的地方长出来的，就跟着它一起走（一直被它盖着）
    const into = absorbInto.get(k);
    if (into) {
      const f0 = stays(into.key) ? cur(was(into.key)!) : from.get(into.key);
      const inside = f0 && area(c) > 0 && (Math.min(c.x + c.w, f0.x + f0.w) - Math.max(c.x, f0.x)) * (Math.min(c.y + c.h, f0.y + f0.h) - Math.max(c.y, f0.y)) >= 0.9 * area(c);
      if (f0 && inside) {
        ghosts.push({ key: k, to: mapBox(c, f0, into.b), z: into.z - 1 });
        continue;
      }
    }
    // 这一次布局里它结构上的位置（被挤成一条线 / 在「其他」框里那一格）：缩回那里，沿原来的行 / 列，不横穿；垫在兄弟下面
    const slot = m.hiddenNow?.get(k);
    if (slot && slot.parentKey === s.parentKey && (slot.parentKey === null ? !cam : stays(slot.parentKey))) {
      const n = slot.parentKey === null ? null : fresh.get(slot.parentKey)!;
      ghosts.push({ key: k, to: slot.b, z: n ? n.z + 1 : 1 });
      continue;
    }
    let a = s.parentKey;
    while (a && !both(a)) a = prev.get(a)?.parentKey ?? null;
    if (a) {
      const o = prev.get(a)!;
      const n = fresh.get(a)!;
      // 父块变成了叶子：缩进它标题栏以下的地方（新写的名字在上面，不叠字）；叶子太矮就缩进整块
      const into2 = n.container || n.b.h > 2 * m.head ? inner(n.b, true) : n.b;
      const to = mapBox(c, inner(cur(o), true), into2);
      // 那一块还是容器：垫在它的子块下面（不会夹在容器和子块中间）；变成了叶子：压在它上面、缩进去（F5，保持原来的上下层次）
      const z = n.container ? n.z + 1 : Math.max(s.z, n.z + (s.z - o.z));
      ghosts.push({ key: k, to, z });
    } else if (cam) {
      const to = mapBox(c, cam.old, cam.now);
      if (cam.out) {
        // 拉远：旧图整个缩进新图里的那一块，残影压在那一块上面
        let host: MotionTile | null = null;
        for (const x of live) if (x.path && m.prevRoot && samePath(x.path, m.prevRoot)) host = x;
        if (!host) for (const x of live) if (x.path && m.prevRoot && isUnder(m.prevRoot, x.path) && (!host || x.depth > host.depth)) host = x;
        ghosts.push({ key: k, to, z: (host?.z ?? 0) + s.z + 1 });
      } else {
        // 推近：新根以外的块移出画面，垫在最底下
        ghosts.push({ key: k, to, z: Math.min(s.z, 1) });
      }
    } else ghosts.push({ key: k, to: c, z: 1 });
  }
  return { jump, from, ghosts, moved, late, cam };
}
