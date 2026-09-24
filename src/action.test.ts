import { describe, expect, it } from "vitest";
import { confirmMatches, gbText } from "./action";
import { cartEligibility, type Explanation, type NodeView } from "./api";

const GB = 1024 ** 3;

function node(ex: Partial<Explanation> | null, extra: Partial<NodeView> = {}): NodeView {
  const e: Explanation | undefined = ex
    ? {
        tier: "safe",
        owner: null,
        category: "cache",
        what: "x",
        why: "",
        lose: "",
        how: ["delete"],
        how_detail: "",
        gain_bytes: 0,
        gain_note: "",
        source: { kind: "rule", id: "r", group: "g", inherited_from: null },
        confidence: 1,
        refs: [],
        verified_on: null,
        notes: [],
        clues: [],
        ...ex,
      }
    : undefined;
  return {
    name: "n",
    path: "C:\\x\\n",
    kind: "Dir",
    logical: 0,
    allocated: 0,
    mtime: 0,
    attrs: 0,
    reparse_tag: 0,
    inaccessible: false,
    error_code: 0,
    reparse_skipped: false,
    cloud_placeholder: false,
    system_reserved: false,
    files: 0,
    dirs: 0,
    child_count: 0,
    percent_of_parent: 0,
    type_label: "",
    explanation: e,
    ...extra,
  };
}

describe("确认框：必须输入合计 GB 数", () => {
  it("两位小数一致才算确认", () => {
    expect(gbText(1.234 * GB)).toBe("1.23");
    expect(confirmMatches("1.23", 1.234 * GB)).toBe(true);
    expect(confirmMatches("1.23 GB", 1.234 * GB)).toBe(true);
    expect(confirmMatches("1.2", 1.234 * GB)).toBe(false);
    expect(confirmMatches("", 1.234 * GB)).toBe(false);
    expect(confirmMatches("abc", 1.234 * GB)).toBe(false);
    expect(confirmMatches("0.00", 3 * 1024 * 1024)).toBe(true);
  });
});

describe("加入清理清单的按钮只给 safe / review 且可以直接删的项", () => {
  it("safe / review 可以", () => {
    expect(cartEligibility(node({ tier: "safe" })).ok).toBe(true);
    expect(cartEligibility(node({ tier: "review", how: ["migrate", "delete"] })).ok).toBe(true);
  });
  it("keep、未知、AI 推测、只能用工具的不行", () => {
    expect(cartEligibility(node({ tier: "keep", how: ["migrate"] }))).toEqual({ ok: false, reason: "keep" });
    expect(cartEligibility(node({ tier: "review", source: { kind: "unknown" } }))).toEqual({ ok: false, reason: "unknown" });
    expect(cartEligibility(node({ tier: "review", source: { kind: "llm", model: "m" } })).ok).toBe(false);
    expect(cartEligibility(node({ tier: "safe", how: ["builtin_tool"] }))).toEqual({ ok: false, reason: "tool" });
  });
  it("系统保留 / 映射 / 无权限 / 其余小文件 / 归属色块 不行", () => {
    expect(cartEligibility(node({}, { system_reserved: true })).ok).toBe(false);
    expect(cartEligibility(node({}, { mapped: true })).ok).toBe(false);
    expect(cartEligibility(node({}, { inaccessible: true })).ok).toBe(false);
    expect(cartEligibility(node({}, { kind: "Rest" })).ok).toBe(false);
    expect(cartEligibility(node({}, { pseudo: "inacc" })).ok).toBe(false);
    expect(cartEligibility(null).ok).toBe(false);
  });
});
