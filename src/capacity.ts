// 首页容量柱的布局（竖放 / 横放；纯函数，便于单元测试，见 capacity.test.ts）。
// 难点（设计稿 README 第 2 点）：四段比例随机器变化，“剩余”只有几 GB 时段高可能只有十几像素，
// 标签会叠在一起。做法：
// 1. 段高按字节比例分配，但每个非空段至少 minSeg 像素（从更高的段里按比例扣回来）；
// 2. 标签先放在各自段的中点，再上下挤开（相邻至少 gap），并限制在柱子的上下边界内；
// 3. 标签离开自己段中点较远时画一条折线引线指回去。

export interface Seg {
  key: string;
  bytes: number;
}

export interface PlacedSeg extends Seg {
  top: number;
  height: number;
}

/**
 * 段高：总高 height（含段间缝 sep），按字节比例分配，非空段至少 minSeg。
 * 字节为 0 的段高度为 0（不画、不写标签）。返回的 top/height 都是整数像素，合计 = height。
 */
export function layoutSegments(segs: Seg[], height: number, minSeg = 14, sep = 2): PlacedSeg[] {
  const live = segs.filter((s) => s.bytes > 0);
  const gaps = Math.max(0, live.length - 1) * sep;
  const avail = Math.max(0, height - gaps);
  const total = live.reduce((a, s) => a + s.bytes, 0);
  const h = new Map<string, number>();
  if (total > 0 && avail > 0) {
    let raw = live.map((s) => (s.bytes / total) * avail);
    const floor = Math.min(minSeg, avail / Math.max(1, live.length));
    // 反复：把低于 floor 的段抬到 floor，从其余段按比例扣
    for (let round = 0; round < 5; round++) {
      const low = raw.map((v) => v < floor);
      if (!low.some(Boolean)) break;
      const need = raw.reduce((a, v, i) => a + (low[i] ? floor - v : 0), 0);
      const pool = raw.reduce((a, v, i) => a + (low[i] ? 0 : v - floor), 0);
      if (pool <= 0) {
        raw = raw.map(() => avail / raw.length);
        break;
      }
      raw = raw.map((v, i) => (low[i] ? floor : v - ((v - floor) / pool) * need));
    }
    // 取整：最大余数法，保证合计正好 = avail
    const ints = raw.map((v) => Math.floor(v));
    let rest = avail - ints.reduce((a, v) => a + v, 0);
    const order = raw.map((v, i) => ({ i, frac: v - Math.floor(v) })).sort((a, b) => b.frac - a.frac);
    for (let k = 0; rest > 0 && k < order.length; k++, rest--) ints[order[k].i]++;
    live.forEach((s, i) => h.set(s.key, ints[i]));
  }
  let y = 0;
  const out: PlacedSeg[] = [];
  let first = true;
  for (const s of segs) {
    const hh = h.get(s.key) ?? 0;
    if (hh > 0 && !first) y += sep;
    out.push({ ...s, top: y, height: hh });
    if (hh > 0) {
      y += hh;
      first = false;
    }
  }
  return out;
}

export interface LabelPos {
  /** 标签中心的纵坐标 */
  y: number;
  /** 想放的位置（段中点） */
  want: number;
  /** 需要引线（离开段中点超过 leaderPx） */
  leader: boolean;
}

/**
 * 标签防重叠：want 是各标签想放的中心（按从上到下的顺序给），相邻中心至少差 gap，
 * 全部落在 [minY, maxY] 内。尽量贴近 want（先往下推、再从底往上推、再从顶往下推）。
 * 标签太多放不下时均匀铺满。
 */
export function layoutLabels(want: number[], gap: number, minY: number, maxY: number, leaderPx = 6): LabelPos[] {
  const n = want.length;
  if (!n) return [];
  const span = maxY - minY;
  let y: number[];
  if ((n - 1) * gap > span) {
    y = want.map((_, i) => (n === 1 ? (minY + maxY) / 2 : minY + (span * i) / (n - 1)));
  } else {
    y = want.map((w) => Math.min(maxY, Math.max(minY, w)));
    for (let i = 1; i < n; i++) y[i] = Math.max(y[i], y[i - 1] + gap);
    if (y[n - 1] > maxY) {
      y[n - 1] = maxY;
      for (let i = n - 2; i >= 0; i--) y[i] = Math.min(y[i], y[i + 1] - gap);
    }
    if (y[0] < minY) {
      y[0] = minY;
      for (let i = 1; i < n; i++) y[i] = Math.max(y[i], y[i - 1] + gap);
    }
  }
  return y.map((v, i) => ({ y: v, want: want[i], leader: Math.abs(v - want[i]) > leaderPx }));
}

/** 字节 → “14.0” 这种 GB 数字（≥ 100 GB 不带小数，和设计稿一致） */
export function gbNum(bytes: number): string {
  const g = bytes / 1024 ** 3;
  if (g >= 100) return g.toFixed(0);
  if (g >= 10) return g.toFixed(1);
  return g.toFixed(g >= 1 ? 1 : 2);
}

/** 字节 → [数字, 单位]：小于 1 GB 用 MB（行内大小用） */
export function sizeParts(bytes: number): [string, string] {
  const g = bytes / 1024 ** 3;
  if (g >= 1) return [g >= 100 ? g.toFixed(0) : g.toFixed(g >= 10 ? 1 : 2), "GB"];
  const m = bytes / 1024 ** 2;
  if (m >= 1) return [m >= 100 ? m.toFixed(0) : m.toFixed(1), "MB"];
  const k = bytes / 1024;
  return [k >= 1 ? k.toFixed(0) : String(bytes), k >= 1 ? "KB" : "B"];
}

// ───── 横放的容量柱（阶段 J：窄窗口 / 竖屏时首页容量柱横过来，标签在柱子下面） ─────

/**
 * “清理后剩余”水位：从柱子起点（剩余那头）往里 afterBytes 字节处的像素位置。
 * 按已经排好的段（含最小段高和段间缝）换算，竖放 / 横放共用。没有或 ≤ 0 时为 null。
 */
export function afterOffset(placed: PlacedSeg[], afterBytes: number | null | undefined): number | null {
  if (afterBytes == null || afterBytes <= 0) return null;
  let acc = 0;
  for (const p of placed) {
    if (p.height <= 0) continue;
    if (afterBytes <= acc + p.bytes + 1) {
      const f = p.bytes > 0 ? (afterBytes - acc) / p.bytes : 1;
      return Math.round(p.top + p.height * Math.min(1, Math.max(0, f)));
    }
    acc += p.bytes;
  }
  return null;
}

export interface HSeg extends Seg {
  left: number;
  width: number;
}

export interface HLabel {
  key: string;
  /** 标签框的左边 */
  left: number;
  /** 标签框中心 */
  x: number;
  /** 想放的位置（段中点） */
  want: number;
  leader: boolean;
}

/**
 * 横放：段宽按字节比例（复用 layoutSegments，非空段至少 minSeg 像素），
 * 标签框（宽 labelW）放在各段中点下方，用 layoutLabels 一维挤开（相邻中心至少 labelW + space），
 * 并整个落在 [0, width] 内。字节为 0 的段不出标签。
 */
export function layoutHorizontal(
  segs: Seg[],
  width: number,
  labelW: number,
  opt: { minSeg?: number; sep?: number; space?: number } = {},
): { segs: HSeg[]; labels: HLabel[] } {
  const placed = layoutSegments(segs, width, opt.minSeg ?? 14, opt.sep ?? 2);
  const out = placed.map((p) => ({ key: p.key, bytes: p.bytes, left: p.top, width: p.height }));
  const live = out.filter((p) => p.width > 0);
  const half = labelW / 2;
  const pos = layoutLabels(
    live.map((p) => p.left + p.width / 2),
    labelW + (opt.space ?? 8),
    Math.min(half, width / 2),
    Math.max(width - half, width / 2),
  );
  return {
    segs: out,
    labels: live.map((p, i) => ({ key: p.key, left: Math.round(pos[i].y - half), x: pos[i].y, want: pos[i].want, leader: pos[i].leader })),
  };
}
