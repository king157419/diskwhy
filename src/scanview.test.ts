import { describe, expect, it } from "vitest";
import { elapsedText, filesCount, scanSegs } from "./scanview";

describe("扫描中的显示（阶段 H）", () => {
  it("大数字是文件数，1 万以上用万", () => {
    expect(filesCount(0)).toEqual({ n: "0", u: "个文件" });
    expect(filesCount(8532)).toEqual({ n: "8,532", u: "个文件" });
    expect(filesCount(10_000)).toEqual({ n: "1", u: "万个文件" });
    expect(filesCount(83_456)).toEqual({ n: "8.3", u: "万个文件" });
    expect(filesCount(832_123)).toEqual({ n: "83.2", u: "万个文件" });
    expect(filesCount(1_234_567)).toEqual({ n: "123", u: "万个文件" });
  });
  it("计时", () => {
    expect(elapsedText(0)).toBe("0 秒");
    expect(elapsedText(12_900)).toBe("12 秒");
    expect(elapsedText(65_000)).toBe("1 分 05 秒");
  });
  it("容量柱只有剩余 / 已用，数字不会超过盘的已用空间（扫描字节再大也不进来）", () => {
    const used = 199 * 1024 ** 3;
    const segs = scanSegs(3 * 1024 ** 3, used);
    expect(segs.map((s) => s.label)).toEqual(["剩余", "已用"]);
    expect(Math.max(...segs.map((s) => s.bytes))).toBeLessThanOrEqual(used);
  });
});
