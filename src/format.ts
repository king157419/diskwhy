export function humanBytes(b: number): string {
  const u = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = b;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${b} B` : `${v.toFixed(v >= 100 ? 1 : 2)} ${u[i]}`;
}

export function pct(part: number, whole: number): string {
  if (!whole) return "0%";
  const p = (part * 100) / whole;
  return p >= 10 ? `${p.toFixed(1)}%` : `${p.toFixed(2)}%`;
}

export function fmtTime(unix: number): string {
  if (!unix) return "未知";
  const d = new Date(unix * 1000);
  const z = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())} ${z(d.getHours())}:${z(d.getMinutes())}`;
}

export function fmtInt(n: number): string {
  return n.toLocaleString("zh-CN");
}

export function fmtDuration(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} 秒`;
  return `${Math.floor(s / 60)} 分 ${Math.round(s % 60)} 秒`;
}

const ATTR_NAMES: [number, string][] = [
  [0x1, "只读"],
  [0x2, "隐藏"],
  [0x4, "系统"],
  [0x200, "稀疏"],
  [0x400, "重解析点"],
  [0x800, "NTFS 压缩"],
  [0x1000, "脱机"],
  [0x4000, "加密"],
  [0x40000, "打开时下载"],
  [0x80000, "始终保留在本机"],
  [0x100000, "仅在线"],
  [0x400000, "访问时下载"],
];

export function attrText(a: number): string {
  const s = ATTR_NAMES.filter(([m]) => (a & m) !== 0).map(([, n]) => n);
  return s.length ? s.join("、") : "无特殊属性";
}

// ───── 阶段 B：解释相关的中文标签 ─────
import type { Explanation, How, TierKey } from "./api";

export const TIER_SHORT: Record<TierKey, string> = {
  safe: "可清理",
  review: "需判断",
  keep: "保留",
  unknown: "未知",
};

export const TIER_BIG: Record<TierKey, string> = {
  safe: "可以放心清理",
  review: "需要你来判断",
  keep: "建议保留，别删",
  unknown: "我还不知道这是什么",
};

export const TIER_HINT: Record<TierKey, string> = {
  safe: "缓存一类的东西，删了会自动重新生成，不丢你的数据。",
  review: "删了有代价，下面写清楚了会失去什么，由你决定。",
  keep: "是你的数据或系统需要的东西；想腾空间请看“该怎么腾”，用迁移或系统工具。",
  unknown: "规则库和启发式都没认出来。先别删，看看下面的线索，或者问问 AI。",
};

export const HOW_LABEL: Record<How, string> = {
  delete: "可以直接删除",
  builtin_tool: "用系统或软件自带的工具清理",
  migrate: "迁到别的盘",
  manual: "需要手动处理",
  none: "不需要处理",
};

export function sourceLabel(e: Explanation): string {
  const s = e.source;
  switch (s.kind) {
    case "rule":
      return s.inherited_from ? `规则库：${s.id}（按上层文件夹「${s.inherited_from}」的规则）` : `规则库：${s.id}（分组 ${s.group}）`;
    case "heuristic":
      return `启发式推测：${s.name}`;
    case "llm":
      return `AI 推测（模型 ${s.model}）`;
    default:
      return "未知";
  }
}
