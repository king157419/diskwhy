<script setup lang="ts">
// 阶段 M：「AI 推测」小标签（悬停 / 钉住的卡片、列表展开行、列表标题下的一行共用）。
// 中性：虚线框 + 一颗四角小星，不用三档颜色、不用 --accent（样式在 base.css 的 .tag.ai / .ai-star）。
</script>

<template>
  <span class="tag ai"
    ><svg class="ai-star" viewBox="0 0 12 12" aria-hidden="true"><path d="M6 .6C6.35 3.6 8.4 5.65 11.4 6 8.4 6.35 6.35 8.4 6 11.4 5.65 8.4 3.6 6.35.6 6 3.6 5.65 5.65 3.6 6 .6Z" /></svg
    >AI 推测</span
  >
</template>
