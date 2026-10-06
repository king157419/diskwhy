// 空间分析的选中逻辑（阶段 J §3.9）：盘根 / Users 这些层也能选中并列在右边。后端命令用假的。
import { beforeEach, describe, expect, it, vi } from "vitest";

type N = { name: string; path: string; kind: string; allocated: number; children: N[] };
const dir = (path: string, allocated: number, children: N[] = []): N => ({
  name: path.split("\\").filter(Boolean).pop() ?? path,
  path,
  kind: "Dir",
  allocated,
  children,
});
const king = dir("C:\\Users\\king", 97, [dir("C:\\Users\\king\\AppData", 60, [dir("C:\\Users\\king\\AppData\\Local", 50)])]);
const TREE = dir("C:\\", 185, [dir("C:\\Users", 98, [king, dir("C:\\Users\\Public", 1)]), dir("C:\\Windows", 31, [dir("C:\\Windows\\WinSxS", 10)])]);
function find(n: N, p: string): N | null {
  if (n.path.toLowerCase() === p.toLowerCase()) return n;
  for (const c of n.children) {
    const r = find(c, p);
    if (r) return r;
  }
  return null;
}

vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  listDir: vi.fn(async (path: string) => {
    const n = find(TREE, path)!;
    return { node: { ...n, children: undefined }, children: n.children, total_children: n.children.length, breadcrumb: [{ name: "C:\\", path: "C:\\" }] };
  }),
  getTreemap: vi.fn(async (path: string) => find(TREE, path)!),
}));

const store = await import("./store");

beforeEach(() => {
  store.listing.value = null;
  store.focus.value = null;
  store.focusKey.value = null;
});

describe("openDir / setFocus 选中", () => {
  it("第一次进入：自动下钻到最大子文件夹（Users 外壳合并到 king）", async () => {
    await store.openDir("C:\\");
    expect(store.listing.value?.node.path).toBe("C:\\");
    expect(store.focusKey.value).toBe("C:\\Users\\king");
  });

  it("之后再打开盘根（面包屑第 0 级）：右边列的就是盘根，不再自动下钻", async () => {
    await store.openDir("C:\\");
    await store.openDir("C:\\", "C:\\");
    expect(store.focus.value?.node.path).toBe("C:\\");
    expect(store.focusKey.value).toBeNull();
    // 没给 focusPath 也一样（比如动作之后重新读视图）
    await store.openDir("C:\\Windows");
    expect(store.focus.value?.node.path).toBe("C:\\Windows");
    expect(store.focusKey.value).toBeNull();
  });

  it("Users 能被选中并列在右边", async () => {
    await store.openDir("C:\\");
    await store.setFocus("C:\\Users");
    expect(store.focusKey.value).toBe("C:\\Users");
    expect(store.focus.value?.children.map((c) => c.name)).toEqual(["king", "Public"]);
  });

  it("setFocus(方块图的根) → 列根自己、不框块（大小写 / 末尾反斜杠不敏感）", async () => {
    await store.openDir("C:\\Users", "C:\\Users\\king");
    await store.setFocus("c:\\users\\");
    expect(store.focus.value?.node.path).toBe("C:\\Users");
    expect(store.focusKey.value).toBeNull();
  });

  it("双击进入一块：它成为方块图的根，右边也列它", async () => {
    await store.openDir("C:\\");
    await store.enterBlock("C:\\Windows");
    expect(store.listing.value?.node.path).toBe("C:\\Windows");
    expect(store.focus.value?.node.path).toBe("C:\\Windows");
    expect(store.focusKey.value).toBeNull();
  });
});
