// 阶段 K：方块图数据的懒加载拼接（graft）、显示树、布局输入和单击翻译。纯函数，不碰后端 / DOM（src/tmtree.test.ts）。
// 方块图的根始终是扫描根；单击往里挖时，链上每一级都按它自己做根单独取一次子树（各自的阈值，细节和历史无关），
// 显示树 = 盘根的数据 + 链上每一级自己的子树依次拼上去（只复制从根到它的那一串节点，其余原样共享）。
import { tmKey, type TreemapNode } from "./api";
import { BOOST, canonPath, collapseChain, layoutTreemap, samePath, type HiddenSlot, type LayoutOptions, type Rect, type TmInput, type TmItem, type TmItemKind } from "./treemap";

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
      // 外壳合并块写的数：链已经走进内层（合并块在链上 ⇒ 链穿过了每一层）时写内层的大小（和面包屑、列表一致），
      // 否则写外层的（和上一级列表一致）。面积始终按外层
      size: boosted && hops.length > 1 ? node.allocated : n.allocated,
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

/**
 * 自动缩放后图的根（rootPath）在上一级图里是哪一块：外壳合并块（DriverStore \ FileRepository）用外层的 key 和合并的名字，
 * 大小写内层的（链穿过了它）。缩放时它画成铺满整张图、顶上一条标题栏的容器，和上一级图里那一块是同一个 key。
 */
export function rootHead(tree: TreemapNode | null, chain: readonly string[], rootPath: string): { key: string; name: string; size: number } | null {
  const node = findTm(tree, rootPath);
  if (!tree || !node) return null;
  const starts = chainLevelStarts(tree, chain);
  const at = chain.findIndex((c) => samePath(c, rootPath));
  // rootPath 所在的那一层（合并块）从链的哪一级开始
  let s = at;
  for (const st of starts) if (st <= at) s = st;
  const outer = s >= 0 ? findTm(tree, chain[s]) : null;
  if (outer && s < at) {
    const { name, hops } = collapseFor(outer, chain);
    if (samePath(hops[hops.length - 1].path, rootPath)) return { key: tmKey(outer), name, size: node.allocated };
  }
  return { key: tmKey(node), name: node.name, size: node.allocated };
}

/**
 * 以 rootPath 为图的根铺一次（TreemapView 用；纯函数，方便测）。参照矩形从扫描根沿原始的层走下来：缩放前后同一套行 / 列结构。
 * 根不是扫描根时，根自己画成铺满整张图的容器（顶上一条和链上祖先同样式的标题栏），没有伪块。
 */
export function layoutMap(
  data: TreemapNode | null,
  chain: readonly string[],
  rootPath: string,
  frame: Rect,
  opt: LayoutOptions,
  extra: readonly TmInput[] = [],
): { root: string; items: TmItem[]; hidden: Map<string, HiddenSlot> } | null {
  if (!data) return null;
  const node = findTm(data, rootPath);
  if (!node) return null;
  const scan = samePath(node.path, data.path);
  const input = toInput(node, chain, { maxDepth: opt.maxDepth, boostDepth: opt.boostDepth ?? 2, extra: scan ? [...extra] : [] });
  /** 没有单独画出来的子项结构上的位置（下一次它们出现时从这里长出来） */
  const hidden = new Map<string, HiddenSlot>();
  if (scan) return { root: node.path, items: layoutTreemap(input, frame, opt, frame, frame, undefined, hidden), hidden };
  const nodes = pathInputs(data, node.path, [...extra]);
  const canon = nodes ? canonPath(nodes, frame, opt) : frame;
  const head = rootHead(data, chain, node.path) ?? { key: tmKey(node), name: node.name, size: node.allocated };
  // 根的 key 用它在上一级图里那一块的 key（外壳合并块 = 外层）：它的子块、「其他」的 key（父 key|其他）缩放前后都一样
  input.key = head.key;
  return { root: node.path, items: layoutTreemap(input, frame, opt, canon, frame, head, hidden), hidden };
}

// ───── 展开时调淡：链外的块统一盖一层页面底色 ─────

/**
 * 展开了东西时要调淡的块（用户定的「统一调淡 · 更淡」：只有一档，不按远近分级、不分左右）。
 * 每块顺着 parentKey 往上找最近的链上的块：找到的是 pk（最深的展开块）= 在 pk 里面，不调淡；
 * 是别的链上的块（祖先容器、外壳合并块、缩放后的根标题条）或者一直到顶都没有 = 在外面，调淡。
 * 链上的块自己从不调淡（祖先容器、它们的标题栏和「收起」按钮、根标题条都清楚）；pk 为空（什么都没展开）或不在图上时一块也不调淡。
 */
export function outsideChain<T extends { key: string; parentKey: string | null; chain: boolean }>(items: readonly T[], pk: string | null): Set<string> {
  const out = new Set<string>();
  const by = new Map(items.map((t) => [t.key, t]));
  if (!pk || !by.has(pk)) return out;
  for (const t of items) {
    if (t.chain) continue;
    let p = t.parentKey;
    while (p !== null) {
      const a = by.get(p);
      if (!a) {
        p = null;
        break;
      }
      if (a.chain) break;
      p = a.parentKey;
    }
    if (p !== pk) out.add(t.key);
  }
  return out;
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

/**
 * 单击容器的标题栏：path = 内层（往里挖），outer = 块的 key（外层路径，收起按它算）。
 * 自动缩放后图顶上的根标题条（depth 0）也一样 = 收起这个根自己（阶段 L：标题条上写着「收起」，就得真的把它收起来；
 * 以前是「收起到这一级」，和下面那一层容器的「收起」做的是同一件事）。根收起以后图的根跟着往上移（镜头拉远）。
 */
export function headHit(it: Pick<TmItem, "key" | "ref">): MapHit | null {
  const n = tileNode(it);
  if (!n) return null;
  return { kind: "head", path: n.path, outer: it.key };
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

/** 链上容器标题栏右端的「收起」按钮：图标 + 「收起」/ 只有图标 / 不画 */
export type HeadPill = "full" | "icon" | "none";

/**
 * 链上容器（已展开、单击标题栏 = 收起）的标题栏：右端放一个「收起」按钮，名字 / 大小在剩下的地方按 fitHead 写。
 * 按钮从完整的（图标 + 「收起」）往下退：名字还能露出 fullMin 个字（或者整个放得下）才放完整的；
 * 再窄换成只有图标的，名字至少露出 iconMin 个字；再窄就不放按钮（整条标题栏照样能点），空间全给名字和大小。
 * pill.full / pill.icon = 按钮占的宽度（含它和前面内容的间隔），avail = 标题栏能写字的宽度（不含按钮）。
 */
export function fitChainHead(
  name: string,
  size: string,
  avail: number,
  m: { name: (s: string) => number; size: (s: string) => number; ell: number; gap: number },
  pill: { full: number; icon: number },
  fullMin = 6,
  iconMin = 4,
): { name: string; cut: boolean; size: boolean; pill: HeadPill } {
  const len = Array.from(name).length;
  const tries: [HeadPill, number, number][] = [
    ["full", pill.full, fullMin],
    ["icon", pill.icon, iconMin],
  ];
  for (const [mode, w, min] of tries) {
    const room = avail - w;
    if (room <= 0) continue;
    const r = fitHead(name, size, room, m);
    if (r.name !== "" && (!r.cut || Array.from(r.name).length >= Math.min(min, len))) return { ...r, pill: mode };
  }
  return { ...fitHead(name, size, avail, m), pill: "none" };
}

// ───── 叶子块上的字（F3）：按实际量出来的字宽 / 行高决定写法，永不硬裁 ─────

/** 块上的写法：名字 + 大小两行 / 竖排 / 一行 / 只写短大小 / 不写 */
export type LabelMode = "full" | "vert" | "line" | "size" | "none";

/** 量字：块上实际用的字体（TreemapView 用 canvas 按探针元素的计算样式量；测试用假字体） */
export interface LabelMetrics {
  /** 名字（两行写法的第一行；big = 大块的大字号） */
  name(s: string, big: boolean): number;
  /** 两行写法第二行的大小 / 只写大小时的短大小（数字等宽：调用方把数字当成「0」量） */
  size(s: string, big: boolean): number;
  /** 一行写法的名字 / 大小 */
  lname(s: string): number;
  lsize(s: string): number;
  /** 竖排的名字 / 大小（竖着的长度） */
  vname(s: string): number;
  vsize(s: string): number;
  /** 小块只写大小时用的小一号的字 */
  tiny(s: string): number;
  /** 再小一号（10px）：30 px 上下的小块只写大小 */
  micro(s: string): number;
  /** 省略号「…」（名字的字体） */
  ell(big: boolean): number;
  ellLine: number;
  ellV: number;
  /** 行高（px）：名字 / 大小（两行写法）、一行写法、竖排（横向占的宽度；uv = 10px 竖排）、只写大小（行高 = 字号；t 小一号、u 再小一号） */
  lh: { a: number; aBig: number; b: number; bBig: number; ln: number; v: number; s: number; t: number; u: number; uv: number };
}

export interface LabelIn {
  /** 块面（tileBox）的宽高、描边 */
  w: number;
  h: number;
  bw: number;
  name: string;
  /** 「其他 410 项」这类：整段放不下就只写「410 项」，再放不下就不写名字 */
  count?: { pre: string; num: string; post: string } | null;
  /** 两行 / 一行写法里的大小（「27.5 MB」）和只写大小时的短大小（「27M」「1.2」） */
  full: string;
  short: string;
  /** C 主题扫不到的伪块：字放在白底小标签里（多出左右 5 + 上下 1 的内边距、两行之间 4） */
  pill?: boolean;
  /** 块大（> 150 × 70）：名字 / 大小用大一号的字 */
  big?: boolean;
}

export interface LabelFit {
  mode: LabelMode;
  big: boolean;
  /** 只写大小、用小一号的字（30 多 px 的小块） */
  tiny?: boolean;
  /** 只写大小、再小一号（10px：30 px 上下的小块贴边 2px 横着写；20 px 宽的细高条竖着写） */
  micro?: boolean;
  /** 实际写的名字（截断时不含省略号，cut = 要在后面画「…」）；只写大小 / 不写时为 "" */
  name: string;
  cut: boolean;
  /** 字那一层四周的留白（px）：上下 iy、左右 ix */
  ix: number;
  iy: number;
}

const NONE: LabelFit = { mode: "none", big: false, name: "", cut: false, ix: 0, iy: 0 };
/** 量字和浏览器排版之间的误差留量（亚像素、字距） */
const SLACK = 1.5;

/** 名字截到 room 里：放得下就整个；放不下就截断加「…」，至少要露出 minChars 个字（名字本身更短就要整个），不然返回 null */
function fitName(name: string, room: number, w: (s: string) => number, ell: number, minChars: number): { name: string; cut: boolean } | null {
  if (room <= 0) return null;
  if (w(name) <= room) return { name, cut: false };
  const chars = Array.from(name);
  let lo = 0;
  let hi = chars.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (w(chars.slice(0, mid).join("")) + ell <= room) lo = mid;
    else hi = mid - 1;
  }
  if (lo < Math.min(minChars, chars.length)) return null;
  const head = chars.slice(0, lo).join("").replace(/[\s\\]+$/, "");
  return head ? { name: head, cut: true } : null;
}

/** 一行标题截到 room 宽：放得下就整个，放不下截断（调用方在后面用界面字体画「…」）。至少留一个字 */
export function fitTitle(name: string, room: number, w: (s: string) => number, ell: number): { name: string; cut: boolean } {
  return fitName(name, room, w, ell, 1) ?? { name: Array.from(name)[0] ?? "", cut: name.length > 1 };
}

/** 「其他 410 项」：整段 → 「410 项」→ 放不下（数字不截） */
function fitCount(c: { pre: string; num: string; post: string }, room: number, w: (s: string) => number): string | null {
  const all = `${c.pre} ${c.num} ${c.post}`;
  if (w(all) <= room) return all;
  const short = `${c.num} ${c.post}`;
  return w(short) <= room ? short : null;
}

/**
 * 叶子块上写什么（F3）：依次试 两行（名字 + 大小）→ 竖排（细高条）→ 一行 → 只写短大小 → 不写。
 * 每种写法都按量出来的字宽 / 行高、扣掉描边和留白算：放得下才写，绝不出现被裁掉半行、「27N」这种字。
 */
export function fitLabel(t: LabelIn, m: LabelMetrics, minChars = 3): LabelFit {
  const { w, h, bw } = t;
  if (w <= 2 * bw + 8 || h <= 2 * bw + 8) return NONE;
  const pill = t.pill ? 10 : 0;
  const pillY = t.pill ? 2 : 0;
  const nameOf = (room: number, wn: (s: string) => number, ell: number) =>
    t.count ? ((s) => (s ? { name: s, cut: false } : null))(fitCount(t.count, room, wn)) : fitName(t.name, room, wn, ell, minChars);

  // 两行：名字 / 大小（大块用大一号的字，放不下再试普通字号）
  for (const big of t.big ? [true, false] : [false]) {
    const ix = t.pill ? 9 : 8;
    const iy = t.pill ? 8 : 7;
    const cw = w - 2 * bw - 2 * ix - SLACK;
    const ch = h - 2 * bw - 2 * iy;
    const need = (big ? m.lh.aBig + m.lh.bBig : m.lh.a + m.lh.b) + (t.pill ? 2 * pillY + 4 : 0);
    if (ch < need || m.size(t.full, big) + pill > cw) continue;
    const nm = nameOf(cw - pill, (s) => m.name(s, big), m.ell(big));
    if (nm) return { mode: "full", big, name: nm.name, cut: nm.cut, ix, iy };
  }
  // 竖排：细高条（展开旁边的块时常见）
  if (w >= 24 && h >= 96 && h >= w * 2.5) {
    const ix = 4;
    const iy = 8;
    const cw = w - 2 * bw - 2 * ix;
    const len = h - 2 * bw - 2 * iy - SLACK;
    if (cw >= m.lh.v + pill && !t.pill) {
      const room = len - 6 - m.vsize(t.short);
      const nm = nameOf(room, (s) => m.vname(s), m.ellV);
      if (nm) return { mode: "vert", big: false, name: nm.name, cut: nm.cut, ix, iy };
    }
  }
  // 一行：名字 + 大小，垂直居中
  {
    const ix = t.pill ? 9 : 8;
    const cw = w - 2 * bw - 2 * ix - SLACK;
    const ch = h - 2 * bw;
    if (ch >= m.lh.ln + 4 + 2 * pillY) {
      const room = cw - pill - 6 - m.lsize(t.full);
      const nm = nameOf(room, (s) => m.lname(s), m.ellLine);
      if (nm) return { mode: "line", big: false, name: nm.name, cut: nm.cut, ix, iy: 0 };
    }
  }
  // 只写短大小（整串放得下才写）：放得下时和别的块一样贴左上（7 / 8），小块收紧到 5 / 居中
  {
    const sw = m.size(t.short, false) + pill;
    const inner = w - 2 * bw - SLACK;
    const ch = h - 2 * bw - 2 * pillY;
    const ix = inner - 16 >= sw ? 8 : inner - 10 >= sw ? 5 : -1;
    if (ix >= 0 && ch >= m.lh.s + 4) {
      const iy = Math.max(2, Math.min(7, Math.floor((ch - m.lh.s) / 2)));
      return { mode: "size", big: false, name: "", cut: false, ix, iy };
    }
    // 再小一点的块：小一号的字、贴边 3px（仍然整串放得下才写）
    const tw = m.tiny(t.short) + pill;
    if (!t.pill && inner - 6 >= tw && ch >= m.lh.t + 4) {
      const iy = Math.max(2, Math.min(5, Math.floor((ch - m.lh.t) / 2)));
      return { mode: "size", big: false, tiny: true, name: "", cut: false, ix: 3, iy };
    }
    // 30 px 上下的小块：再小一号（10px）、贴边 2px（C 主题墨框里一排没字的小格子看上去像没画完）
    const uw = m.micro(t.short);
    if (!t.pill && inner - 4 >= uw && ch >= m.lh.u + 4) {
      const iy = Math.max(2, Math.min(5, Math.floor((ch - m.lh.u) / 2)));
      return { mode: "size", big: false, tiny: true, micro: true, name: "", cut: false, ix: 2, iy };
    }
  }
  // 窄高的小块（横着写不下「27M」）：大小竖着写；再窄用 10px 竖着写
  if (!t.pill && h >= w * 1.3) {
    const cw = w - 2 * bw;
    const len = h - 2 * bw - SLACK;
    const sl = m.vsize(t.short);
    if (cw >= m.lh.v + 4 && len >= sl + 10) {
      const iy = Math.min(8, Math.floor((len - sl) / 2));
      const ix = Math.max(2, Math.min(4, Math.floor((cw - m.lh.v) / 2)));
      return { mode: "vert", big: false, name: "", cut: false, ix, iy };
    }
    const su = m.micro(t.short);
    if (cw >= m.lh.uv + 3 && len >= su + 8) {
      const iy = Math.min(8, Math.floor((len - su) / 2));
      const ix = Math.max(1, Math.min(4, Math.floor((cw - m.lh.uv) / 2)));
      return { mode: "vert", big: false, micro: true, name: "", cut: false, ix, iy };
    }
  }
  return NONE;
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
