// 方块图的悬停卡片 / 钉住的卡片放在哪（阶段 L）：卡片挂在 <body> 上、position: fixed，坐标都是窗口坐标。
// 卡片的大小是量出来的（内容全部写出来、不截断，高度不固定），放不下就翻到另一边，最后夹在给定的范围里：永远整张看得见。
// 纯函数（TreemapView 调，测试直接测）。

export interface PlaceBox {
  x: number;
  y: number;
  w: number;
  h: number;
}
export interface PlaceSize {
  w: number;
  h: number;
}

/** 夹在 [lo, hi] 里；卡片比范围还大（hi < lo）时贴着 lo（左边 / 上边总是露出来） */
function clampIn(v: number, lo: number, hi: number): number {
  return hi < lo ? lo : Math.min(Math.max(v, lo), hi);
}

/** 两个框重叠的面积（不重叠 = 0） */
function overlapArea(a: PlaceBox, b: PlaceBox): number {
  const w = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
  return w > 0 && h > 0 ? w * h : 0;
}

/**
 * 跟着鼠标的卡片：默认在指针右下（dx, dy）；右边放不下翻到指针左边，下面放不下翻到指针上面（离指针 up）；
 * 最后夹在 area 里（四周留 pad）。
 * avoid = 别压住的框（钉住的卡片）：默认的位置压住了它，就换到指针另一边（先左右换、再上下换、再都换），
 * 只换到不用夹就整张放得下的那一边（夹过去的卡片会盖住指针）；哪边都压，取压得最少的（一样多就用默认的）。
 */
export function placeAtPointer(
  p: { x: number; y: number },
  s: PlaceSize,
  area: PlaceBox,
  avoid: PlaceBox | null = null,
  o: { dx: number; dy: number; up: number; pad: number } = { dx: 14, dy: 16, up: 10, pad: 8 },
): { x: number; y: number } {
  const L = area.x + o.pad;
  const T = area.y + o.pad;
  const R = area.x + area.w - o.pad;
  const B = area.y + area.h - o.pad;
  const xs = { right: p.x + o.dx, left: p.x - o.dx - s.w };
  const ys = { down: p.y + o.dy, up: p.y - o.up - s.h };
  const fitsX = { right: xs.right + s.w <= R, left: xs.left >= L };
  const fitsY = { down: ys.down + s.h <= B, up: ys.up >= T };
  const at = (h: "right" | "left", v: "down" | "up") => ({ x: clampIn(xs[h], L, R - s.w), y: clampIn(ys[v], T, B - s.h) });
  const h0 = fitsX.right ? "right" : "left";
  const v0 = fitsY.down ? "down" : "up";
  const first = at(h0, v0);
  const hit = (q: { x: number; y: number }) => (avoid ? overlapArea({ ...q, w: s.w, h: s.h }, avoid) : 0);
  let best = first;
  let bestHit = hit(first);
  if (bestHit === 0) return first;
  const h1 = h0 === "right" ? "left" : "right";
  const v1 = v0 === "down" ? "up" : "down";
  for (const [h, v] of [
    [h1, v0],
    [h0, v1],
    [h1, v1],
  ] as const) {
    // 换过去的那一边要放得下（没换的那一边和默认的一样）
    if ((h !== h0 && !fitsX[h]) || (v !== v0 && !fitsY[v])) continue;
    const q = at(h, v);
    const n = hit(q);
    if (n < bestHit) {
      best = q;
      bestHit = n;
      if (n === 0) break;
    }
  }
  return best;
}

/**
 * 钉在块旁边的卡片（b = 块的框）：先放块的右边、上边对齐块的上边；放不下放左边；
 * 左右都放不下就放块的下面（水平对着块的中间），再不行放上面；都不行（块几乎铺满）就压在块里面的右下角（不挡块左上角的名字）。
 * 始终夹在 area 里（四周留 pad）。
 */
export function placeBeside(b: PlaceBox, s: PlaceSize, area: PlaceBox, gap = 10, pad = 6): { x: number; y: number } {
  const L = area.x + pad;
  const T = area.y + pad;
  const R = area.x + area.w - pad;
  const B = area.y + area.h - pad;
  const sideY = clampIn(b.y, T, B - s.h);
  if (b.x + b.w + gap + s.w <= R) return { x: b.x + b.w + gap, y: sideY };
  if (b.x - gap - s.w >= L) return { x: b.x - gap - s.w, y: sideY };
  const midX = clampIn(b.x + b.w / 2 - s.w / 2, L, R - s.w);
  if (b.y + b.h + gap + s.h <= B) return { x: midX, y: b.y + b.h + gap };
  if (b.y - gap - s.h >= T) return { x: midX, y: b.y - gap - s.h };
  return { x: clampIn(b.x + b.w - gap - s.w, L, R - s.w), y: clampIn(b.y + b.h - gap - s.h, T, B - s.h) };
}
