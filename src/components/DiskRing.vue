<script setup lang="ts">
// 概览页的磁盘环：整圈 = 卷总容量；已用部分按三档色标分段（可清理 / 需判断 / 保留 / 未覆盖 / 量不到），剩下的是可用空间。
import { computed } from "vue";
import type { ScanSummary } from "../api";
import { humanBytes, pct } from "../format";

const props = defineProps<{ summary: ScanSummary; size?: number }>();

const R = 54;
const C = 2 * Math.PI * R;

const segs = computed(() => {
  const s = props.summary;
  const t = s.tiers;
  const total = s.total_bytes || 1;
  const tiered = t.safe_bytes + t.review_bytes + t.keep_bytes + t.uncovered_bytes;
  const rest = Math.max(0, s.used_bytes - tiered);
  const list = [
    { key: "safe", label: "可清理", bytes: t.safe_bytes, color: "var(--t-safe)" },
    { key: "review", label: "需判断", bytes: t.review_bytes, color: "var(--t-review)" },
    { key: "keep", label: "保留", bytes: t.keep_bytes, color: "var(--t-keep)" },
    { key: "unknown", label: "规则未覆盖", bytes: t.uncovered_bytes, color: "var(--t-unknown)" },
    { key: "rest", label: "量不到 / 系统保留", bytes: rest, color: "var(--c-inacc)" },
  ].filter((x) => x.bytes > 0);
  const gap = 1.2;
  let off = 0;
  return list.map((x) => {
    const len = Math.max(0, (x.bytes / total) * C - gap);
    const seg = { ...x, dash: `${len} ${C}`, offset: -off };
    off += (x.bytes / total) * C;
    return seg;
  });
});
</script>

<template>
  <div class="ring-wrap" :style="{ width: (size ?? 164) + 'px', height: (size ?? 164) + 'px' }">
    <svg viewBox="0 0 140 140" class="ring" role="img" :aria-label="`已用 ${humanBytes(summary.used_bytes)}，共 ${humanBytes(summary.total_bytes)}`">
      <circle cx="70" cy="70" :r="R" class="track" />
      <g transform="rotate(-90 70 70)">
        <circle
          v-for="s in segs"
          :key="s.key"
          cx="70"
          cy="70"
          :r="R"
          class="seg"
          :stroke="s.color"
          :stroke-dasharray="s.dash"
          :stroke-dashoffset="s.offset"
        >
          <title>{{ s.label }}：{{ humanBytes(s.bytes) }}（占整盘 {{ pct(s.bytes, summary.total_bytes) }}）</title>
        </circle>
      </g>
    </svg>
    <div class="center">
      <div class="c-num num">{{ pct(summary.used_bytes, summary.total_bytes) }}</div>
      <div class="c-label">已用</div>
    </div>
  </div>
</template>

<style scoped>
.ring-wrap {
  position: relative;
  flex: none;
}
.ring {
  width: 100%;
  height: 100%;
}
.track {
  fill: none;
  stroke: var(--surface-inset);
  stroke-width: 13;
}
.seg {
  fill: none;
  stroke-width: 13;
  transition: stroke-dasharray var(--dur-3) var(--ease);
}
.center {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}
.c-num {
  font-size: var(--fs-2xl);
  font-weight: 600;
  letter-spacing: -0.03em;
  line-height: 1;
}
.c-label {
  margin-top: 4px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
</style>
