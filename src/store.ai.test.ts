// 阶段 M：收到 AI 的答案时补进**所有**缓存着的视图对象（大小写不敏感）——盘根的列表、右边正在看的列表（行 + 标题那一项）、
// 方块图的显示树、盘根的数据、按文件夹缓存的子树（graft）、补节点（stub）——不重新取数据，界面（响应式）马上跟上。
// 还有：进度事件 / 刚做完的那一闪 / 重试 / 手动问 AI。后端命令和事件都是假的。
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { computed } from "vue";
import type { AiNote, AiStatus, DirListing, NodeView, TreemapNode } from "./api";

type R = { name: string; path: string; kind: "Dir" | "File"; size: number; unknown: boolean; children: R[] };
const MAX_KIDS = 5;
function D(path: string, children: R[], unknown = false): R {
  return { name: path.split("\\").filter(Boolean).pop() ?? path, path, kind: "Dir", size: children.reduce((s, c) => s + c.size, 0), unknown, children: children.sort((a, b) => b.size - a.size) };
}
const F = (path: string, size: number, unknown = false): R => ({ name: path.split("\\").pop()!, path, kind: "File", size, unknown, children: [] });
let ROOT: R;
function build() {
  const U = "C:\\Users\\king\\AppData";
  const temp = D(U + "\\Local\\Temp", [D(U + "\\Local\\Temp\\t1", [F(U + "\\Local\\Temp\\t1\\f", 20, true)], true), F(U + "\\Local\\Temp\\x.bin", 10, true)], true);
  const local = D(U + "\\Local", [temp, D(U + "\\Local\\Google", [F(U + "\\Local\\Google\\g", 20)])], true);
  const appdata = D(U, [local, D(U + "\\Roaming", [F(U + "\\Roaming\\r", 10, true)], true)], true);
  const king = D("C:\\Users\\king", [appdata, D("C:\\Users\\king\\下载", [F("C:\\Users\\king\\下载\\a.iso", 30)]), F("C:\\Users\\king\\x.dat", 7)]);
  const users = D("C:\\Users", [king, D("C:\\Users\\Public", [F("C:\\Users\\Public\\p", 1)])]);
  const winsxs = D("C:\\Windows\\WinSxS", Array.from({ length: 8 }, (_, i) => D(`C:\\Windows\\WinSxS\\w${i}`, [F(`C:\\Windows\\WinSxS\\w${i}\\f`, 10 - i, true)], true)));
  const win = D("C:\\Windows", [winsxs, D("C:\\Windows\\System32", [F("C:\\Windows\\System32\\a.dll", 21)])]);
  ROOT = D("C:\\", [users, win, D("C:\\Mystery", [F("C:\\Mystery\\m.bin", 9, true)], true), F("C:\\hiberfil.sys", 12)]);
  ROOT.name = "C:\\";
}
function find(p: string): R | null {
  const want = p.replace(/\\+$/, "").toLowerCase();
  const walk = (n: R): R | null => (n.path.replace(/\\+$/, "").toLowerCase() === want ? n : n.children.reduce<R | null>((a, c) => a ?? walk(c), null));
  return walk(ROOT);
}
function tm(n: R, level: number, depth: number): TreemapNode {
  const me = {
    name: n.name,
    path: n.path,
    kind: n.kind,
    allocated: n.size,
    tier: n.unknown ? "unknown" : "keep",
    what: "",
    owner: null,
    lose_kind: null,
    category: n.unknown ? "unknown" : "system",
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
const view = (n: R): NodeView =>
  ({
    name: n.name,
    path: n.path,
    kind: n.kind,
    allocated: n.size,
    child_count: n.children.length,
    explanation: {
      tier: n.unknown ? "review" : "keep",
      source: n.unknown ? { kind: "unknown" } : { kind: "rule", id: "r", group: "g", inherited_from: null },
      what: n.unknown ? "" : "系统文件",
      lose: n.unknown ? "说不准" : "别动",
      clues: [],
      how: [],
    },
  }) as unknown as NodeView;

const note = (what: string): AiNote => ({ what, owner: "", why: "", lose: "删了会怎样：" + what, confidence: 0.6, model: "glm-4.7-flash", at: "2026-10-11", size: 1 });
const IDLE: AiStatus = { running: false, done: 0, total: 0, failed: 0, error: null };
const backend = { status: { ...IDLE } as AiStatus, ask: note("手动问到的") };
/** 假的事件总线：listen 记下处理函数，测试里直接调 */
const handlers = new Map<string, (e: { payload: unknown }) => void>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, h: (e: { payload: unknown }) => void) => {
    handlers.set(name, h);
    return () => handlers.delete(name);
  }),
}));
vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  listDir: vi.fn(async (path: string): Promise<DirListing> => {
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
  aiStatus: vi.fn(async () => ({ ...backend.status })),
  aiRunNow: vi.fn(async () => undefined),
  askAi: vi.fn(async () => backend.ask),
}));

const api = await import("./api");
const store = await import("./store");
const { pathKey, findTm } = await import("./tmtree");
const getTreemap = vi.mocked(api.getTreemap);

const A = "C:\\Users\\king\\AppData";
const tmAt = (p: string) => findTm(store.tm.value, p);
/** 文件（findTm 只顺着文件夹走）：在父文件夹的子项里找 */
const tmFile = (p: string) => findTm(store.tm.value, p.slice(0, p.lastIndexOf("\\")))?.children.find((c) => pathKey(c.path) === pathKey(p)) ?? null;

beforeEach(async () => {
  build();
  store.prefetchCfg.idle = false;
  store.summary.value = { root: "C:\\", buckets: {} } as never;
  store.listing.value = null;
  await store.goExplore();
  getTreemap.mockClear();
});

describe("列表：行 + 标题那一项，大小写不敏感，界面马上跟上", () => {
  it("右边正在看的列表：行和标题都补上（computed 跟着变 = 响应式）", async () => {
    await store.digTo(A);
    const row = computed(() => store.focus.value?.children.find((c) => c.name === "Local")?.explanation?.ai?.what);
    const head = computed(() => store.focus.value?.node.explanation?.ai?.what);
    expect(row.value).toBeUndefined();
    store.applyAiAnswer("c:\\USERS\\king\\appdata\\LOCAL", note("本机程序数据"));
    expect(row.value).toBe("本机程序数据");
    store.applyAiAnswer(A.toUpperCase() + "\\", note("用户的程序数据"));
    expect(head.value).toBe("用户的程序数据");
  });

  it("盘根的列表（右边看的是别的文件夹时也补）", async () => {
    await store.digTo(A);
    store.applyAiAnswer("c:\\mystery", note("某个程序的数据"));
    expect(store.listing.value?.children.find((c) => c.name === "Mystery")?.explanation?.ai?.what).toBe("某个程序的数据");
    // 回到盘根：右边列表就是它
    await store.digTo(null);
    expect(store.focus.value?.children.find((c) => c.name === "Mystery")?.explanation?.ai?.what).toBe("某个程序的数据");
  });

  it("认得出来的不补（AI 不改任何认得出来的东西）", async () => {
    store.applyAiAnswer("C:\\Windows", note("不该出现"));
    expect(store.listing.value?.children.find((c) => c.name === "Windows")?.explanation?.ai).toBeUndefined();
    expect(tmAt("C:\\Windows")?.ai).toBeUndefined();
  });
});

describe("方块图：显示树、盘根的数据、按文件夹缓存的子树、补节点", () => {
  it("显示树里的块马上有；收起再挖开（从缓存重拼，不重新取）还在", async () => {
    await store.digTo(A);
    // Temp 只在 AppData 自己取回来的子树里（盘根只取了 3 层）
    store.applyAiAnswer("C:\\USERS\\KING\\APPDATA\\LOCAL\\TEMP", note("临时文件"));
    store.applyAiAnswer("c:\\users\\king\\appdata\\local\\temp\\X.BIN", note("一个大文件"));
    expect(tmAt(A + "\\Local\\Temp")?.ai?.what).toBe("临时文件");
    expect(tmFile(A + "\\Local\\Temp\\x.bin")?.ai?.what).toBe("一个大文件");
    getTreemap.mockClear();
    await store.digTo(null);
    await store.digTo(A);
    expect(getTreemap).not.toHaveBeenCalled();
    expect(tmAt(A + "\\Local\\Temp")?.ai?.what).toBe("临时文件");
    expect(tmFile(A + "\\Local\\Temp\\x.bin")?.ai?.what).toBe("一个大文件");
  });

  it("不在当前链上的缓存子树（收起以后 / 预取的）也补上：再挖开时就有", async () => {
    await store.digTo(A);
    await store.digTo(null);
    // 这时显示树里没有 Local / Temp，只有 AppData 自己取回来的子树（缓存）里有
    expect(tmAt(A + "\\Local")).toBeNull();
    store.applyAiAnswer("c:\\users\\king\\appdata\\local\\TEMP", note("临时文件"));
    store.applyAiAnswer("C:\\USERS\\KING\\APPDATA\\Local", note("本机程序数据"));
    store.applyAiAnswer("C:\\Users\\King\\AppData", note("用户的程序数据"));
    getTreemap.mockClear();
    await store.digTo(A);
    expect(getTreemap).not.toHaveBeenCalled();
    expect(tmAt(A + "\\Local\\Temp")?.ai?.what).toBe("临时文件");
    expect(tmAt(A + "\\Local")?.ai?.what).toBe("本机程序数据");
    expect(tmAt(A)?.ai?.what).toBe("用户的程序数据");
  });

  it("盘根的数据（链收起后显示树就是它）和链上的拼接副本都补上", async () => {
    await store.digTo(A + "\\Local");
    store.applyAiAnswer(A.toLowerCase(), note("用户的程序数据"));
    // 链上：AppData 是拼接出来的副本
    expect(tmAt(A)?.ai?.what).toBe("用户的程序数据");
    await store.digTo(null);
    expect(store.chain.value).toEqual([]);
    expect(tmAt(A)?.ai?.what).toBe("用户的程序数据");
    // 再挖开：拼接副本从补过的数据复制出来
    await store.digTo(A + "\\Local\\Temp");
    expect(tmAt(A)?.ai?.what).toBe("用户的程序数据");
  });

  it("补节点（父级数据里没有、用列表补的）：补上，重拼以后还在", async () => {
    await store.digTo("C:\\Windows\\WinSxS\\w7");
    expect(tmAt("C:\\Windows\\WinSxS\\w7")?.ai).toBeUndefined();
    store.applyAiAnswer("c:\\windows\\winsxs\\W7", note("组件存储的一份备份"));
    expect(tmAt("C:\\Windows\\WinSxS\\w7")?.ai?.what).toBe("组件存储的一份备份");
    getTreemap.mockClear();
    await store.digTo("C:\\Windows");
    await store.digTo("C:\\Windows\\WinSxS\\w7");
    expect(getTreemap).not.toHaveBeenCalled();
    expect(tmAt("C:\\Windows\\WinSxS\\w7")?.ai?.what).toBe("组件存储的一份备份");
  });

  it("收到过的推测记在 aiNotes（小写键）：数据里还没带上时显示用它补", () => {
    store.applyAiAnswer("C:\\Not\\Fetched\\Yet", note("还没取到的"));
    expect(store.aiNotes.value.get("c:\\not\\fetched\\yet")?.what).toBe("还没取到的");
  });
});

describe("事件 / 进度 / 重试 / 手动问 AI", () => {
  afterEach(() => {
    vi.useRealTimers();
    store.applyAiStatus({ ...IDLE });
  });

  it("listenAi：先取一次当前进度（界面重载后重新附着），之后跟着 ai-status / ai-answer 事件", async () => {
    backend.status = { running: true, done: 1, total: 5, failed: 0, error: null };
    await store.listenAi();
    expect(store.aiStatus.value).toMatchObject({ running: true, done: 1, total: 5 });
    handlers.get(api.AI_STATUS_EVENT)!({ payload: { running: true, done: 2, total: 5, failed: 0, error: null } });
    expect(store.aiStatus.value.done).toBe(2);
    await store.digTo(A);
    handlers.get(api.AI_ANSWER_EVENT)!({ payload: { path: "C:\\users\\king\\appdata\\roaming", note: note("跟着账号走的设置") } });
    expect(store.focus.value?.children.find((c) => c.name === "Roaming")?.explanation?.ai?.what).toBe("跟着账号走的设置");
    expect(tmAt(A + "\\Roaming")?.ai?.what).toBe("跟着账号走的设置");
  });

  it("全部问完闪一下问到的个数，约 3 秒后消失；出错 / 被打断不闪", () => {
    vi.useFakeTimers();
    store.applyAiStatus({ running: true, done: 0, total: 4, failed: 0, error: null });
    store.applyAiStatus({ running: false, done: 4, total: 4, failed: 1, error: null });
    expect(store.aiFlash.value).toBe(3);
    vi.advanceTimersByTime(store.AI_FLASH_MS - 10);
    expect(store.aiFlash.value).toBe(3);
    vi.advanceTimersByTime(20);
    expect(store.aiFlash.value).toBeNull();
    store.applyAiStatus({ running: true, done: 0, total: 4, failed: 0, error: null });
    store.applyAiStatus({ running: false, done: 2, total: 4, failed: 0, error: "密钥不对" });
    expect(store.aiFlash.value).toBeNull();
    expect(store.aiStatus.value.error).toBe("密钥不对");
  });

  it("重试：错误先收起，让后端重新开始", async () => {
    store.applyAiStatus({ running: false, done: 2, total: 4, failed: 0, error: "密钥不对" });
    await store.aiRetry();
    expect(api.aiRunNow).toHaveBeenCalled();
    expect(store.aiStatus.value.error).toBeNull();
  });

  it("新的扫描开始：小标签收起", async () => {
    store.applyAiStatus({ running: true, done: 2, total: 4, failed: 0, error: null });
    store.scanRoot.value = "C:\\";
    const p = store.doScan().catch(() => {});
    expect(store.aiStatus.value.running).toBe(false);
    await p;
  });

  it("手动问 AI：答案同样补进列表和方块图", async () => {
    await store.digTo(A);
    const n = await store.askAiNow(A + "\\Local");
    expect(n.what).toBe("手动问到的");
    expect(store.focus.value?.children.find((c) => c.name === "Local")?.explanation?.ai?.what).toBe("手动问到的");
    expect(tmAt(A + "\\Local")?.ai?.what).toBe("手动问到的");
  });
});
