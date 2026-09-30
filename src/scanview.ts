// 首页扫描中的显示（阶段 H）。扫描过程中累计的字节是去重前的原始数（硬链接、商店应用重定向会重复算），
// 可能比整个盘还大，所以扫描中**不显示任何扫描字节数**：大数字是看过的文件数，容量柱只画“剩余 / 已用”，
// 已用那段做成流动的“扫描中”样式，等结果出来再换成三档。

/** 文件数 → 大数字 + 单位：1 万以下直接写，1 万以上用“万”（83 万个文件） */
export function filesCount(files: number): { n: string; u: string } {
  const f = Math.max(0, Math.floor(files || 0));
  if (f < 10_000) return { n: f.toLocaleString("zh-CN"), u: "个文件" };
  const w = f / 10_000;
  return { n: w >= 100 ? String(Math.floor(w)) : (Math.floor(w * 10) / 10).toFixed(1).replace(/\.0$/, ""), u: "万个文件" };
}

/** 已经扫了多久：“12 秒” / “1 分 05 秒” */
export function elapsedText(ms: number): string {
  const s = Math.max(0, Math.floor((ms || 0) / 1000));
  return s < 60 ? `${s} 秒` : `${Math.floor(s / 60)} 分 ${String(s % 60).padStart(2, "0")} 秒`;
}

export interface ScanSeg {
  key: string;
  kind: string;
  label: string;
  bytes: number;
}

/** 扫描中的容量柱：只用盘自己报告的剩余 / 已用（不会超过盘的大小），已用段是流动的“扫描中”样式 */
export function scanSegs(freeBytes: number, usedBytes: number): ScanSeg[] {
  return [
    { key: "free", kind: "free", label: "剩余", bytes: Math.max(0, freeBytes) },
    { key: "scanning", kind: "scanned", label: "已用", bytes: Math.max(0, usedBytes) },
  ];
}
