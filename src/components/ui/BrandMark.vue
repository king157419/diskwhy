<script setup lang="ts">
// 品牌标：一个被分成三段的磁盘环（可清理 / 需判断 / 保留），中间是强调色的“为什么”点。
withDefaults(defineProps<{ size?: number }>(), { size: 32 });
</script>

<template>
  <svg :width="size" :height="size" viewBox="0 0 32 32" aria-hidden="true" class="brand-mark">
    <rect x="0.5" y="0.5" width="31" height="31" rx="9" class="bm-bg" />
    <g fill="none" stroke-width="3.2" stroke-linecap="round" transform="rotate(-90 16 16)">
      <circle cx="16" cy="16" r="9" stroke="var(--t-safe)" stroke-dasharray="21 56.55" stroke-dashoffset="0" />
      <circle cx="16" cy="16" r="9" stroke="var(--t-review)" stroke-dasharray="12 56.55" stroke-dashoffset="-26.2" />
      <circle cx="16" cy="16" r="9" stroke="var(--t-keep)" stroke-dasharray="8 56.55" stroke-dashoffset="-43.4" />
    </g>
    <circle cx="16" cy="16" r="3.1" fill="var(--accent)" />
  </svg>
</template>

<style scoped>
.brand-mark {
  flex: none;
  display: block;
}
.bm-bg {
  fill: var(--surface-2);
  stroke: var(--border-2);
}
</style>
