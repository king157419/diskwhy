<script setup lang="ts">
import { computed } from "vue";
import type { RuleBytes, TierKey, TierSummary } from "../api";
import { humanBytes, pct } from "../format";

const props = defineProps<{ tiers: TierSummary; usedBytes: number }>();

function top(list: RuleBytes[]): string {
  return list
    .slice(0, 3)
    .map((r) => `${r.owner} ${humanBytes(r.bytes)}`)
    .join(" · ");
}

const cells = computed<{ key: TierKey; label: string; bytes: number; top: string; note: string }[]>(() => [
  { key: "safe", label: "可以放心清理", bytes: props.tiers.safe_bytes, top: top(props.tiers.top_safe), note: "缓存、临时文件、日志等，删了会自动重建" },
  { key: "review", label: "需要你来判断", bytes: props.tiers.review_bytes, top: top(props.tiers.top_review), note: "删了有代价，逐项看说明再决定" },
  { key: "keep", label: "建议保留", bytes: props.tiers.keep_bytes, top: top(props.tiers.top_keep), note: "系统和你的数据；可迁移或用系统工具" },
  { key: "unknown", label: "规则还没覆盖", bytes: props.tiers.uncovered_bytes, top: "", note: "列表里会用启发式推测，或显示为未知" },
]);
</script>

<template>
  <section class="tiers card" aria-label="本次扫描能腾多少">
    <div class="head">
      <div>
        <div class="card-title">本次能腾多少</div>
        <div
          class="muted small"
          title="按规则库把去重后的每个字节归到一档；硬链接和商店应用目录映射只算一次。量不到的“无权限”部分不在这四项里。"
        >
          按规则库归档 · 命中 <span class="num">{{ tiers.rules_hit }}/{{ tiers.rules_total }}</span> 条规则
        </div>
      </div>
    </div>
    <div class="hero">
      <div class="hero-num num">{{ humanBytes(tiers.safe_bytes) }}</div>
      <div class="hero-text">
        <div><span class="dot dot-safe"></span> 可以放心清理，删了会自动重建</div>
        <div class="muted small">
          另有 <b class="num">{{ humanBytes(tiers.review_bytes) }}</b> 删了有代价，需要你逐项判断
        </div>
      </div>
    </div>
    <div class="cells">
      <div v-for="c in cells" :key="c.key" class="cell" :class="'c-' + c.key" :title="c.note">
        <div class="cell-label"><i class="dot" :class="'dot-' + c.key"></i>{{ c.label }}</div>
        <div class="cell-bytes num">{{ humanBytes(c.bytes) }}</div>
        <div class="cell-bar"><i :style="{ width: Math.min(100, (c.bytes * 100) / (usedBytes || 1)) + '%' }"></i></div>
        <div class="cell-top" :title="c.top || c.note">
          <span class="num">{{ pct(c.bytes, usedBytes) }}</span> · {{ c.top || c.note }}
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.tiers {
  padding: 20px 22px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.head .small {
  margin-top: 2px;
}
.hero {
  display: flex;
  align-items: center;
  gap: 18px;
  padding: 16px 18px;
  border-radius: var(--r-md);
  background:
    radial-gradient(120% 140% at 0% 0%, var(--t-safe-bg), transparent 60%),
    var(--surface-2);
  border: 1px solid var(--border-1);
}
.hero-num {
  font-size: var(--fs-3xl);
  font-weight: 600;
  letter-spacing: -0.035em;
  line-height: 1;
  color: var(--t-safe-fg);
}
.hero-text {
  display: flex;
  flex-direction: column;
  gap: 3px;
  font-size: var(--fs-sm);
}
.hero-text .dot {
  margin-right: 4px;
  vertical-align: 1px;
}
.cells {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 10px;
}
.cell {
  padding: 12px 14px 12px;
  border-radius: var(--r-md);
  background: var(--surface-2);
  border: 1px solid var(--border-1);
  min-width: 0;
  transition:
    border-color var(--dur-1) var(--ease),
    background var(--dur-1) var(--ease);
}
.cell:hover {
  border-color: var(--border-2);
  background: var(--surface-3);
}
.cell-label {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  white-space: nowrap;
}
.cell-bytes {
  margin-top: 6px;
  font-size: var(--fs-xl);
  font-weight: 600;
  letter-spacing: -0.02em;
}
.cell-bar {
  margin: 8px 0 7px;
  height: 3px;
  border-radius: 2px;
  background: var(--surface-inset);
  overflow: hidden;
}
.cell-bar i {
  display: block;
  height: 100%;
  border-radius: inherit;
}
.c-safe .cell-bar i {
  background: var(--t-safe);
}
.c-review .cell-bar i {
  background: var(--t-review);
}
.c-keep .cell-bar i {
  background: var(--t-keep);
}
.c-unknown .cell-bar i {
  background: var(--t-unknown);
}
.cell-top {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
