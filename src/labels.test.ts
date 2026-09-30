import { describe, expect, it } from "vitest";
import type { LoseKind } from "./api";
import { cleanCat, clipWidth, isHardTag, LOSE_LABEL, LOSE_ORDER, loseLabel, worstLose } from "./labels";

describe("后果标签 lose_kind → 中文", () => {
  it("八种都有固定的标签", () => {
    const want: Record<LoseKind, string> = {
      irreversible: "找不回来",
      repair: "影响修复",
      unsure: "需你确认",
      reinstall: "要重装",
      relogin: "要重登录",
      feature: "功能变少",
      redownload: "要重下载",
      slower: "删了变慢",
    };
    expect(LOSE_LABEL).toEqual(want);
    for (const k of LOSE_ORDER) expect(loseLabel(k)).toBe(want[k]);
    expect(loseLabel(null)).toBeNull();
    expect(loseLabel(undefined)).toBeNull();
  });

  it("只有“找不回来”是实心标签", () => {
    expect(isHardTag("irreversible")).toBe(true);
    for (const k of LOSE_ORDER.filter((x) => x !== "irreversible")) expect(isHardTag(k)).toBe(false);
  });

  it("多个后果取最坏的（找不回来 > 影响修复 > … > 删了变慢）", () => {
    expect(worstLose(["slower", "redownload", "relogin"])).toBe("relogin");
    expect(worstLose(["unsure", "repair"])).toBe("repair");
    expect(worstLose(["slower", null, "irreversible"])).toBe("irreversible");
    expect(worstLose([null, undefined])).toBeNull();
  });
});

describe("清理页分类", () => {
  it("按规则分组归类，模型一律算 AI 模型", () => {
    expect(cleanCat("windows-system", "cache")).toBe("sys");
    expect(cleanCat("browsers", "cache")).toBe("browser");
    expect(cleanCat("browsers", "model")).toBe("ai");
    expect(cleanCat("heuristic", "build")).toBe("dev");
    expect(cleanCat("cn-apps", "log")).toBe("app");
    expect(cleanCat("cn-apps", "userdata")).toBe("data");
    expect(cleanCat("misc", "installer")).toBe("app");
  });
  it("按显示宽度截断", () => {
    expect(clipWidth("微信聊天数据", 12)).toBe("微信聊天数据");
    expect(clipWidth("微信聊天数据很多", 12)).toBe("微信聊天数据…");
    expect(clipWidth("abc", 2)).toBe("ab…");
  });
});

import { shortPath } from "./labels";
describe("路径缩写", () => {
  it("用户目录缩成环境变量", () => {
    expect(shortPath(String.raw`C:\Users\king\AppData\Local\npm-cache`)).toBe(String.raw`%LOCALAPPDATA%\npm-cache`);
    expect(shortPath(String.raw`C:\Users\king\AppData\Roaming\Code\Cache`)).toBe(String.raw`%APPDATA%\Code\Cache`);
    expect(shortPath(String.raw`C:\Users\king\.m2\repository`)).toBe(String.raw`%USERPROFILE%\.m2\repository`);
    expect(shortPath(String.raw`C:\ProgramData\Package Cache`)).toBe(String.raw`%ProgramData%\Package Cache`);
    expect(shortPath(String.raw`C:\Windows\Temp`)).toBe(String.raw`C:\Windows\Temp`);
  });
});
