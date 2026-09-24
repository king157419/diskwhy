<script setup lang="ts">
// 空状态：纯 SVG 线稿插画 + 标题 + 说明 + 操作。
defineProps<{ kind: "disk" | "folder" | "cart" | "quarantine" | "select"; title: string; text?: string; compact?: boolean }>();
</script>

<template>
  <div class="empty" :class="{ compact }">
    <svg class="illus" viewBox="0 0 160 120" aria-hidden="true">
      <ellipse cx="80" cy="62" rx="64" ry="46" class="glow" />
      <ellipse cx="80" cy="106" rx="46" ry="5" class="shadow" />
      <!-- 磁盘 + 放大镜 -->
      <g v-if="kind === 'disk'">
        <rect x="38" y="28" width="84" height="62" rx="13" class="body" />
        <rect x="38" y="28" width="84" height="62" rx="13" class="line" />
        <circle cx="80" cy="59" r="19" class="track" />
        <path d="M80 40 a19 19 0 0 1 17.9 12.6" class="arc arc-safe" />
        <path d="M98.6 60.5 a19 19 0 0 1 -6.8 13.4" class="arc arc-review" />
        <path d="M84.5 77.5 a19 19 0 0 1 -10 0.5" class="arc arc-keep" />
        <circle cx="80" cy="59" r="4" class="accent-fill" />
        <circle cx="49" cy="80" r="2.2" class="dim-fill" />
        <g transform="translate(108 76)">
          <circle cx="0" cy="0" r="11" class="glass" />
          <circle cx="0" cy="0" r="11" class="line accent-stroke" />
          <path d="M8 8 L16 16" class="line accent-stroke thick" />
        </g>
      </g>
      <!-- 文件夹 -->
      <g v-else-if="kind === 'folder'">
        <path d="M40 36 h24 l8 8 h48 a6 6 0 0 1 6 6 v40 a6 6 0 0 1 -6 6 h-80 a6 6 0 0 1 -6 -6 v-48 a6 6 0 0 1 6 -6z" class="body" />
        <path d="M40 36 h24 l8 8 h48 a6 6 0 0 1 6 6 v40 a6 6 0 0 1 -6 6 h-80 a6 6 0 0 1 -6 -6 v-48 a6 6 0 0 1 6 -6z" class="line" />
        <path d="M56 68 h46 M56 79 h28" class="line dim" />
        <circle cx="116" cy="36" r="10" class="glass" />
        <circle cx="116" cy="36" r="10" class="line accent-stroke" />
        <path d="M111.5 36 h9 M116 31.5 v9" class="line accent-stroke" />
      </g>
      <!-- 清理清单：一张清单纸 -->
      <g v-else-if="kind === 'cart'">
        <rect x="50" y="22" width="60" height="76" rx="10" class="body" />
        <rect x="50" y="22" width="60" height="76" rx="10" class="line" />
        <rect x="60" y="37" width="10" height="10" rx="3" class="line accent-stroke" />
        <path d="M62.5 42 l2.2 2.2 l3.6 -4.2" class="line accent-stroke" />
        <path d="M76 42 h24" class="line dim" />
        <rect x="60" y="55" width="10" height="10" rx="3" class="line" />
        <path d="M76 60 h19" class="line dim" />
        <rect x="60" y="73" width="10" height="10" rx="3" class="line" />
        <path d="M76 78 h14" class="line dim" />
        <circle cx="114" cy="92" r="10" class="glass" />
        <circle cx="114" cy="92" r="10" class="line accent-stroke" />
        <path d="M109.5 92 h9 M114 87.5 v9" class="line accent-stroke" />
      </g>
      <!-- 隔离区：盒子 + 盾 -->
      <g v-else-if="kind === 'quarantine'">
        <path d="M42 50 l38 -17 l38 17 v34 l-38 17 l-38 -17z" class="body" />
        <path d="M42 50 l38 -17 l38 17 v34 l-38 17 l-38 -17z M42 50 l38 17 l38 -17 M80 67 v34" class="line" />
        <path d="M118 22 c4.5 0 8.5 -1.6 11 -3.8 v9.5 c0 6 -4.5 10 -11 12.3 c-6.5 -2.3 -11 -6.3 -11 -12.3 v-9.5 c2.5 2.2 6.5 3.8 11 3.8z" class="glass" />
        <path d="M118 22 c4.5 0 8.5 -1.6 11 -3.8 v9.5 c0 6 -4.5 10 -11 12.3 c-6.5 -2.3 -11 -6.3 -11 -12.3 v-9.5 c2.5 2.2 6.5 3.8 11 3.8z" class="line accent-stroke" />
        <path d="M113.5 28.5 l3 3 l6 -6.5" class="line accent-stroke" />
      </g>
      <!-- 选一项看详情 -->
      <g v-else>
        <rect x="36" y="32" width="88" height="15" rx="5" class="body" />
        <rect x="36" y="53" width="88" height="15" rx="5" class="body sel" />
        <rect x="36" y="74" width="88" height="15" rx="5" class="body" />
        <path d="M45 39.5 h32 M45 81.5 h24" class="line dim" />
        <path d="M45 60.5 h40" class="line accent-stroke" />
        <path d="M102 62 l11 25 l4 -10 l10 -4z" class="cursor" />
      </g>
    </svg>
    <h3 class="empty-title">{{ title }}</h3>
    <p v-if="text" class="empty-text">{{ text }}</p>
    <div v-if="$slots.default" class="empty-actions"><slot /></div>
  </div>
</template>

<style scoped>
.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  padding: var(--sp-10) var(--sp-6);
  animation: rise-in var(--dur-3) var(--ease);
}
.empty.compact {
  padding: var(--sp-6) var(--sp-4);
}
.illus {
  width: 184px;
  height: 138px;
  margin-bottom: var(--sp-3);
}
.compact .illus {
  width: 128px;
  height: 96px;
}
.empty-title {
  font-size: var(--fs-lg);
  font-weight: 600;
  letter-spacing: -0.01em;
}
.compact .empty-title {
  font-size: var(--fs-md);
}
.empty-text {
  margin-top: 6px;
  max-width: 460px;
  color: var(--text-2);
  font-size: var(--fs-sm);
  line-height: 1.65;
}
.empty-actions {
  margin-top: var(--sp-5);
  display: flex;
  gap: var(--sp-2);
  flex-wrap: wrap;
  justify-content: center;
}
.glow {
  fill: var(--accent);
  opacity: 0.07;
}
.shadow {
  fill: var(--text-1);
  opacity: 0.06;
}
.body {
  fill: var(--surface-2);
}
.body.sel {
  fill: var(--accent-soft);
}
.track {
  fill: none;
  stroke: var(--surface-3);
  stroke-width: 4;
}
.line {
  fill: none;
  stroke: var(--text-3);
  stroke-width: 1.8;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.line.dim {
  stroke: var(--border-3);
}
.line.thick {
  stroke-width: 3.2;
}
.accent-stroke {
  stroke: var(--accent);
}
.accent-fill {
  fill: var(--accent);
}
.dim-fill {
  fill: var(--text-3);
}
.glass {
  fill: var(--surface-1);
}
.cursor {
  fill: var(--text-1);
  stroke: var(--surface-1);
  stroke-width: 1.5;
  stroke-linejoin: round;
}
.arc {
  fill: none;
  stroke-width: 4;
  stroke-linecap: round;
}
.arc-safe {
  stroke: var(--t-safe);
}
.arc-review {
  stroke: var(--t-review);
}
.arc-keep {
  stroke: var(--t-keep);
}
</style>
