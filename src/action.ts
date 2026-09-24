// 阶段 D 前端的纯逻辑（便于单元测试）。
const GB = 1024 * 1024 * 1024;

/** 合计字节 → 两位小数的 GB 文本（确认框要求用户输入的数字） */
export function gbText(bytes: number): string {
  return (bytes / GB).toFixed(2);
}

/** 防手滑确认：用户输入的数字（允许带 “GB” 等字样）四舍五入到两位小数后必须等于合计 */
export function confirmMatches(input: string, bytes: number): boolean {
  const cleaned = input.replace(/[^\d.]/g, "");
  if (!cleaned) return false;
  const x = Number.parseFloat(cleaned);
  return Number.isFinite(x) && x.toFixed(2) === gbText(bytes);
}
