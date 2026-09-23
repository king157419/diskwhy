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
