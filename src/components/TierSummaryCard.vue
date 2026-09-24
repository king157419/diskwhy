<script setup lang="ts">
// 全盘各档合计（按规则库归档）。第 1 轮评审后：
// - 标题改成“这些空间分别是什么”（保留的那部分不能腾，原来的“能腾多少”和内容矛盾）；
// - 副标题写具体的文件夹名（规则命中的最大一处），不再是会重名的软件名，放不下就换行；
// - 点卡片：列表只看这一档（再点一次取消）。
import { computed } from "vue";
import type { RuleBytes, TierKey, TierSummary } from "../api";
import { TIER_HINT, TIER_SHORT, humanBytes } from "../format";
import TierChip from "./TierChip.vue";

const props = defineProps<{ tiers: TierSummary; usedBytes: number; filter: TierKey | null }>();
const emit = defineEmits<{ filter: [t: TierKey | null] }>();

function lastSeg(p: string | undefined): string {
  if (!p) return "";
  const s = p.replace(/\\+$/, "").split("\\");
  return s[s.length - 1] || p;
}

function top(list: RuleBytes[]): string {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const r of list) {
    const name = r.id.startsWith("guard:") ? r.what : lastSeg(r.top_path) || r.owner;
    if (seen.has(name)) continue;
    seen.add(name);
    out.push(`${name} ${humanBytes(r.top_path_bytes ?? r.bytes)}`);
    if (out.length >= 3) break;
  }
  return out.join("、");
}

const cells = computed(() => [
  {
    key: "safe" as TierKey,
    label: TIER_SHORT.safe,
    bytes: props.tiers.safe_bytes,
    top: top(props.tiers.top_safe),
    note: TIER_HINT.safe + ((props.tiers.safe_tool_bytes ?? 0) > 0 ? `；其中 ${humanBytes(props.tiers.safe_tool_bytes ?? 0)} 要用写明的工具清，别直接删文件夹` : ""),
  },
  { key: "review" as TierKey, label: TIER_SHORT.review, bytes: props.tiers.review_bytes, top: top(props.tiers.top_review), note: TIER_HINT.review },
  { key: "keep" as TierKey, label: TIER_SHORT.keep, bytes: props.tiers.keep_bytes, top: top(props.tiers.top_keep), note: TIER_HINT.keep },
  {
    key: "unknown" as TierKey,
    label: "规则库没覆盖",
    bytes: props.tiers.uncovered_bytes,
    top: "",
    note: "规则库里没有写到的地方。列表里会按内容推测（标“推测”），推测不出的标“未认出”；点这张卡片，列表只看“未认出”的项。",
  },
]);

function click(k: TierKey) {
  emit("filter", props.filter === k ? null : k);
}
</script>

<template>
  <section class="tiers card" aria-label="全盘各档合计">
    <div class="attr-head">
      <span class="attr-title">这些空间分别是什么</span>
      <span class="muted small" title="按规则库把去重后的每个字节归到一档；硬链接和商店应用目录映射只算一次。量不到的“无权限”部分不在这四项里。">
        按规则库归档（命中 {{ tiers.rules_hit }}/{{ tiers.rules_total }} 条规则）· 点卡片只看这一档
      </span>
    </div>
    <div class="tier-cells">
      <button
        v-for="c in cells"
        :key="c.key"
        type="button"
        class="tier-cell"
        :class="['tc-' + c.key, { 'tc-on': filter === c.key, 'tc-dim': filter && filter !== c.key }]"
        :title="c.note + (c.top ? '\n最大的几处：' + c.top : '')"
        :aria-pressed="filter === c.key"
        @click="click(c.key)"
      >
        <div class="tc-label"><TierChip :tier="c.key" :text="c.label" /></div>
        <div class="tc-bytes">{{ humanBytes(c.bytes) }}</div>
        <div class="tc-top muted small">{{ c.top ? "最大：" + c.top : c.note }}</div>
      </button>
    </div>
    <p v-if="(tiers.guarded_bytes ?? 0) > 0" class="tiers-foot muted small">
      其中 {{ humanBytes(tiers.guarded_bytes ?? 0) }} 在缓存 / 临时文件夹里、但看起来是你的数据（聊天记录、文档、代码、虚拟磁盘等），已经从“可清理”里拿出来，记在“需判断”或“保留”。
    </p>
  </section>
</template>
