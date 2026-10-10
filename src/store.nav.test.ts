// 空间分析的导航（阶段 K：一张平面图，单击原地展开）。后端命令用假的：
// get_treemap 像真后端一样按 depth 截断、每层最多发 MAX_KIDS 个（其余并进 hidden_*），list_dir 给真实的面包屑。
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DirListing, TreemapNode } from "./api";

type R = { name: string; path: string; kind: "Dir" | "File"; size: number; children: R[] };
const MAX_KIDS = 5;
function D(path: string, children: R[]): R {
  return { name: path.split("\\").filter(Boolean).pop() ?? path, path, kind: "Dir", size: children.reduce((s, c) => s + c.size, 0), children: children.sort((a, b) => b.size - a.size) };
}
const F = (path: string, size: number): R => ({ name: path.split("\\").pop()!, path, kind: "File", size, children: [] });
let ROOT: R;
function build() {
  const temp = D("C:\\Users\\king\\AppData\\Local\\Temp", [D("C:\\Users\\king\\AppData\\Local\\Temp\\t1", [F("C:\\Users\\king\\AppData\\Local\\Temp\\t1\\f", 20)]), D("C:\\Users\\king\\AppData\\Local\\Temp\\t2", [F("C:\\Users\\king\\AppData\\Local\\Temp\\t2\\f", 10)])]);
  const local = D("C:\\Users\\king\\AppData\\Local", [temp, D("C:\\Users\\king\\AppData\\Local\\Google", [F("C:\\Users\\king\\AppData\\Local\\Google\\g", 20)])]);
  const appdata = D("C:\\Users\\king\\AppData", [local, D("C:\\Users\\king\\AppData\\Roaming", [F("C:\\Users\\king\\AppData\\Roaming\\r", 10)])]);
  const king = D("C:\\Users\\king", [appdata, D("C:\\Users\\king\\下载", [F("C:\\Users\\king\\下载\\a.iso", 30)]), F("C:\\Users\\king\\x.dat", 7)]);
  const users = D("C:\\Users", [king, D("C:\\Users\\Public", [F("C:\\Users\\Public\\p", 1)])]);
  const winsxs = D("C:\\Windows\\WinSxS", Array.from({ length: 8 }, (_, i) => D(`C:\\Windows\\WinSxS\\w${i}`, [F(`C:\\Windows\\WinSxS\\w${i}\\f`, 10 - i)])));
  const win = D("C:\\Windows", [winsxs, D("C:\\Windows\\System32", [F("C:\\Windows\\System32\\a.dll", 21)])]);
  ROOT = D("C:\\", [users, win, F("C:\\hiberfil.sys", 12)]);
  ROOT.name = "C:\\";
}
function find(p: string): R | null {
  const want = p.replace(/\\+$/, "").toLowerCase();
  const walk = (n: R): R | null => (n.path.replace(/\\+$/, "").toLowerCase() === want ? n : n.children.reduce<R | null>((a, c) => a ?? (c.kind === "Dir" ? walk(c) : null), null));
  return walk(ROOT);
}
function tm(n: R, level: number, depth: number): TreemapNode {
  const me = {
    name: n.name,
    path: n.path,
    kind: n.kind,
    allocated: n.size,
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
    can_open: n.kind === "Dir" && n.children.length > 0,
    children: [] as TreemapNode[],
    hidden_bytes: 0,
    hidden_count: 0,
  } as TreemapNode;
  if (level >= depth || n.kind !== "Dir") return me;
  n.children.forEach((c, i) => {
    if (i >= MAX_KIDS) {
      me.hidden_bytes += c.size;
      me.hidden_count++;
    } else me.children.push(tm(c, level + 1, depth));
  });
  return me;
}
const view = (n: R) => ({ name: n.name, path: n.path, kind: n.kind, allocated: n.size, child_count: n.children.length }) as never;
/** list_dir 的延迟（测快速连点时过期的结果） */
const gates = new Map<string, Promise<void>>();

vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  listDir: vi.fn(async (path: string): Promise<DirListing> => {
    await gates.get(path.toLowerCase());
    const n = find(path);
    if (!n) throw `找不到 ${path}`;
    const bc: { name: string; path: string }[] = [];
    let cur: R | null = ROOT;
    while (cur) {
      bc.push({ name: cur.name, path: cur.path });
      if (cur === n) break;
      cur = cur.children.find((c) => c.kind === "Dir" && (n.path.toLowerCase() + "\\").startsWith(c.path.toLowerCase() + "\\")) ?? null;
    }
    return { node: view(n), children: n.children.map(view), total_children: n.children.length, breadcrumb: bc };
  }),
  getTreemap: vi.fn(async (path: string, depth = 3) => {
    const n = find(path);
    if (!n) throw `找不到 ${path}`;
    return tm(n, 0, depth);
  }),
}));

const api = await import("./api");
const store = await import("./store");
const getTreemap = vi.mocked(api.getTreemap);

beforeEach(async () => {
  build();
  gates.clear();
  store.summary.value = { root: "C:\\", buckets: {} } as never;
  store.listing.value = null;
  await store.goExplore();
  getTreemap.mockClear();
});

const paths = () => store.chain.value.map((p) => p.split("\\").pop());

describe("一进分析页", () => {
  it("什么都不展开：链为空，方块图 = 整个盘，右边列表 = 盘根", () => {
    expect(store.chain.value).toEqual([]);
    expect(store.tm.value?.path).toBe("C:\\");
    expect(store.focus.value?.node.path).toBe("C:\\");
    expect(store.focusKey.value).toBeNull();
    expect(store.crumbs.value.map((c) => c.name)).toEqual(["C:"]);
  });
});

describe("digTo：单击往里挖", () => {
  it("链 = 根 → 它（祖先从数据里找），右边列表 = 它，面包屑跟着", async () => {
    await store.digTo("C:\\Users\\king\\AppData");
    expect(paths()).toEqual(["Users", "king", "AppData"]);
    expect(store.focus.value?.node.path).toBe("C:\\Users\\king\\AppData");
    expect(store.focusKey.value).toBe("C:\\Users\\king\\AppData");
    expect(store.crumbs.value.map((c) => c.name)).toEqual(["C:", "Users", "king", "AppData"]);
    // 方块图的数据始终从扫描根开始（自动缩放由方块图按像素决定）
    expect(store.tm.value?.path).toBe("C:\\");
  });

  it("下面不够两层：取它的子树拼进来（按路径缓存，再挖一次不重取）", async () => {
    // 盘根取了 3 层：Users / king / AppData；AppData 的子项没有
    await store.digTo("C:\\Users\\king\\AppData");
    expect(getTreemap).toHaveBeenCalledWith("C:\\Users\\king\\AppData", 3);
    const local = store.tm.value!.children[0].children[0].children[0].children[0];
    expect(local.path).toBe("C:\\Users\\king\\AppData\\Local");
    expect(local.children.map((c) => c.name)).toEqual(["Temp", "Google"]);
    getTreemap.mockClear();
    await store.digTo("C:\\Users");
    await store.digTo("c:\\users\\KING\\appdata\\");
    expect(getTreemap).not.toHaveBeenCalled();
  });

  it("数据里没有的文件夹（每层个数上限外）：先取父级；还没有就用列表补一个节点", async () => {
    await store.digTo("C:\\Windows\\WinSxS\\w7");
    expect(paths()).toEqual(["Windows", "WinSxS", "w7"]);
    expect(store.focus.value?.node.path).toBe("C:\\Windows\\WinSxS\\w7");
    const winsxs = store.tm.value!.children.find((c) => c.name === "Windows")!.children.find((c) => c.name === "WinSxS")!;
    expect(winsxs.children.some((c) => c.name === "w7")).toBe(true);
    expect(winsxs.hidden_count).toBe(2);
  });

  it("快速连点：只有最后一次的列表结果才用", async () => {
    let open!: () => void;
    gates.set("c:\\windows", new Promise<void>((r) => (open = r)));
    const a = store.digTo("C:\\Windows");
    const b = store.digTo("C:\\Users\\king");
    await b;
    open();
    await a;
    expect(paths()).toEqual(["Users", "king"]);
    expect(store.focus.value?.node.path).toBe("C:\\Users\\king");
  });
});

describe("收起", () => {
  beforeEach(async () => {
    await store.digTo("C:\\Users\\king\\AppData\\Local");
  });
  it("面包屑第 i 级 = 链截到第 i 级（0 = 全部收起）", async () => {
    await store.collapseTo(3);
    expect(paths()).toEqual(["Users", "king", "AppData"]);
    await store.collapseTo(0);
    expect(store.chain.value).toEqual([]);
    expect(store.focus.value?.node.path).toBe("C:\\");
  });
  it("Backspace 收起一级（Users \\ king 合并成一块，一次收起）；Esc 全部收起", async () => {
    await store.collapseOne();
    expect(paths()).toEqual(["Users", "king", "AppData"]);
    await store.collapseOne();
    expect(paths()).toEqual(["Users", "king"]);
    await store.collapseOne();
    expect(store.chain.value).toEqual([]);
    await store.digTo("C:\\Windows\\System32");
    await store.collapseAll();
    expect(store.chain.value).toEqual([]);
  });
  it("只有列表（看不见方块图）时 Backspace 按文件夹一级一级往上", async () => {
    await store.collapseOne(false);
    await store.collapseOne(false);
    expect(paths()).toEqual(["Users", "king"]);
    await store.collapseOne(false);
    expect(paths()).toEqual(["Users"]);
  });
});

describe("mapClick：方块图的单击", () => {
  it("单击文件夹 = 往里挖；单击另一个顶层块 = 链换到那边", async () => {
    await store.mapClick({ kind: "dir", path: "C:\\Users\\king" }, { at: 1000, detail: 1 });
    expect(paths()).toEqual(["Users", "king"]);
    await store.mapClick({ kind: "dir", path: "C:\\Windows" }, { at: 2000, detail: 1 });
    expect(paths()).toEqual(["Windows"]);
  });

  it("标题栏：在链上 = 收起到它的父级（合并块按外层）；不在链上 = 往里挖", async () => {
    await store.digTo("C:\\Users\\king\\AppData\\Local");
    await store.mapClick({ kind: "head", path: "C:\\Users\\king\\AppData", outer: "C:\\Users\\king\\AppData" }, { at: 1000, detail: 1 });
    expect(paths()).toEqual(["Users", "king"]);
    await store.mapClick({ kind: "head", path: "C:\\Users\\king", outer: "C:\\Users" }, { at: 2000, detail: 1 });
    expect(store.chain.value).toEqual([]);
    await store.mapClick({ kind: "head", path: "C:\\Windows", outer: "C:\\Windows" }, { at: 3000, detail: 1 });
    expect(paths()).toEqual(["Windows"]);
  });

  it("单击文件：链设为它的父文件夹，选中它，列表展开那一行；伪块 / 其他只选中", async () => {
    await store.mapClick({ kind: "leaf", key: "C:\\Users\\king\\下载\\a.iso", parent: "C:\\Users\\king\\下载", row: "C:\\Users\\king\\下载\\a.isoFile" }, { at: 1000, detail: 1 });
    expect(paths()).toEqual(["Users", "king", "下载"]);
    expect(store.selKey.value).toBe("C:\\Users\\king\\下载\\a.iso");
    expect(store.openRow.value).toBe("C:\\Users\\king\\下载\\a.isoFile");
    // 父文件夹已经是当前展开的：链不动
    await store.mapClick({ kind: "leaf", key: "C:\\Users\\king\\下载|其他", parent: "C:\\Users\\king\\下载", row: null }, { at: 2000, detail: 1 });
    expect(paths()).toEqual(["Users", "king", "下载"]);
    expect(store.selKey.value).toBe("C:\\Users\\king\\下载|其他");
    expect(store.openRow.value).toBeNull();
    // 盘根上的伪块：全部收起，选中它
    await store.mapClick({ kind: "leaf", key: "pseudo:sysest", parent: "C:\\", row: null }, { at: 3000, detail: 1 });
    expect(store.chain.value).toEqual([]);
    expect(store.selKey.value).toBe("pseudo:sysest");
  });

  it("双击防抖（鼠标）：detail ≥ 2 的第二下落在刚展开的那块或它的祖先 / 后代上就忽略；点别的块照常处理", async () => {
    await store.mapClick({ kind: "head", path: "C:\\Windows", outer: "C:\\Windows" }, { at: 1000, detail: 1 });
    // 第二下落在刚放大的 Windows 标题栏上：本来会收起
    await store.mapClick({ kind: "head", path: "C:\\Windows", outer: "C:\\Windows" }, { at: 1450, detail: 2 });
    expect(paths()).toEqual(["Windows"]);
    // 第二下落在 Windows 里面的块上（系统判定的双击，间隔多长都跟系统设置走）：也不会连挖进去；三连击同样
    await store.mapClick({ kind: "dir", path: "C:\\Windows\\System32" }, { at: 1460, detail: 2 });
    await store.mapClick({ kind: "dir", path: "C:\\Windows\\System32" }, { at: 1700, detail: 3 });
    expect(paths()).toEqual(["Windows"]);
    // 普通的单击（detail = 1）照常
    await store.mapClick({ kind: "dir", path: "C:\\Windows\\System32" }, { at: 1750, detail: 1 });
    expect(paths()).toEqual(["Windows", "System32"]);
    // 快速点别的块（不在刚展开那块的祖先 / 后代上）：照常处理
    await store.mapClick({ kind: "dir", path: "C:\\Users\\king" }, { at: 1800, detail: 2 });
    expect(paths()).toEqual(["Users", "king"]);
  });

  it("双击防抖（键盘 Enter / Space，detail = 0）：300 ms 内的第二下忽略，之后正常", async () => {
    await store.mapClick({ kind: "dir", path: "C:\\Windows" }, { at: 5000, detail: 0 });
    await store.mapClick({ kind: "dir", path: "C:\\Windows\\System32" }, { at: 5200, detail: 0 });
    expect(paths()).toEqual(["Windows"]);
    await store.mapClick({ kind: "dir", path: "C:\\Windows\\System32" }, { at: 5400, detail: 0 });
    expect(paths()).toEqual(["Windows", "System32"]);
  });

  it("单击当前展开块（pk）自己的空白处：什么都不做（不重取列表、不收起列表里展开的行）", async () => {
    await store.mapClick({ kind: "leaf", key: "C:\\Windows\\System32\\a.dll", parent: "C:\\Windows\\System32", row: "C:\\Windows\\System32\\a.dllFile" }, { at: 1000, detail: 1 });
    expect(store.openRow.value).toBe("C:\\Windows\\System32\\a.dllFile");
    const focusBefore = store.focus.value;
    vi.mocked(api.listDir).mockClear();
    await store.mapClick({ kind: "dir", path: "C:\\Windows\\System32" }, { at: 3000, detail: 1 });
    expect(paths()).toEqual(["Windows", "System32"]);
    expect(store.openRow.value).toBe("C:\\Windows\\System32\\a.dllFile");
    expect(store.focus.value).toBe(focusBefore);
    expect(api.listDir).not.toHaveBeenCalled();
  });

  it("单击「其他」：记下它的内容签名；clearSel 取消选中", async () => {
    await store.mapClick({ kind: "leaf", key: "C:\\Windows|其他", parent: "C:\\Windows", row: null, sig: "3|30" }, { at: 1000, detail: 1 });
    expect(paths()).toEqual(["Windows"]);
    expect(store.selKey.value).toBe("C:\\Windows|其他");
    expect(store.selSig.value).toBe("3|30");
    store.clearSel();
    expect(store.selKey.value).toBeNull();
    expect(store.selSig.value).toBeNull();
  });
});

describe("数据：链上每一级都按它自己做根取一次（M3）", () => {
  it("盘根的数据里 Windows 下面已经够两层，也照样取 Windows 自己的子树（每个文件夹只取一次）", async () => {
    await store.digTo("C:\\Windows");
    expect(getTreemap).toHaveBeenCalledWith("C:\\Windows", 3);
    getTreemap.mockClear();
    await store.digTo("C:\\Windows\\System32");
    await store.collapseAll();
    await store.digTo("C:\\Windows\\System32");
    // Windows 不再取；System32 只取一次
    expect(getTreemap.mock.calls.map((c) => c[0])).toEqual(["C:\\Windows\\System32"]);
  });
  it("跳着挖（一次延长好几级）：中间每一级也按自己取，和一级一级点下去是同一张图", async () => {
    await store.digTo("C:\\Users\\king\\AppData\\Local");
    const direct = JSON.stringify(store.tm.value);
    await store.collapseAll();
    await store.digTo("C:\\Users");
    await store.digTo("C:\\Users\\king");
    await store.digTo("C:\\Users\\king\\AppData");
    await store.digTo("C:\\Users\\king\\AppData\\Local");
    expect(JSON.stringify(store.tm.value)).toBe(direct);
  });
  it("补出来的节点：清理之后重新读，链还在、节点照样补上", async () => {
    await store.digTo("C:\\Windows\\WinSxS\\w7");
    await store.reloadViews();
    expect(paths()).toEqual(["Windows", "WinSxS", "w7"]);
    const winsxs = store.tm.value!.children.find((c) => c.name === "Windows")!.children.find((c) => c.name === "WinSxS")!;
    expect(winsxs.children.some((c) => c.name === "w7")).toBe(true);
  });
});

describe("面包屑 / 列表标题立刻跟上（M8）", () => {
  it("链一变，pkInfo 立刻是新文件夹的名字和大小（列表数据还没回来）", async () => {
    let open!: () => void;
    gates.set("c:\\windows\\system32", new Promise<void>((r) => (open = r)));
    await store.digTo("C:\\Windows");
    const p = store.digTo("C:\\Windows\\System32");
    // 链变了（System32 自己的数据很快就回来；没回来最多等 90 ms）
    await vi.waitFor(() => expect(store.chain.value).toHaveLength(2));
    expect(store.pkInfo.value).toMatchObject({ name: "System32", size: 21 });
    // 列表还是 Windows 的
    expect(store.focus.value?.node.path).toBe("C:\\Windows");
    open();
    await p;
    expect(store.focus.value?.node.path).toBe("C:\\Windows\\System32");
    await store.collapseAll();
    expect(store.pkInfo.value).toMatchObject({ name: "C:", size: ROOT.size });
  });
});

describe("深链", () => {
  it("链很深时方块图数据仍是整棵（从扫描根开始，自动缩放由方块图按像素决定），面包屑从盘根开始", async () => {
    await store.digTo("C:\\Users\\king\\AppData\\Local\\Temp");
    expect(store.tm.value?.path).toBe("C:\\");
    expect(store.crumbs.value[0].path).toBe("C:\\");
    expect(store.crumbs.value).toHaveLength(6);
  });
});

describe("reloadViews：清理之后", () => {
  it("清缓存重取；链上不存在了的路径截掉", async () => {
    await store.digTo("C:\\Users\\king\\AppData\\Local\\Temp");
    // 清理把 Local 整个删掉了
    const appdata = find("C:\\Users\\king\\AppData")!;
    appdata.children = appdata.children.filter((c) => c.name !== "Local");
    getTreemap.mockClear();
    await store.reloadViews();
    expect(paths()).toEqual(["Users", "king", "AppData"]);
    expect(store.focus.value?.node.path).toBe("C:\\Users\\king\\AppData");
    expect(getTreemap).toHaveBeenCalledWith("C:\\", 3);
  });
  it("链上都还在：链不变，数据是新的", async () => {
    await store.digTo("C:\\Windows\\System32");
    find("C:\\Windows\\System32")!.children.push(F("C:\\Windows\\System32\\new.dll", 1));
    await store.reloadViews();
    expect(paths()).toEqual(["Windows", "System32"]);
    expect(store.focus.value?.children.map((c) => c.name)).toContain("new.dll");
  });
});
