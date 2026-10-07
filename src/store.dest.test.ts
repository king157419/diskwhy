// 去向记忆（阶段 J 修订）：用户在下拉里明确选过的去向按源盘记住，下次打开弹层还用它（仍然能用时）。后端命令用假的。
import { beforeEach, describe, expect, it, vi } from "vitest";

const GB = 1024 ** 3;
const prep = vi.fn(async (drive: string) => ({ status: "ok", dest_root: drive }));
const preview = vi.fn(async (_items: unknown, strategy: unknown, _on: unknown, safeDirect: boolean) => ({
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
}));

vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  prepareQuarantine: (d: string) => prep(d),
  previewPlan: (a: unknown, b: unknown, c: unknown, d: boolean) => preview(a, b, c, d),
  cancelPreview: vi.fn(async () => {}),
}));

const store = await import("./store");

const drive = (root: string, free: number) => ({ root, label: "", file_system: "NTFS", total_bytes: 500 * GB, free_bytes: free });
const group = (key: string, tier: "safe" | "review", path: string) => ({
  key,
  tier,
  name: key,
  rule_id: key,
  group: "dev-caches",
  category: "cache",
  bytes: GB,
  deletable_bytes: GB,
  paths: [{ path, bytes: GB, deletable: true }],
});

function setup(root: string, keys: string[]) {
  store.summary.value = {
    root,
    candidates: [group("safe1", "safe", root + "cache"), group("rev1", "review", root + "old")],
  } as never;
  store.env.value = { system_drive: "C:\\" } as never;
  store.actEnv.value = { drives: [drive("C:\\", 20 * GB), drive("D:\\", 40 * GB), drive("E:\\", 80 * GB)] } as never;
  store.checked.value = new Set(keys);
}

beforeEach(() => {
  store.forgetDestChoices();
  store.closeReport();
  prep.mockClear();
  preview.mockClear();
});

describe("去向记忆（按源盘，只在这次运行里）", () => {
  it("扫 D 盘、默认回收站：明确选了 E 盘暂存区，下次打开还是它", async () => {
    // D 盘的默认暂存盘只有 E（C 是系统盘不当默认）；先让 E 不够大 → 默认回收站
    setup("D:\\", ["safe1", "rev1"]);
    store.actEnv.value = { drives: [drive("C:\\", 20 * GB), drive("D:\\", 40 * GB), drive("E:\\", 0.5 * GB)] } as never;
    await store.doPreview();
    expect(store.dest.value.mode).toBe("recycle");
    expect(store.dest.value.explicit).toBe(false);

    await store.chooseDest("stash", "C:\\");
    expect(store.dest.value).toMatchObject({ mode: "stash", drive: "C:\\", status: "ok", explicit: true });
    store.closeReport();

    await store.doPreview();
    expect(store.dest.value).toMatchObject({ mode: "stash", drive: "C:\\", dest: "C:\\", status: "ok", explicit: true });
    expect(preview.mock.lastCall?.[1]).toEqual({ kind: "quarantine", dest_root: "C:\\" });
  });

  it("明确选回收站：下次还是回收站，可放心清理的也进回收站", async () => {
    setup("C:\\", ["safe1", "rev1"]);
    await store.doPreview();
    expect(store.dest.value.mode).toBe("stash"); // C 盘的默认：E 盘暂存区（剩余多的在前）
    await store.chooseDest("recycle");
    store.closeReport();
    await store.doPreview();
    expect(store.dest.value).toMatchObject({ mode: "recycle", explicit: true });
    expect(preview.mock.lastCall?.[1]).toEqual({ kind: "recycle_bin" });
    expect(preview.mock.lastCall?.[3]).toBe(false);
  });

  it("按源盘分开记：C 盘的选择不影响 D 盘", async () => {
    setup("C:\\", ["safe1", "rev1"]);
    await store.chooseDest("permanent");
    expect(store.rememberedDest("C:\\")?.mode).toBe("permanent");
    expect(store.rememberedDest("D:\\")).toBeNull();
    setup("D:\\", ["safe1", "rev1"]);
    await store.doPreview();
    expect(store.dest.value.explicit).toBe(false);
  });

  it("记住的暂存盘不能用了（空间不够 / 建不了）→ 回到默认", async () => {
    setup("C:\\", ["safe1", "rev1"]);
    await store.chooseDest("stash", "D:\\");
    store.closeReport();
    prep.mockImplementationOnce(async () => ({ status: "failed", message: "x" }) as never);
    await store.doPreview();
    expect(store.dest.value).toMatchObject({ mode: "stash", drive: "E:\\", explicit: false });
  });

  it("只有可放心清理时选「直接删除」= 默认，不记（免得下次需你判断的也直接删）", async () => {
    setup("D:\\", ["safe1"]);
    await store.chooseDest("recycle");
    expect(store.rememberedDest("D:\\")?.mode).toBe("recycle");
    await store.chooseDest("permanent");
    expect(store.rememberedDest("D:\\")).toBeNull();
  });

  it("其他文件夹：记住路径，没选（取消）不记", async () => {
    setup("D:\\", ["safe1", "rev1"]);
    await store.chooseDest("folder");
    expect(store.rememberedDest("D:\\")).toBeNull();
    await store.chooseDest("folder", "E:\\备份");
    store.closeReport();
    await store.doPreview();
    expect(store.dest.value).toMatchObject({ mode: "folder", dest: "E:\\备份", status: "ok", explicit: true });
  });
});
