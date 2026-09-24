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
import type { Explanation, How, SourceKey, TierKey } from "./api";

// 第 1 轮冷启动评审：每一档只用一个名字（可清理 / 需判断 / 保留 / 未认出），列表、图例、汇总卡、详情都一样。
export const TIER_SHORT: Record<TierKey, string> = {
  safe: "可清理",
  review: "需判断",
  keep: "保留",
  unknown: "未认出",
  zero: "不占空间",
};

export const TIER_BIG: Record<TierKey, string> = {
  safe: "可清理",
  review: "需判断",
  keep: "保留",
  unknown: "未认出",
  zero: "不占空间",
};

/** 每一档一句大白话（图例、颜色说明卡、详情里都用这一句） */
export const TIER_HINT: Record<TierKey, string> = {
  safe: "缓存、临时文件一类，删了会自己长回来，不丢你的数据",
  review: "删了有代价，看“删了会失去什么”再决定",
  keep: "你的数据或系统要用的，别删；想腾地方可以迁走或用自带工具",
  unknown: "还没认出来。先不动它不会有任何损失",
  zero: "空的或只是个链接，不占空间，不用管",
};

/** 解释来源：列表每行的小标签和顶部图例（“未认出”已经是档位，不再当来源重复出现） */
export const SOURCE_SHORT: Record<SourceKey, string> = {
  rule: "规则库",
  heuristic: "推测",
  llm: "AI",
  unknown: "",
};

export const SOURCE_HINT: Record<SourceKey, string> = {
  rule: "规则库里查得到，写明了出处",
  heuristic: "规则库里没有，按里面的文件特征推测",
  llm: "AI 根据名字猜的，只当参考",
  unknown: "还没认出来，只列线索",
};

export const HOW_LABEL: Record<How, string> = {
  delete: "可以直接删除",
  builtin_tool: "用系统或软件自带的工具清理",
  migrate: "迁到别的盘",
  manual: "需要手动处理",
  none: "先不用管",
};

export function sourceLabel(e: Explanation): string {
  const s = e.source;
  switch (s.kind) {
    case "rule":
      return (
        (s.inherited_from ? `规则库：${s.id}（按上层文件夹「${s.inherited_from}」的规则）` : `规则库：${s.id}（分组 ${s.group}）`) +
        (e.observed_only ? " · 本机观察，没找到官方出处" : "")
      );
    case "heuristic":
      return `推测：${s.name}`;
    case "llm":
      return `AI 推测（模型 ${s.model}）`;
    case "attribution":
      return "归属统计（用卷已用空间差额推算，不是规则库）";
    default:
      return "未知";
  }
}
