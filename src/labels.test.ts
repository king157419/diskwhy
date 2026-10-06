import { describe, expect, it } from "vitest";
import type { LoseKind } from "./api";
import { advice, checkLine, cleanCat, clipWidth, driveName, isHardTag, LOSE_LABEL, LOSE_ORDER, loseLabel, loseTag, worstLose } from "./labels";

describe("后果标签 lose_kind → 中文", () => {
  it("八种都有固定的标签", () => {
    const want: Record<LoseKind, string> = {
      irreversible: "找不回来",
      repair: "以后修不了",
      unsure: "可能有你的东西",
      reinstall: "删了要重装",
      relogin: "要重新登录",
      feature: "少一个功能",
      redownload: "要重新下载",
      slower: "下次会慢一点",
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

describe("预检进度一行（阶段 G）", () => {
  const base = { dirs: 0, entries: 0, checked: 0, to_check: 0, locked: 0, rm_done: 0, rm_total: 0 };
  it("各阶段的短字", () => {
    expect(checkLine(null).text).toBe("正在检查");
    expect(checkLine({ ...base, phase: "list", entries: 460723 }).text).toBe("正在列出文件 460,723");
    expect(checkLine({ ...base, phase: "check", checked: 1200, to_check: 396065 }).text).toBe("正在检查 1,200 / 396,065");
    expect(checkLine({ ...base, phase: "who", rm_done: 3, rm_total: 30 }).text).toBe("正在查占用的程序 3 / 30");
  });
  it("进度条单调往前、不超过 100", () => {
    const seq = [
      checkLine(null).pct,
      checkLine({ ...base, phase: "list" }).pct,
      checkLine({ ...base, phase: "check", checked: 0, to_check: 10 }).pct,
      checkLine({ ...base, phase: "check", checked: 10, to_check: 10 }).pct,
      checkLine({ ...base, phase: "who" }).pct,
      checkLine({ ...base, phase: "done" }).pct,
    ];
    for (let i = 1; i < seq.length; i++) expect(seq[i]).toBeGreaterThanOrEqual(seq[i - 1]);
    expect(Math.max(...seq)).toBeLessThanOrEqual(100);
    expect(checkLine({ ...base, phase: "check", checked: 0, to_check: 0 }).pct).toBe(5);
  });
});

describe("具体后果标签和建议（阶段 H）", () => {
  it("行上的标签优先用规则写的具体短语，没有再退回通用说法", () => {
    expect(loseTag({ lose_short: "不能休眠", lose_kind: "feature" })).toBe("不能休眠");
    expect(loseTag({ lose_short: "  ", lose_kind: "feature" })).toBe("少一个功能");
    expect(loseTag({ lose_short: null, lose_kind: "irreversible" })).toBe("找不回来");
    expect(loseTag({ lose_kind: null })).toBeNull();
    for (const k of LOSE_ORDER) expect([...LOSE_LABEL[k]].length).toBeLessThanOrEqual(8);
  });
  it("建议由档位 + 怎么清推出来，how_detail 是第二句", () => {
    expect(advice("safe", ["delete"]).head).toBe("可以删，会自动重建");
    expect(advice("review", ["delete"]).head).toBe("看一眼里面，确定不要了再删");
    expect(advice("review", ["migrate", "delete"]).head).toBe("看一眼里面，确定不要了再删");
    expect(advice("review", ["manual"]).head).toBe("自己打开看看，手动删");
    expect(advice("review", ["builtin_tool"], "", "windows-system").head).toBe("别直接删，用 Windows 自带工具清");
    expect(advice("review", ["builtin_tool"], "", "cn-apps").head).toBe("别直接删，用软件自带的清理");
    expect(advice("review", ["migrate"]).head).toBe("别删，搬到 D 盘更好");
    expect(advice("keep", ["migrate"]).head).toBe("别删，搬到 D 盘更好");
    expect(advice("review", ["none"]).head).toBe("别动");
    expect(advice("keep", ["none"]).head).toBe("别动");
    expect(advice("safe", ["delete"], "  关掉 Chrome 后删  ").detail).toBe("关掉 Chrome 后删");
    expect(advice("safe", ["delete"]).detail).toBe("");
  });
});

describe("driveName（盘名从扫描根推导）", () => {
  it("盘根和文件夹都取盘符", () => {
    expect(driveName("C:\\")).toBe("C 盘");
    expect(driveName("d:\\")).toBe("D 盘");
    expect(driveName("E:\\Games\\x")).toBe("E 盘");
  });
  it("认不出盘符", () => {
    expect(driveName("")).toBe("磁盘");
    expect(driveName(null)).toBe("磁盘");
    expect(driveName("\\\\server\\share")).toBe("磁盘");
  });
});
