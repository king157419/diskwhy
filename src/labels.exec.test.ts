// 阶段 J：清理执行相关的纯函数（预计时间、剩余时间、去向一行、跳过汇总、完成页小字、上次中断）
import { describe, expect, it } from "vitest";
import {
  bytesPair,
  countText,
  destLine,
  doneRows,
  driveOf,
  estText,
  interruptedCause,
  interruptedLine,
  isDriveRoot,
  stashFolder,
  kaiSafe,
  mainSkip,
  pctText,
  processedLine,
  pushSample,
  remainSecs,
  remainText,
  skipHeadline,
  skipRows,
  skipSummary,
  whenText,
  type RateSample,
} from "./labels";

const GB = 1024 ** 3;
const MB = 1024 ** 2;

describe("预计用时 estText", () => {
  it("不到 15 秒是「几秒」，之后约 N 秒 / 约 N 分钟 / 约 N 小时", () => {
    expect(estText(0)).toBe("几秒");
    expect(estText(14.9)).toBe("几秒");
    expect(estText(NaN)).toBe("几秒");
    expect(estText(15)).toBe("约 20 秒");
    expect(estText(44)).toBe("约 40 秒");
    expect(estText(60)).toBe("约 1 分钟");
    expect(estText(132)).toBe("约 2 分钟");
    expect(estText(59 * 60)).toBe("约 59 分钟");
    expect(estText(5400)).toBe("约 1.5 小时");
  });
});

describe("剩余时间：5 秒滑窗速率", () => {
  const feed = (pts: [number, number][]) => pts.reduce<RateSample[]>((s, [t, c]) => pushSample(s, { t, c }), []);
  it("只留最近 5 秒的采样；时间倒退（换了一次执行）就清空", () => {
    const s = feed([
      [0, 0],
      [1000, 10],
      [4000, 40],
      [7000, 70],
    ]);
    expect(s.map((x) => x.t)).toEqual([4000, 7000]);
    expect(pushSample(s, { t: 100, c: 1 })).toEqual([{ t: 100, c: 1 }]);
  });
  it("前 2 秒、窗口不到 1 秒、速率 ≤ 0 时不给数", () => {
    expect(remainSecs(feed([[0, 0], [1500, 15]]), 100)).toBeNull();
    expect(remainSecs(feed([[3000, 30], [3500, 35]]), 100)).toBeNull();
    expect(remainSecs(feed([[3000, 30], [6000, 30]]), 100)).toBeNull();
    expect(remainSecs([], 100)).toBeNull();
  });
  it("按窗口里的速率算：每秒 10 个单位，还剩 600 → 60 秒", () => {
    const s = feed([
      [2000, 20],
      [4000, 40],
      [6000, 60],
    ]);
    expect(remainSecs(s, 660)).toBeCloseTo(60);
  });
  it("只看最近 5 秒：前面快后面慢时按慢的算", () => {
    const s = feed([
      [0, 0],
      [3000, 3000],
      [10000, 3070],
      [15000, 3120],
    ]);
    // 窗口 10–15 秒：50 单位 / 5 秒 = 10/秒；剩 1880 → 188 秒
    expect(remainSecs(s, 5000)).toBeCloseTo(188);
  });
  it("文字：剩约 N 分（≥ 1 分）/ 剩约 N 秒（按 5 秒取整）/ 快好了（< 10 秒且 ≥ 80%）", () => {
    expect(remainText(null)).toBe("");
    expect(remainText(NaN, 50)).toBe("");
    expect(remainText(3)).toBe("快好了");
    expect(remainText(9.9, 80)).toBe("快好了");
    // 剩不到 10 秒，但进度还不到 80%：不说「快好了」
    expect(remainText(3, 79)).toBe("剩约 5 秒");
    expect(remainText(9, 37)).toBe("剩约 10 秒");
    expect(remainText(0.4, 10)).toBe("剩约 5 秒");
    expect(remainText(10, 95)).toBe("剩约 10 秒");
    expect(remainText(14.9)).toBe("剩约 15 秒");
    expect(remainText(42)).toBe("剩约 40 秒");
    expect(remainText(43)).toBe("剩约 45 秒");
    expect(remainText(59)).toBe("剩约 55 秒");
    expect(remainText(60)).toBe("剩约 1 分");
    expect(remainText(61)).toBe("剩约 2 分");
    expect(remainText(120)).toBe("剩约 2 分");
    expect(remainText(3600)).toBe("剩约 1 小时");
    expect(remainText(3600 + 600)).toBe("剩约 1 小时 10 分");
  });
  it("前 2 秒什么都不写（remainSecs 给 null → 空）", () => {
    const s = feed([
      [500, 5],
      [1900, 19],
    ]);
    expect(remainText(remainSecs(s, 100), 19)).toBe("");
  });
});

describe("执行中：百分比大数字 + 已处理一行", () => {
  it("百分比向下取整，没做完不到 100", () => {
    expect(pctText(0)).toBe("0");
    expect(pctText(-3)).toBe("0");
    expect(pctText(NaN)).toBe("0");
    expect(pctText(0.4)).toBe("0");
    expect(pctText(37.9)).toBe("37");
    expect(pctText(99.99)).toBe("99");
    expect(pctText(100)).toBe("100");
    expect(pctText(130)).toBe("100");
  });
  it("个数：< 1 万原样带千分位，≥ 1 万写「万」", () => {
    expect(countText(0)).toBe("0");
    expect(countText(9800)).toBe("9,800");
    expect(countText(10000)).toBe("1.0 万");
    expect(countText(41234)).toBe("4.1 万");
    expect(countText(396000)).toBe("39.6 万");
    expect(countText(99999)).toBe("9.9 万");
    expect(countText(1234567)).toBe("123 万");
  });
  it("字节按总量的单位写", () => {
    expect(bytesPair(12.3 * GB, 29.3 * GB)).toBe("12.3 / 29.3 GB");
    expect(bytesPair(0, 29.3 * GB)).toBe("0 / 29.3 GB");
    expect(bytesPair(0.4 * GB, 2 * GB)).toBe("0.40 / 2.0 GB");
    expect(bytesPair(120 * MB, 480 * MB)).toBe("120 / 480 MB");
    expect(bytesPair(3 * MB, 4.5 * MB)).toBe("3.0 / 4.5 MB");
    expect(bytesPair(40 * GB, 29.3 * GB)).toBe("29.3 / 29.3 GB");
  });
  it("一行：「已处理 12.3 / 29.3 GB · 4.1 万 / 39.6 万个」", () => {
    expect(processedLine({ bytes_done: 12.3 * GB, total_bytes: 29.3 * GB, done: 41000, total: 396000 })).toBe("已处理 12.3 / 29.3 GB · 4.1 万 / 39.6 万个");
    expect(processedLine({ bytes_done: 120 * MB, total_bytes: 490 * MB, done: 2460, total: 9800 })).toBe("已处理 120 / 490 MB · 2,460 / 9,800 个");
    expect(processedLine({ bytes_done: 120 * MB, total_bytes: 490 * MB, done: 9000, total: 12000 })).toBe("已处理 120 / 490 MB · 9,000 / 1.2 万个");
    // 个数不明（旧后端）只写字节
    expect(processedLine({ bytes_done: GB, total_bytes: 2 * GB, done: 0, total: 0 })).toBe("已处理 1.0 / 2.0 GB");
  });
});

describe("去向一行 destLine", () => {
  const txt = (p: ReturnType<typeof destLine>) => p.map((x) => `${x.label} ${x.dest}`).join(" · ");
  it("可放心清理直接删 · 需你判断暂存到 D 盘", () => {
    expect(txt(destLine({ safe: true, review: true, safeDest: "permanent", reviewDest: "quarantine", drive: "D 盘" }))).toBe(
      "可放心清理 直接删 · 需你判断 暂存到 D 盘",
    );
  });
  it("哪一档是空的就不写那一段", () => {
    expect(txt(destLine({ safe: true, review: false, safeDest: "permanent", reviewDest: "quarantine", drive: "D 盘" }))).toBe("可放心清理 直接删");
    expect(txt(destLine({ safe: false, review: true, safeDest: "permanent", reviewDest: "recycle" }))).toBe("需你判断 进回收站");
    expect(destLine({ safe: false, review: false, safeDest: "permanent", reviewDest: "recycle" })).toEqual([]);
  });
  it("明确选回收站：两档都进回收站", () => {
    expect(txt(destLine({ safe: true, review: true, safeDest: "recycle", reviewDest: "recycle" }))).toBe("可放心清理 进回收站 · 需你判断 进回收站");
  });
});

describe("跳过汇总（解释 30.1 → 29.3）", () => {
  const cb = (count: number, bytes: number) => ({ count, bytes });
  it("各原因 + 被拦下 + 变小的部分，合计 = 估算（含被拦下）− 真正要处理的", () => {
    const scan = 30.1 * GB;
    const total = 29.3 * GB;
    const blockedBytes = 0.2 * GB;
    const s = skipSummary({
      total_skipped: 539,
      total_bytes: total,
      scan_total_bytes: scan,
      skipped_bytes: 0.7 * GB,
      skipped_by_reason: { locked: cb(412, 0.5 * GB), hot: cb(98, 0.15 * GB), hardlink: cb(29, 0.05 * GB) },
      items: [
        { blocked: null, scan_allocated: scan },
        { blocked: "受保护", scan_allocated: blockedBytes },
      ],
    });
    expect(s.count).toBe(540);
    expect(s.bytes).toBeCloseTo(scan + blockedBytes - total, -3);
    expect(s.main).toBe("正在使用");
    expect(s.rows.map((r) => r.code)).toEqual(["locked", "blocked", "hot", "shrunk", "hardlink"]);
    expect(s.rows.find((r) => r.code === "shrunk")!.bytes).toBeCloseTo(0.1 * GB, -3);
    expect(skipHeadline(s)).toBe("跳过 540 个 · 1.00 GB · 正在使用");
  });
  it("主要原因按字节（不是按个数），「已变小」也算", () => {
    const s = skipSummary({
      total_skipped: 600,
      total_bytes: 10 * GB,
      scan_total_bytes: 12 * GB,
      skipped_bytes: 0.5 * GB,
      skipped_by_reason: { hot: cb(580, 0.1 * GB), locked: cb(20, 0.4 * GB) },
      items: [],
    });
    // 变小了 1.5 GB，比任何一个原因都多
    expect(s.main).toBe("已变小");
    expect(skipHeadline(s)).toBe("跳过 600 个 · 2.00 GB · 已变小");
    const t = skipSummary({ total_skipped: 600, total_bytes: 10 * GB, skipped_by_reason: { hot: cb(580, 0.1 * GB), locked: cb(20, 0.4 * GB) }, items: [] });
    expect(t.main).toBe("正在使用");
    expect(mainSkip([])).toBeUndefined();
  });
  it("变小不到 1 MB 不单列；没有跳过时标题为空", () => {
    const s = skipSummary({ total_skipped: 0, total_bytes: 10 * GB, scan_total_bytes: 10 * GB + 0.5 * MB, skipped_bytes: 0, skipped_by_reason: {}, items: [] });
    expect(s.rows).toEqual([]);
    expect(skipHeadline(s)).toBe("");
  });
  it("只是比扫描时小了：「少了 X」", () => {
    const s = skipSummary({ total_skipped: 0, total_bytes: 10 * GB, scan_total_bytes: 10 * GB + 300 * MB, skipped_bytes: 0, skipped_by_reason: {}, items: [] });
    expect(skipHeadline(s)).toBe("少了 300 MB");
  });
  it("新原因有标签：硬链接、回收站放不下；旧报告只有个数也能列", () => {
    expect(skipRows({ hardlink: cb(3, 0), recycle_unavailable: cb(1, 5 * GB) }).map((r) => r.label)).toEqual(["回收站放不下", "硬链接"]);
    const old = skipSummary({ total_skipped: 6, total_bytes: 1, items: [{ blocked: null, scan_allocated: 1, skipped_by_reason: { locked: 4, hot: 2 } }] });
    expect(old.rows.map((r) => [r.label, r.count])).toEqual([
      ["正在使用", 4],
      ["刚改过", 2],
    ]);
    expect(skipHeadline(old)).toBe("跳过 6 个 · 正在使用");
  });
});

describe("完成页小字 doneRows", () => {
  const z = { count: 0, bytes: 0 };
  it("只列非零的：直接删除 / 暂存到 D 盘 / 回收站 · 还没腾出 / 跳过 / 没处理", () => {
    const rows = doneRows(
      {
        by_dest: { permanent: { count: 9, bytes: 28.1 * GB }, quarantine: { count: 2, bytes: 2 * GB }, recycle: { count: 1, bytes: 0.4 * GB } },
        ok_bytes: 30.5 * GB,
        skipped: 565,
        failed: 0,
        not_processed: { count: 2624, bytes: GB },
        skipped_by_reason: { hot: { count: 500, bytes: 0.1 * GB }, locked: { count: 65, bytes: 0.6 * GB } },
      },
      "D 盘",
    );
    expect(rows.map((r) => `${r.label} ${r.value}${r.note ? " · " + r.note : ""}`)).toEqual([
      "直接删除 28.10 GB · 9 个".replace("28.10", "28.1"),
      "暂存到 D 盘 2.00 GB · 2 个",
      "回收站 410 MB · 还没腾出",
      "跳过 565 个 · 正在使用",
      "没处理 2,624 个",
    ]);
  });
  it("回收站没收的单独一行（不算进直接删除）", () => {
    const rows = doneRows({
      by_dest: { permanent: z, quarantine: z, recycle: { count: 20, bytes: GB }, nuked: { count: 1, bytes: 4096 } },
      ok_bytes: GB,
      skipped: 0,
      failed: 0,
    });
    expect(rows.map((r) => [r.key, r.label, r.note])).toEqual([
      ["recycle", "回收站", "还没腾出"],
      ["nuked", "回收站没收", "找不回"],
    ]);
  });
  it("全是零：一行都没有", () => {
    expect(doneRows({ by_dest: { permanent: z, quarantine: z, recycle: z }, ok_bytes: 0, skipped: 0, failed: 0 })).toEqual([]);
  });
});

describe("上次中断 / 盘名 / 文楷缺字", () => {
  it("一行 + 原因", () => {
    expect(interruptedLine({ processed: 323851 })).toBe("上次清理中断 · 已处理 323,851 个");
    expect(interruptedCause({ exit_code: "0x8f", crash_logged: false })).toBe("被结束");
    expect(interruptedCause({ exit_code: "0x8f", crash_logged: true })).toBe("程序出错，已记录");
    expect(interruptedCause({ exit_code: null })).toBe("原因不明");
  });
  it("时间：今天 / 某月某日", () => {
    const now = new Date(2026, 9, 6, 12, 0);
    expect(whenText(new Date(2026, 9, 6, 5, 52).getTime() / 1000, now)).toBe("今天 05:52");
    expect(whenText(new Date(2026, 9, 5, 23, 1).getTime() / 1000, now)).toBe("10 月 5 日 23:01");
  });
  it("从路径 / 标签里取盘名", () => {
    expect(driveOf("暂存区（D:\\）")).toBe("D 盘");
    expect(driveOf("e:\\备份")).toBe("E 盘");
    expect(driveOf("回收站")).toBeUndefined();
  });
  it("暂存区的真实位置：盘根是默认，别的是以前选过的文件夹", () => {
    expect(isDriveRoot("D:\\")).toBe(true);
    expect(isDriveRoot("e:")).toBe(true);
    expect(isDriveRoot("D:\\备份")).toBe(false);
    expect(isDriveRoot("")).toBe(false);
    expect(stashFolder("D:\\备份\\")).toBe("D:\\备份\\DiskWhy暂存区");
    expect(stashFolder("D:\\")).toBe("D:\\DiskWhy暂存区");
  });
  it("文楷子集已重做：界面文案都能用标题字体", () => {
    expect(kaiSafe("清理完成")).toBe(true);
    expect(kaiSafe("暂存区")).toBe(true);
    expect(kaiSafe("AI 识别")).toBe(true);
  });
});
