// 阶段 K：方块图数据的懒加载拼接（graft）与查找
import { describe, expect, it } from "vitest";
import type { TreemapNode } from "./api";
import { ancestorsIn, chainLevelStarts, collapseFor, findTm, graft, insertChild, isUnder, pathKey } from "./tmtree";

function n(path: string, allocated: number, children: TreemapNode[] = [], extra: Partial<TreemapNode> = {}): TreemapNode {
  return {
    name: path.split("\\").filter(Boolean).pop() ?? path,
    path,
    kind: "Dir",
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
    can_open: true,
    children,
    hidden_bytes: 0,
    hidden_count: 0,
    ...extra,
  };
}
const file = (path: string, a: number) => n(path, a, [], { kind: "File", can_open: false });

// C:\ → Users → king → AppData（AppData 的子项没取：到了上次取的最深一层）
const appdata = n("C:\\Users\\king\\AppData", 50);
const king = n("C:\\Users\\king", 97, [appdata, n("C:\\Users\\king\\下载", 40, [file("C:\\Users\\king\\下载\\a.iso", 40)]), file("C:\\Users\\king\\x.dat", 7)]);
const users = n("C:\\Users", 98, [king, n("C:\\Users\\Public", 1, [file("C:\\Users\\Public\\p", 1)])]);
const win = n("C:\\Windows", 30, [n("C:\\Windows\\System32", 20, [file("C:\\Windows\\System32\\a.dll", 20)])], { hidden_bytes: 10, hidden_count: 4 });
const TREE = n("C:\\", 128, [users, win]);

describe("findTm / ancestorsIn", () => {
  it("按路径找文件夹：大小写、末尾反斜杠不敏感；只顺着前缀走", () => {
    expect(findTm(TREE, "c:\\users\\KING\\")).toBe(king);
    expect(findTm(TREE, "C:")).toBe(TREE);
    expect(findTm(TREE, "C:\\Users\\nobody")).toBeNull();
    expect(findTm(TREE, "C:\\Users2")).toBeNull();
    expect(findTm(null, "C:\\")).toBeNull();
  });
  it("从根（不含）到它（含）的路径串", () => {
    expect(ancestorsIn(TREE, "C:\\Users\\king\\AppData")).toEqual(["C:\\Users", "C:\\Users\\king", "C:\\Users\\king\\AppData"]);
    expect(ancestorsIn(TREE, "C:\\")).toEqual([]);
    expect(ancestorsIn(TREE, "C:\\Users\\king\\nope")).toBeNull();
    expect(ancestorsIn(TREE, "D:\\x")).toBeNull();
  });
  it("pathKey / isUnder", () => {
    expect(pathKey("C:\\Users\\")).toBe("c:\\users");
    expect(isUnder("C:\\Users\\king", "c:\\users")).toBe(true);
    expect(isUnder("C:\\Users", "C:\\Users")).toBe(false);
    expect(isUnder("C:\\Users2", "C:\\Users")).toBe(false);
  });
});

describe("graft / insertChild", () => {
  it("拼接：换掉那个文件夹的 children / hidden_*，只复制从根到它的那一串，其余原样共享", () => {
    const sub = n("C:\\Users\\king\\AppData", 50, [n("C:\\Users\\king\\AppData\\Local", 45, [file("C:\\Users\\king\\AppData\\Local\\t", 45)])], { hidden_bytes: 5, hidden_count: 12 });
    const t2 = graft(TREE, sub);
    expect(t2).not.toBe(TREE);
    const a2 = findTm(t2, "C:\\Users\\king\\AppData")!;
    expect(a2.children.map((c) => c.name)).toEqual(["Local"]);
    expect(a2.hidden_bytes).toBe(5);
    expect(a2.hidden_count).toBe(12);
    // 没动的分支是同一个对象；原树不变
    expect(findTm(t2, "C:\\Windows")).toBe(win);
    expect(findTm(t2, "C:\\Users\\Public")).toBe(users.children[1]);
    expect(appdata.children).toHaveLength(0);
    // 找不到的路径：原样返回
    expect(graft(TREE, n("C:\\nope", 1))).toBe(TREE);
  });
  it("父级数据里没有的子文件夹：补进去，从父的 hidden_* 里扣掉", () => {
    const t2 = insertChild(TREE, "C:\\Windows", n("C:\\Windows\\Temp", 3, [], { can_open: false }));
    const w2 = findTm(t2, "C:\\Windows")!;
    expect(w2.children.map((c) => c.name)).toEqual(["System32", "Temp"]);
    expect(w2.hidden_bytes).toBe(7);
    expect(w2.hidden_count).toBe(3);
    // 已经有了就不重复
    expect(insertChild(t2, "C:\\Windows", n("C:\\Windows\\Temp", 3))).toBe(t2);
  });
});

describe("外壳合并与「图上的层」", () => {
  it("Users 里几乎全是 king：合成一块；链从 Users 拐进 Public 时不合并", () => {
    expect(collapseFor(users, []).node).toBe(king);
    expect(collapseFor(users, ["C:\\Users", "C:\\Users\\king"]).node).toBe(king);
    expect(collapseFor(users, ["C:\\Users", "C:\\Users\\Public"]).node).toBe(users);
  });
  it("chainLevelStarts：Users \\ king 是一层，AppData 一层", () => {
    expect(chainLevelStarts(TREE, ["C:\\Users", "C:\\Users\\king", "C:\\Users\\king\\AppData"])).toEqual([0, 2]);
    expect(chainLevelStarts(TREE, ["C:\\Users"])).toEqual([0]);
    expect(chainLevelStarts(TREE, ["C:\\Users", "C:\\Users\\Public"])).toEqual([0, 1]);
    expect(chainLevelStarts(TREE, ["C:\\Windows", "C:\\Windows\\System32"])).toEqual([0, 1]);
    expect(chainLevelStarts(TREE, [])).toEqual([]);
  });
});

// ───── 阶段 K 修订（M2 / M3）：外壳块的身份和大小、显示树、单击翻译 ─────
import { buildDisplay, headHit, otherSig, pathInputs, rootCandidates, tileHit, tileParent, toInput, type Stub } from "./tmtree";
import { BOOST } from "./treemap";

describe("toInput：外壳合并块的 key / 大小用外层（M2）", () => {
  const opts = { maxDepth: 2, boostDepth: 2 };
  it("Users \\ king：key = 外层、大小 = 外层（列表里的 Users），名字写两层，里面铺 king 的子项；外层多出来的并进「其他」", () => {
    const r = toInput(TREE, [], opts);
    const u = r.children!.find((c) => c.key === "C:\\Users")!;
    expect(u.name).toBe("Users \\ king");
    expect(u.value).toBe(98);
    expect(u.children!.map((c) => c.name)).toEqual(["AppData", "下载", "x.dat"]);
    // Public 一个子项 + king 自己没发来的 → 「其他」里的项数
    expect(u.hiddenCount).toBe(1);
    expect((u.ref as { path: string }).path).toBe("C:\\Users\\king");
    // 原始的每一层（算基础结构用）
    expect(u.via!.map((v) => v.key)).toEqual(["C:\\Users", "C:\\Users\\king"]);
    expect(u.via![0].children!.map((c) => c.key)).toEqual(["C:\\Users\\king", "C:\\Users\\Public"]);
  });
  it("数据拼进来、外壳从「不合并」变成「合并」：块的 key 不变", () => {
    // king 还没有子项数据时不合并
    const bare = n("C:\\", 128, [n("C:\\Users", 98, [n("C:\\Users\\king", 97), n("C:\\Users\\Public", 1)])]);
    const before = toInput(bare, [], opts).children![0];
    const after = toInput(TREE, [], opts).children![0];
    expect(before.name).toBe("Users");
    expect(after.name).toBe("Users \\ king");
    expect(before.key).toBe(after.key);
    expect(before.value).toBe(after.value);
  });
  it("链正好停在外壳外层（面包屑点 Users）：不合并，显示 king / Public（和列表一致），Users 在链上", () => {
    const u = toInput(TREE, ["C:\\Users"], opts).children!.find((c) => c.key === "C:\\Users")!;
    expect(u.name).toBe("Users");
    expect(u.boost).toBe(BOOST);
    expect(u.children!.map((c) => c.name)).toEqual(["king", "Public"]);
    expect(u.via).toBeUndefined();
    // 链再往下一级（king）：又合并成一块，块仍在链上
    const m = toInput(TREE, ["C:\\Users", "C:\\Users\\king"], opts).children!.find((c) => c.key === "C:\\Users")!;
    expect(m.name).toBe("Users \\ king");
    expect(m.boost).toBe(BOOST);
  });
  it("只转换会画到的层：链外两层，链上的块下面另外两层", () => {
    const deep = (p: string, d: number): TreemapNode => n(p, 8, d ? [deep(p + "\\a", d - 1), deep(p + "\\b", d - 1)] : [file(p + "\\f", 8)]);
    const T = n("C:\\", 32, [deep("C:\\x", 5), deep("C:\\y", 5)]);
    const depth = (t: TmInputLike): number => 1 + Math.max(0, ...(t.children ?? []).map(depth));
    const r = toInput(T, [], opts);
    expect(depth(r)).toBe(3); // 根 + 两层
    const on = toInput(T, ["C:\\x"], opts);
    expect(depth(on.children!.find((c) => c.key === "C:\\x")!)).toBe(3); // x + 子项 + 孙项
    expect(depth(on.children!.find((c) => c.key === "C:\\y")!)).toBe(2);
  });
});
type TmInputLike = { children?: TmInputLike[] };

describe("buildDisplay：显示树 = 盘根的数据 + 链上每一级自己的子树（M3）", () => {
  const winOwn = n("C:\\Windows", 30, [n("C:\\Windows\\System32", 20, [file("C:\\Windows\\System32\\a.dll", 12), file("C:\\Windows\\System32\\b.dll", 8)]), n("C:\\Windows\\Temp", 6, [file("C:\\Windows\\Temp\\t", 6)])], { hidden_bytes: 4, hidden_count: 2 });
  const cache = new Map<string, TreemapNode>([["c:\\windows", winOwn]]);
  it("只拼链上的：链上有 Windows 才用 Windows 自己的数据；收起以后回到盘根自己的数据（同一条链永远是同一张图）", () => {
    const on = buildDisplay(TREE, ["C:\\Windows"], cache, new Map());
    expect(findTm(on, "C:\\Windows")!.children.map((c) => c.name)).toEqual(["System32", "Temp"]);
    const off = buildDisplay(TREE, [], cache, new Map());
    expect(off).toBe(TREE);
  });
  it("补节点：祖先的数据重拼以后照样补上（不会因为父级被较粗的数据盖掉而消失）", () => {
    const stub = n("C:\\Windows\\Fonts", 2, [], { can_open: false });
    const stubs = new Map<string, Stub>([["c:\\windows\\fonts", { parent: "C:\\Windows", node: stub }]]);
    // Windows 自己的数据里也没有 Fonts（超过每层的个数上限）
    const t = buildDisplay(TREE, ["C:\\Windows", "C:\\Windows\\Fonts"], cache, stubs)!;
    const w = findTm(t, "C:\\Windows")!;
    expect(w.children.map((c) => c.name)).toEqual(["System32", "Temp", "Fonts"]);
    expect(w.hidden_bytes).toBe(2);
    // 每次都从数据源重拼：缓存了的子树不会「取过但没拼进去」
    const sysOwn = n("C:\\Windows\\System32", 20, [file("C:\\Windows\\System32\\c.dll", 20)]);
    const t2 = buildDisplay(TREE, ["C:\\Windows", "C:\\Windows\\System32"], new Map([...cache, ["c:\\windows\\system32", sysOwn]]), stubs)!;
    expect(findTm(t2, "C:\\Windows\\System32")!.children.map((c) => c.name)).toEqual(["c.dll"]);
  });
});

describe("参照矩形的原始各层 / 自动缩放的候选根", () => {
  it("pathInputs：扫描根（带伪块）→ … → 目标，每层只带子项的 key / 值", () => {
    const extra = [{ key: "pseudo:sysest", name: "系统保留", value: 5 }];
    const p = pathInputs(TREE, "C:\\Users\\king", extra)!;
    expect(p.map((x) => x.key)).toEqual(["C:\\", "C:\\Users", "C:\\Users\\king"]);
    expect(p[0].value).toBe(133);
    expect(p[0].children!.map((c) => c.key)).toEqual(["C:\\Users", "C:\\Windows", "pseudo:sysest"]);
    expect(pathInputs(TREE, "C:\\nope")).toBeNull();
  });
  it("rootCandidates：扫描根，然后链在图上每一层的最后一级（外壳取内层）", () => {
    expect(rootCandidates(TREE, ["C:\\Users", "C:\\Users\\king", "C:\\Users\\king\\AppData"])).toEqual(["C:\\", "C:\\Users\\king"]);
    expect(rootCandidates(TREE, ["C:\\Windows", "C:\\Windows\\System32"])).toEqual(["C:\\", "C:\\Windows"]);
  });
});

describe("单击翻译：方块 → MapHit", () => {
  const item = (key: string, ref: unknown, extra: Partial<{ kind: "node" | "other"; parentKey: string | null; mergedCount: number; value: number }> = {}) => ({
    key,
    ref,
    kind: "node" as const,
    parentKey: null,
    mergedCount: 0,
    value: 1,
    ...extra,
  });
  it("文件夹 = 往里挖（合并块给内层）；文件 = 选中 + 列表那一行；其他 = 选中 + 签名；伪块 = 只选中", () => {
    expect(tileHit(item("C:\\Users", king), "C:\\")).toEqual({ kind: "dir", path: "C:\\Users\\king" });
    expect(tileHit(item("C:\\Users\\king\\x.dat", king.children[2]), "C:\\Users\\king")).toEqual({ kind: "leaf", key: "C:\\Users\\king\\x.dat", parent: "C:\\Users\\king", row: "C:\\Users\\king\\x.datFile" });
    const o = item("C:\\Windows|其他", undefined, { kind: "other", mergedCount: 4, value: 10.4 });
    expect(tileHit(o, "C:\\Windows")).toEqual({ kind: "leaf", key: "C:\\Windows|其他", parent: "C:\\Windows", row: null, sig: "4|10" });
    expect(otherSig({ mergedCount: 4, value: 10.4 })).toBe("4|10");
    expect(tileHit(item("pseudo:sysest", { pseudo: "sysest" }), "C:\\")).toEqual({ kind: "leaf", key: "pseudo:sysest", parent: "C:\\", row: null });
  });
  it("标题栏：path = 内层（往里挖），outer = 块的 key（收起按外层算）", () => {
    expect(headHit(item("C:\\Users", king))).toEqual({ kind: "head", path: "C:\\Users\\king", outer: "C:\\Users" });
    expect(headHit(item("pseudo:x", { pseudo: "x" }))).toBeNull();
  });
  it("块所在的文件夹：顶层块 = 图的根（自动缩放后是缩放后的根）；其余 = 父块（合并块取内层）", () => {
    const byKey = new Map([["C:\\Users", item("C:\\Users", king)]]);
    expect(tileParent({ parentKey: null }, byKey, "C:\\Users\\king")).toBe("C:\\Users\\king");
    expect(tileParent({ parentKey: "C:\\Users" }, byKey, "C:\\")).toBe("C:\\Users\\king");
    expect(tileParent({ parentKey: "nope" }, byKey, "C:\\")).toBeNull();
  });
});

// ───── 第 2 轮（外观 / 键盘）：方块 → MapHit 的整段翻译、块上的字、标题栏放不放得下、方向键 ─────
import { countLabel, countText, fitHead, hitOf, navStep, navTargets, tipName, type NavBox } from "./tmtree";

describe("hitOf：方块 → MapHit（TreemapView 只调这一个）", () => {
  const tile = (key: string, ref: unknown, parentKey: string | null, extra: Partial<{ kind: "node" | "other"; mergedCount: number; value: number }> = {}) => ({
    key,
    ref,
    parentKey,
    kind: "node" as const,
    mergedCount: 0,
    value: 1,
    ...extra,
  });
  const usersTile = tile("C:\\Users", king, null);
  const byKey = new Map([["C:\\Users", usersTile]]);
  it("外壳合并块：身体 = 往里挖到内层；标题栏的 outer = 外层（收起按外层算）", () => {
    expect(hitOf(usersTile, byKey, "C:\\\\")).toEqual({ kind: "dir", path: "C:\\Users\\king" });
    expect(headHit(usersTile)).toEqual({ kind: "head", path: "C:\\Users\\king", outer: "C:\\Users" });
  });
  it("「其余小文件」：列表行的 key = 父文件夹路径 + Rest；所在的文件夹 = 合并块的内层", () => {
    const rest = n("C:\\Users\\king", 3, [], { kind: "Rest", name: "（其余 2380 个文件）", can_open: false });
    expect(hitOf(tile("C:\\Users\\king|rest", rest, "C:\\Users"), byKey, "C:\\\\")).toEqual({
      kind: "leaf",
      key: "C:\\Users\\king|rest",
      parent: "C:\\Users\\king",
      row: "C:\\Users\\kingRest",
    });
  });
  it("「其他」/ 伪块：所在的文件夹 = 父块；顶层的 = 图的根（自动缩放后 = 缩放后的根）", () => {
    const o = tile("C:\\Users|其他|3", undefined, "C:\\Users", { kind: "other", mergedCount: 3, value: 5 });
    expect(hitOf(o, byKey, "C:\\\\")).toEqual({ kind: "leaf", key: "C:\\Users|其他|3", parent: "C:\\Users\\king", row: null, sig: "3|5" });
    expect(hitOf(tile("pseudo:inacc", { pseudo: "inacc" }, null), byKey, "C:\\\\")).toEqual({ kind: "leaf", key: "pseudo:inacc", parent: "C:\\\\", row: null });
    const sys = n("C:\\Windows\\System32", 20, [], { can_open: true });
    expect(hitOf(tile("C:\\Windows\\System32", sys, null), byKey, "C:\\Windows")).toEqual({ kind: "dir", path: "C:\\Windows\\System32" });
    const f = file("C:\\Windows\\a.log", 2);
    expect(hitOf(tile("C:\\Windows\\a.log", f, null), byKey, "C:\\Windows")).toEqual({ kind: "leaf", key: "C:\\Windows\\a.log", parent: "C:\\Windows", row: "C:\\Windows\\a.logFile" });
  });
});

describe("块上的字", () => {
  it("countLabel：「其他 410 项」「（其余 2380 个文件）」拆成 前缀 / 数字 / 后缀（截断时保住数字）；别的名字 = null", () => {
    expect(countLabel({ kind: "other", name: "其他 410 项", mergedCount: 410 })).toEqual({ pre: "其他", num: "410", post: "项" });
    const rest = n("C:\\Temp", 3, [], { kind: "Rest", name: "（其余 2380 个文件）" });
    const c = countLabel({ kind: "node", name: rest.name, mergedCount: 0, ref: rest })!;
    expect(c).toEqual({ pre: "其余", num: "2380", post: "个" });
    expect(countText(c)).toBe("其余 2380 个");
    expect(countLabel({ kind: "node", name: "Temp", mergedCount: 0, ref: n("C:\\Temp", 3) })).toBeNull();
    expect(tipName({ name: rest.name, ref: rest })).toBe("其余 2380 个文件");
    expect(tipName({ name: "Temp", ref: n("C:\\Temp", 3) })).toBe("Temp");
  });

  // 等宽假字体：每个字 10px，省略号 8px
  const M = { name: (s: string) => Array.from(s).length * 10, size: (s: string) => Array.from(s).length * 10, ell: 8, gap: 6 };
  it("fitHead：放得下就名字 + 大小", () => {
    expect(fitHead("Windows", "31.6 GB", 200, M)).toEqual({ name: "Windows", cut: false, size: true });
  });
  it("fitHead：带着大小名字还能露出 ≥ 6 个字：截名字、留大小", () => {
    // 名字 20 字；大小 6 字（60）+ 间隔 6 → 名字可用 150 − 66 − 8 = 76 → 7 个字
    expect(fitHead("SoftwareDistribution", "2.4 GB", 150, M)).toEqual({ name: "Softwar", cut: true, size: true });
  });
  it("fitHead：带着大小名字露不出 6 个字：不写大小，空间全给名字", () => {
    // 可用 120：带大小只剩 120 − 66 − 8 = 46 → 4 个字 < 6 → 不写大小，名字可用 112 → 11 个字
    expect(fitHead("Program Files (x86)", "4.6 GB", 120, M)).toEqual({ name: "Program Fil", cut: true, size: false });
    // 不写大小时名字整个放得下：不截
    expect(fitHead("DriverStore", "3.1 GB", 120, M)).toEqual({ name: "DriverStore", cut: false, size: false });
    // 名字本身不到 6 个字、带大小放不下：先保名字
    expect(fitHead("Temp", "1.1 GB", 90, M)).toEqual({ name: "Temp", cut: false, size: false });
  });
  it("fitHead：截在合并块名字的分隔处：不留末尾的「 \\ 」", () => {
    // 「.ollama \\ models」：可用 160 → 带大小能露 8 个字「.ollama 」；可用 180 → 10 个字（到反斜杠后面）→ 都去掉末尾的空格和反斜杠
    expect(fitHead(".ollama \\ models", "7.7 GB", 160, M)).toEqual({ name: ".ollama", cut: true, size: true });
    expect(fitHead(".ollama \\ models", "7.7 GB", 180, M)).toEqual({ name: ".ollama", cut: true, size: true });
  });
  it("fitHead：窄到名字只露得出一两个字：只写大小（「W…」猜不出是什么）；大小也放不下就照样截名字", () => {
    // 数字比名字窄（每个 6px）
    const N = { ...M, size: (s: string) => s.length * 6 };
    expect(fitHead("WinSxS", "9.2 GB", 36, N)).toEqual({ name: "", cut: false, size: true });
    expect(fitHead("WinSxS", "9.2 GB", 30, N)).toEqual({ name: "Wi", cut: true, size: false });
  });
});

describe("键盘：roving tabindex 的走法", () => {
  // 图：左边一个顶层叶子 A（0..100），右边链上的容器 W（110..400，标题栏 26px），W 里两个子块 S（上）和 T（下），
  // 右下角一个不在链上的容器 P（里面的子块 P1 不能直接走到）
  const items = [
    { key: "A", parentKey: null, container: false, chain: false, b: { x: 0, y: 0, w: 100, h: 300 } },
    { key: "W", parentKey: null, container: true, chain: true, b: { x: 110, y: 0, w: 290, h: 200 } },
    { key: "S", parentKey: "W", container: false, chain: false, b: { x: 116, y: 30, w: 278, h: 80 } },
    { key: "T", parentKey: "W", container: false, chain: false, b: { x: 116, y: 114, w: 278, h: 80 } },
    { key: "P", parentKey: null, container: true, chain: false, b: { x: 110, y: 210, w: 290, h: 90 } },
    { key: "P1", parentKey: "P", container: false, chain: false, b: { x: 116, y: 236, w: 100, h: 60 } },
    { key: "tiny", parentKey: null, container: false, chain: false, b: { x: 0, y: 305, w: 6, h: 6 } },
  ];
  const boxes: NavBox[] = navTargets(items, (t) => t.b, 26, (t) => t.key === "tiny");
  it("navTargets：顶层块 + 链上容器里的块；链上的容器只占它的标题栏；不在链上的容器算一整块；跳过的不算", () => {
    expect(boxes.map((b) => b.key).sort()).toEqual(["A", "P", "S", "T", "W"]);
    expect(boxes.find((b) => b.key === "W")).toEqual({ key: "W", x: 110, y: 0, w: 290, h: 26 });
    expect(boxes.find((b) => b.key === "P")!.h).toBe(90);
  });
  it("navStep：按画面位置走（侧向对齐的优先）；从链上容器的标题栏往下 = 走进它的第一行子块，往上回到标题栏", () => {
    expect(navStep(boxes, "W", "down")).toBe("S");
    expect(navStep(boxes, "S", "up")).toBe("W");
    expect(navStep(boxes, "S", "down")).toBe("T");
    expect(navStep(boxes, "T", "down")).toBe("P");
    expect(navStep(boxes, "T", "left")).toBe("A");
    expect(navStep(boxes, "A", "right")).toBe("T");
    expect(navStep(boxes, "A", "left")).toBeNull();
    expect(navStep(boxes, "nope", "left")).toBeNull();
  });
});

// ───── 第 3 轮：外壳合并块的数字（F6）、缩放后的根标题条（F7）、块上的字按量字决定写法（F3） ─────
import { fitLabel, layoutMap, rootHead, type LabelMetrics } from "./tmtree";
import { DEFAULT_LAYOUT } from "./treemap";

describe("F6：外壳合并块写哪一层的大小", () => {
  const opts = { maxDepth: 2, boostDepth: 2 };
  it("链停在外层 / 不在链上：写外层（和上一级列表一致）；链走进内层：写内层（和面包屑、列表一致）；面积始终按外层", () => {
    const off = toInput(TREE, [], opts).children!.find((c) => c.key === "C:\\Users")!;
    expect(off.name).toBe("Users \\ king");
    expect(off.size).toBe(98);
    expect(off.value).toBe(98);
    const at = toInput(TREE, ["C:\\Users"], opts).children!.find((c) => c.key === "C:\\Users")!;
    expect(at.name).toBe("Users");
    expect(at.size).toBe(98);
    for (const ch of [
      ["C:\\Users", "C:\\Users\\king"],
      ["C:\\Users", "C:\\Users\\king", "C:\\Users\\king\\AppData"],
    ]) {
      const inn = toInput(TREE, ch, opts).children!.find((c) => c.key === "C:\\Users")!;
      expect(inn.name).toBe("Users \\ king");
      expect(inn.size).toBe(97);
      expect(inn.value).toBe(98);
    }
  });
});

describe("F7：自动缩放后图顶上的根标题条", () => {
  const deep = (p: string, d: number, k = 4): TreemapNode => n(p, 0, d ? Array.from({ length: k }, (_, i) => deep(`${p}\\d${i}`, d - 1, k)) : [file(`${p}\\f`, 10)]);
  const fix = (t: TreemapNode): TreemapNode => {
    t.children.forEach(fix);
    if (t.children.length) t.allocated = t.children.reduce((s, c) => s + c.allocated, 0);
    return t;
  };
  const T = fix(n("C:\\", 0, [deep("C:\\a", 4), deep("C:\\b", 3), n("C:\\Users", 0, [deep("C:\\Users\\king", 3), file("C:\\Users\\p", 1)])]));
  const frame = { x: 0, y: 0, w: 900, h: 700 };
  const opt = { ...DEFAULT_LAYOUT, minPx: 8, maxDepth: 2, boostDepth: 2, headerPx: 26, pad: 4, padEnd: 6, minContainerPx: 60 };
  it("根是扫描根：没有标题条；根不是扫描根：根画成铺满整张图的容器（深度 0），子块的 parentKey = 它", () => {
    const flat = layoutMap(T, [], "C:\\", frame, opt)!;
    expect(flat.items.some((i) => i.depth === 0)).toBe(false);
    const chain = ["C:\\a", "C:\\a\\d0", "C:\\a\\d0\\d1"];
    const z = layoutMap(T, chain, "C:\\a\\d0", frame, opt)!;
    const head = z.items.find((i) => i.depth === 0)!;
    expect(head).toMatchObject({ key: "C:\\a\\d0", name: "d0", container: true, chain: true, parentKey: null });
    expect(head.rect).toEqual(frame);
    for (const it of z.items.filter((i) => i.depth === 1)) expect(it.parentKey).toBe(head.key);
    // 单击标题条 = 收起到这个根这一级（链停在它这里，和点面包屑上它那一级一样）
    expect(headHit(head)).toEqual({ kind: "dir", path: "C:\\a\\d0" });
    // 链上别的容器的标题栏照旧 = 收起它
    const pkBlock = z.items.find((i) => i.key === "C:\\a\\d0\\d1")!;
    expect(headHit(pkBlock)).toEqual({ kind: "head", path: "C:\\a\\d0\\d1", outer: "C:\\a\\d0\\d1" });
  });
  it("缩放的根是外壳合并块的内层：标题条用外层的 key 和合并的名字（和上一级图里那一块是同一个 key），大小写内层", () => {
    const chain = ["C:\\Users", "C:\\Users\\king", "C:\\Users\\king\\d0"];
    expect(rootHead(T, chain, "C:\\Users\\king")).toEqual({ key: "C:\\Users", name: "Users \\ king", size: findTm(T, "C:\\Users\\king")!.allocated });
    const z = layoutMap(T, chain, "C:\\Users\\king", frame, opt)!;
    const head = z.items.find((i) => i.depth === 0)!;
    expect(head.key).toBe("C:\\Users");
    const flat = layoutMap(T, chain, "C:\\", frame, opt)!;
    // 同一个 key：缩放前它是图里的一块（链上），缩放后它铺满整张图
    expect(flat.items.find((i) => i.key === "C:\\Users")!.chain).toBe(true);
    // 子块的 key 缩放前后一样（父 key 一样）
    const kids = (L: typeof z.items) => new Set(L.filter((i) => i.parentKey === "C:\\Users").map((i) => i.key));
    const a = kids(flat.items);
    const b = kids(z.items);
    expect([...b].filter((k) => a.has(k)).length).toBeGreaterThan(0);
  });
});

describe("F3：块上的字按量出来的宽度 / 行高决定写法，永不硬裁", () => {
  // 假字体：每个字 7px（数字也 7px）、省略号 6px；行高 15.6 / 一行写法 11.5 / 竖排 15 / 只写大小 12 / 小一号 11
  const W = (k: number) => (s: string) => Array.from(s).length * k;
  const M: LabelMetrics = {
    name: (s, big) => W(big ? 9 : 7)(s),
    size: (s, big) => W(big ? 8 : 7)(s),
    lname: W(6.7),
    lsize: W(6.7),
    vname: W(7),
    vsize: W(7),
    tiny: W(6.4),
    micro: W(5.8),
    ell: (big) => (big ? 8 : 6),
    ellLine: 6,
    ellV: 6,
    lh: { a: 15.6, aBig: 19.5, b: 15.6, bBig: 17, ln: 11.5, v: 15, s: 12, t: 11, u: 10, uv: 12.5 },
  };
  const base = { name: "Program Files (x86)", full: "4.6 GB", short: "4.6" };
  it("两行写法要放得下：C 主题 2px 墨框 + 上下 7px 留白 + 两行字 ≈ 49.2px；不够高就降成一行", () => {
    expect(fitLabel({ ...base, w: 200, h: 49, bw: 2 }, M).mode).toBe("line");
    expect(fitLabel({ ...base, w: 200, h: 50, bw: 2 }, M).mode).toBe("full");
    // 没有描边（A / B）：45.2px 就够
    expect(fitLabel({ ...base, w: 200, h: 46, bw: 0 }, M).mode).toBe("full");
  });
  it("第二行的大小整串放得下才写两行；名字截断时至少露出 3 个字，截断用「…」", () => {
    const f = fitLabel({ ...base, w: 80, h: 80, bw: 2 }, M);
    // 可用宽 80 − 4 − 16 − 1.5 = 58.5：大小 6 × 7 = 42 放得下；名字截到 (58.5 − 6) / 7 = 7 个字
    expect(f).toMatchObject({ mode: "full", name: "Program", cut: true });
    // 再窄：名字露不出 3 个字 → 不写两行
    expect(fitLabel({ ...base, w: 42, h: 80, bw: 2 }, M).mode).not.toBe("full");
  });
  it("只写短大小：整串放得下才写（不会出现「27N」），放不下就不写", () => {
    const t = { name: "x", full: "27.5 MB", short: "27M" };
    // 21px 的「27M」+ 左右 5 + 墨框 2 × 2 + 误差 1.5 = 35.5：36 宽写；更窄用小一号的字（19.2 + 6 + 4 + 1.5 = 30.7）
    expect(fitLabel({ ...t, w: 36, h: 36, bw: 2 }, M)).toMatchObject({ mode: "size" });
    expect(fitLabel({ ...t, w: 31, h: 31, bw: 2 }, M)).toMatchObject({ mode: "size", tiny: true });
    // 再窄：10px、贴边 2px（17.4 + 4 + 4 + 1.5 = 26.9）——C 主题 30 px 上下的小格子不再一排空着
    const u = fitLabel({ ...t, w: 29, h: 29, bw: 2 }, M);
    expect(u).toMatchObject({ mode: "size", tiny: true, micro: true, ix: 2 });
    expect(fitLabel({ ...t, w: 26, h: 26, bw: 2 }, M).mode).toBe("none");
    // 窄高的小块：大小竖着写；再窄（20 px 宽）用 10px 竖着写
    expect(fitLabel({ ...t, w: 26, h: 60, bw: 2 }, M)).toMatchObject({ mode: "vert", name: "" });
    expect(fitLabel({ ...t, w: 21, h: 49, bw: 2 }, M)).toMatchObject({ mode: "vert", name: "", micro: true });
    expect(fitLabel({ ...t, w: 18, h: 49, bw: 2 }, M).mode).toBe("none");
  });
  it("「其他 410 项」：整段 → 「410 项」→ 不写名字（数字不截）", () => {
    const c = { name: "其他 410 项", count: { pre: "其他", num: "410", post: "项" }, full: "1.2 GB", short: "1.2" };
    expect(fitLabel({ ...c, w: 120, h: 60, bw: 2 }, M)).toMatchObject({ mode: "full", name: "其他 410 项", cut: false });
    expect(fitLabel({ ...c, w: 70, h: 60, bw: 2 }, M)).toMatchObject({ mode: "full", name: "410 项", cut: false });
    const small = fitLabel({ ...c, w: 52, h: 60, bw: 2 }, M);
    expect(["其他 410 项", "410 项", ""]).toContain(small.name);
  });
  it("随机 5000 个块：选中的写法里每一行字（按同一把尺子量）都在块里面，绝不超出", () => {
    let seed = 3;
    const rnd = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
    const names = ["Windows", "Program Files (x86)", "下载", "nv12.inf_amd64_172a4", "ubuntu-24.04.1-desktop-amd64.iso", "a"];
    for (let i = 0; i < 5000; i++) {
      const w = 8 + rnd() * 240;
      const h = 8 + rnd() * 160;
      const bw = rnd() < 0.5 ? 2 : 0;
      const name = names[Math.floor(rnd() * names.length)];
      const full = rnd() < 0.5 ? "312 MB" : "12.7 GB";
      const short = full === "312 MB" ? "0.3" : "12.7";
      const f = fitLabel({ name, full, short, w, h, bw, big: w > 150 && h > 70 }, M);
      const iw = w - 2 * bw - 2 * f.ix;
      const ih = h - 2 * bw - 2 * f.iy;
      const nm = (big: boolean) => M.name(f.name, big) + (f.cut ? M.ell(big) : 0);
      if (f.mode === "full") {
        expect(Math.max(nm(f.big), M.size(full, f.big))).toBeLessThanOrEqual(iw);
        expect(f.big ? M.lh.aBig + M.lh.bBig : M.lh.a + M.lh.b).toBeLessThanOrEqual(ih);
      } else if (f.mode === "line") {
        expect(M.lname(f.name) + (f.cut ? M.ellLine : 0) + 6 + M.lsize(full)).toBeLessThanOrEqual(iw);
        expect(M.lh.ln).toBeLessThanOrEqual(h - 2 * bw);
      } else if (f.mode === "size") {
        expect(f.micro ? M.micro(short) : f.tiny ? M.tiny(short) : M.size(short, false)).toBeLessThanOrEqual(iw);
        expect(f.micro ? M.lh.u : f.tiny ? M.lh.t : M.lh.s).toBeLessThanOrEqual(ih);
      } else if (f.mode === "vert") {
        expect(f.micro ? M.lh.uv : M.lh.v).toBeLessThanOrEqual(iw);
        expect(M.vname(f.name) + (f.cut ? M.ellV : 0) + (f.name ? 6 : 0) + (f.micro ? M.micro(short) : M.vsize(short))).toBeLessThanOrEqual(ih);
      }
      if (f.mode !== "none" && f.mode !== "size" && f.name && !f.cut) expect(f.name).toBe(name);
      if (f.cut) expect(Array.from(f.name).length).toBeGreaterThanOrEqual(3);
    }
  });
});
