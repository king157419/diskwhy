// 阶段 M：自动问 AI 的前端纯函数（src/ai.test.ts）。
// - 收到 ai-answer 时把缓存里**每一份**同路径（大小写不敏感）的视图对象都补上 ai：列表行 / 列表标题（DirListing）、
//   方块图数据（盘根、按文件夹缓存的子树、补节点、当前的显示树）——各个视图不用重新取数据就能跟上；
// - AI 只是「认不出来」时多一句推测：只补 source = unknown（方块图 tier = unknown）的项，档位 / 颜色 / 清理候选一律不变；
// - 显示用的小工具：取一项的推测、工具栏小标签的状态。
import type { AiNote, AiStatus, DirListing, Explanation, NodeKind, NodeView, TreemapNode } from "./api";
import { pathKey } from "./tmtree";

/** 收到过的推测（pathKey → 推测）：数据还没带上它的时候（请求在推测存好之前就发出去了）用它补 */
export type AiNotes = ReadonlyMap<string, AiNote>;

/** 能用的推测：至少要有「这是什么」 */
export function usableNote(n: AiNote | null | undefined): n is AiNote {
  return !!n && typeof n.what === "string" && n.what.trim() !== "";
}

const sameNote = (a: AiNote | undefined, b: AiNote) =>
  a === b || (!!a && a.what === b.what && a.lose === b.lose && a.at === b.at && a.model === b.model && a.owner === b.owner);

/** 列表里的一项（NodeView）认不出来时它的推测；认得出来 / 其余小文件 / 伪块 / 没有推测 → null */
export function noteFor(
  n: { path: string; kind: NodeKind; explanation?: Explanation; pseudo?: string } | null | undefined,
  notes?: AiNotes,
): AiNote | null {
  if (!n || n.kind === "Rest" || n.pseudo) return null;
  const e = n.explanation;
  if (!e || e.source.kind !== "unknown") return null;
  const a = e.ai ?? notes?.get(pathKey(n.path));
  return usableNote(a) ? a : null;
}

/** 方块图里的一块（TreemapNode）认不出来时它的推测 */
export function tmNoteFor(n: TreemapNode | null | undefined, notes?: AiNotes): AiNote | null {
  if (!n || n.kind === "Rest" || n.tier !== "unknown") return null;
  const a = n.ai ?? notes?.get(pathKey(n.path));
  return usableNote(a) ? a : null;
}

/**
 * 方块图的外壳合并块（「Ollama \ updates_v2」：外层几乎全是内层，合成一块，块上的说明用内层）的推测：
 * 内层自己有就用内层的；内层认不出来、又没问过时，用链上离它最近的、问过的上层的——后端自动识别一条“套娃”链只问最上面那个
 * （ai_store.rs ai_candidates，≥ 90% 算同一样东西），而合并块（≥ 95%）显示的是最里面那层，不补就永远看不到推测。
 * 内层认得出来（有自己的说明）时不借上层的。outerPath = 合并块的 key（外层路径）。
 */
export function shellNoteFor(tree: TreemapNode | null | undefined, outerPath: string, inner: TreemapNode | null | undefined, notes?: AiNotes): AiNote | null {
  const own = tmNoteFor(inner, notes);
  if (own || !inner || inner.kind === "Rest" || inner.tier !== "unknown" || !tree) return own;
  const ik = pathKey(inner.path);
  const ok = pathKey(outerPath);
  if (ik === ok || !ik.startsWith(ok + "\\")) return null;
  // 从根沿着内层的路径往下走，记下外层（含）到内层（不含）之间的每一层
  const hops: TreemapNode[] = [];
  let cur: TreemapNode | null = tree;
  while (cur && pathKey(cur.path) !== ik) {
    const ck = pathKey(cur.path);
    if (ck === ok || ck.startsWith(ok + "\\")) hops.push(cur);
    cur = cur.children.find((c) => c.kind === "Dir" && ik.startsWith(pathKey(c.path) + "\\")) ?? null;
  }
  for (let i = hops.length - 1; i >= 0; i--) {
    const a = tmNoteFor(hops[i], notes);
    if (a) return a;
  }
  return null;
}

/**
 * 认不出来的项，后端的说法结尾叫人去问 AI（heuristics.rs 未知项）：「建议」是「……，或者问 AI。」，「删了会怎样」是
 * 「……，或者点“问 AI”让 AI 猜一猜。」。这一行没有「问 AI」可点（已经有推测了 / 没开 AI）时去掉这半句，句号留着
 */
export function withoutAskAi(detail: string): string {
  return detail.replace(/[，,、]?\s*(或者|或)(点\s*)?[“"「]?问\s*AI[”"」]?(\s*让\s*AI\s*猜一猜)?(?=[。.]?\s*$)/, "");
}

/** 推测里没写「删了会怎样」时的说法（不能退回认不出来时那句「……点“问 AI”」——已经问过了） */
export const AI_LOSE_FALLBACK = "没说清楚，先确认再决定。";

/** 推测里「删了会怎样」：没写就用 AI_LOSE_FALLBACK */
export function aiLose(note: AiNote): string {
  return note.lose.trim() || AI_LOSE_FALLBACK;
}

// ───── 设置：哪些地址要密钥（和 llm.rs host_of / key_optional / LlmSettings::usable 一致） ─────

/** 地址里的主机名（小写，不含端口 / 用户名；IPv6 去掉方括号） */
export function hostOf(baseUrl: string): string {
  let s = baseUrl.trim();
  const i = s.indexOf("://");
  if (i >= 0) s = s.slice(i + 3);
  s = s.split(/[/?#]/)[0] ?? "";
  const at = s.lastIndexOf("@");
  if (at >= 0) s = s.slice(at + 1);
  const h = s.startsWith("[") ? (s.slice(1).split("]")[0] ?? "") : (s.split(":")[0] ?? "");
  return h.replace(/\.+$/, "").toLowerCase();
}

/** 本机 / 局域网的地址可以不填密钥；各家云服务都要 */
export function keyOptional(baseUrl: string): boolean {
  const h = hostOf(baseUrl);
  if (!h || (!h.includes(".") && !h.includes(":"))) return true;
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(h);
  if (v4) {
    const [a, b] = [Number(v4[1]), Number(v4[2])];
    return a === 127 || a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254) || h === "0.0.0.0";
  }
  if (h.includes(":")) return h === "::1" || /^f[cd][0-9a-f]{0,2}:/.test(h) || /^fe[89ab][0-9a-f]?:/.test(h);
  return [".localhost", ".local", ".lan", ".internal", ".home.arpa"].some((s) => h.endsWith(s));
}

/** 云服务的地址没填密钥 */
export function missingKey(s: { base_url: string; api_key: string }): boolean {
  return !s.api_key.trim() && !keyOptional(s.base_url);
}

/** AI 能不能用（= 后端 LlmSettings::usable）：开着、填了地址和模型，云服务还要有密钥 */
export function llmUsable(s: { enabled: boolean; base_url: string; model: string; api_key: string }): boolean {
  return s.enabled && !!s.base_url.trim() && !!s.model.trim() && !missingKey(s);
}

// ───── 补缓存（就地改：列表是 Vue 的响应式对象，改它的属性界面就会跟上；方块图数据是普通对象，调用方另外通知） ─────

function patchView(v: NodeView, key: string, note: AiNote): boolean {
  if (v.kind === "Rest" || v.pseudo || pathKey(v.path) !== key) return false;
  const e = v.explanation;
  if (!e || e.source.kind !== "unknown" || sameNote(e.ai, note)) return false;
  e.ai = note;
  return true;
}

/** 列表（标题那一项 + 每一行）里路径是 key（pathKey）的项补上推测；返回改了几项 */
export function patchListing(l: DirListing | null | undefined, key: string, note: AiNote): number {
  if (!l) return 0;
  let n = patchView(l.node, key, note) ? 1 : 0;
  for (const c of l.children) if (patchView(c, key, note)) n++;
  return n;
}

function patchTm(n: TreemapNode, note: AiNote): boolean {
  if (n.tier !== "unknown" || sameNote(n.ai, note)) return false;
  n.ai = note;
  return true;
}

/**
 * 方块图数据里路径是 key 的那一块补上推测（文件夹 / 文件都行；「其余小文件」的 path 是父文件夹，不算）。
 * 只顺着路径前缀往下走（不遍历整棵树）。返回改了几块（0 / 1）
 */
export function patchTree(t: TreemapNode | null | undefined, key: string, note: AiNote): number {
  let cur = t ?? null;
  if (cur && cur.kind !== "Rest" && pathKey(cur.path) === key) return patchTm(cur, note) ? 1 : 0;
  while (cur) {
    if (!key.startsWith(pathKey(cur.path) + "\\")) return 0;
    let next: TreemapNode | null = null;
    for (const c of cur.children) {
      if (c.kind === "Rest") continue;
      const ck = pathKey(c.path);
      if (ck === key) return patchTm(c, note) ? 1 : 0;
      if (c.kind === "Dir" && key.startsWith(ck + "\\")) {
        next = c;
        break;
      }
    }
    cur = next;
  }
  return 0;
}

// ───── 工具栏的小标签：进行中「AI 3/12」/ 出错「AI 出错」/ 刚做完「AI ✓」（约 3 秒，不写个数）/ 不显示 ─────

export type AiChip = { kind: "run"; done: number; total: number } | { kind: "err"; message: string } | { kind: "done"; ok: number };

/** 刚做完的那一闪写几个（问到的个数）；flash = null / 0 不闪 */
export function aiChip(s: AiStatus | null | undefined, flash: number | null): AiChip | null {
  if (!s) return null;
  if (s.running) return s.total > 0 ? { kind: "run", done: Math.max(0, Math.min(s.done, s.total)), total: s.total } : null;
  if (s.error) return { kind: "err", message: s.error };
  if (flash) return { kind: "done", ok: flash };
  return null;
}

/**
 * 进度从「进行中」变成「停了」：全部问完、没出错 → 问到了几个（= 问完的 − 失败的；≤ 0 不闪）；
 * 出错停下 / 被新的扫描打断（没问完）/ 本来就没在跑 → null
 */
export function finishedOk(prev: AiStatus | null | undefined, next: AiStatus): number | null {
  if (!prev?.running || next.running || next.error || next.total <= 0 || next.done < next.total) return null;
  const ok = next.done - next.failed;
  return ok > 0 ? ok : null;
}

export const AI_IDLE: AiStatus = { running: false, done: 0, total: 0, failed: 0, error: null };

/**
 * 后端「测试连接」的报错是「短中文（技术细节）」（llm.rs AskError::detailed，比如「密钥不对（HTTP 401：{…}）」）：
 * 拆成屏幕上写的短话和悬停才看的细节。没有括号的原样当短话。
 */
export function splitErr(msg: string): { short: string; detail: string } {
  const m = msg.trim();
  const i = m.indexOf("（");
  if (i <= 0 || !m.endsWith("）")) return { short: m, detail: "" };
  return { short: m.slice(0, i).trim(), detail: m.slice(i + 1, -1).trim() };
}
