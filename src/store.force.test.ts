// 清理页勾选（2026-11 修订）：不能直接删的也能勾（二次确认后强制，只进回收站 / 暂存区），只有硬拦的不能勾；
// 全选连强制的一起（问一次）；已选只有一个数（GB）；暂存区建不了时默认流程不出现授权，悄悄改回收站；换去向悄悄重新检查。
import { beforeEach, describe, expect, it, vi } from "vitest";

const GB = 1024 ** 3;
const prep = vi.fn(async (drive: string) => ({ status: "ok", dest_root: drive }) as unknown);
const rep = (strategy: unknown, safeDirect: boolean, pending = false) => ({
  plan_id: "p",
  strategy,
  safe_direct: safeDirect,
  items: [],
  total_entries: 0,
  total_bytes: 0,
  total_skipped: 0,
  warnings: [],
  locked_processes: [],
  locked_count: 0,
  hot_count: 0,
  cloud_count: 0,
  link_count: 0,
  review_items: [],
  can_execute: true,
  quarantine_choices: [],
  timing_ms: {},
  pending,
  est_secs: 3,
});
type OnEvent = (e: { kind: string; report?: unknown; progress?: unknown }) => void;
const preview = vi.fn(async (_items: unknown, strategy: unknown, on: OnEvent, safeDirect: boolean) => {
  on({ kind: "quote", report: rep(strategy, safeDirect, true) });
  return rep(strategy, safeDirect);
});

vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  prepareQuarantine: (d: string) => prep(d),
  previewPlan: (a: unknown, b: unknown, c: OnEvent, d: boolean) => preview(a, b, c, d),
  cancelPreview: vi.fn(async () => {}),
}));

const store = await import("./store");

const drive = (root: string, free: number) => ({ root, label: "", file_system: "NTFS", total_bytes: 500 * GB, free_bytes: free });
type P = { path: string; bytes: number; deletable: boolean; hard?: string | null };
const group = (key: string, tier: "safe" | "review", paths: P[]) => ({
  key,
  tier,
  name: key,
  rule_id: key,
  group: "dev-caches",
  category: "cache",
  lose_kind: null,
  lose_short: null,
  how: [],
  bytes: paths.reduce((a, p) => a + p.bytes, 0),
  deletable_bytes: paths.filter((p) => p.deletable).reduce((a, p) => a + p.bytes, 0),
  paths,
});

function setup(root = "C:\\") {
  store.summary.value = {
    root,
    candidates: [
      group("safe1", "safe", [{ path: root + "cache", bytes: 1 * GB, deletable: true }]),
      // 一部分能直接删、一部分要手动（强制）
      group("mixed", "safe", [
        { path: root + "a", bytes: 2 * GB, deletable: true },
        { path: root + "b", bytes: 3 * GB, deletable: false, hard: null },
      ]),
      // 整组只能手动删（以前是虚线框不能勾）
      group("manual", "review", [{ path: root + "m", bytes: 4 * GB, deletable: false, hard: null }]),
      // 硬拦：保护路径
      group("prot", "safe", [{ path: root + "Windows\\x", bytes: 5 * GB, deletable: false, hard: "系统目录" }]),
    ],
  } as never;
  store.env.value = { system_drive: "C:\\" } as never;
  store.actEnv.value = { drives: [drive("C:\\", 20 * GB), drive("D:\\", 40 * GB)] } as never;
  store.resetSelection();
}
const g = (k: string) => store.candidates.value.find((x) => x.key === k)!;

beforeEach(() => {
  prep.mockImplementation(async (d: string) => ({ status: "ok", dest_root: d }));
  store.forgetDestChoices();
  store.closeReport();
  prep.mockClear();
  preview.mockClear();
  setup();
});

describe("勾选：不能直接删的二次确认后也能勾，硬拦的不能", () => {
  it("默认：可放心清理里能直接删的勾上（只算能直接删的部分），强制部分不算", () => {
    expect([...store.checked.value].sort()).toEqual(["mixed", "safe1"]);
    expect(store.checkState(g("mixed"))).toBe("part");
    expect(store.checkedBytes.value).toBe(3 * GB);
  });

  it("硬拦的不能勾，带短原因；能勾的没有原因", () => {
    expect(store.canCheck(g("prot"))).toBe(false);
    expect(store.hardWhy(g("prot"))).toBe("系统目录");
    expect(store.canCheck(g("manual"))).toBe(true);
    expect(store.hardWhy(g("manual"))).toBeNull();
    store.toggleGroup("prot");
    expect(store.forceAsk.value).toBeNull();
    expect(store.checked.value.has("prot")).toBe(false);
  });

  it("勾只能手动删的：先问；「仍要删」后勾上，清单里带 forced", () => {
    store.toggleGroup("manual");
    expect(store.checked.value.has("manual")).toBe(false);
    expect(store.forceAsk.value?.keys).toEqual(["manual"]);
    expect(store.forceAsk.value?.plainKeys).toEqual([]);
    store.answerForce("all");
    expect(store.checkState(g("manual"))).toBe("on");
    expect(store.checkedBytes.value).toBe(7 * GB);
    expect(store.planItems()).toContainEqual({ path: "C:\\m", expect_rule: "manual", forced: true });
    // 再点一下：取消（不再问）
    store.toggleGroup("manual");
    expect(store.forceAsk.value).toBeNull();
    expect(store.checked.value.has("manual")).toBe(false);
  });

  it("「算了」：什么都不变", () => {
    store.toggleGroup("manual");
    store.answerForce(null);
    expect(store.checked.value.has("manual")).toBe(false);
    expect(store.checkedBytes.value).toBe(3 * GB);
  });

  it("全选：问一次；「仍要删」连强制的一起勾满，硬拦的除外；再点全取消", () => {
    store.setGroups(store.candidates.value.map((x) => x.key), true);
    expect(store.forceAsk.value?.keys.sort()).toEqual(["manual", "mixed"]);
    store.answerForce("all");
    expect(store.checkedBytes.value).toBe(10 * GB);
    expect(["safe1", "mixed", "manual"].every((k) => store.checkState(g(k)) === "on")).toBe(true);
    expect(store.checked.value.has("prot")).toBe(false);
    const items = store.planItems();
    expect(items.filter((i) => i.forced).map((i) => i.path).sort()).toEqual(["C:\\b", "C:\\m"]);
    expect(items.some((i) => i.path.includes("Windows"))).toBe(false);
    store.setGroups(store.candidates.value.map((x) => x.key), false);
    expect(store.checkedBytes.value).toBe(0);
  });

  it("全选选「只选能删的」：能直接删的部分照常勾，强制的不算", () => {
    store.checked.value = new Set();
    store.setGroups(["safe1", "mixed", "manual"], true);
    store.answerForce("plain");
    expect([...store.checked.value].sort()).toEqual(["mixed", "safe1"]);
    expect(store.checkedBytes.value).toBe(3 * GB);
    expect(store.planItems().some((i) => i.forced)).toBe(false);
  });

  it("「看看都是什么」里加一个认不出来的文件夹：先问，确认后在清单里、带 forced；「算了」就拿掉", () => {
    const node = { name: "mystery", path: "C:\\mystery", kind: "Dir", allocated: 6 * GB } as never;
    store.addPathToCleanup(node);
    expect(store.forceAsk.value?.keys).toEqual(["x:C:\\mystery"]);
    store.answerForce(null);
    expect(store.candidates.value.some((x) => x.key === "x:C:\\mystery")).toBe(false);
    store.addPathToCleanup(node);
    store.answerForce("all");
    expect(g("x:C:\\mystery").tier).toBe("keep");
    expect(store.groupCat(g("x:C:\\mystery"))).toBe("mine");
    expect(store.planItems()).toContainEqual({ path: "C:\\mystery", expect_rule: null, forced: true });
    // 已经是候选的路径：直接勾那一组
    store.addPathToCleanup({ name: "m", path: "C:\\m", kind: "Dir", allocated: 4 * GB } as never);
    expect(store.forceAsk.value?.keys).toEqual(["manual"]);
  });
});

describe("暂存区：默认流程里不出现授权", () => {
  it("D 盘盘根要管理员才能建 → 默认悄悄改回收站，不弹授权；底栏估算也不再按暂存算", async () => {
    store.toggleGroup("manual");
    store.answerForce("all");
    prep.mockImplementation(async () => ({ status: "needs_elevation" }));
    expect(store.freesByDefault.value).toBe(3 * GB + 4 * GB);
    await store.doPreview();
    expect(store.dest.value).toMatchObject({ mode: "recycle", explicit: false });
    expect(preview.mock.lastCall?.[1]).toEqual({ kind: "recycle_bin" });
    // 可放心清理照样直接删（不是用户明确选的回收站）
    expect(preview.mock.lastCall?.[3]).toBe(true);
    // 这个盘记成建不了：底栏的「剩余 → 」只算直接删的
    expect(store.freesByDefault.value).toBe(1 * GB + 2 * GB);
  });

  it("强制加入的按「需你判断」的去向走（要解析暂存区）", async () => {
    store.toggleGroup("mixed");
    store.answerForce("all");
    await store.doPreview();
    expect(prep).toHaveBeenCalledWith("D:\\");
    expect(store.dest.value).toMatchObject({ mode: "stash", drive: "D:\\", status: "ok" });
  });

  it("换去向：悄悄重新检查，弹层不回到“正在检查”", async () => {
    store.toggleGroup("manual");
    store.answerForce("all");
    await store.doPreview();
    expect(store.report.value?.pending).toBe(false);
    let seenPending = false;
    let seenRecheck = false;
    preview.mockImplementationOnce(async (_i, strategy, on, safeDirect) => {
      on({ kind: "quote", report: rep(strategy, safeDirect, true) });
      seenPending = !!store.report.value?.pending;
      seenRecheck = store.rechecking.value;
      return rep(strategy, safeDirect);
    });
    await store.chooseDest("recycle");
    expect(seenPending).toBe(false);
    expect(seenRecheck).toBe(true);
    expect(store.rechecking.value).toBe(false);
    expect(store.report.value?.strategy).toEqual({ kind: "recycle_bin" });
  });
});
