import { describe, expect, it } from "vitest";
import { afterOffset, gbNum, layoutHorizontal, layoutLabels, layoutSegments, sizeParts } from "./capacity";

const GB = 1024 ** 3;

describe("layoutSegments", () => {
  it("按比例分配，合计正好等于总高（含缝）", () => {
    const s = layoutSegments(
      [
        { key: "free", bytes: 14 * GB },
        { key: "safe", bytes: 32 * GB },
        { key: "review", bytes: 50 * GB },
        { key: "other", bytes: 103 * GB },
      ],
      572,
    );
    const last = s[s.length - 1];
    expect(last.top + last.height).toBe(572);
    expect(s.reduce((a, x) => a + x.height, 0)).toBe(572 - 3 * 2);
    // 顺序从上到下、互不重叠
    for (let i = 1; i < s.length; i++) expect(s[i].top).toBeGreaterThanOrEqual(s[i - 1].top + s[i - 1].height);
    expect(s[3].height).toBeGreaterThan(s[2].height);
  });

  it("很小的段至少 minSeg 像素", () => {
    const s = layoutSegments(
      [
        { key: "free", bytes: 0.3 * GB },
        { key: "safe", bytes: 0.2 * GB },
        { key: "review", bytes: 40 * GB },
        { key: "other", bytes: 150 * GB },
      ],
      572,
      14,
    );
    expect(s[0].height).toBeGreaterThanOrEqual(14);
    expect(s[1].height).toBeGreaterThanOrEqual(14);
    expect(s.reduce((a, x) => a + x.height, 0)).toBe(572 - 3 * 2);
  });

  it("字节为 0 的段不画，也不占缝", () => {
    const s = layoutSegments(
      [
        { key: "free", bytes: 10 * GB },
        { key: "safe", bytes: 0 },
        { key: "other", bytes: 90 * GB },
      ],
      100,
      14,
      2,
    );
    expect(s[1].height).toBe(0);
    expect(s[0].height + s[2].height).toBe(98);
    expect(s[2].top).toBe(s[0].height + 2);
  });
});

describe("layoutLabels（标签防重叠）", () => {
  it("离得够远时就放在段中点，不画引线", () => {
    const l = layoutLabels([20, 120, 300], 34, 0, 572);
    expect(l.map((x) => x.y)).toEqual([20, 120, 300]);
    expect(l.every((x) => !x.leader)).toBe(true);
  });

  it("挤在一起时上下推开，相邻至少 gap，并画引线", () => {
    const l = layoutLabels([8, 20, 30, 400], 34, 0, 572);
    for (let i = 1; i < l.length; i++) expect(l[i].y - l[i - 1].y).toBeGreaterThanOrEqual(34);
    expect(l[0].y).toBeGreaterThanOrEqual(0);
    expect(l.some((x) => x.leader)).toBe(true);
    expect(l[3].y).toBe(400);
  });

  it("贴底的一串标签从底往上推，不越界", () => {
    const l = layoutLabels([560, 565, 570], 30, 0, 572);
    expect(l[2].y).toBeLessThanOrEqual(572);
    expect(l[1].y).toBeLessThanOrEqual(l[2].y - 30);
    expect(l[0].y).toBeLessThanOrEqual(l[1].y - 30);
  });

  it("放不下时均匀铺满", () => {
    const l = layoutLabels([0, 1, 2, 3], 100, 0, 150);
    expect(l[0].y).toBe(0);
    expect(l[3].y).toBe(150);
  });
});

describe("数字格式", () => {
  it("GB 数字和设计稿一致", () => {
    expect(gbNum(14 * GB)).toBe("14.0");
    expect(gbNum(103.4 * GB)).toBe("103");
    expect(gbNum(4.2 * GB)).toBe("4.2");
    expect(gbNum(0.5 * GB)).toBe("0.50");
  });
  it("行内大小：小于 1 GB 用 MB", () => {
    expect(sizeParts(1.55 * GB)).toEqual(["1.55", "GB"]);
    expect(sizeParts(380 * 1024 ** 2)).toEqual(["380", "MB"]);
    expect(sizeParts(27.4 * GB)).toEqual(["27.4", "GB"]);
  });
});

describe("afterOffset（清理后剩余水位）", () => {
  it("落在第二段中间", () => {
    const s = layoutSegments(
      [
        { key: "free", bytes: 10 * GB },
        { key: "safe", bytes: 10 * GB },
        { key: "other", bytes: 80 * GB },
      ],
      102,
      1,
      2,
    );
    // free 0..10, safe 12..22 → free + 一半 safe = 17
    expect(afterOffset(s, 15 * GB)).toBe(17);
  });
  it("没有或为 0 时不画", () => {
    const s = layoutSegments([{ key: "a", bytes: GB }], 100);
    expect(afterOffset(s, 0)).toBeNull();
    expect(afterOffset(s, undefined)).toBeNull();
  });
});

describe("layoutHorizontal（横放容量柱）", () => {
  const segs = [
    { key: "free", bytes: 14 * GB },
    { key: "safe", bytes: 23.7 * GB },
    { key: "review", bytes: 42.3 * GB },
    { key: "keep", bytes: 119 * GB },
  ];

  it("段从左到右铺满宽度，互不重叠", () => {
    const { segs: h } = layoutHorizontal(segs, 656, 116);
    const last = h[h.length - 1];
    expect(h[0].left).toBe(0);
    expect(last.left + last.width).toBe(656);
    for (let i = 1; i < h.length; i++) expect(h[i].left).toBeGreaterThanOrEqual(h[i - 1].left + h[i - 1].width);
  });

  it("标签框不重叠、整个在柱宽之内（720 宽窗口）", () => {
    const W = 656;
    const LW = 116;
    const { labels } = layoutHorizontal(segs, W, LW, { space: 8 });
    expect(labels).toHaveLength(4);
    for (const l of labels) {
      expect(l.left).toBeGreaterThanOrEqual(0);
      expect(l.left + LW).toBeLessThanOrEqual(W);
    }
    for (let i = 1; i < labels.length; i++) expect(labels[i].left).toBeGreaterThanOrEqual(labels[i - 1].left + LW + 8);
  });

  it("很窄的剩余段：标签被挤开并画引线，顺序不变", () => {
    const tiny = [
      { key: "free", bytes: 0.4 * GB },
      { key: "safe", bytes: 0.3 * GB },
      { key: "review", bytes: 30 * GB },
      { key: "keep", bytes: 160 * GB },
    ];
    const { segs: h, labels } = layoutHorizontal(tiny, 800, 116);
    expect(h[0].width).toBeGreaterThanOrEqual(14);
    expect(h[1].width).toBeGreaterThanOrEqual(14);
    expect(labels.map((l) => l.key)).toEqual(["free", "safe", "review", "keep"]);
    expect(labels[0].left).toBe(0);
    expect(labels.some((l) => l.leader)).toBe(true);
    for (let i = 1; i < labels.length; i++) expect(labels[i].x).toBeGreaterThan(labels[i - 1].x);
  });

  it("字节为 0 的段没有标签", () => {
    const { labels } = layoutHorizontal(
      [
        { key: "free", bytes: 10 * GB },
        { key: "safe", bytes: 0 },
        { key: "keep", bytes: 90 * GB },
      ],
      600,
      116,
    );
    expect(labels.map((l) => l.key)).toEqual(["free", "keep"]);
  });
});
