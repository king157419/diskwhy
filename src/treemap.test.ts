import { describe, expect, it } from "vitest";
import { crumbTarget, DEFAULT_LAYOUT, hitTest, layoutTreemap, samePath, squarify, type Rect, type TmInput } from "./treemap";

const area = (r: Rect) => r.w * r.h;
const EPS = 1e-6;

function overlaps(a: Rect, b: Rect): boolean {
  const ix = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
  const iy = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
  return ix > 1e-6 && iy > 1e-6;
}

function inside(r: Rect, outer: Rect): boolean {
  return r.x >= outer.x - EPS && r.y >= outer.y - EPS && r.x + r.w <= outer.x + outer.w + EPS && r.y + r.h <= outer.y + outer.h + EPS;
}

describe("squarify", () => {
  it("论文里的经典例子：6,6,4,3,2,2,1 铺满 6×4，面积成比例、互不重叠", () => {
    const vals = [6, 6, 4, 3, 2, 2, 1];
    const rect = { x: 0, y: 0, w: 6, h: 4 };
    const out = squarify(vals.map((v, i) => ({ value: v, data: i })), rect);
    expect(out).toHaveLength(7);
    const total = out.reduce((s, p) => s + area(p.rect), 0);
    expect(total).toBeCloseTo(24, 6);
    for (const p of out) {
      expect(area(p.rect)).toBeCloseTo(p.value, 6); // 总值 24 = 面积 24
      expect(inside(p.rect, rect)).toBe(true);
    }
    for (let i = 0; i < out.length; i++) for (let j = i + 1; j < out.length; j++) expect(overlaps(out[i].rect, out[j].rect)).toBe(false);
    // 前两块（6,6）按算法应各是 3×2 的方块
    expect(out[0].rect.w).toBeCloseTo(3, 6);
    expect(out[0].rect.h).toBeCloseTo(2, 6);
  });

  it("长宽比比 slice-and-dice 好：200 个随机块最差长宽比的中位数 < 3", () => {
    let seed = 42;
    const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
    const items = Array.from({ length: 200 }, (_, i) => ({ value: Math.floor(rnd() * 1e9) + 1, data: i }));
    const rect = { x: 0, y: 0, w: 1000, h: 600 };
    const out = squarify(items, rect);
    const ratios = out.map((p) => Math.max(p.rect.w / p.rect.h, p.rect.h / p.rect.w)).sort((a, b) => a - b);
    expect(ratios[Math.floor(ratios.length / 2)]).toBeLessThan(3);
    expect(out.reduce((s, p) => s + area(p.rect), 0)).toBeCloseTo(600000, 3);
  });

  it("值为 0 / 负数的项被忽略；空输入、零面积返回空", () => {
    expect(squarify([{ value: 0, data: 1 }, { value: -5, data: 2 }], { x: 0, y: 0, w: 10, h: 10 })).toHaveLength(0);
    expect(squarify([{ value: 3, data: 1 }], { x: 0, y: 0, w: 0, h: 10 })).toHaveLength(0);
    const one = squarify([{ value: 3, data: 1 }], { x: 5, y: 5, w: 10, h: 20 });
    expect(one[0].rect).toEqual({ x: 5, y: 5, w: 10, h: 20 });
  });
});

describe("layoutTreemap", () => {
  const tree: TmInput = {
    key: "C:",
    name: "C:",
    value: 952.5,
    children: [
      {
        key: "Users",
        name: "Users",
        value: 600,
        children: [
          { key: "Users/a", name: "a", value: 400, children: [{ key: "Users/a/x", name: "x", value: 400 }] },
          { key: "Users/b", name: "b", value: 199.9 },
          { key: "Users/tiny", name: "tiny", value: 0.1 },
        ],
      },
      { key: "Windows", name: "Windows", value: 300 },
      ...Array.from({ length: 50 }, (_, i) => ({ key: `small${i}`, name: `s${i}`, value: 0.05 })),
    ],
    hidden: 50,
    hiddenCount: 7,
  };
  const rect = { x: 0, y: 0, w: 800, h: 500 };

  it("小于 6px 的块并进“其他”，其余块都 ≥ 6px；同层互不重叠、都在父块里面", () => {
    const items = layoutTreemap(tree, rect);
    for (const it of items) expect(Math.min(it.rect.w, it.rect.h)).toBeGreaterThanOrEqual(DEFAULT_LAYOUT.minPx - EPS);
    const top = items.filter((i) => i.depth === 1);
    const other = top.find((i) => i.kind === "other")!;
    expect(other).toBeTruthy();
    // 后端没发来的 50 + 50 个 0.05 的小块（800×500 里每个约 21px²，最短边 < 6px）
    expect(other.value).toBeCloseTo(52.5, 6);
    expect(other.mergedCount).toBe(57);
    for (let i = 0; i < top.length; i++) for (let j = i + 1; j < top.length; j++) expect(overlaps(top[i].rect, top[j].rect)).toBe(false);
    // 根层面积按值分配
    const users = top.find((i) => i.key === "Users")!;
    expect(area(users.rect) / area(rect)).toBeCloseTo(600 / 952.5, 6);
    // 子块在父块里面
    const byKey = new Map(items.map((i) => [i.key, i]));
    for (const it of items) if (it.parentKey) expect(inside(it.rect, byKey.get(it.parentKey)!.rect)).toBe(true);
  });

  it("最多嵌套 maxDepth 层；够大的目录画成容器", () => {
    const items = layoutTreemap(tree, rect, { ...DEFAULT_LAYOUT, maxDepth: 2 });
    expect(Math.max(...items.map((i) => i.depth))).toBe(2);
    expect(items.find((i) => i.key === "Users")!.container).toBe(true);
    expect(items.find((i) => i.key === "Users/a")!.container).toBe(false);
    const deep = layoutTreemap(tree, rect);
    expect(deep.some((i) => i.key === "Users/a/x" && i.depth === 3)).toBe(true);
    // 0.1 字节的 tiny 在 Users 里被并进 Users 的“其他”，但“其他”本身也太小就不画
    expect(deep.some((i) => i.key === "Users/tiny")).toBe(false);
  });

  it("命中测试返回最深的块", () => {
    const items = layoutTreemap(tree, rect);
    const x = items.find((i) => i.key === "Users/a/x")!;
    const hit = hitTest(items, x.rect.x + x.rect.w / 2, x.rect.y + x.rect.h / 2);
    expect(hit?.key).toBe("Users/a/x");
    expect(hitTest(items, -5, -5)).toBeNull();
  });

  it("性能：根层 200 块 + 两层嵌套的布局 < 50ms", () => {
    const big: TmInput = {
      key: "r",
      name: "r",
      value: 0,
      children: Array.from({ length: 200 }, (_, i) => ({
        key: `d${i}`,
        name: `d${i}`,
        value: 1000 - i * 4,
        children: Array.from({ length: 30 }, (_, j) => ({ key: `d${i}/${j}`, name: `${j}`, value: 30 - j })),
      })),
    };
    big.value = big.children!.reduce((s, c) => s + c.value, 0);
    const t0 = performance.now();
    const items = layoutTreemap(big, { x: 0, y: 0, w: 1280, h: 560 });
    const ms = performance.now() - t0;
    expect(items.length).toBeGreaterThan(200);
    expect(ms).toBeLessThan(50);
  });
});

import { collapseChain, type ChainNode } from "./treemap";

describe("collapseChain（外壳文件夹合并）", () => {
  const dir = (name: string, allocated: number, children: ChainNode[] = []): ChainNode => ({ name, kind: "Dir", allocated, children });
  it("几乎全是一个子文件夹时合成一块", () => {
    const king = dir("king", 99, [dir("AppData", 60, [dir("x", 1)]), dir("下载", 39)]);
    const users = dir("Users", 100, [king, dir("Public", 1)]);
    const r = collapseChain(users);
    expect(r.node).toBe(king);
    expect(r.name).toBe("Users \\ king");
  });
  it("子文件夹不够大、或者没有更深的数据时不合并", () => {
    const a = dir("a", 100, [dir("b", 80, [dir("c", 80)]), dir("d", 20)]);
    expect(collapseChain(a).node).toBe(a);
    const e = dir("e", 100, [dir("f", 100)]);
    expect(collapseChain(e).node).toBe(e);
  });
});

import { tileBox, type TileTokens } from "./treemap";

describe("tileBox（阶段 I：圆角 / 描边 / 投影按块大小收敛）", () => {
  const C: TileTokens = { gap: 4, r: 6, bw: 2, shadow: 4, flatPx: 16, shadowMinPx: 48, patternMinPx: 24 };

  it("圆角 = min(主题上限, w/4, h/4)，大块封顶在主题上限", () => {
    expect(tileBox({ x: 0, y: 0, w: 204, h: 104 }, C, false).r).toBe(6);
    const b = tileBox({ x: 0, y: 0, w: 24, h: 60 }, C, false); // 块面 20×56
    expect(b.w).toBe(20);
    expect(b.r).toBe(5);
    expect(tileBox({ x: 0, y: 0, w: 22, h: 22 }, C, false).r).toBe(4); // 18 / 4 → 4
  });

  it("最短边 < 16px 画平块：没有描边 / 圆角 / 投影 / 条纹", () => {
    const b = tileBox({ x: 10, y: 10, w: 19, h: 80 }, C);
    expect(b).toMatchObject({ w: 15, flat: true, r: 0, bw: 0, shadow: 0, pattern: false });
  });

  it("投影只给 ≥ 48px 的块，而且画在格子里面：块面 + 投影 + 间隙 = 格子，不会压到邻块", () => {
    const small = tileBox({ x: 0, y: 0, w: 50, h: 40 }, C);
    expect(small.shadow).toBe(0);
    const cell = { x: 100.4, y: 50.6, w: 200.3, h: 120.2 };
    const b = tileBox(cell, C);
    expect(b.shadow).toBe(4);
    expect(b.x + b.w + b.shadow + C.gap).toBe(Math.round(cell.x + cell.w));
    expect(b.y + b.h + b.shadow + C.gap).toBe(Math.round(cell.y + cell.h));
  });

  it("像素对齐后相邻格子之间的空隙处处一样宽", () => {
    const xs = [0, 33.3, 66.7, 101.2];
    for (let i = 0; i + 1 < xs.length; i++) {
      const a = tileBox({ x: xs[i], y: 0, w: xs[i + 1] - xs[i], h: 30 }, { ...C, shadow: 0 });
      const n = tileBox({ x: xs[i + 1], y: 0, w: 40, h: 30 }, { ...C, shadow: 0 });
      expect(n.x - (a.x + a.w)).toBe(C.gap);
    }
  });

  it("斜条纹只在块面 ≥ 24px 时出现", () => {
    expect(tileBox({ x: 0, y: 0, w: 30, h: 30 }, C, false).pattern).toBe(true); // 26
    expect(tileBox({ x: 0, y: 0, w: 26, h: 30 }, C, false).pattern).toBe(false); // 22
  });
});

describe("尾巴合并（阶段 I）", () => {
  // 一个文件夹：几个大子项 + 一长串越来越小的尾巴（像 Windows / Program Files 那样）
  const kids: TmInput[] = [
    { key: "a", name: "a", value: 5000 },
    { key: "b", name: "b", value: 2500 },
    { key: "c", name: "c", value: 1200 },
    ...Array.from({ length: 60 }, (_, i) => ({ key: `t${i}`, name: `t${i}`, value: 300 / (i + 1) })),
  ];
  const total = kids.reduce((s, k) => s + k.value, 0);
  const tree: TmInput = { key: "r", name: "r", value: total, children: kids };
  const rect = { x: 0, y: 0, w: 300, h: 200 };

  it("太小的尾巴整段并成唯一一个“其他 N 项”，剩下的块都 ≥ minPx，且“其他”之外的块都比被并掉的大", () => {
    const items = layoutTreemap(tree, rect, { ...DEFAULT_LAYOUT, minPx: 8 });
    const others = items.filter((i) => i.kind === "other");
    expect(others).toHaveLength(1);
    const o = others[0];
    expect(o.name).toBe(`其他 ${o.mergedCount} 项`);
    expect(o.mergedCount).toBeGreaterThan(10);
    const nodes = items.filter((i) => i.kind === "node");
    expect(nodes.length + o.mergedCount).toBe(kids.length);
    for (const it of items) expect(Math.min(it.rect.w, it.rect.h)).toBeGreaterThanOrEqual(8 - EPS);
    // 是“尾巴”：留下来的块的最小值 > 被合并的块的最大值
    const kept = new Set(nodes.map((n) => n.key));
    const minKept = Math.min(...kids.filter((k) => kept.has(k.key)).map((k) => k.value));
    const maxMerged = Math.max(...kids.filter((k) => !kept.has(k.key)).map((k) => k.value));
    expect(minKept).toBeGreaterThan(maxMerged);
    expect(o.value).toBeCloseTo(kids.filter((k) => !kept.has(k.key)).reduce((s, k) => s + k.value, 0), 6);
  });

  it("“其他”自己也太小时不画，也不留空洞：兄弟块铺满整个格子", () => {
    const t2: TmInput = { key: "r", name: "r", value: 1000.01, children: [{ key: "a", name: "a", value: 600 }, { key: "b", name: "b", value: 400 }, { key: "z", name: "z", value: 0.01 }] };
    const items = layoutTreemap(t2, rect);
    expect(items.some((i) => i.kind === "other" || i.key === "z")).toBe(false);
    expect(items.reduce((s, i) => s + area(i.rect), 0)).toBeCloseTo(area(rect), 6);
  });
});

describe("空间分析导航（阶段 J）", () => {
  it("samePath：大小写、末尾反斜杠不敏感", () => {
    expect(samePath("C:\\", "c:")).toBe(true);
    expect(samePath("C:\\Users\\", "c:\\users")).toBe(true);
    expect(samePath("C:\\Users", "C:\\Users\\king")).toBe(false);
    expect(samePath(null, "C:\\")).toBe(false);
  });
  it("crumbTarget：第 0 级打开并选中盘根，其余级打开上一级、选中自己", () => {
    const bc = [{ path: "C:\\" }, { path: "C:\\Users" }, { path: "C:\\Users\\king" }];
    expect(crumbTarget(bc, 0)).toEqual({ open: "C:\\", focus: "C:\\" });
    expect(crumbTarget(bc, 1)).toEqual({ open: "C:\\", focus: "C:\\Users" });
    expect(crumbTarget(bc, 2)).toEqual({ open: "C:\\Users", focus: "C:\\Users\\king" });
    expect(crumbTarget(bc, 3)).toBeNull();
    expect(crumbTarget([], 0)).toBeNull();
  });
});
