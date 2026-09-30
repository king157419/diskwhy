// 界面上的短标签（阶段 F / H）。后果标签只认规则 / 启发式给出的 lose_short / lose_kind，不按文案关键词猜。
import type { PreviewProgress, Category, How, LoseKind } from "./api";

/** 后果类型 → 通用标签（规则没写 lose_short 时的兜底），顺序即严重程度（从最坏到最轻） */
export const LOSE_LABEL: Record<LoseKind, string> = {
  irreversible: "找不回来",
  repair: "以后修不了",
  unsure: "可能有你的东西",
  reinstall: "删了要重装",
  relogin: "要重新登录",
  feature: "少一个功能",
  redownload: "要重新下载",
  slower: "下次会慢一点",
};

export const LOSE_ORDER: LoseKind[] = ["irreversible", "repair", "unsure", "reinstall", "relogin", "feature", "redownload", "slower"];

export function loseLabel(k: LoseKind | null | undefined): string | null {
  return k ? (LOSE_LABEL[k] ?? null) : null;
}

/** 行上的标签：优先用规则写的具体短语（lose_short），没有再用 lose_kind 的通用说法 */
export function loseTag(x: { lose_short?: string | null; lose_kind: LoseKind | null | undefined }): string | null {
  const s = x.lose_short?.trim();
  return s ? s : loseLabel(x.lose_kind);
}

/** “找不回来”是实心标签，其余浅底 */
export function isHardTag(k: LoseKind | null | undefined): boolean {
  return k === "irreversible";
}

/** 几项里最坏的后果（没有就 null） */
export function worstLose(kinds: (LoseKind | null | undefined)[]): LoseKind | null {
  let best: LoseKind | null = null;
  for (const k of kinds) {
    if (!k) continue;
    if (best === null || LOSE_ORDER.indexOf(k) < LOSE_ORDER.indexOf(best)) best = k;
  }
  return best;
}

// ───── 清理页左侧的分类 ─────

export type CleanCat = "sys" | "browser" | "dev" | "ai" | "app" | "data";
export const CLEAN_CATS: { id: CleanCat; name: string }[] = [
  { id: "sys", name: "系统缓存" },
  { id: "browser", name: "浏览器" },
  { id: "dev", name: "开发工具" },
  { id: "ai", name: "AI 模型" },
  { id: "app", name: "应用缓存" },
  { id: "data", name: "软件数据" },
];
export const CLEAN_CAT_NAME: Record<CleanCat, string> = Object.fromEntries(CLEAN_CATS.map((c) => [c.id, c.name])) as Record<
  CleanCat,
  string
>;

/** 规则文件分组 + 分类 → 清理页分类 */
export function cleanCat(group: string, category: Category): CleanCat {
  if (category === "model") return "ai";
  switch (group) {
    case "windows-system":
      return "sys";
    case "browsers":
      return "browser";
    case "dev-caches":
    case "heuristic":
      return "dev";
    case "ai-models":
      return "ai";
    default:
      return category === "cache" || category === "log" || category === "installer" ? "app" : "data";
  }
}

// ───── 空间分析列表的灰色类别（≤ 8 字） ─────

export const CATEGORY_LABEL: Record<Category, string> = {
  cache: "缓存",
  log: "日志",
  installer: "安装包",
  build: "编译生成",
  model: "AI 模型",
  userdata: "数据",
  system: "系统文件",
  vm: "虚拟机",
  trash: "回收站",
  app: "程序",
  unknown: "",
};

/** 字符串按显示宽度截断（中文算 2），超出加 “…” */
export function clipWidth(s: string, width: number): string {
  let w = 0;
  let out = "";
  for (const ch of s) {
    const cw = /[\x00-\x7f]/.test(ch) ? 1 : 2;
    if (w + cw > width) return out + "…";
    w += cw;
    out += ch;
  }
  return out;
}

// ───── 路径显示 ─────

/** 把常见的用户目录缩写成环境变量（%LOCALAPPDATA%\npm-cache），和规则里的写法一致，行里更短 */
export function shortPath(p: string): string {
  const rules: [RegExp, string][] = [
    [/^[A-Za-z]:\\Users\\[^\\]+\\AppData\\Local(?=\\|$)/i, "%LOCALAPPDATA%"],
    [/^[A-Za-z]:\\Users\\[^\\]+\\AppData\\Roaming(?=\\|$)/i, "%APPDATA%"],
    [/^[A-Za-z]:\\Users\\[^\\]+(?=\\|$)/i, "%USERPROFILE%"],
    [/^[A-Za-z]:\\ProgramData(?=\\|$)/i, "%ProgramData%"],
  ];
  for (const [re, v] of rules) if (re.test(p)) return p.replace(re, v);
  return p;
}

// ───── 预检进度（阶段 G：确认弹层里“正在检查”一行） ─────

/** 预检进度 → 一行短字 + 进度条百分比（列文件占 5%，逐个检查占 90%，问占用程序占最后一点） */
export function checkLine(p: PreviewProgress | null): { text: string; pct: number } {
  const n = (x: number) => x.toLocaleString("zh-CN");
  if (!p || p.phase === "plan") return { text: "正在检查", pct: 2 };
  if (p.phase === "list") return { text: `正在列出文件 ${n(p.entries)}`, pct: 5 };
  if (p.phase === "check") return { text: `正在检查 ${n(p.checked)} / ${n(p.to_check)}`, pct: 5 + (p.to_check ? (p.checked * 90) / p.to_check : 0) };
  if (p.phase === "who") return { text: `正在查占用的程序 ${p.rm_done} / ${p.rm_total}`, pct: 97 };
  return { text: "正在检查", pct: 99 };
}

// ───── 展开行的「建议」（阶段 H：由档位 + 怎么清推出来，how_detail 作为第二句） ─────

export type AdviceKind = "delete" | "look" | "manual" | "tool" | "apptool" | "migrate" | "keep";

/**
 * 建议的第一句：safe 且能删 → 可以删；review 且能删 → 先看再删；只能手动 → 手动删；自带工具 → 用工具清；迁移 → 搬到 D 盘；其余 → 别动。
 * “自带工具”要分清是 Windows 的（规则在 windows-system 组）还是软件自己的（微信 / 飞书 / conda …的清理功能）。
 */
export function adviceKind(tier: string, how: How[], group?: string | null): AdviceKind {
  const tool: AdviceKind = group === "windows-system" ? "tool" : "apptool";
  if (tier === "keep") return how.includes("migrate") ? "migrate" : how.includes("builtin_tool") ? tool : "keep";
  if (how.includes("delete")) return tier === "safe" ? "delete" : "look";
  if (how.includes("builtin_tool")) return tool;
  if (how.includes("manual")) return "manual";
  if (how.includes("migrate")) return "migrate";
  return "keep";
}

export const ADVICE_TEXT: Record<AdviceKind, string> = {
  delete: "可以删，会自动重建",
  look: "看一眼里面，确定不要了再删",
  manual: "自己打开看看，手动删",
  tool: "别直接删，用 Windows 自带工具清",
  apptool: "别直接删，用软件自带的清理",
  migrate: "别删，搬到 D 盘更好",
  keep: "别动",
};

/** 建议：第一句 + 规则的 how_detail（有的话） */
export function advice(tier: string, how: How[], howDetail?: string | null, group?: string | null): { kind: AdviceKind; head: string; detail: string } {
  const kind = adviceKind(tier, how, group);
  return { kind, head: ADVICE_TEXT[kind], detail: (howDetail ?? "").trim() };
}
