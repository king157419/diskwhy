<script setup lang="ts">
import { computed } from "vue";
import type { RuleBytes, TierSummary } from "../api";
import { humanBytes } from "../format";

const props = defineProps<{ tiers: TierSummary; usedBytes: number }>();

function top(list: RuleBytes[]): string {
  return list
    .slice(0, 3)
    .map((r) => `${r.owner}·${humanBytes(r.bytes)}`)
    .join("、");
}

const cells = computed(() => [
  { key: "safe", label: "可以放心清理", bytes: props.tiers.safe_bytes, top: top(props.tiers.top_safe), note: "缓存、临时文件、日志等，删了会自动重建" },
  { key: "review", label: "需要你来判断", bytes: props.tiers.review_bytes, top: top(props.tiers.top_review), note: "删了有代价，逐项看说明再决定" },
  { key: "keep", label: "建议保留", bytes: props.tiers.keep_bytes, top: top(props.tiers.top_keep), note: "系统和你的数据；可迁移或用系统工具" },
  { key: "unknown", label: "规则还没覆盖", bytes: props.tiers.uncovered_bytes, top: "", note: "列表里会用启发式推测，或显示为未知" },
]);
</script>

<template>
  <section class="tiers card" aria-label="本次扫描各档合计">
    <div class="attr-head">
      <span class="attr-title">本次扫描能腾多少</span>
      <span class="muted small" title="按规则库把去重后的每个字节归到一档；硬链接和商店应用目录映射只算一次。量不到的“无权限”部分不在这四项里。">按规则库归档，命中 {{ tiers.rules_hit }}/{{ tiers.rules_total }} 条规则</span>
    </div>
    <div class="tier-cells">
      <div v-for="c in cells" :key="c.key" class="tier-cell" :class="'tc-' + c.key" :title="c.note">
        <div class="tc-label"><i class="dot" :class="'dot-' + c.key"></i>{{ c.label }}</div>
        <div class="tc-bytes">{{ humanBytes(c.bytes) }}</div>
        <div class="tc-top muted small">{{ c.top || c.note }}</div>
      </div>
    </div>
  </section>
</template>
