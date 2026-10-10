// 阶段 K 第 3 轮（F5）：两次布局之间怎么动。重点：面包屑一次退回好几级（拉远）时，原来展开的块里的子块
// 要从旧画面的位置出发、压在它们要缩进去的那一块上面淡出——第 0 帧不能被那一块盖住、露出一整块空板。
import { describe, expect, it } from "vitest";
import type { TreemapNode } from "./api";
import { containFrom, DUR, EASE, keepsApart, overlapSpan, planMotion, tileZ, type Box, type MotionOut, type MotionTile } from "./tmmotion";
import { bigEnough, contentBox, DEFAULT_LAYOUT, pickMapRoot, samePath, type HiddenSlot, type LayoutOptions, type TmItem } from "./treemap";
import { isUnder, layoutMap, rootCandidates, tileNode } from "./tmtree";

function n(path: string, allocated: number, children: TreemapNode[] = [], kind: "Dir" | "File" = "Dir"): TreemapNode {
  return {
    name: path.split("\\").filter(Boolean).pop() ?? path,
    path,
    kind,
    allocated,
    tier: "keep",
    what: "",
    owner: null,
    lose_kind: null,
    category: "unknown",
    inaccessible: false,
    reparse_skipped: false,
    system_reserved: false,
    cloud_placeholder: false,
    hardlink_dup: false,
    mapped: false,
    can_open: kind === "Dir",
    children,
    hidden_bytes: 0,
    hidden_count: 0,
  };
}
/** 一棵够深的树：每层 4 个子文件夹（大小递减）+ 一个文件 */
function deep(p: string, d: number): TreemapNode {
  const kids = d ? [0, 1, 2, 3].map((i) => deep(`${p}\\d${i}`, d - 1)) : [];
  const t = n(p, 0, [...kids, n(`${p}\\f.bin`, 5 + d, [], "File")]);
  kids.forEach((k, i) => (k.allocated *= 1 / (i + 1)));
  return t;
}
function fix(t: TreemapNode): TreemapNode {
  t.children.forEach(fix);
  if (t.kind === "Dir") t.allocated = t.children.reduce((s, c) => s + c.allocated, 0) || 1;
  t.children.sort((a, b) => b.allocated - a.allocated);
  return t;
}
const T = fix(n("C:\\", 0, [deep("C:\\a", 6), deep("C:\\b", 4), deep("C:\\c", 3)]));
const frame = { x: 0, y: 0, w: 1100, h: 760 };
const opt = { ...DEFAULT_LAYOUT, minPx: 10, maxDepth: 2, boostDepth: 2, headerPx: 26, pad: 4, padEnd: 6, minContainerPx: 70 };
const inner = (b: Box, container: boolean): Box => (container ? contentBox(b, opt) : b);
const tiles = (items: TmItem[]): MotionTile[] =>
  items.map((it) => ({ key: it.key, rk: it.key, parentKey: it.parentKey, depth: it.depth, container: it.container, path: tileNode(it)?.path ?? null, z: tileZ(it), b: it.rect, merged: it.merged }));
const slots = (h: Map<string, HiddenSlot>) => new Map([...h].map(([k, s]) => [k, { b: s.rect, parentKey: s.parentKey }]));
const overlap = (a: Box, b: Box) => Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y));
const within = (a: Box, b: Box, e = 1) => a.x >= b.x - e && a.y >= b.y - e && a.x + a.w <= b.x + b.w + e && a.y + a.h <= b.y + b.h + e;

/** 从 (chainA, rootA) 拉远到 (chainB, rootB)：返回两次的块和动法 */
function zoomOut(chainA: string[], rootA: string, chainB: string[], rootB: string) {
  const A = layoutMap(T, chainA, rootA, frame, opt)!;
  const B = layoutMap(T, chainB, rootB, frame, opt)!;
  const prev = new Map(tiles(A.items).map((t) => [t.key, t]));
  const live = tiles(B.items);
  const plan = planMotion({ prev, cur: (t) => t.b, live, prevRoot: A.root, root: B.root, inner, frame, head: 26, hidden: slots(A.hidden), hiddenNow: slots(B.hidden) });
  return { A, B, prev, live, plan };
}

/** 第 0 帧：残影都在旧位置；前后都在的块也在旧位置。任何一个残影都不能被一个（第 0 帧就看得见的）前后都在的块盖住 */
function checkFrame0(r: ReturnType<typeof zoomOut>) {
  const liveByKey = new Map(r.live.map((t) => [t.key, t]));
  let checked = 0;
  for (const g of r.plan.ghosts) {
    const s = r.prev.get(g.key)!;
    for (const L of r.live) {
      if (!r.prev.has(L.key) || r.plan.jump.has(L.key)) continue; // 新出现的块第 0 帧是透明的
      const at0 = r.prev.get(L.key)!.b;
      if (overlap(at0, s.b) < 4) continue;
      // L 是这个残影原来的祖先（它在旧图里就在残影下面）：残影必须压在它上面
      let a = s.parentKey;
      let isAnc = false;
      while (a) {
        if (a === L.key) isAnc = true;
        a = r.prev.get(a)?.parentKey ?? null;
      }
      if (!isAnc) continue;
      checked++;
      expect(g.z, `${g.key} 被 ${L.key} 盖住`).toBeGreaterThan(liveByKey.get(L.key)!.z);
    }
  }
  return checked;
}

describe("F5：面包屑一次退回好几级（拉远），原来展开的块里的子块从旧画面淡出，不留空板", () => {
  const chain5 = ["C:\\a", "C:\\a\\d0", "C:\\a\\d0\\d0", "C:\\a\\d0\\d0\\d0", "C:\\a\\d0\\d0\\d0\\d0"];

  it("退回 2 级：旧的 pk 在新图里成了叶子，它的子块都成了残影、压在它上面、跟着它缩小", () => {
    const r = zoomOut(chain5, "C:\\a\\d0\\d0\\d0", chain5.slice(0, 3), "C:\\a");
    const pkOld = chain5[4];
    const pkNew = r.live.find((t) => t.key === pkOld)!;
    expect(pkNew).toBeTruthy();
    expect(pkNew.container).toBe(false);
    const kids = [...r.prev.values()].filter((t) => t.parentKey === pkOld);
    expect(kids.length).toBeGreaterThan(2);
    for (const k of kids) {
      const g = r.plan.ghosts.find((x) => x.key === k.key)!;
      expect(g, `${k.key} 没有残影`).toBeTruthy();
      expect(g.z).toBeGreaterThan(pkNew.z);
      // 缩进旧 pk 现在的那一块里（跟着它缩小）
      expect(within(g.to, pkNew.b)).toBe(true);
    }
    expect(checkFrame0(r)).toBeGreaterThan(2);
  });

  it("退回 3 级：旧的 pk 在新图里画不出来了，它的子块跟着还画得出来的最近的祖先（或镜头）缩小，同样压在上面", () => {
    const r = zoomOut(chain5, "C:\\a\\d0\\d0\\d0", chain5.slice(0, 2), "C:\\");
    const pkOld = chain5[4];
    expect(r.live.some((t) => t.key === pkOld)).toBe(false);
    const kids = [...r.prev.values()].filter((t) => t.parentKey === pkOld);
    expect(kids.length).toBeGreaterThan(2);
    // 新图里画得出来的最深的那一层祖先
    let host: MotionTile | null = null;
    for (const t of r.live) if (t.path && (isUnder(pkOld, t.path) || t.path === pkOld) && (!host || t.depth > host.depth)) host = t;
    expect(host).toBeTruthy();
    for (const k of kids) {
      const g = r.plan.ghosts.find((x) => x.key === k.key)!;
      expect(g, `${k.key} 没有残影`).toBeTruthy();
      expect(g.z).toBeGreaterThan(host!.z);
      expect(within(g.to, host!.b)).toBe(true);
    }
    checkFrame0(r);
  });

  it("推近 / 拉远都有镜头；拉远时旧图整个缩进新图里旧根的那一块", () => {
    const r = zoomOut(chain5, "C:\\a\\d0\\d0\\d0", chain5.slice(0, 2), "C:\\");
    expect(r.plan.cam?.out).toBe(true);
    // 旧根（铺满整张图的那一块）的所有残影都去了新图里旧根所在的区域
    for (const g of r.plan.ghosts) expect(within(g.to, r.plan.cam!.now, 2)).toBe(true);
  });
});

describe("「其他」：新出现的「其他」从它吸收的那些块原来的地方长出来", () => {
  it("把一个顶层块放大：被挤细的兄弟并进尾部的「其他」，它从这些兄弟原来合起来的框（或者父块的映射）长出来，不压住留下来的兄弟", () => {
    const A = layoutMap(T, [], "C:\\", frame, { ...opt, minPx: 30 })!;
    const B = layoutMap(T, ["C:\\c"], "C:\\", frame, { ...opt, minPx: 30 })!;
    const prev = new Map(tiles(A.items).map((t) => [t.key, t]));
    const live = tiles(B.items);
    const plan = planMotion({ prev, cur: (t) => t.b, live, prevRoot: A.root, root: B.root, inner, frame, head: 26 });
    for (const x of live) {
      if (!x.merged || prev.has(x.key)) continue;
      const f = plan.from.get(x.key);
      if (!f) continue;
      for (const o of prev.values()) {
        if (o.parentKey !== x.parentKey || !live.some((l) => l.key === o.key) || x.merged.includes(o.key)) continue;
        expect(overlap(f, o.b)).toBeLessThan(4);
      }
    }
  });
});

describe("过渡的几何小工具", () => {
  it("overlapSpan：移动的框和静止的框重叠的进度区间（交点在 [0, 1] 外面也算对）", () => {
    const s = { x: 799, y: 721, w: 85, h: 47 };
    // Program Files (x86) 从 (722,705,118,59) 移到 (799,559,81,154)：右边一直 > 801，上下在 p ≈ 0.8 之后分开
    const r = overlapSpan({ x: 722, y: 705, w: 118, h: 59 }, { x: 799, y: 559, w: 81, h: 154 }, s)!;
    expect(r[0]).toBe(0);
    expect(r[1]).toBeCloseTo(41 / 51, 3);
    expect(overlapSpan({ x: 0, y: 0, w: 10, h: 10 }, { x: 0, y: 0, w: 10, h: 10 }, { x: 50, y: 0, w: 10, h: 10 })).toBeNull();
    // 从右边扫过去
    const t = overlapSpan({ x: 100, y: 0, w: 10, h: 10 }, { x: 0, y: 0, w: 10, h: 10 }, { x: 45, y: 0, w: 10, h: 10 })!;
    expect(t[0]).toBeCloseTo(0.47, 5);
    expect(t[1]).toBeCloseTo(0.63, 5);
  });
  it("containFrom：长大的父块从什么进度起包住静止的子块", () => {
    expect(containFrom({ x: 0, y: 0, w: 10, h: 10 }, { x: 0, y: 0, w: 100, h: 100 }, { x: 40, y: 40, w: 10, h: 10 })).toBeCloseTo(39 / 90, 4);
    expect(containFrom({ x: 0, y: 0, w: 100, h: 100 }, { x: 0, y: 0, w: 100, h: 100 }, { x: 40, y: 40, w: 10, h: 10 })).toBe(0);
  });
});

// ── 整个过渡逐帧模拟（和截图脚本的 LIVE-LIVE 口径一样）：两块都活着、不是父子、上面那块不透明（不是「其他」幕布）、下面那块看得见，就算横穿 ──
const FADE = 180;
function crossings(prev: Map<string, MotionTile>, live: MotionTile[], plan: MotionOut, cur: (t: MotionTile) => Box = (t) => t.b, op0?: Map<string, number>): string[] {
  const by = new Map(live.map((t) => [t.key, t]));
  const anc = (a: string, b: MotionTile) => {
    for (let p = b.parentKey; p; p = by.get(p)?.parentKey ?? null) if (p === a) return true;
    return false;
  };
  const stays = (k: string) => prev.has(k) && !plan.jump.has(k);
  const start = (x: MotionTile) => (stays(x.key) ? cur(prev.get(x.key)!) : (plan.from.get(x.key) ?? x.b));
  const hull = (x: MotionTile) => {
    const a = start(x);
    const b = x.b;
    const x0 = Math.min(a.x, b.x);
    const y0 = Math.min(a.y, b.y);
    return { x: x0, y: y0, w: Math.max(a.x + a.w, b.x + b.w) - x0, h: Math.max(a.y + a.h, b.y + b.h) - y0 };
  };
  const hs = live.map(hull);
  const out: string[] = [];
  for (let i = 0; i < live.length; i++) {
    for (let j = i + 1; j < live.length; j++) {
      const a = live[i];
      const b = live[j];
      if (overlap(hs[i], hs[j]) < 200 || anc(a.key, b) || anc(b.key, a)) continue;
      const top = a.z > b.z || (a.z === b.z && a.key > b.key) ? a : b;
      const low = top === a ? b : a;
      if (top.merged) continue;
      for (let t = 0; t <= 320; t += 8) {
        const p = EASE(Math.min(1, t / DUR));
        const at = (x: MotionTile) => {
          const s0 = start(x);
          return { x: s0.x + (x.b.x - s0.x) * p, y: s0.y + (x.b.y - s0.y) * p, w: s0.w + (x.b.w - s0.w) * p, h: s0.h + (x.b.h - s0.h) * p };
        };
        const op = (x: MotionTile) => {
          // 前后都在：被打断时还在淡入的块从当时的不透明度接着淡到 1（CSS 过渡就是这样接着走的）
          if (stays(x.key)) {
            const o0 = op0?.get(x.key) ?? 1;
            return o0 + (1 - o0) * EASE(Math.min(1, t / FADE));
          }
          const d = plan.late.get(x.key) ?? 0;
          return t < d ? 0 : EASE(Math.min(1, (t - d) / FADE));
        };
        if (op(top) < 0.95 || op(low) < 0.6) continue;
        const A = at(a);
        const B = at(b);
        const ix = Math.min(A.x + A.w, B.x + B.w) - Math.max(A.x, B.x);
        const iy = Math.min(A.y + A.h, B.y + B.h) - Math.max(A.y, B.y);
        if (ix > 3 && iy > 3 && ix * iy > 200) {
          out.push(`${t}ms ${top.key} 压在 ${low.key} 上 ${Math.round(ix)}×${Math.round(iy)}`);
          break;
        }
      }
    }
  }
  return out;
}

/** 不规则的随机树：每层 2–26 项、大小幂律分布、文件夹和文件混着，最深 4 层（会出现被挤掉的行、行末「其他」、尾部「其他」） */
function randomTree(seed: number): TreemapNode {
  let s = seed;
  const r = () => (s = (s * 1103515245 + 12345) % 2147483648) / 2147483648;
  const grow = (path: string, depth: number): TreemapNode => {
    const kids: TreemapNode[] = [];
    const nk = depth >= 4 ? 0 : Math.floor(2 + r() * (depth === 0 ? 12 : 25));
    // 盘根是「C:\」：子项是「C:\d0」（不能多一个反斜杠，不然链对不上）
    const sub = (x: string) => (path.endsWith("\\") ? path : path + "\\") + x;
    for (let i = 0; i < nk; i++) {
      const q = r();
      if (q < 0.1 && depth < 3) {
        // 外壳文件夹（像 Users \ king）：几乎全部空间都在唯一一个子文件夹里，再带两个很小的文件
        const sh = sub(`h${i}`);
        const big = grow(`${sh}\\s`, depth + 2);
        kids.push(n(sh, 0, [big, n(`${sh}\\t0`, 1, [], "File"), n(`${sh}\\t1`, 1, [], "File")]));
      } else if (q < 0.55 && depth < 4) kids.push(grow(sub(`d${i}`), depth + 1));
      else kids.push(n(sub(`f${i}`), Math.round(Math.pow(r(), 3) * 1000) + 1, [], "File"));
    }
    return n(path, 0, kids);
  };
  return fix(grow("C:\\", 0));
}

describe("第 4 轮：新出现的块不横穿留下的兄弟（被挤掉的行回来、从「其他」里分出来、缩放）", () => {
  // C 主题的参数：最小块 24 / 第 2 层起 32，标题栏 26；A / B：19 / 21、没有投影；窄窗口（竖屏）的图
  const copt: LayoutOptions = { minPx: 24, minPxDeep: 32, maxDepth: 2, boostDepth: 2, headerPx: 26, pad: 6, padEnd: 10, minContainerPx: 90 };
  const aopt: LayoutOptions = { minPx: 19, minPxDeep: 21, maxDepth: 2, boostDepth: 2, headerPx: 26, pad: 4, padEnd: 4, minContainerPx: 90 };
  /** 和 TreemapView 一样选图的根（pk 的内容区不够大就把根下移）、算动法 */
  const rig = (o: LayoutOptions, fr: Box) => {
    const state = (t: TreemapNode, chain: string[]) => {
      const pk = chain[chain.length - 1];
      const cands = chain.length ? rootCandidates(t, chain) : [t.path];
      const fits = (r: { items: TmItem[] }) => {
        const it = r.items.find((x) => x.chain && x.depth > 0 && samePath(tileNode(x)?.path, pk));
        return !!it && bigEnough(contentBox(it.rect, o), fr);
      };
      return pickMapRoot(cands, (root) => layoutMap(t, chain, root, fr, o), fits)!.result;
    };
    const step = (A: ReturnType<typeof state>, B: ReturnType<typeof state>, cur: (t: MotionTile) => Box = (t) => t.b, pending?: Set<string>) => {
      const prev = new Map(tiles(A.items).map((t) => [t.key, t]));
      const live = tiles(B.items);
      const cinner = (b: Box, container: boolean): Box => (container ? contentBox(b, o) : b);
      const plan = planMotion({ prev, cur, live, prevRoot: A.root, root: B.root, inner: cinner, frame: fr, head: 26, hidden: slots(A.hidden), hiddenNow: slots(B.hidden), pending });
      return { prev, live, plan };
    };
    return { state, step };
  };
  const { state, step } = rig(copt, { x: 0, y: 0, w: 1100, h: 760 });
  const dirs = (t: TreemapNode): string[] => [t.path, ...t.children.filter((c) => c.kind === "Dir").flatMap(dirs)];
  const chainOf = (p: string) => {
    const parts = p.split("\\").filter(Boolean);
    return parts.slice(1).map((_, i) => "C:\\" + parts.slice(1, i + 2).join("\\"));
  };

  it("放大一块把前面的兄弟挤掉（整行不画）以后收起：它们从原来位置上的一条线长回来，不从尾部「其他」飞回去", () => {
    let found = 0;
    for (const seed of [5, 9, 17, 23, 31]) {
      const t = randomTree(seed);
      // 挖开第 1 / 2 层的文件夹：被挤掉的是顶层 / 第 2 层的兄弟（收起以后它们还画得出来）
      for (const d of dirs(t).filter((p) => chainOf(p).length === 1 || chainOf(p).length === 2)) {
        const A = state(t, chainOf(d));
        const B = state(t, []);
        if (A.root !== t.path) continue;
        const { prev, live, plan } = step(A, B);
        for (const x of live) {
          const f = plan.from.get(x.key);
          const h = A.hidden.get(x.key);
          if (!f || !h || prev.has(x.key) || h.parentKey !== x.parentKey || (x.parentKey !== null && !prev.has(x.parentKey))) continue;
          // 上一次被挤成了一条线（整行权重为 0 / 权重交给了邻居）的块：从那条线长出来
          if (h.rect.w > 0.5 && h.rect.h > 0.5) continue;
          found++;
          expect(Math.min(f.w, f.h)).toBeLessThan(1);
          for (const o of live) if (o.parentKey === x.parentKey && prev.has(o.key) && o.key !== x.key && !o.merged) expect(keepsApart(f, prev.get(o.key)!.b, x.b, o.b), `${x.key} × ${o.key}`).toBe(true);
        }
      }
    }
    expect(found).toBeGreaterThan(0);
  });

  it("父块是新出现的（外壳拆开后的 king）：跟着它长出来的新块要和换了父块、还在走的兄弟分得开，不然原地等它让开", () => {
    const T0 = (key: string, parentKey: string | null, depth: number, container: boolean, b: Box): MotionTile => ({ key, rk: key, parentKey, depth, container, path: key, z: tileZ({ depth, container, kind: "node" }), b });
    const P = T0("P", null, 1, true, { x: 0, y: 0, w: 1000, h: 600 });
    // 上一次：S 直接在 P 里（外壳合并），在右边
    const prev = new Map([P, T0("S", "P", 2, false, { x: 600, y: 100, w: 300, h: 200 })].map((t) => [t.key, t]));
    // 这一次：P 里多了一层新容器 Q（king），S 换到 Q 里、往左走；新块 N 正好出现在 S 原来的地方
    const live = [P, T0("Q", "P", 2, true, { x: 6, y: 26, w: 984, h: 564 }), T0("S", "Q", 3, false, { x: 20, y: 60, w: 300, h: 200 }), T0("N", "Q", 3, false, { x: 600, y: 100, w: 300, h: 200 })];
    const plan = planMotion({ prev, cur: (t) => t.b, live, prevRoot: "C:\\", root: "C:\\", inner: (b, c) => (c ? contentBox(b, opt) : b), frame, head: 26 });
    expect(crossings(prev, live, plan)).toEqual([]);
    expect(plan.late.get("N") ?? 0).toBeGreaterThan(0);
  });

  it("外壳合并块拆开（链在 Users \\ king 里面 → 面包屑退回 Users）：子块换了父块也照样查，不横穿", () => {
    const f = (p: string, v: number) => n(p, v, [], "File");
    const kingKids = [
      n("C:\\Users\\king\\AppData", 0, [f("C:\\Users\\king\\AppData\\a", 3500), f("C:\\Users\\king\\AppData\\b", 900)]),
      n("C:\\Users\\king\\.cache", 0, [f("C:\\Users\\king\\.cache\\a", 1500)]),
      n("C:\\Users\\king\\下载", 0, [f("C:\\Users\\king\\下载\\a", 1240)]),
      n("C:\\Users\\king\\.ollama", 0, [f("C:\\Users\\king\\.ollama\\a", 770)]),
      n("C:\\Users\\king\\文档", 0, [f("C:\\Users\\king\\文档\\a", 610)]),
      n("C:\\Users\\king\\xwechat_files", 0, [f("C:\\Users\\king\\xwechat_files\\a", 590), f("C:\\Users\\king\\xwechat_files\\b", 20)]),
      n("C:\\Users\\king\\.rustup", 0, [f("C:\\Users\\king\\.rustup\\a", 310)]),
      n("C:\\Users\\king\\桌面", 0, [f("C:\\Users\\king\\桌面\\a", 290), f("C:\\Users\\king\\桌面\\b", 10)]),
      n("C:\\Users\\king\\.cargo", 0, [f("C:\\Users\\king\\.cargo\\a", 110)]),
      ...Array.from({ length: 20 }, (_, i) => f(`C:\\Users\\king\\x${i}`, 20 / (i + 1))),
    ];
    const T2 = fix(
      n("C:\\", 0, [
        n("C:\\Users", 0, [n("C:\\Users\\king", 0, kingKids), n("C:\\Users\\Public", 0, [f("C:\\Users\\Public\\a", 30)])]),
        n("C:\\Windows", 0, [f("C:\\Windows\\a", 2000), f("C:\\Windows\\b", 1160)]),
        n("C:\\Program Files", 0, [f("C:\\Program Files\\a", 1930)]),
        f("C:\\hiberfil.sys", 1270),
        n("C:\\ProgramData", 0, [f("C:\\ProgramData\\a", 1020)]),
        f("C:\\pagefile.sys", 800),
      ]),
    );
    const bad: string[] = [];
    for (const [W, H] of [
      [884, 768],
      [700, 480],
    ]) {
      const R = rig(copt, { x: 0, y: 0, w: W, h: H });
      for (const deep of ["C:\\Users\\king\\xwechat_files", "C:\\Users\\king\\桌面", "C:\\Users\\king\\AppData"]) {
        const A = R.state(T2, ["C:\\Users", "C:\\Users\\king", deep]);
        const B = R.state(T2, ["C:\\Users"]);
        for (const [X, Y] of [
          [A, B],
          [B, A],
        ]) {
          const { prev, live, plan } = R.step(X, Y);
          bad.push(...crossings(prev, live, plan).map((c) => `${W}×${H} ${deep}：${c}`));
        }
      }
    }
    expect(bad).toEqual([]);
  });

  it("随机树上随机来回 720 次（三套参数；挖开、收起、换一块、退回好几级、缩放）：逐帧模拟，没有一帧两块不透明的活块压在一起", () => {
    const bad: string[] = [];
    let n0 = 0;
    const rigs = [
      ["C 宽", rig(copt, { x: 0, y: 0, w: 1100, h: 760 })],
      ["A/B 宽", rig(aopt, { x: 0, y: 0, w: 1300, h: 800 })],
      ["C 竖屏", rig(copt, { x: 0, y: 0, w: 700, h: 480 })],
    ] as const;
    for (const [name, R] of rigs) {
      for (const seed of [3, 7, 11, 19]) {
        const t = randomTree(seed);
        const ds = dirs(t);
        let s = seed * 7 + 1;
        const r = () => (s = (s * 1103515245 + 12345) % 2147483648) / 2147483648;
        let chain: string[] = [];
        let A = R.state(t, chain);
        /** 上一次过渡：被打断时块在半路上的位置 */
        let last: { start: (x: MotionTile) => Box; plan: MotionOut } | null = null;
        for (let i = 0; i < 60; i++) {
          const q = r();
          if (q < 0.15) chain = [];
          else if (q < 0.3) chain = chain.slice(0, Math.max(0, chain.length - 1 - Math.floor(r() * 3)));
          else {
            // 挖开图上看得见的一个文件夹块（多半是链附近的）
            const vis = A.items.filter((it) => it.kind === "node" && tileNode(it)?.kind === "Dir" && it.depth > 0);
            const pick = vis.length && r() < 0.8 ? tileNode(vis[Math.floor(r() * vis.length)])!.path : ds[Math.floor(r() * ds.length)];
            chain = chainOf(pick);
          }
          const B = R.state(t, chain);
          // 三分之一的时候上一次过渡还没走完就又点了（40–200 ms）：从块在半路上的位置算起
          let cur = (x: MotionTile): Box => x.b;
          let pending: Set<string> | undefined;
          let op0: Map<string, number> | undefined;
          if (last && r() < 0.33) {
            const L = last;
            const ms = 40 + r() * 160;
            const p = EASE(ms / DUR);
            // 还在等着淡入的块（看不见）：这一次当新块重新算（TreemapView 一样传 pending）
            pending = new Set([...L.plan.late].filter(([, d]) => d + 30 > ms).map(([k]) => k));
            op0 = new Map();
            for (const k of L.plan.from.keys()) {
              const d = L.plan.late.get(k) ?? 0;
              op0.set(k, ms < d ? 0 : EASE(Math.min(1, (ms - d) / FADE)));
            }
            cur = (x) => {
              const s0 = L.start(x);
              return { x: s0.x + (x.b.x - s0.x) * p, y: s0.y + (x.b.y - s0.y) * p, w: s0.w + (x.b.w - s0.w) * p, h: s0.h + (x.b.h - s0.h) * p };
            };
          }
          const { prev, live, plan } = R.step(A, B, cur, pending);
          n0++;
          for (const c of crossings(prev, live, plan, cur, op0)) bad.push(`${name} 树 ${seed} 第 ${i} 步 ${A.root} → ${B.root}：${c}`);
          // 这一步每块从哪里出发（被打断时下一步从半路算起）
          const c0 = cur;
          last = { start: (x) => (prev.has(x.key) && !plan.jump.has(x.key) ? c0(prev.get(x.key)!) : (plan.from.get(x.key) ?? x.b)), plan };
          A = B;
        }
      }
    }
    expect(n0).toBe(720);
    expect(bad.slice(0, 8)).toEqual([]);
  });
});
