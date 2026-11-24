import { describe, expect, it } from "vitest";
import { placeAtPointer, placeBeside } from "./tipplace";

// 窗口 1000 × 600；卡片 280 × 150
const WIN = { x: 0, y: 0, w: 1000, h: 600 };
const S = { w: 280, h: 150 };
const inside = (p: { x: number; y: number }, s = S, a = WIN, pad = 8) =>
  p.x >= a.x + pad && p.y >= a.y + pad && p.x + s.w <= a.x + a.w - pad && p.y + s.h <= a.y + a.h - pad;

describe("placeAtPointer：跟着鼠标的卡片", () => {
  it("地方够：在指针右下", () => {
    expect(placeAtPointer({ x: 100, y: 100 }, S, WIN)).toEqual({ x: 114, y: 116 });
  });
  it("右边放不下：翻到指针左边（不压住指针）", () => {
    const p = placeAtPointer({ x: 900, y: 100 }, S, WIN);
    expect(p).toEqual({ x: 900 - 14 - 280, y: 116 });
    expect(inside(p)).toBe(true);
  });
  it("下面放不下：翻到指针上面", () => {
    const p = placeAtPointer({ x: 100, y: 560 }, S, WIN);
    expect(p).toEqual({ x: 114, y: 560 - 10 - 150 });
    expect(inside(p)).toBe(true);
  });
  it("右下角：左、上都翻，整张在窗口里", () => {
    const p = placeAtPointer({ x: 990, y: 595 }, S, WIN);
    expect(p.x + S.w).toBeLessThan(990);
    expect(p.y + S.h).toBeLessThan(595);
    expect(inside(p)).toBe(true);
  });
  it("翻过去也放不下（窄窗口 / 卡片很高）：夹在窗口里", () => {
    const tall = { w: 280, h: 500 };
    const p = placeAtPointer({ x: 300, y: 300 }, tall, WIN);
    expect(inside(p, tall)).toBe(true);
    const narrow = { x: 0, y: 0, w: 320, h: 600 };
    const q = placeAtPointer({ x: 160, y: 100 }, S, narrow);
    expect(inside(q, S, narrow)).toBe(true);
  });
  it("卡片比窗口还大：贴着左上角（开头总是看得见）", () => {
    expect(placeAtPointer({ x: 50, y: 50 }, { w: 2000, h: 900 }, WIN)).toEqual({ x: 8, y: 8 });
  });
});

describe("placeAtPointer：别压住钉住的卡片（avoid）", () => {
  const box = (p: { x: number; y: number }, s = S) => ({ ...p, w: s.w, h: s.h });
  const hits = (a: { x: number; y: number; w: number; h: number }, b: { x: number; y: number; w: number; h: number }) =>
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
  it("默认的位置没压住：和不给 avoid 一样", () => {
    const pin = { x: 600, y: 50, w: 280, h: 150 };
    expect(placeAtPointer({ x: 100, y: 100 }, S, WIN, pin)).toEqual(placeAtPointer({ x: 100, y: 100 }, S, WIN));
  });
  it("右下压住了钉住的卡片：换到指针左边", () => {
    const pin = { x: 420, y: 300, w: 280, h: 200 };
    const p = placeAtPointer({ x: 400, y: 280 }, S, WIN, pin);
    expect(p).toEqual({ x: 400 - 14 - 280, y: 296 });
    expect(hits(box(p), pin)).toBe(false);
  });
  it("左右都压住（钉住的卡片很宽）：换到指针上面", () => {
    const pin = { x: 0, y: 300, w: 1000, h: 200 };
    const p = placeAtPointer({ x: 400, y: 280 }, S, WIN, pin);
    expect(p).toEqual({ x: 414, y: 280 - 10 - 150 });
    expect(hits(box(p), pin)).toBe(false);
  });
  it("靠右下角（默认已经翻到左上）压住了，右边 / 下边又放不下：留在默认的位置（不夹过去盖住指针）", () => {
    const pin = { x: 500, y: 380, w: 300, h: 200 };
    const p = placeAtPointer({ x: 990, y: 595 }, S, WIN, pin);
    expect(p).toEqual(placeAtPointer({ x: 990, y: 595 }, S, WIN));
    expect(inside(p)).toBe(true);
  });
  it("哪边都压住：取压得最少的那边", () => {
    // 指针在 (500, 300)：右下 42000、左下 27900、右上 25200、左上 16740（px²）
    const pin = { x: 300, y: 200, w: 500, h: 300 };
    const p = placeAtPointer({ x: 500, y: 300 }, S, WIN, pin);
    expect(p).toEqual({ x: 500 - 14 - 280, y: 300 - 10 - 150 });
  });
  it("换过去的那边放不下（会被夹回来盖住指针）：不换", () => {
    const pin = { x: 120, y: 120, w: 300, h: 300 };
    const p = placeAtPointer({ x: 100, y: 100 }, S, WIN, pin);
    // 左边、上边都只有 100 px：放不下，留在默认的右下（压住也只能这样）
    expect(p).toEqual({ x: 114, y: 116 });
  });
});

describe("placeBeside：钉在块旁边的卡片", () => {
  const MAP = { x: 20, y: 100, w: 900, h: 480 };
  const inMap = (p: { x: number; y: number }) => inside(p, S, MAP, 6);
  it("右边放得下：放在块的右边，上边对齐", () => {
    expect(placeBeside({ x: 40, y: 150, w: 100, h: 80 }, S, MAP)).toEqual({ x: 150, y: 150 });
  });
  it("靠下的块：卡片往上挪，底边不出图", () => {
    const p = placeBeside({ x: 40, y: 540, w: 100, h: 30 }, S, MAP);
    expect(p.x).toBe(150);
    expect(p.y + S.h).toBe(MAP.y + MAP.h - 6);
    expect(inMap(p)).toBe(true);
  });
  it("右边放不下：放左边", () => {
    expect(placeBeside({ x: 700, y: 150, w: 200, h: 80 }, S, MAP)).toEqual({ x: 700 - 10 - 280, y: 150 });
  });
  it("左右都放不下（很宽的块）：放下面，再不行放上面", () => {
    const below = placeBeside({ x: 30, y: 120, w: 880, h: 100 }, S, MAP);
    expect(below.y).toBe(230);
    expect(inMap(below)).toBe(true);
    const above = placeBeside({ x: 30, y: 400, w: 880, h: 170 }, S, MAP);
    expect(above.y).toBe(400 - 10 - 150);
    expect(inMap(above)).toBe(true);
  });
  it("块几乎铺满整张图：压在块里面的右下角，整张在图里", () => {
    const p = placeBeside({ x: 24, y: 104, w: 892, h: 472 }, S, MAP);
    expect(inMap(p)).toBe(true);
    expect(p.x + S.w).toBe(24 + 892 - 10);
    expect(p.y + S.h).toBe(104 + 472 - 10);
  });
});
