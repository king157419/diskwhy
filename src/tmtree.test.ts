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
