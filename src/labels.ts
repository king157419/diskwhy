// 界面上的短标签（阶段 F）。后果标签只认规则 / 启发式给出的 lose_kind，不按文案关键词猜。
import type { PreviewProgress, Category, LoseKind } from "./api";

/** 后果类型 → 标签文字，顺序即严重程度（从最坏到最轻） */
export const LOSE_LABEL: Record<LoseKind, string> = {
  irreversible: "找不回来",
  repair: "影响修复",
  unsure: "需你确认",
  reinstall: "要重装",
  relogin: "要重登录",
  feature: "功能变少",
  redownload: "要重下载",
  slower: "删了变慢",
};

export const LOSE_ORDER: LoseKind[] = ["irreversible", "repair", "unsure", "reinstall", "relogin", "feature", "redownload", "slower"];

export function loseLabel(k: LoseKind | null | undefined): string | null {
  return k ? (LOSE_LABEL[k] ?? null) : null;
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
  build: "编译产物",
  model: "AI 模型",
  userdata: "数据",
  system: "系统文件",
  vm: "虚拟磁盘",
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
