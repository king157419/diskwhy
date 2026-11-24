// 阶段 M：自动问 AI 的纯函数——补缓存（大小写不敏感、只补认不出来的）、取推测、工具栏小标签的状态。
import { describe, expect, it } from "vitest";
import type { AiNote, AiStatus, DirListing, Explanation, NodeView, TreemapNode } from "./api";
import {
  AI_LOSE_FALLBACK,
  aiChip,
  aiLose,
  finishedOk,
  hostOf,
  keyOptional,
  llmUsable,
  missingKey,
  noteFor,
  patchListing,
  patchTree,
  shellNoteFor,
  splitErr,
  tmNoteFor,
  usableNote,
  withoutAskAi,
} from "./ai";

const note = (what = "微信的聊天文件", lose = "会丢聊天里的图片和文件"): AiNote => ({
  what,
  owner: "微信",
  why: "",
  lose,
  confidence: 0.7,
  model: "glm-4.7-flash",
  at: "2026-10-11",
  size: 1024,
});

const expl = (unknown: boolean): Explanation =>
  ({
    tier: unknown ? "review" : "safe",
    owner: null,
    category: unknown ? "unknown" : "cache",
    what: unknown ? "" : "缓存",
    why: "",
    lose: unknown ? "说不准" : "会自动重建",
    lose_kind: null,
    how: [],
    how_detail: "",
    gain_bytes: 0,
    gain_note: "",
    source: unknown ? { kind: "unknown" } : { kind: "rule", id: "r", group: "g", inherited_from: null },
    confidence: 1,
    refs: [],
    verified_on: null,
    notes: [],
    clues: [],
  }) as Explanation;

const view = (path: string, kind: NodeView["kind"] = "Dir", unknown = true, extra: Partial<NodeView> = {}): NodeView =>
  ({ name: path.split("\\").pop(), path, kind, allocated: 1, explanation: expl(unknown), ...extra }) as NodeView;

const tmn = (path: string, kind: TreemapNode["kind"], tier: TreemapNode["tier"], children: TreemapNode[] = []): TreemapNode =>
  ({ name: path.split("\\").pop() || path, path, kind, allocated: 1, tier, children, hidden_bytes: 0, hidden_count: 0 }) as TreemapNode;

describe("noteFor / tmNoteFor：只有认不出来的才有推测", () => {
  it("数据里带着的推测", () => {
    const v = view("C:\\Users\\king\\xwechat_files");
    v.explanation!.ai = note();
    expect(noteFor(v)?.what).toBe("微信的聊天文件");
  });
  it("数据里没有时用收到过的（pathKey：大小写 / 末尾反斜杠不敏感）", () => {
    const v = view("C:\\Users\\King\\XWechat_Files\\");
    const m = new Map([["c:\\users\\king\\xwechat_files", note()]]);
    expect(noteFor(v, m)?.owner).toBe("微信");
  });
  it("认得出来 / 其余小文件 / 伪块 / 没有解释 → null", () => {
    const m = new Map([["c:\\x", note()]]);
    expect(noteFor(view("C:\\x", "Dir", false), m)).toBeNull();
    expect(noteFor(view("C:\\x", "Rest"), m)).toBeNull();
    expect(noteFor(view("C:\\x", "Dir", true, { pseudo: "inacc" }), m)).toBeNull();
    expect(noteFor({ path: "C:\\x", kind: "Dir" }, m)).toBeNull();
    expect(noteFor(null, m)).toBeNull();
  });
  it("没写「这是什么」的推测不算", () => {
    const v = view("C:\\x");
    v.explanation!.ai = note("   ");
    expect(noteFor(v)).toBeNull();
    expect(usableNote(note(""))).toBe(false);
  });
  it("方块图：tier = unknown 才算，「其余小文件」不算", () => {
    const m = new Map([["c:\\x", note()]]);
    expect(tmNoteFor(tmn("C:\\X", "Dir", "unknown"), m)?.what).toBe("微信的聊天文件");
    expect(tmNoteFor(tmn("C:\\X", "Dir", "keep"), m)).toBeNull();
    expect(tmNoteFor(tmn("C:\\X", "Rest", "unknown"), m)).toBeNull();
    const own = tmn("C:\\y", "File", "unknown");
    own.ai = note("自己带的");
    expect(tmNoteFor(own)?.what).toBe("自己带的");
  });
  it("删了会怎样：推测没写就用中性的兜底（不退回叫人去点「问 AI」的那句）", () => {
    expect(aiLose(note("x", "  "))).toBe(AI_LOSE_FALLBACK);
    expect(AI_LOSE_FALLBACK).not.toContain("问");
    expect(aiLose(note("x", "会丢设置"))).toBe("会丢设置");
  });
});

describe("patchListing：标题那一项 + 每一行，大小写不敏感", () => {
  const listing = (): DirListing => ({
    node: view("C:\\Users\\king\\AppData"),
    children: [view("C:\\Users\\king\\AppData\\Local"), view("C:\\Users\\king\\AppData\\Roaming", "Dir", false), view("C:\\Users\\king\\AppData", "Rest")],
    total_children: 3,
    breadcrumb: [],
  });
  it("行", () => {
    const l = listing();
    expect(patchListing(l, "c:\\users\\king\\appdata\\local", note())).toBe(1);
    expect(l.children[0].explanation?.ai?.what).toBe("微信的聊天文件");
    expect(l.node.explanation?.ai).toBeUndefined();
  });
  it("标题那一项（其余小文件的 path 是父文件夹，不补）", () => {
    const l = listing();
    expect(patchListing(l, "c:\\users\\king\\appdata", note())).toBe(1);
    expect(l.node.explanation?.ai).toBeDefined();
    expect(l.children[2].explanation?.ai).toBeUndefined();
  });
  it("认得出来的不补；同一个推测补两次只算一次；null 列表", () => {
    const l = listing();
    expect(patchListing(l, "c:\\users\\king\\appdata\\roaming", note())).toBe(0);
    expect(l.children[1].explanation?.ai).toBeUndefined();
    const n = note();
    expect(patchListing(l, "c:\\users\\king\\appdata\\local", n)).toBe(1);
    expect(patchListing(l, "c:\\users\\king\\appdata\\local", { ...n })).toBe(0);
    expect(patchListing(l, "c:\\users\\king\\appdata\\local", note("换了说法"))).toBe(1);
    expect(patchListing(null, "c:\\x", n)).toBe(0);
  });
});

describe("patchTree：顺着路径前缀找到那一块", () => {
  const tree = () =>
    tmn("C:\\", "Dir", "keep", [
      tmn("C:\\Users", "Dir", "keep", [
        tmn("C:\\Users\\king", "Dir", "unknown", [
          tmn("C:\\Users\\king\\big.bin", "File", "unknown"),
          tmn("C:\\Users\\king", "Rest", "unknown"),
          tmn("C:\\Users\\king\\docs", "Dir", "keep"),
        ]),
        tmn("C:\\Users\\kingdom", "Dir", "unknown"),
      ]),
      tmn("C:\\hiberfil.sys", "File", "review"),
    ]);
  it("深处的文件夹 / 文件（大小写不敏感）", () => {
    const t = tree();
    expect(patchTree(t, "c:\\users\\king", note())).toBe(1);
    expect(t.children[0].children[0].ai).toBeDefined();
    // 「其余小文件」和父文件夹同一个 path：不补
    expect(t.children[0].children[0].children[1].ai).toBeUndefined();
    expect(patchTree(t, "c:\\users\\king\\big.bin", note("大文件"))).toBe(1);
    expect(t.children[0].children[0].children[0].ai?.what).toBe("大文件");
  });
  it("前缀相同的兄弟（king / kingdom）不会认错", () => {
    const t = tree();
    expect(patchTree(t, "c:\\users\\kingdom", note())).toBe(1);
    expect(t.children[0].children[1].ai).toBeDefined();
    expect(t.children[0].children[0].ai).toBeUndefined();
  });
  it("子树的根自己（按文件夹缓存的子树）", () => {
    const sub = tmn("C:\\Users\\king", "Dir", "unknown", [tmn("C:\\Users\\king\\big.bin", "File", "unknown")]);
    expect(patchTree(sub, "c:\\users\\king", note())).toBe(1);
    expect(sub.ai).toBeDefined();
  });
  it("认得出来的 / 树里没有的 / 不在这棵子树下的 → 0", () => {
    const t = tree();
    expect(patchTree(t, "c:\\users\\king\\docs", note())).toBe(0);
    expect(patchTree(t, "c:\\hiberfil.sys", note())).toBe(0);
    expect(patchTree(t, "c:\\nope\\x", note())).toBe(0);
    expect(patchTree(tmn("D:\\data", "Dir", "unknown"), "c:\\users\\king", note())).toBe(0);
    expect(patchTree(null, "c:\\x", note())).toBe(0);
  });
});

describe("工具栏小标签", () => {
  const st = (o: Partial<AiStatus>): AiStatus => ({ running: false, done: 0, total: 0, failed: 0, error: null, ...o });
  it("进行中：AI 3/12（还没算出要问几个时不显示）", () => {
    expect(aiChip(st({ running: true, done: 3, total: 12 }), null)).toEqual({ kind: "run", done: 3, total: 12 });
    expect(aiChip(st({ running: true, done: 14, total: 12 }), null)).toEqual({ kind: "run", done: 12, total: 12 });
    expect(aiChip(st({ running: true, total: 0 }), null)).toBeNull();
  });
  it("出错：一句短话；刚做完：问到几个；平时不显示", () => {
    expect(aiChip(st({ error: "密钥不对", done: 2, total: 12 }), null)).toEqual({ kind: "err", message: "密钥不对" });
    expect(aiChip(st({ done: 12, total: 12 }), 11)).toEqual({ kind: "done", ok: 11 });
    expect(aiChip(st({ done: 12, total: 12 }), null)).toBeNull();
    expect(aiChip(st({}), 0)).toBeNull();
    expect(aiChip(null, 3)).toBeNull();
  });
  it("finishedOk：全部问完才闪（出错 / 被打断 / 本来没在跑都不闪）", () => {
    const run = st({ running: true, done: 11, total: 12 });
    expect(finishedOk(run, st({ done: 12, total: 12, failed: 1 }))).toBe(11);
    expect(finishedOk(run, st({ done: 2, total: 12, error: "密钥不对" }))).toBeNull();
    expect(finishedOk(run, st({ done: 5, total: 12 }))).toBeNull();
    expect(finishedOk(st({ done: 12, total: 12 }), st({ done: 12, total: 12 }))).toBeNull();
    expect(finishedOk(run, st({ done: 2, total: 2, failed: 2 }))).toBeNull();
    expect(finishedOk(run, st({ running: true, done: 12, total: 12 }))).toBeNull();
  });
});

describe("splitErr（测试连接的报错：屏幕上只写短话，细节悬停看）", () => {
  it("「短中文（细节）」拆开", () => {
    expect(splitErr('密钥不对（HTTP 401：{"error":{"code":"1000","message":"身份验证失败。"}}）')).toEqual({
      short: "密钥不对",
      detail: 'HTTP 401：{"error":{"code":"1000","message":"身份验证失败。"}}',
    });
    expect(splitErr("连不上（http://127.0.0.1:1/v1/chat/completions：io: 由于目标计算机积极拒绝（10061））")).toEqual({
      short: "连不上",
      detail: "http://127.0.0.1:1/v1/chat/completions：io: 由于目标计算机积极拒绝（10061）",
    });
  });
  it("没有括号 / 括号在开头 / 不是结尾：原样", () => {
    expect(splitErr("请先填写地址和模型名")).toEqual({ short: "请先填写地址和模型名", detail: "" });
    expect(splitErr("（奇怪）")).toEqual({ short: "（奇怪）", detail: "" });
    expect(splitErr("测试线程异常：a（b）c")).toEqual({ short: "测试线程异常：a（b）c", detail: "" });
  });
});

describe("withoutAskAi（有推测时「建议」不再说「或者问 AI」）", () => {
  it("去掉结尾的「，或者问 AI」，句号留着", () => {
    expect(withoutAskAi("先弄清楚是哪个软件的：搜索文件夹名、看看里面的文件，或者问 AI。")).toBe("先弄清楚是哪个软件的：搜索文件夹名、看看里面的文件。");
  });
  it("「删了会怎样」结尾的「，或者点“问 AI”让 AI 猜一猜」也去掉（heuristics.rs 未知项的 lose 原文）", () => {
    expect(withoutAskAi("不知道。在弄清楚之前请不要删——可以根据下面的线索搜一下文件夹名，或者点“问 AI”让 AI 猜一猜。")).toBe(
      "不知道。在弄清楚之前请不要删——可以根据下面的线索搜一下文件夹名。",
    );
    expect(withoutAskAi("先看看，或者点「问 AI」。")).toBe("先看看。");
  });
  it("别的说法原样", () => {
    expect(withoutAskAi("确认项目不在运行，删掉整个 node_modules。")).toBe("确认项目不在运行，删掉整个 node_modules。");
    expect(withoutAskAi("")).toBe("");
    expect(withoutAskAi("问 AI 之前先看看。")).toBe("问 AI 之前先看看。");
    expect(withoutAskAi("或者问 AI 一下再说，别急着删。")).toBe("或者问 AI 一下再说，别急着删。");
  });
});

describe("哪些地址要密钥（和后端 key_optional / usable 一致）", () => {
  it("hostOf", () => {
    expect(hostOf("https://open.bigmodel.cn/api/paas/v4")).toBe("open.bigmodel.cn");
    expect(hostOf("http://127.0.0.1:11434/v1")).toBe("127.0.0.1");
    expect(hostOf("http://[::1]:11434/v1")).toBe("::1");
    expect(hostOf("https://user:pw@API.DeepSeek.com.")).toBe("api.deepseek.com");
    expect(hostOf("")).toBe("");
  });
  it("本机 / 局域网不用密钥，云服务要", () => {
    for (const u of ["http://127.0.0.1:11434/v1", "http://localhost:11434/v1", "http://[::1]:1/v1", "http://192.168.1.20:11434/v1", "http://10.0.0.5/v1", "http://172.20.1.1/v1", "http://ollama.local/v1", "http://gpu-box:8000/v1", "http://[fd00::5]:8000/v1"])
      expect(keyOptional(u), u).toBe(true);
    for (const u of ["https://open.bigmodel.cn/api/paas/v4", "https://dashscope.aliyuncs.com/compatible-mode/v1", "https://api.deepseek.com", "https://api.openai.com/v1", "http://8.8.8.8/v1", "http://172.32.0.1/v1"])
      expect(keyOptional(u), u).toBe(false);
  });
  it("llmUsable / missingKey", () => {
    const s = { enabled: true, base_url: "https://open.bigmodel.cn/api/paas/v4", model: "glm-4.7-flash", api_key: " " };
    expect(missingKey(s)).toBe(true);
    expect(llmUsable(s)).toBe(false);
    expect(llmUsable({ ...s, api_key: "abc" })).toBe(true);
    expect(llmUsable({ ...s, api_key: "abc", enabled: false })).toBe(false);
    expect(llmUsable({ ...s, base_url: "http://127.0.0.1:11434/v1", api_key: "" })).toBe(true);
    expect(llmUsable({ ...s, base_url: "http://127.0.0.1:11434/v1", model: " " })).toBe(false);
  });
});

describe("shellNoteFor（方块图外壳合并块：内层没问过时借链上问过的上层的推测）", () => {
  // C:\ → Local → Ollama（问过）→ updates_v2（套娃，没问）→ 66387…（套娃，没问）
  const build = () => {
    const deep = tmn("C:\\Local\\Ollama\\updates_v2\\66387", "Dir", "unknown");
    const upd = tmn("C:\\Local\\Ollama\\updates_v2", "Dir", "unknown", [deep]);
    const oll = tmn("C:\\Local\\Ollama", "Dir", "unknown", [upd]);
    const local = tmn("C:\\Local", "Dir", "unknown", [oll]);
    const root = tmn("C:\\", "Dir", "unknown", [local]);
    return { root, local, oll, upd, deep };
  };
  it("内层自己有推测：用内层的", () => {
    const { root, upd } = build();
    upd.ai = note("Ollama 的更新安装包");
    expect(shellNoteFor(root, "C:\\Local\\Ollama", upd)?.what).toBe("Ollama 的更新安装包");
  });
  it("内层没问过：用外层（合并块的 key）的；中间层有就用离内层最近的", () => {
    const { root, oll, upd, deep } = build();
    oll.ai = note("Ollama 的缓存和更新");
    expect(shellNoteFor(root, "C:\\Local\\Ollama", upd)?.what).toBe("Ollama 的缓存和更新");
    expect(shellNoteFor(root, "C:\\Local\\Ollama", deep)?.what).toBe("Ollama 的缓存和更新");
    // 收到过的（数据里还没带上）也算，大小写不敏感
    const m = new Map([["c:\\local\\ollama\\updates_v2", note("更新文件夹")]]);
    expect(shellNoteFor(root, "c:\\LOCAL\\ollama", deep, m)?.what).toBe("更新文件夹");
  });
  it("外层之外的上级不借；内层认得出来 / 不在外层下面 / 没有树 → 不借", () => {
    const { root, local, oll, upd } = build();
    local.ai = note("上上级");
    expect(shellNoteFor(root, "C:\\Local\\Ollama", upd)).toBeNull();
    oll.ai = note("外层");
    const known = { ...upd, tier: "review" } as TreemapNode;
    expect(shellNoteFor(root, "C:\\Local\\Ollama", known)).toBeNull();
    expect(shellNoteFor(root, "C:\\Other", upd)).toBeNull();
    expect(shellNoteFor(null, "C:\\Local\\Ollama", upd)).toBeNull();
    // 外层自己认得出来（不是 unknown）：它的推测不算
    oll.tier = "keep";
    expect(shellNoteFor(root, "C:\\Local\\Ollama", upd)).toBeNull();
  });
});
