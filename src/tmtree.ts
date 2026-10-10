// 阶段 K：方块图数据的懒加载拼接（graft）、显示树、布局输入和单击翻译。纯函数，不碰后端 / DOM（src/tmtree.test.ts）。
// 方块图的根始终是扫描根；单击往里挖时，链上每一级都按它自己做根单独取一次子树（各自的阈值，细节和历史无关），
// 显示树 = 盘根的数据 + 链上每一级自己的子树依次拼上去（只复制从根到它的那一串节点，其余原样共享）。
import { tmKey, type TreemapNode } from "./api";
import { BOOST, collapseChain, samePath, type TmInput, type TmItem, type TmItemKind } from "./treemap";

/**
 * “外壳”合并（Users \ king）。展开链停在外壳这一级（链的最后一级就是 Users），或者从外壳拐进别的子文件夹
 * （Users → Public）时不合并：图上看得见 Users 里的 king / Public / Default，和列表一致。
 * 方块图（toInput）和「图上的层」（chainLevelStarts）共用这一份规则。
 */
export function collapseFor(n: TreemapNode, chain: readonly string[]) {
  return collapseChain(n, 0.95, 3, (from, to) => {
    const i = chain.findIndex((c) => samePath(c, from.path));
    if (i < 0) return false;
    const nx = chain[i + 1];
    return nx === undefined || !samePath(nx, to.path);
  });
}

/**
 * 展开链在图上分成几层：合并成一块的外壳（Users \ king）算一层。返回每一层在链里的起始下标。
 * 数据里找不到的路径各算一层。
 */
export function chainLevelStarts(tree: TreemapNode | null, chain: readonly string[]): number[] {
  const starts: number[] = [];
  let i = 0;
  while (i < chain.length) {
    starts.push(i);
    const n = findTm(tree, chain[i]);
    let j = i + 1;
    if (n) {
      const { hops } = collapseFor(n, chain);
      while (j < chain.length && j - i < hops.length && samePath(hops[j - i].path, chain[j])) j++;
    }
    i = j;
  }
  return starts;
}

/** 自动缩放时可以当图的根的路径（由浅到深）：扫描根，然后链在图上每一层的最后一级（外壳取内层） */
export function rootCandidates(tree: TreemapNode | null, chain: readonly string[]): string[] {
  if (!tree) return [];
  const starts = chainLevelStarts(tree, chain);
  return [tree.path, ...starts.slice(1).map((s) => chain[s - 1])];
}

const norm = (p: string) => p.replace(/[\\/]+$/, "").toLowerCase();

/** p 在 dir 里面（或就是 dir）：大小写不敏感 */
function within(p: string, dir: string): boolean {
  const a = norm(p);
  const b = norm(dir);
  return a === b || a.startsWith(b + "\\");
}

/** 按路径找文件夹节点（只顺着路径前缀往下走）；找不到返回 null */
export function findTm(tree: TreemapNode | null, path: string): TreemapNode | null {
  let cur = tree;
  while (cur) {
    if (samePath(cur.path, path)) return cur;
    cur = cur.children.find((c) => c.kind === "Dir" && within(path, c.path)) ?? null;
  }
  return null;
}

/** 从根（不含）到 path（含）的文件夹路径串；不在树里返回 null。path 就是根时返回 [] */
export function ancestorsIn(tree: TreemapNode | null, path: string): string[] | null {
  const out: string[] = [];
  let cur = tree;
  if (!cur || !within(path, cur.path)) return null;
  while (cur) {
    if (samePath(cur.path, path)) return out;
    cur = cur.children.find((c) => c.kind === "Dir" && within(path, c.path)) ?? null;
    if (cur) out.push(cur.path);
  }
  return null;
}

/** 把 sub（以某个文件夹为根取回来的子树）拼进 tree：换掉那个文件夹的 children / hidden_*。找不到就原样返回 */
export function graft(tree: TreemapNode, sub: TreemapNode): TreemapNode {
  return replaceAt(tree, sub.path, (n) => ({ ...n, children: sub.children, hidden_bytes: sub.hidden_bytes, hidden_count: sub.hidden_count, can_open: n.can_open || sub.can_open }));
}

/**
 * 父文件夹的数据里没有这个子文件夹（被并进了“其他”，或者超过了每层的个数上限）：
 * 补一个节点进去，并从父的 hidden_* 里扣掉它。
 */
export function insertChild(tree: TreemapNode, parentPath: string, child: TreemapNode): TreemapNode {
  return replaceAt(tree, parentPath, (p) => {
    if (p.children.some((c) => c.kind === child.kind && samePath(c.path, child.path))) return p;
    const children = [...p.children, child].sort((a, b) => b.allocated - a.allocated);
    const took = Math.min(p.hidden_bytes, child.allocated);
    return { ...p, children, hidden_bytes: p.hidden_bytes - took, hidden_count: Math.max(0, p.hidden_count - (took > 0 ? 1 : 0)) };
  });
}

function replaceAt(tree: TreemapNode, path: string, f: (n: TreemapNode) => TreemapNode): TreemapNode {
  if (samePath(tree.path, path)) return f(tree);
  const i = tree.children.findIndex((c) => c.kind === "Dir" && within(path, c.path));
  if (i < 0) return tree;
  const next = replaceAt(tree.children[i], path, f);
  if (next === tree.children[i]) return tree;
  const children = tree.children.slice();
  children[i] = next;
  return { ...tree, children };
}

/** 缓存键：大小写不敏感、忽略末尾反斜杠 */
export const pathKey = norm;

/** a 是 b 下面的（不含 b 自己） */
export function isUnder(a: string, b: string): boolean {
  return within(a, b) && !samePath(a, b);
}

/** 父级数据里没有、用列表信息补的节点：补在哪个父文件夹下面 */
export interface Stub {
  parent: string;
  node: TreemapNode;
}

/**
 * 显示树：盘根的数据，链上每一级依次——数据里没有就补上（stubs），按它自己取过就把它自己的子树拼上（cache）。
 * 只看链和缓存，不看逛过哪里：同一条链（数据取回来之后）永远是同一张图；收起以后回到盘根自己的数据。
 */
export function buildDisplay(root: TreemapNode | null, chain: readonly string[], cache: ReadonlyMap<string, TreemapNode>, stubs: ReadonlyMap<string, Stub>): TreemapNode | null {
  if (!root) return null;
  let t = root;
  for (const p of chain) {
    const k = pathKey(p);
    const s = stubs.get(k);
    if (s && !findTm(t, p)) t = insertChild(t, s.parent, s.node);
    const sub = cache.get(k);
    if (sub && !samePath(sub.path, t.path)) t = graft(t, sub);
  }
  return t;
}

// ───── 布局输入 ─────

export interface InputOpts {
  /** 链外：根下面画几层（同 LayoutOptions.maxDepth） */
  maxDepth: number;
  /** 链上的块下面另外画几层（同 LayoutOptions.boostDepth） */
  boostDepth: number;
  /** 根上额外的块（扫描根的伪块：无权限 / 系统保留 / 未归属） */
  extra?: TmInput[];
}

/** 只带一层子项 key / 值的浅输入：算「基础结构」时沿原始的层往下走用 */
export function shallowInput(n: TreemapNode, extra: readonly TmInput[] = []): TmInput {
  const r: TmInput = {
    key: tmKey(n),
    name: n.name,
    value: n.allocated,
    children: n.children.map((c) => ({ key: tmKey(c), name: c.name, value: c.allocated })),
    hidden: n.hidden_bytes,
    hiddenCount: n.hidden_count,
    ref: n,
  };
  for (const x of extra) {
    r.children!.push({ key: x.key, name: x.name, value: x.value });
    r.value += x.value;
  }
  return r;
}

/** 从扫描根到 path 的原始各层（浅输入，扫描根带伪块）；path 不在树里返回 null */
export function pathInputs(tree: TreemapNode | null, path: string, extra: readonly TmInput[] = []): TmInput[] | null {
  const anc = ancestorsIn(tree, path);
  if (!tree || !anc) return null;
  const out = [shallowInput(tree, extra)];
  for (const p of anc) {
    const n = findTm(tree, p);
    if (!n) return null;
    out.push(shallowInput(n));
  }
  return out;
}

/**
 * 方块图的布局输入（根自己不出块，只铺它的子项）。
 * - 外壳合并块（Users \ king）：key 和大小用**外层**（和列表、面包屑一致；数据拼进来、外壳从“不合并”变成“合并”时块的身份不变，
 *   原地过渡不会断），里面铺内层的子项；外层多出来的那点（Public、Default …）并进“其他”。ref = 内层节点（往里挖、配色、说明用）；
 *   via = 原始的每一层（算基础结构用）。
 * - 只转换布局真会画到的层（链上的块下面 boostDepth 层，其余 maxDepth 层，和 layoutTreemap 的预算一致）：逛得越多树越大也不变慢。
 */
export function toInput(root: TreemapNode, chain: readonly string[], o: InputOpts): TmInput {
  const on = new Set(chain.map(pathKey));
  const conv = (n: TreemapNode, budget: number): TmInput => {
    const { node, name, hops } = collapseFor(n, chain);
    const boosted = n.kind === "Dir" && hops.some((h) => on.has(pathKey(h.path)));
    const inner = boosted ? Math.max(o.boostDepth, budget - 1) : budget - 1;
    let hiddenCount = node.hidden_count;
    for (let k = 0; k < hops.length - 1; k++) hiddenCount += hops[k].children.length - 1 + hops[k].hidden_count;
    return {
      key: tmKey(n),
      name,
      value: n.allocated,
      children: inner >= 1 && node.children.length ? node.children.map((c) => conv(c, inner)) : undefined,
      hidden: node.hidden_bytes,
      hiddenCount,
      boost: boosted ? BOOST : undefined,
      via: hops.length > 1 ? hops.map((h) => shallowInput(h)) : undefined,
      ref: node,
    };
  };
  const r: TmInput = {
    key: tmKey(root),
    name: root.name,
    value: root.allocated,
    children: root.children.map((c) => conv(c, o.maxDepth)),
    hidden: root.hidden_bytes,
    hiddenCount: root.hidden_count,
    ref: root,
  };
  for (const x of o.extra ?? []) {
    r.children!.push(x);
    r.value += x.value;
  }
  return r;
}

// ───── 单击翻译（TreemapView 把 DOM 事件翻译成 MapHit，store.mapClick 执行） ─────

/** 方块图里被单击的东西 */
export type MapHit =
  /** 文件夹块（任意深度；“外壳”合并块给内层路径） */
  | { kind: "dir"; path: string }
  /** 容器的标题栏：在链上 = 收起到它的父级；不在链上 = 往里挖。outer = 合并块的外层路径 */
  | { kind: "head"; path: string; outer: string }
  /**
   * 文件 / 其余小文件 / 打不开的文件夹（row = 列表里对应那一行的 key）、「其他」/ 伪块（row = null）。
   * sig：「其他」块的内容签名（项数 | 大小）——它所在的文件夹重新取数、原来的「其他」被拆开以后签名对不上，就不再算选中
   */
  | { kind: "leaf"; key: string; parent: string | null; row: string | null; sig?: string };

/** 块的数据节点（伪块没有） */
export function tileNode(it: { ref?: unknown }): TreemapNode | null {
  const x = it.ref as TreemapNode | { pseudo: string } | undefined;
  return x && typeof x === "object" && "path" in x ? x : null;
}

/** 块所在的文件夹：顶层块 = 图的根；其余 = 父块（合并块取内层） */
export function tileParent(it: { parentKey: string | null }, byKey: ReadonlyMap<string, { ref?: unknown }>, mapRoot: string | null): string | null {
  if (!it.parentKey) return mapRoot;
  const p = byKey.get(it.parentKey);
  return (p && tileNode(p)?.path) ?? null;
}

/** 「其他」块的内容签名（项数 | 大小） */
export function otherSig(it: { mergedCount: number; value: number }): string {
  return `${it.mergedCount}|${Math.round(it.value)}`;
}

/** 单击块的身体：能打开的文件夹 = 往里挖；文件 / 其余小文件 / 打不开的文件夹 = 选中并展开列表那一行；其他 / 伪块 = 只选中 */
export function tileHit(it: { key: string; kind: TmItemKind; ref?: unknown; mergedCount: number; value: number }, parent: string | null): MapHit {
  const n = tileNode(it);
  if (it.kind === "other") return { kind: "leaf", key: it.key, parent, row: null, sig: otherSig(it) };
  if (!n) return { kind: "leaf", key: it.key, parent, row: null };
  if (n.kind === "Dir" && n.can_open) return { kind: "dir", path: n.path };
  return { kind: "leaf", key: it.key, parent, row: n.path + n.kind };
}

/** 单击容器的标题栏：path = 内层（往里挖），outer = 块的 key（外层路径，收起按它算） */
export function headHit(it: Pick<TmItem, "key" | "ref">): MapHit | null {
  const n = tileNode(it);
  return n ? { kind: "head", path: n.path, outer: it.key } : null;
}

/**
 * 单击块的身体 → MapHit（TreemapView 只做这一步翻译）：所在的文件夹按 tileParent（顶层块 = 图的根，自动缩放后是缩放后的根；
 * 其余 = 父块，合并块取内层），再按块的种类翻译（tileHit）。「其余小文件」的列表行 key = 父文件夹路径 + "Rest"。
 */
export function hitOf(
  it: { key: string; kind: TmItemKind; ref?: unknown; mergedCount: number; value: number; parentKey: string | null },
  byKey: ReadonlyMap<string, { ref?: unknown }>,
  mapRoot: string | null,
): MapHit {
  return tileHit(it, tileParent(it, byKey, mapRoot));
}

// ───── 块上的文字 ─────

/**
 * 「其他 410 项」「（其余 2380 个文件）」这类带数字的名字：拆成 前缀 / 数字 / 后缀，块上写短的「其余 2380 个」，
 * 截断时先截前缀、保住数字（不会出现「其余 23…」这种看上去像别的数的字）。不是这类名字返回 null。
 */
export function countLabel(it: { kind: TmItemKind; name: string; mergedCount: number; ref?: unknown }): { pre: string; num: string; post: string } | null {
  if (it.kind === "other") return { pre: "其他", num: String(it.mergedCount), post: "项" };
  const n = tileNode(it);
  if (n?.kind !== "Rest") return null;
  const m = /其余\s*([\d,，]+)\s*个/.exec(n.name);
  return m ? { pre: "其余", num: m[1].replace(/，/g, ","), post: "个" } : null;
}

/** 带数字名字的整段文字（「其余 2380 个」） */
export const countText = (c: { pre: string; num: string; post: string }) => `${c.pre} ${c.num} ${c.post}`;

/** 悬停卡片上的名字：「其余小文件」去掉全角括号（“其余 2380 个文件”），其余照原样 */
export function tipName(it: { name: string; ref?: unknown }): string {
  const n = tileNode(it);
  return n?.kind === "Rest" ? n.name.replace(/^（(.*)）$/, "$1") : it.name;
}

/**
 * 容器标题栏写什么：名字放得下就名字 + 大小；放不下时，带着大小名字还能露出至少 minChars 个字才写大小，
 * 不然大小不写、空间全给名字（名字比大小重要）；窄到名字只露得出一两个字时只写大小。截断用「…」（由调用方用界面字体画，
 * 不用标题字体里居中的「···」）。m = 量文字宽度（px）。
 */
export function fitHead(
  name: string,
  size: string,
  avail: number,
  m: { name: (s: string) => number; size: (s: string) => number; ell: number; gap: number },
  minChars = 6,
): { name: string; cut: boolean; size: boolean } {
  const chars = Array.from(name);
  const fit = (room: number) => {
    // 能放下的最多字数（前缀宽度单调递增：二分）
    let lo = 0;
    let hi = chars.length;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (m.name(chars.slice(0, mid).join("")) <= room) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  };
  const nameW = m.name(name);
  const sizeW = m.size(size) + m.gap;
  if (nameW + sizeW <= avail) return { name, cut: false, size: true };
  /** 截到 k 个字；截在合并块名字的分隔处（「.ollama \\ …」）时把末尾的空格和反斜杠去掉 */
  const head = (k: number) => chars.slice(0, k).join("").replace(/[\s\\]+$/, "");
  const withSize = fit(avail - sizeW - m.ell);
  if (withSize >= Math.min(minChars, chars.length)) return { name: head(withSize), cut: true, size: true };
  if (nameW <= avail) return { name, cut: false, size: false };
  const n = fit(avail - m.ell);
  // 窄到名字只露得出一两个字（「W…」猜不出是什么）：只写大小（全名在标题栏的提示里）
  if (n < 3 && sizeW - m.gap <= avail) return { name: "", cut: false, size: true };
  return { name: head(n), cut: true, size: false };
}

// ───── 键盘：只有一块在 Tab 序列里，方向键按画面位置在块之间走 ─────

export interface NavBox {
  key: string;
  x: number;
  y: number;
  w: number;
  h: number;
}
export type NavDir = "left" | "right" | "up" | "down";

/**
 * 键盘能走到的块：顶层块，以及「展开链上的容器」里面的块（一层层往下）；不在链上的容器算一整块（Enter 往里挖之后才走得进去）。
 * 链上的容器用它的标题栏当位置（标题栏 = 「收起」按钮，往下就走进它的子块）。skip 的块（离场 / 太小的平块）不算。
 */
export function navTargets<T extends { key: string; parentKey: string | null; container: boolean; chain: boolean }>(
  list: readonly T[],
  box: (t: T) => { x: number; y: number; w: number; h: number },
  head: number,
  skip: (t: T) => boolean = () => false,
): NavBox[] {
  const by = new Map(list.map((t) => [t.key, t]));
  const open = (t: T | undefined) => !!t && t.container && t.chain;
  const memo = new Map<string, boolean>();
  const reach = (t: T): boolean => {
    const c = memo.get(t.key);
    if (c !== undefined) return c;
    let ok = false;
    if (t.parentKey === null) ok = true;
    else {
      const p = by.get(t.parentKey);
      ok = !!p && open(p) && reach(p);
    }
    memo.set(t.key, ok);
    return ok;
  };
  const out: NavBox[] = [];
  for (const t of list) {
    if (skip(t) || !reach(t)) continue;
    const b = box(t);
    out.push({ key: t.key, x: b.x, y: b.y, w: b.w, h: open(t) ? Math.min(head, b.h) : b.h });
  }
  return out;
}

/**
 * 方向键：从 from 往 dir 走到画面上最近的一块。只看整块都在那一侧的块（容器的标题栏在它子块的上面，
 * 所以从第一行子块往上 = 回到容器的标题栏）；距离 = 沿方向的间隔 + 2 × 侧向错开的距离 + 0.3 × 中心线的偏差
 * （同样挨着的几块里取和自己对齐的那块），同分取中心更对齐的。
 */
export function navStep(boxes: readonly NavBox[], from: string, dir: NavDir): string | null {
  const c = boxes.find((b) => b.key === from);
  if (!c) return null;
  const E = 1;
  let best: string | null = null;
  let bestScore = Infinity;
  let bestAlign = Infinity;
  for (const t of boxes) {
    if (t.key === from) continue;
    let gap: number;
    let side: number;
    let align: number;
    if (dir === "right" || dir === "left") {
      gap = dir === "right" ? t.x - (c.x + c.w) : c.x - (t.x + t.w);
      side = Math.max(0, t.y - (c.y + c.h), c.y - (t.y + t.h));
      align = Math.abs(t.y + t.h / 2 - (c.y + c.h / 2));
    } else {
      gap = dir === "down" ? t.y - (c.y + c.h) : c.y - (t.y + t.h);
      side = Math.max(0, t.x - (c.x + c.w), c.x - (t.x + t.w));
      align = Math.abs(t.x + t.w / 2 - (c.x + c.w / 2));
    }
    if (gap < -E) continue;
    const score = Math.max(0, gap) + 2 * side + 0.3 * align;
    if (score < bestScore - 0.5 || (Math.abs(score - bestScore) <= 0.5 && align < bestAlign)) {
      best = t.key;
      bestScore = score;
      bestAlign = align;
    }
  }
  return best;
}
