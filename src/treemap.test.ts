import { describe, expect, it } from "vitest";
import { BOOST, boostWeight, DEFAULT_LAYOUT, hitTest, layoutTreemap, samePath, squarify, squarifyBoosted, type Rect, type TmInput, type TmItem } from "./treemap";

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

describe("空间分析导航", () => {
  it("samePath：大小写、末尾反斜杠不敏感", () => {
    expect(samePath("C:\\", "c:")).toBe(true);
    expect(samePath("C:\\Users\\", "c:\\users")).toBe(true);
    expect(samePath("C:\\Users", "C:\\Users\\king")).toBe(false);
    expect(samePath(null, "C:\\")).toBe(false);
  });
});

// ───── 阶段 K：展开链上的块在原地放大（BOOST） ─────

describe("boost：展开链上的块至少占父块内容区的 60%", () => {
  const kids = (vals: number[], boostAt = -1): TmInput[] =>
    vals.map((v, i) => ({ key: `k${i}`, name: `k${i}`, value: v, boost: i === boostAt ? BOOST : undefined }));
  const root = (children: TmInput[], extra: Partial<TmInput> = {}): TmInput => ({
    key: "r",
    name: "r",
    value: children.reduce((s, c) => s + c.value, 0) + (extra.hidden ?? 0),
    children,
    ...extra,
  });
  const rect = { x: 0, y: 0, w: 900, h: 600 };
  const VALS = [500, 300, 120, 80, 60, 40, 30, 20, 10, 5];

  it("有效权重：eff / (eff + 其余) = boost；本来就更大就不动", () => {
    const eff = boostWeight(10, 90, 0.6);
    expect(eff / (eff + 90)).toBeCloseTo(0.6, 9);
    expect(boostWeight(80, 20, 0.6)).toBe(80);
  });

  for (const at of [1, 3, 6]) {
    it(`第 ${at + 1} 大的块被放大：面积 ≥ 60%、不重叠、不出界、兄弟比例 / 值不变（boost 在 ${at}）`, () => {
      const items = layoutTreemap(root(kids(VALS, at)), rect, { ...DEFAULT_LAYOUT, minPx: 1 });
      const top = items.filter((i) => i.depth === 1);
      const b = top.find((i) => i.key === `k${at}`)!;
      expect(b.chain).toBe(true);
      expect(area(b.rect) / area(rect)).toBeGreaterThanOrEqual(BOOST - 1e-9);
      for (let i = 0; i < top.length; i++) {
        expect(inside(top[i].rect, rect)).toBe(true);
        for (let j = i + 1; j < top.length; j++) expect(overlaps(top[i].rect, top[j].rect)).toBe(false);
      }
      // 输出的值是真实值；兄弟之间面积比 = 值比
      for (const it of top) if (it.kind === "node") expect(it.value).toBe(VALS[Number(it.key.slice(1))]);
      const sib = top.filter((i) => i.kind === "node" && i.key !== b.key);
      for (const s of sib) expect(area(s.rect) / area(sib[0].rect)).toBeCloseTo(s.value / sib[0].value, 6);
    });
  }

  it("原地长大：分行不变，任意两块在放大前后都沿同一个方向分开（过渡动画里没有块横穿别的块）", () => {
    const before = squarify(VALS.map((v, i) => ({ value: v, data: i })), rect, true);
    for (const at of [1, 4, 7]) {
      const after = squarifyBoosted(VALS.map((v, i) => ({ value: v, data: i })), rect, (d) => d === at, BOOST);
      const A = new Map(before.map((p) => [p.data, p.rect]));
      const B = new Map(after.map((p) => [p.data, p.rect]));
      // 两块之间的分隔：i 在 j 左边 / 右边 / 上边 / 下边
      const seps = (m: Map<number, Rect>, i: number, j: number) => {
        const a = m.get(i)!;
        const b = m.get(j)!;
        return [a.x + a.w <= b.x + 1e-6, b.x + b.w <= a.x + 1e-6, a.y + a.h <= b.y + 1e-6, b.y + b.h <= a.y + 1e-6];
      };
      for (let i = 0; i < VALS.length; i++)
        for (let j = i + 1; j < VALS.length; j++) {
          const sa = seps(A, i, j);
          const sb = seps(B, i, j);
          // 同一个分隔在前后都成立 → 线性插值的每一帧里它们都不重叠
          expect(sa.some((s, k) => s && sb[k])).toBe(true);
        }
      // 放大的块的新位置和原来的位置有重叠（是“在原地”长大）
      expect(overlaps(A.get(at)!, B.get(at)!)).toBe(true);
    }
  });

  it("本来就 ≥ 60% 的块：布局和不放大完全一样", () => {
    const plain = layoutTreemap(root(kids([700, 200, 100])), rect);
    const boosted = layoutTreemap(root(kids([700, 200, 100], 0)), rect);
    expect(boosted.map((i) => i.rect)).toEqual(plain.map((i) => i.rect));
  });

  it("放大的块再小也不会被并进“其他”；它的兄弟太小照样并进“其他”；被挤细的尾巴不画成细条", () => {
    const vals = [5000, 3000, 2000, ...Array.from({ length: 40 }, () => 1)];
    const r = { x: 0, y: 0, w: 300, h: 200 };
    // 不放大时 k20（值 1）会被并掉
    expect(layoutTreemap(root(kids(vals), { hidden: 500, hiddenCount: 9 }), r).some((i) => i.key === "k20")).toBe(false);
    const items = layoutTreemap(root(kids(vals, 20), { hidden: 500, hiddenCount: 9 }), r);
    const b = items.find((i) => i.key === "k20")!;
    expect(b).toBeTruthy();
    expect(area(b.rect) / (300 * 200)).toBeGreaterThanOrEqual(BOOST - 1e-9);
    // 其余值为 1 的兄弟都不单独画（不会为了露出 k20 把一串细条画出来）
    expect(items.filter((i) => i.kind === "node" && i.value === 1 && i.key !== "k20")).toHaveLength(0);
    // 画出来的块（包括「其他」）最短边都 ≥ minPx：被放大挤细的尾巴要么是一块够大的「其他」，要么交给前一格
    for (const it of items) expect(Math.min(it.rect.w, it.rect.h)).toBeGreaterThanOrEqual(DEFAULT_LAYOUT.minPx - EPS);
    const other = items.find((i) => i.kind === "other");
    if (other) expect(other.mergedCount).toBe(39 + 9);
    // 面积不留洞
    expect(items.reduce((s, i) => s + area(i.rect), 0)).toBeCloseTo(300 * 200, 6);
  });

  it("第 4 轮：被挤小的尾部太薄时接到最后一行末尾成「其他」——回收站这类小项不会从图上消失，前一块也不会被悄悄撑大", () => {
    // 盘根：Users 92.3 / Windows 31.6（展开）/ Program Files 19.3 / hiberfil 12.7 / ProgramData 10.2 / pagefile 8.0 / Program Files (x86) 4.6 /
    // $Recycle.Bin 1.2 / swapfile 0.25 / DumpStack 0.0001（GB）
    const names = ["Users", "Windows", "PF", "hiberfil", "ProgramData", "pagefile", "PF86", "Recycle", "swapfile", "DumpStack"];
    const vals = [92.3, 31.6, 19.3, 12.7, 10.2, 8.0, 4.6, 1.2, 0.25, 0.0001];
    const ch = vals.map((v, i) => ({ key: names[i], name: names[i], value: v, boost: i === 1 ? BOOST : undefined }));
    const items = layoutTreemap(root(ch), { x: 0, y: 0, w: 888, h: 772 }, { ...DEFAULT_LAYOUT, minPx: 24 });
    const by = new Map(items.map((i) => [i.key, i]));
    // 回收站：要么自己画出来，要么在一块「其他」里（写着它的大小）
    const other = items.find((i) => i.kind === "other");
    expect(by.has("Recycle") || !!other?.merged?.includes("Recycle")).toBe(true);
    if (other) {
      expect(Math.min(other.rect.w, other.rect.h)).toBeGreaterThanOrEqual(24 - EPS);
      expect(other.value).toBeCloseTo(1.2 + 0.25 + 0.0001, 6);
    }
    // 没被放大的块：面积 / 值处处一样（Program Files (x86) 不再多吃回收站那 30%）
    const dens = ["PF", "hiberfil", "ProgramData", "pagefile", "PF86"].map((k) => area(by.get(k)!.rect) / by.get(k)!.value);
    for (const d of dens) expect(d / dens[0]).toBeCloseTo(1, 6);
    // 面积不留洞
    expect(items.reduce((s, i) => s + area(i.rect), 0)).toBeCloseTo(888 * 772, 6);
  });

  it("链上的块下面再画两层（pk 的子项和孙项），链外的兄弟照旧只画一层子项", () => {
    // 4 层深的树：r → a → b → c → d → e；a / b 在链上（pk = b），x 是 a 的兄弟
    const leafs = (p: string, n: number, d: number): TmInput[] =>
      Array.from({ length: n }, (_, i) => {
        const key = `${p}/${i}`;
        return { key, name: key, value: 10, children: d > 0 ? leafs(key, n, d - 1) : undefined };
      });
    const sum = (c: TmInput[]) => c.reduce((s, x) => s + x.value, 0);
    const fix = (n: TmInput): TmInput => {
      if (n.children) {
        n.children = n.children.map(fix);
        n.value = sum(n.children);
      }
      return n;
    };
    const b: TmInput = fix({ key: "b", name: "b", value: 0, boost: BOOST, children: leafs("b", 3, 3) });
    const a: TmInput = fix({ key: "a", name: "a", value: 0, boost: BOOST, children: [b, ...leafs("a", 2, 3)] });
    const x: TmInput = fix({ key: "x", name: "x", value: 0, children: leafs("x", 3, 3) });
    const r = fix({ key: "r", name: "r", value: 0, children: [a, x] });
    const opt = { ...DEFAULT_LAYOUT, maxDepth: 2, boostDepth: 2, minContainerPx: 30, minPx: 2, headerPx: 10 };
    const items = layoutTreemap(r, { x: 0, y: 0, w: 1600, h: 1200 }, opt);
    const depthOf = (pre: string) => Math.max(...items.filter((i) => i.key.startsWith(pre)).map((i) => i.depth));
    // a 在第 1 层、b 在第 2 层；b 的子项第 3 层、孙项第 4 层，不再往下
    expect(items.find((i) => i.key === "b")!.depth).toBe(2);
    expect(depthOf("b/")).toBe(4);
    // a 的其他子项（链外）只画一层子项：a/0 在第 2 层，它的子项在第 3 层
    expect(depthOf("a/")).toBe(3);
    // 链外的顶层兄弟 x：照旧两层
    expect(depthOf("x/")).toBe(2);
    expect(items.filter((i: TmItem) => i.chain).map((i) => i.key)).toEqual(["a", "b"]);
  });
});

// ───── 阶段 K 修订（M1 / M4）：结构稳定、镜头、按像素缩放 ─────
import { bigEnough, canonPath, contentBox, mapBox, pickMapRoot } from "./treemap";

describe("结构稳定：容器的行 / 列结构只按数据和基础参照矩形定，放大 / 挤小 / 换链都不重排", () => {
  let seed = 7;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
  /** 随机树：幂律大小，有的带后端没发来的尾巴 */
  function tree(key: string, depth: number): TmInput {
    const n = 2 + Math.floor(rnd() * 14);
    const kids: TmInput[] = Array.from({ length: n }, (_, i) => {
      const k = `${key}/${i}`;
      if (depth > 0 && rnd() < 0.7) return tree(k, depth - 1);
      return { key: k, name: k, value: Math.round(1000 / (i + 1) ** (0.8 + rnd()) + 1) };
    });
    const hidden = rnd() < 0.4 ? Math.round(rnd() * 300) : 0;
    return { key, name: key, value: kids.reduce((s, c) => s + c.value, 0) + hidden, children: kids, hidden };
  }
  /** 给链上的节点加 boost（复制一份） */
  function withChain(t: TmInput, chain: string[]): TmInput {
    const on = new Set(chain);
    const f = (n: TmInput): TmInput => ({ ...n, boost: on.has(n.key) ? BOOST : undefined, children: n.children?.map(f) });
    return f(t);
  }
  /** 随机挑一条链（从 t 往下，只挑文件夹） */
  function pickChain(t: TmInput, len: number): string[] {
    const out: string[] = [];
    let cur = t;
    for (let i = 0; i < len; i++) {
      const dirs = (cur.children ?? []).filter((c) => c.children?.length);
      if (!dirs.length) break;
      cur = dirs[Math.floor(rnd() * Math.min(dirs.length, 6))];
      out.push(cur.key);
    }
    return out;
  }
  const opt = { minPx: 8, maxDepth: 2, boostDepth: 2, headerPx: 26, pad: 6, padEnd: 12, minContainerPx: 90 };
  const seps = (a: Rect, b: Rect) => [a.x + a.w <= b.x + 1e-6, b.x + b.w <= a.x + 1e-6, a.y + a.h <= b.y + 1e-6, b.y + b.h <= a.y + 1e-6];
  /** 两次布局里都画出来的同一个容器的子块：任意两块在两次里都沿同一个方向分开（线性插值的每一帧都不重叠） */
  function stable(A: TmItem[], B: TmItem[]): string | null {
    const kidsOf = (L: TmItem[]) => {
      const m = new Map<string, Map<string, Rect>>();
      for (const it of L) {
        if (it.kind !== "node") continue;
        const p = it.parentKey ?? "";
        if (!m.has(p)) m.set(p, new Map());
        m.get(p)!.set(it.key, it.rect);
      }
      return m;
    };
    const ka = kidsOf(A);
    const kb = kidsOf(B);
    for (const [p, ma] of ka) {
      const mb = kb.get(p);
      if (!mb) continue;
      const both = [...ma.keys()].filter((k) => mb.has(k));
      for (let i = 0; i < both.length; i++)
        for (let j = i + 1; j < both.length; j++) {
          const sa = seps(ma.get(both[i])!, ma.get(both[j])!);
          const sb = seps(mb.get(both[i])!, mb.get(both[j])!);
          if (!sa.some((s, k) => s && sb[k])) return `${p}: ${both[i]} x ${both[j]}`;
        }
    }
    return null;
  }

  it("随机 300 棵树 × 多条链：同一个容器的子块在任意两个状态之间都不会横穿（包括被挤小的兄弟容器）", () => {
    const rect = { x: 0, y: 0, w: 900, h: 640 };
    let containersChecked = 0;
    for (let t = 0; t < 300; t++) {
      const T = tree("r", 3);
      const c1 = pickChain(T, 1);
      const first = T.children!.find((c) => c.key === c1[0]);
      const c2 = first ? [...c1, ...pickChain(first, 1)] : c1;
      const c3 = pickChain(T, 3);
      const states = [[], c1, c2, c3].map((ch) => layoutTreemap(withChain(T, ch), rect, opt));
      for (let a = 0; a < states.length; a++)
        for (let b = a + 1; b < states.length; b++) {
          expect(stable(states[a], states[b])).toBeNull();
        }
      containersChecked += states[1].filter((i) => i.container && !i.chain).length;
    }
    // 真的比较到了被挤小、但还画着子块的链外容器
    expect(containersChecked).toBeGreaterThan(100);
  });

  it("伸缩后不画细条（F2）：任何状态下画出来的块（包括「其他」）最短边都 ≥ minPx，每个容器里不留洞", () => {
    const rect = { x: 0, y: 0, w: 900, h: 640 };
    const o2 = { ...opt, minPx: 20, minPxDeep: 26 };
    let squeezedMerged = 0;
    for (let t = 0; t < 300; t++) {
      const T = tree("r", 3);
      for (const ch of [[], pickChain(T, 1), pickChain(T, 2), pickChain(T, 3)]) {
        const L = layoutTreemap(withChain(T, ch), rect, o2);
        const byKey = new Map(L.map((i) => [i.key, i]));
        const kidsArea = new Map<string, number>();
        for (const it of L) {
          const need = it.depth >= 2 ? 26 : 20;
          expect(Math.min(it.rect.w, it.rect.h)).toBeGreaterThanOrEqual(need - EPS);
          const pk = it.parentKey ?? "";
          kidsArea.set(pk, (kidsArea.get(pk) ?? 0) + area(it.rect));
          if (it.kind === "other" && ch.length) squeezedMerged++;
        }
        // 子块合起来正好铺满父块的内容区（被挤掉的行挪进了「其他」/ 交给了前一格，不留空洞）
        for (const [pk, a] of kidsArea) {
          const box = pk ? contentBox(byKey.get(pk)!.rect, o2) : rect;
          expect(a / area(box)).toBeCloseTo(1, 4);
        }
      }
    }
    expect(squeezedMerged).toBeGreaterThan(100);
  });

  it("「其他」的 key 不带项数：同一个容器的尾巴在放大 / 不放大两种状态下是同一块（过渡连续）", () => {
    const vals = [400, 250, 180, 120, 90, ...Array.from({ length: 30 }, (_, i) => 40 / (i + 1))];
    const kids2: TmInput[] = vals.map((v, i) => ({ key: `k${i}`, name: `k${i}`, value: v, boost: undefined }));
    const T: TmInput = { key: "r", name: "r", value: vals.reduce((a, b) => a + b, 0), children: kids2 };
    const rect = { x: 0, y: 0, w: 800, h: 500 };
    const o2 = { ...opt, minPx: 20 };
    const A = layoutTreemap(T, rect, o2).filter((i) => i.kind === "other");
    const B = layoutTreemap({ ...T, children: kids2.map((c) => (c.key === "k1" ? { ...c, boost: BOOST } : c)) }, rect, o2).filter((i) => i.kind === "other");
    expect(A.map((i) => i.key)).toContain("r|其他");
    expect(B.map((i) => i.key)).toContain("r|其他");
    // 放大以后被挤细的兄弟也并进来了：项数变多，key 不变
    const a = A.find((i) => i.key === "r|其他")!;
    const b = B.find((i) => i.key === "r|其他")!;
    expect(b.mergedCount).toBeGreaterThan(a.mergedCount);
    for (const it of [...A, ...B]) expect(it.key).toMatch(/^r\|其他(\|\d+)?$/);
  });

  it("数据变细（懒加载取回更多子项、尾巴变小）：大块的位置一点不动，新块出现在原来「其他」的地方", () => {
    const big = [400, 250, 180, 120, 90];
    const small = Array.from({ length: 30 }, (_, i) => 40 / (i + 1));
    const total = big.reduce((a, b) => a + b, 0) + small.reduce((a, b) => a + b, 0);
    const kid = (v: number, i: number): TmInput => ({ key: `k${i}`, name: `k${i}`, value: v });
    const coarse: TmInput = { key: "r", name: "r", value: total, children: big.map(kid), hidden: total - big.reduce((a, b) => a + b, 0), hiddenCount: 30 };
    const fine: TmInput = { key: "r", name: "r", value: total, children: [...big, ...small].map(kid) };
    const rect = { x: 0, y: 0, w: 800, h: 500 };
    const A = new Map(layoutTreemap(coarse, rect, opt).map((i) => [i.key, i.rect]));
    const B = new Map(layoutTreemap(fine, rect, opt).map((i) => [i.key, i.rect]));
    for (const k of ["k0", "k1", "k2"]) {
      const a = A.get(k)!;
      const b = B.get(k)!;
      expect(b.x).toBeCloseTo(a.x, 6);
      expect(b.y).toBeCloseTo(a.y, 6);
      expect(b.w).toBeCloseTo(a.w, 6);
      expect(b.h).toBeCloseTo(a.h, 6);
    }
    // 细数据里多出来的块都落在粗数据「其他」那块里面
    const otherKey = [...A.keys()].find((k) => k.startsWith("r|其他"))!;
    const other = A.get(otherKey)!;
    for (const [k, r] of B) if (!A.has(k) && !k.startsWith("r|其他")) expect(inside(r, other)).toBe(true);
  });

  it("被挤小的链外容器照样画出够大的行（不再整块不画），里面没有细条", () => {
    const rect = { x: 0, y: 0, w: 900, h: 640 };
    let kept = 0;
    for (let t = 0; t < 200; t++) {
      const T = tree("r", 2);
      const ch = pickChain(T, 1);
      const L = layoutTreemap(withChain(T, ch), rect, opt);
      const byKey = new Map(L.map((i) => [i.key, i]));
      for (const it of L) {
        const parent = it.parentKey ? byKey.get(it.parentKey) : undefined;
        if (parent && !parent.chain) expect(Math.min(it.rect.w, it.rect.h)).toBeGreaterThanOrEqual(opt.minPx - EPS);
      }
      // 链外的容器（被放大的兄弟挤小了）还画着子块
      kept += L.filter((i) => i.depth === 1 && !i.chain && i.container).length;
    }
    expect(kept).toBeGreaterThan(50);
  });
});

describe("正在看的文件夹（pk）里的子文件夹不按「被挤变形」藏起来", () => {
  // WinSxS 这种：盘根视图里近似正方形，展开 Windows 以后变成又高又窄的一条（形状变了）。它是 pk 的子项，正是要看的细节：
  // 照样画子块（放不下的行并进「其他」），不变成一整块没内容的叶子。（只按「被挤变形」判断时，这里约一半会被藏掉）
  let seed = 7;
  const rnd = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
  function tree(key: string, depth: number): TmInput {
    const n = 2 + Math.floor(rnd() * 30);
    const kids: TmInput[] = [];
    for (let i = 0; i < n; i++) {
      const v = Math.round(1000 * rnd() ** 3) + 1;
      if (depth > 0 && rnd() < 0.5) kids.push(tree(`${key}/${i}`, depth - 1));
      else kids.push({ key: `${key}/${i}`, name: `${key}/${i}`, value: v });
    }
    return { key, name: key, value: kids.reduce((s, c) => s + c.value, 0), children: kids };
  }
  it("随机 200 棵树：展开第一层的一个文件夹后，它里面有子项、两边都 ≥ 2 × 容器门槛的子文件夹全都画成容器", () => {
    const opt = { minPx: 20, maxDepth: 2, boostDepth: 2, headerPx: 26, pad: 6, padEnd: 12, minContainerPx: 90 };
    const rect = { x: 0, y: 0, w: 880, h: 760 };
    let checked = 0;
    for (let t = 0; t < 200; t++) {
      const T = tree("r", 3);
      const dirs = T.children!.filter((c) => c.children?.length);
      if (!dirs.length) continue;
      const pk = dirs[0].key;
      const B = { ...T, children: T.children!.map((c) => (c.key === pk ? { ...c, boost: BOOST } : c)) };
      const kids = new Map(B.children!.find((c) => c.key === pk)!.children!.map((c) => [c.key, c]));
      for (const it of layoutTreemap(B, rect, opt)) {
        if (it.parentKey !== pk || it.kind !== "node" || !kids.get(it.key)?.children?.length) continue;
        if (Math.min(it.rect.w, it.rect.h) < 2 * opt.minContainerPx) continue;
        checked++;
        expect(it.container).toBe(true);
      }
    }
    expect(checked).toBeGreaterThan(100);
  });
});

describe("自动缩放（M4）：按像素决定；镜头推近时块按同一个比例移动", () => {
  it("bigEnough：一般要 ≥ 360×260；图矮于 500 px 时要 ≥ 图面积的 30%", () => {
    expect(bigEnough({ x: 0, y: 0, w: 400, h: 300 }, { w: 1100, h: 760 })).toBe(true);
    expect(bigEnough({ x: 0, y: 0, w: 300, h: 300 }, { w: 1100, h: 760 })).toBe(false);
    expect(bigEnough({ x: 0, y: 0, w: 400, h: 200 }, { w: 1100, h: 760 })).toBe(false);
    // 矮图（760×380）：340×120 = 14%，不够；420×230 = 33%，够
    expect(bigEnough({ x: 0, y: 0, w: 340, h: 120 }, { w: 760, h: 380 })).toBe(false);
    expect(bigEnough({ x: 0, y: 0, w: 420, h: 230 }, { w: 760, h: 380 })).toBe(true);
    expect(bigEnough({ x: 0, y: 0, w: -4, h: 230 }, { w: 760, h: 380 })).toBe(false);
  });

  it("pickMapRoot：从浅到深试，第一个够大的；都不够用最深的；画不出来的跳过", () => {
    const size: Record<string, number> = { a: 100, b: 200, c: 400, d: 900 };
    expect(pickMapRoot(["a", "b", "c", "d"], (r) => size[r], (s) => s >= 300)?.root).toBe("c");
    expect(pickMapRoot(["a", "b"], (r) => size[r], (s) => s >= 300)?.root).toBe("b");
    expect(pickMapRoot(["x", "a"], (r) => (r === "x" ? null : size[r]), () => true)?.root).toBe("a");
  });

  it("mapBox：框里的相对位置按比例映射", () => {
    expect(mapBox({ x: 15, y: 30, w: 10, h: 20 }, { x: 10, y: 20, w: 20, h: 40 }, { x: 0, y: 0, w: 200, h: 400 })).toEqual({ x: 50, y: 100, w: 100, h: 200 });
  });

  it("镜头一致：以链上一层为根重铺（参照矩形沿原始的层走下来），它里面的块 ≈ 原图里的位置按「它的内容区 → 整张图」放大", () => {
    const leafs = (p: string, n: number, d: number): TmInput[] =>
      Array.from({ length: n }, (_, i) => {
        const key = `${p}/${i}`;
        const v = Math.round(100 / (i + 1)) + 3;
        return d > 0 ? { key, name: key, value: 0, children: leafs(key, n, d - 1) } : { key, name: key, value: v };
      });
    const fix = (n: TmInput): TmInput => {
      if (n.children) {
        n.children = n.children.map(fix);
        n.value = n.children.reduce((s, c) => s + c.value, 0);
      }
      return n;
    };
    const R = fix({ key: "r", name: "r", value: 0, children: leafs("r", 7, 3) });
    const chain = ["r/1", "r/1/0", "r/1/0/2"];
    const on = new Set(chain);
    const mark = (n: TmInput): TmInput => ({ ...n, boost: on.has(n.key) ? BOOST : undefined, children: n.children?.map(mark) });
    const M = mark(R);
    const opt = { minPx: 8, maxDepth: 2, boostDepth: 2, headerPx: 26, pad: 6, padEnd: 12, minContainerPx: 90 };
    const frame = { x: 0, y: 0, w: 1100, h: 760 };
    const before = layoutTreemap(M, frame, opt);
    const a = M.children!.find((c) => c.key === "r/1")!;
    const canon = canonPath([R, R.children!.find((c) => c.key === "r/1")!], frame, opt);
    const after = layoutTreemap(a, frame, opt, canon, frame);
    const aItem = before.find((i) => i.key === "r/1")!;
    const lens = (r: Rect) => mapBox(r, contentBox(aItem.rect, opt), frame);
    const B = new Map(after.map((i) => [i.key, i.rect]));
    let n = 0;
    for (const it of before) {
      if (!it.key.startsWith("r/1/") || it.kind !== "node") continue;
      const b = B.get(it.key);
      if (!b) continue;
      const m = lens(it.rect);
      // 允许的误差：容器标题栏 / 内边距不跟着放大（固定像素），层层累积几个百分点
      expect(Math.abs(m.x - b.x)).toBeLessThan(frame.w * 0.05);
      expect(Math.abs(m.y - b.y)).toBeLessThan(frame.h * 0.06);
      expect(Math.abs(m.w - b.w)).toBeLessThan(frame.w * 0.05);
      expect(Math.abs(m.h - b.h)).toBeLessThan(frame.h * 0.06);
      n++;
    }
    expect(n).toBeGreaterThan(5);
  });
});
