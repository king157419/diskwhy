// 界面上的短标签（阶段 F / H）。后果标签只认规则 / 启发式给出的 lose_short / lose_kind，不按文案关键词猜。
import type { ByDest, CountBytes, DestKind, Interrupted, PreviewProgress, Category, How, LoseKind } from "./api";
import { sizeParts } from "./capacity";

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
  migrate: "别删，搬到别的盘更好",
  keep: "别动",
};

/** 建议：第一句 + 规则的 how_detail（有的话） */
export function advice(tier: string, how: How[], howDetail?: string | null, group?: string | null): { kind: AdviceKind; head: string; detail: string } {
  const kind = adviceKind(tier, how, group);
  return { kind, head: ADVICE_TEXT[kind], detail: (howDetail ?? "").trim() };
}

// ───── 盘名（阶段 J：界面上不再写死“C 盘”） ─────

/** 扫描根 → “C 盘” / “D 盘”；扫的是某个文件夹时取它所在的盘；认不出盘符时写“磁盘” */
export function driveName(root: string | null | undefined): string {
  const m = /^([A-Za-z]):/.exec((root ?? "").trim());
  return m ? `${m[1].toUpperCase()} 盘` : "磁盘";
}

// ───── 清理执行（阶段 J：确认弹层的去向 / 预计时间 / 跳过、执行中的剩余时间、完成页、上次中断） ─────

const size = (b: number) => sizeParts(b).join(" ");

/** 跳过原因 → 2–5 字 */
export const SKIP_LABEL: Record<string, string> = {
  hot: "刚改过",
  cloud: "云端文件",
  locked: "正在使用",
  no_permission: "没权限",
  protected: "受保护",
  changed: "被改过",
  missing: "已不在",
  unreadable: "读不到",
  hardlink: "硬链接",
  recycle_unavailable: "回收站放不下",
  /** 前端合成：整项被护栏拦下 */
  blocked: "被拦下",
  /** 前端合成：扫描之后变小了（文件已经没了 / 硬链接不占空间 …） */
  shrunk: "已变小",
};
/** 悬停时的一句补充（只在 title 里出现） */
export const SKIP_TIP: Record<string, string> = {
  hot: "30 分钟内改过，可能正在用",
  cloud: "OneDrive 等云端文件，不碰",
  locked: "被程序占用",
  no_permission: "没有删除权限",
  protected: "系统保护的位置",
  changed: "检查之后又被改过",
  missing: "已经不在了",
  unreadable: "读不到文件信息",
  hardlink: "硬链接，删了不腾空间",
  recycle_unavailable: "太大或路径太长，送进回收站会被直接删掉",
  blocked: "整项被拦下，不会动",
  shrunk: "比扫描时小了：有的文件已经没了，硬链接和很小的文件不占空间",
};

/** 预计用时（确认弹层主按钮旁）：< 15 秒「几秒」，< 1 分「约 N 秒」，其余「约 N 分钟」/「约 N 小时」 */
export function estText(secs: number): string {
  if (!Number.isFinite(secs) || secs < 15) return "几秒";
  if (secs < 60) return `约 ${Math.round(secs / 10) * 10} 秒`;
  const min = Math.round(secs / 60);
  if (min < 60) return `约 ${min} 分钟`;
  const h = secs / 3600;
  return `约 ${h < 10 ? Math.round(h * 2) / 2 : Math.round(h)} 小时`;
}

/** 执行中的一次采样：elapsed_ms → cost_done */
export interface RateSample {
  t: number;
  c: number;
}

/** 加一个采样，只留最近 windowMs（默认 5 秒）里的；时间倒退（重新附着到另一次执行）时清空重来 */
export function pushSample(samples: RateSample[], s: RateSample, windowMs = 5000): RateSample[] {
  const last = samples[samples.length - 1];
  const base = last && s.t < last.t ? [] : samples;
  return [...base, s].filter((x) => x.t >= s.t - windowMs);
}

/** 按滑窗速率算剩余秒数。刚开始（elapsed < minElapsedMs）或窗口太短（< 1 秒）、速率 ≤ 0 时返回 null（界面上不显示） */
export function remainSecs(samples: RateSample[], costTotal: number, minElapsedMs = 2000): number | null {
  if (samples.length < 2) return null;
  const a = samples[0];
  const b = samples[samples.length - 1];
  if (b.t < minElapsedMs || b.t - a.t < 1000) return null;
  const rate = (b.c - a.c) / (b.t - a.t);
  if (!(rate > 0)) return null;
  return Math.max(0, costTotal - b.c) / rate / 1000;
}

/**
 * 剩余时间（执行中进度条下面）：null（前 2 秒 / 速率不明）→ 空；≥ 1 分「剩约 N 分」（≥ 1 小时「剩约 N 小时 M 分」）；
 * < 1 分「剩约 N 秒」（按 5 秒取整，至少 5）；只有剩不到 10 秒**而且**进度 ≥ 80% 时才说「快好了」
 * （大文件先做完、进度还在 30% 时速率估出来的“几秒”不可信）。
 */
export function remainText(secs: number | null, pct = 100): string {
  if (secs === null || !Number.isFinite(secs) || secs < 0) return "";
  if (secs < 10 && pct >= 80) return "快好了";
  if (secs < 60) return `剩约 ${Math.min(55, Math.max(5, Math.round(secs / 5) * 5))} 秒`;
  const min = Math.ceil(secs / 60);
  if (min < 60) return `剩约 ${min} 分`;
  const h = Math.floor(min / 60);
  const m = min % 60;
  return m ? `剩约 ${h} 小时 ${m} 分` : `剩约 ${h} 小时`;
}

/** 执行中的大数字：按估算成本加权的百分比（和进度条同一个值），向下取整，没做完不显示 100 */
export function pctText(pct: number): string {
  if (!Number.isFinite(pct) || pct <= 0) return "0";
  return String(Math.min(100, Math.floor(pct)));
}

/** 个数：< 1 万「9,800」，≥ 1 万「4.1 万」，≥ 100 万「123 万」 */
export function countText(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0";
  if (n < 10000) return Math.round(n).toLocaleString("zh-CN");
  const w = n / 10000;
  return `${w >= 100 ? Math.round(w) : (Math.floor(w * 10) / 10).toFixed(1)} 万`;
}

/** 已处理的量按总量的单位写（同一个单位，好比较）：「12.3 / 29.3 GB」「0 / 480 MB」 */
export function bytesPair(done: number, total: number): string {
  const [, unit] = sizeParts(Math.max(total, done));
  const div = unit === "GB" ? 1024 ** 3 : unit === "MB" ? 1024 ** 2 : unit === "KB" ? 1024 : 1;
  const f = (b: number) => {
    const v = b / div;
    if (v <= 0) return "0";
    if (unit === "B" || v >= 100) return Math.round(v).toString();
    return v.toFixed(v >= 1 ? 1 : 2);
  };
  return `${f(Math.min(done, total || done))} / ${f(total)} ${unit}`;
}

/** 执行中进度条下面的一行小字：「已处理 12.3 / 29.3 GB · 4.1 万 / 39.6 万个」（个数不明时只有字节） */
export function processedLine(p: { bytes_done: number; total_bytes: number; done: number; total: number }): string {
  const parts = [`已处理 ${bytesPair(p.bytes_done, p.total_bytes)}`];
  if (p.total > 0) {
    const t = countText(p.total);
    parts.push(`${countText(Math.min(p.done, p.total))} / ${t}${t.endsWith("万") ? "" : " "}个`);
  }
  return parts.join(" · ");
}

/** 去向 → 短字（destLine 里用） */
export function destWord(d: DestKind, drive?: string): string {
  if (d === "permanent") return "直接删";
  if (d === "recycle") return "进回收站";
  return drive ? `暂存到 ${drive}` : "暂存";
}

export interface DestPart {
  tier: "safe" | "review";
  label: string;
  dest: string;
}

/** 确认弹层的去向行（§3.1）：「可放心清理 直接删 · 需你判断 暂存到 D 盘」，哪一档是空的就不写那一段 */
export function destLine(o: { safe: boolean; review: boolean; safeDest: DestKind; reviewDest: DestKind; drive?: string }): DestPart[] {
  const out: DestPart[] = [];
  if (o.safe) out.push({ tier: "safe", label: "可放心清理", dest: destWord(o.safeDest, o.drive) });
  if (o.review) out.push({ tier: "review", label: "需你判断", dest: destWord(o.reviewDest, o.drive) });
  return out;
}

export interface SkipRow {
  code: string;
  label: string;
  tip: string;
  count: number;
  bytes: number;
}

/** 跳过原因 → 行（字节多的在前，同字节按个数） */
export function skipRows(byReason: Record<string, CountBytes | number> | null | undefined): SkipRow[] {
  return Object.entries(byReason ?? {})
    .map(([code, v]) => {
      const cb = typeof v === "number" ? { count: v, bytes: 0 } : v;
      return { code, label: SKIP_LABEL[code] ?? code, tip: SKIP_TIP[code] ?? "", count: cb.count, bytes: cb.bytes };
    })
    .filter((r) => r.count > 0 || r.bytes > 0)
    .sort((a, b) => b.bytes - a.bytes || b.count - a.count);
}

/** 字节最多的原因（同字节按个数）；没有为 undefined */
export function mainSkip(rows: SkipRow[]): SkipRow | undefined {
  let best: SkipRow | undefined;
  for (const r of rows) if (!best || r.bytes > best.bytes || (r.bytes === best.bytes && r.count > best.count)) best = r;
  return best;
}

export interface SkipSummary {
  /** 跳过的条目数 + 被拦下的项数 */
  count: number;
  /** = 扫描时的估算（含被拦下的）− 真正要处理的（解释 30.1 → 29.3） */
  bytes: number;
  /** 最主要的原因（字节最多的那一行，含“已变小”） */
  main: string;
  rows: SkipRow[];
}

/**
 * 确认弹层「跳过」一行（预检做完之后）：跳过的各原因 + 被拦下的整项 + 比扫描时变小的部分，合计正好等于
 * 「扫描时的估算（含被拦下的）− 真正要处理的」，所以数字从 30.1 变成 29.3 时，这一行就是那 0.8。
 */
export function skipSummary(r: {
  total_skipped: number;
  total_bytes: number;
  scan_total_bytes?: number;
  skipped_bytes?: number;
  skipped_by_reason?: Record<string, CountBytes>;
  items: { blocked: string | null; scan_allocated: number; skipped_by_reason?: Record<string, number> }[];
}): SkipSummary {
  let byReason: Record<string, CountBytes | number> | undefined = r.skipped_by_reason;
  if (!byReason) {
    // 旧报告没有按原因的字节：用各项的个数
    const m: Record<string, number> = {};
    for (const it of r.items) for (const [k, n] of Object.entries(it.skipped_by_reason ?? {})) m[k] = (m[k] ?? 0) + n;
    byReason = m;
  }
  const rows = skipRows(byReason);
  const skippedBytes = r.skipped_bytes ?? rows.reduce((a, x) => a + x.bytes, 0);
  const blocked = r.items.filter((i) => i.blocked);
  if (blocked.length) {
    rows.push({
      code: "blocked",
      label: SKIP_LABEL.blocked,
      tip: SKIP_TIP.blocked,
      count: blocked.length,
      bytes: blocked.reduce((a, i) => a + i.scan_allocated, 0),
    });
  }
  const rest = r.scan_total_bytes !== undefined ? r.scan_total_bytes - r.total_bytes - skippedBytes : 0;
  if (rest >= 1024 * 1024) rows.push({ code: "shrunk", label: SKIP_LABEL.shrunk, tip: SKIP_TIP.shrunk, count: 0, bytes: rest });
  rows.sort((a, b) => b.bytes - a.bytes || b.count - a.count);
  // 主要原因 = 字节最多的那一行（“已变小”也算一种原因）
  const main = mainSkip(rows);
  return {
    count: r.total_skipped + blocked.length,
    bytes: rows.reduce((a, x) => a + x.bytes, 0),
    main: main?.label ?? "",
    rows,
  };
}

/** 「跳过 539 个 · 0.8 GB · 正在使用」（没有个数、只是变小了时「少了 0.1 GB」；什么都没有为空） */
export function skipHeadline(s: SkipSummary): string {
  if (s.count > 0) return [`跳过 ${s.count.toLocaleString("zh-CN")} 个`, s.bytes > 0 ? size(s.bytes) : "", s.main].filter(Boolean).join(" · ");
  if (s.bytes > 0) return `少了 ${size(s.bytes)}`;
  return "";
}

export interface DoneRow {
  key: "permanent" | "quarantine" | "recycle" | "nuked" | "failed" | "skipped" | "not_processed";
  label: string;
  value: string;
  note?: string;
}

/** 完成页的小字（§3.5）：只列非零的。直接删除 / 暂存到 D 盘 / 回收站 · 还没腾出 / 没成功 / 跳过 N（主要原因按字节）/ 没处理 N */
export function doneRows(
  r: {
    by_dest?: ByDest;
    ok_bytes: number;
    skipped: number;
    failed: number;
    not_processed?: CountBytes;
    skipped_by_reason?: Record<string, CountBytes>;
  },
  stashDrive?: string,
): DoneRow[] {
  const out: DoneRow[] = [];
  const bd = r.by_dest;
  if (bd) {
    // 个数放在小字里：几十万个小文件的字节数可能只有几 MB 甚至 0（存在 MFT 里），只写字节会像“什么都没做”
    const n = (c: number) => `${countText(c)}${countText(c).endsWith("万") ? "" : " "}个`;
    if (bd.permanent.count || bd.permanent.bytes)
      out.push({ key: "permanent", label: "直接删除", value: size(bd.permanent.bytes), note: bd.permanent.count ? n(bd.permanent.count) : undefined });
    if (bd.quarantine.count || bd.quarantine.bytes)
      out.push({
        key: "quarantine",
        label: stashDrive ? `暂存到 ${stashDrive}` : "暂存",
        value: size(bd.quarantine.bytes),
        note: bd.quarantine.count ? n(bd.quarantine.count) : undefined,
      });
    if (bd.recycle.count || bd.recycle.bytes) out.push({ key: "recycle", label: "回收站", value: size(bd.recycle.bytes), note: "还没腾出" });
    // 本该进回收站、却被 Windows 直接删掉的（正常不会有）
    if (bd.nuked && (bd.nuked.count || bd.nuked.bytes)) out.push({ key: "nuked", label: "回收站没收", value: size(bd.nuked.bytes), note: "找不回" });
  } else if (r.ok_bytes > 0) {
    out.push({ key: "permanent", label: "已处理", value: size(r.ok_bytes) });
  }
  if (r.failed > 0) out.push({ key: "failed", label: "没成功", value: `${r.failed.toLocaleString("zh-CN")} 个` });
  if (r.skipped > 0) {
    const main = mainSkip(skipRows(r.skipped_by_reason));
    out.push({ key: "skipped", label: "跳过", value: `${r.skipped.toLocaleString("zh-CN")} 个`, note: main?.label });
  }
  const np = r.not_processed;
  if (np && np.count > 0) out.push({ key: "not_processed", label: "没处理", value: `${np.count.toLocaleString("zh-CN")} 个` });
  return out;
}

/** 首页「上次清理中断 · 已处理 N 个」 */
export function interruptedLine(i: Pick<Interrupted, "processed">): string {
  return `上次清理中断 · 已处理 ${i.processed.toLocaleString("zh-CN")} 个`;
}

/** 中断的原因（展开后一行）：程序出错 / Windows 记下的退出码 / 不知道 */
export function interruptedCause(i: Pick<Interrupted, "exit_code" | "crash_logged">): string {
  if (i.crash_logged) return "程序出错，已记录";
  if (i.exit_code) return "被结束"; // 退出码只在悬停里给（界面上不放十六进制）
  return "原因不明";
}

/** unix 秒 → 「今天 05:52」/「10 月 6 日 05:52」 */
export function whenText(unixSecs: number, now = new Date()): string {
  const d = new Date(unixSecs * 1000);
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  return d.toDateString() === now.toDateString() ? `今天 ${hm}` : `${d.getMonth() + 1} 月 ${d.getDate()} 日 ${hm}`;
}

/** 盘名从路径里取（"暂存区（D:\\x）" 这类标签也行）；没有盘符为 undefined */
export function driveOf(s: string | null | undefined): string | undefined {
  const m = /([A-Za-z]):\\/.exec(s ?? "");
  return m ? driveName(m[0]) : undefined;
}

/** C 主题标题字体（霞鹜文楷子集）里没有的字：含这些字的元素改用界面字体，避免一个词里两种字体（重做子集要下载上游字体） */
export const KAI_MISSING = "识暂获密钥义千授准备崩溃退码";
export function kaiSafe(s: string): boolean {
  for (const ch of s) if (KAI_MISSING.includes(ch)) return false;
  return true;
}

/** 「D:\」「d:」这样的盘根 */
export function isDriveRoot(p: string | null | undefined): boolean {
  return /^[A-Za-z]:\\?$/.test((p ?? "").trim());
}

/** 暂存区 dest_root → 真正放东西的文件夹：「D:\备份」→「D:\备份\DiskWhy暂存区」（批次放在它下面） */
export function stashFolder(destRoot: string): string {
  return destRoot.replace(/\\+$/, "") + "\\DiskWhy暂存区";
}
