<script setup lang="ts">
import { computed } from "vue";
import { HardDrive } from "lucide-vue-next";
import type { ScanProgress } from "../api";
import { fmtDuration, fmtInt, humanBytes } from "../format";

const props = defineProps<{ progress: ScanProgress | null; root: string; cancelling: boolean }>();
defineEmits<{ cancel: [] }>();

const ratio = computed(() => {
  const p = props.progress;
  if (!p || !p.used_bytes) return 0;
  return Math.min(1, p.bytes / p.used_bytes);
});
</script>

<template>
  <section class="scan card" aria-live="polite">
    <div class="scan-top">
      <div class="pulse" aria-hidden="true">
        <span class="ring r1"></span>
        <span class="ring r2"></span>
        <span class="core"><HardDrive :size="26" :stroke-width="1.6" /></span>
      </div>
      <div class="scan-head">
        <div class="eyebrow">正在扫描</div>
        <div class="scan-root">{{ root }}</div>
        <div class="muted small">只读取目录和文件的属性，不会修改、不会上传任何东西</div>
      </div>
      <button class="btn" :disabled="cancelling" @click="$emit('cancel')">
        {{ cancelling ? "正在取消…" : "取消扫描" }}
      </button>
    </div>

    <div class="measure">
      <div class="big num">{{ humanBytes(progress?.bytes ?? 0) }}</div>
      <div class="muted small" v-if="progress?.used_bytes">
        已量出 · 卷已用 <span class="num">{{ humanBytes(progress.used_bytes) }}</span> ·
        <span class="num">{{ (ratio * 100).toFixed(1) }}%</span>
      </div>
    </div>
    <div class="meter scan-meter"><i :style="{ width: Math.max(1.5, ratio * 100) + '%' }"></i></div>

    <div class="stats">
      <div><span class="k">已扫文件</span><b class="num">{{ fmtInt(progress?.files ?? 0) }}</b></div>
      <div><span class="k">已扫目录</span><b class="num">{{ fmtInt(progress?.dirs ?? 0) }}</b></div>
      <div><span class="k">无权限目录</span><b class="num">{{ fmtInt(progress?.inaccessible ?? 0) }}</b></div>
      <div><span class="k">用时</span><b class="num">{{ fmtDuration(progress?.elapsed_ms ?? 0) }}</b></div>
    </div>
    <div class="current mono" :title="progress?.current_path">{{ progress?.current_path || "准备中…" }}</div>
  </section>
</template>

<style scoped>
.scan {
  max-width: 760px;
  margin: 0 auto;
  padding: 28px 30px 24px;
  animation: rise-in var(--dur-3) var(--ease);
}
.scan-top {
  display: flex;
  align-items: center;
  gap: 20px;
}
.scan-head {
  flex: 1;
  min-width: 0;
}
.scan-root {
  font-size: var(--fs-xl);
  font-weight: 650;
  letter-spacing: -0.01em;
  margin: 2px 0 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pulse {
  position: relative;
  width: 64px;
  height: 64px;
  flex: none;
}
.core {
  position: absolute;
  inset: 8px;
  border-radius: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--accent-soft);
  color: var(--accent);
  animation: breathe 2.4s ease-in-out infinite;
}
.ring {
  position: absolute;
  inset: 0;
  border-radius: 22px;
  border: 1.5px solid var(--accent);
  opacity: 0;
  animation: ripple 2.4s ease-out infinite;
}
.ring.r2 {
  animation-delay: 1.2s;
}
@keyframes breathe {
  0%,
  100% {
    transform: scale(1);
    box-shadow: 0 0 0 0 var(--accent-soft);
  }
  50% {
    transform: scale(1.04);
    box-shadow: 0 0 24px 2px var(--accent-soft);
  }
}
@keyframes ripple {
  0% {
    transform: scale(0.75);
    opacity: 0.55;
  }
  100% {
    transform: scale(1.25);
    opacity: 0;
  }
}
.measure {
  display: flex;
  align-items: baseline;
  gap: 14px;
  margin: 26px 0 10px;
}
.big {
  font-size: var(--fs-3xl);
  font-weight: 600;
  letter-spacing: -0.03em;
  line-height: 1;
}
.scan-meter {
  height: 8px;
}
.scan-meter > i {
  background: linear-gradient(90deg, var(--accent-fill), var(--accent));
  position: relative;
  overflow: hidden;
}
.scan-meter > i::after {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.28), transparent);
  animation: shimmer 1.8s linear infinite;
}
@keyframes shimmer {
  from {
    transform: translateX(-100%);
  }
  to {
    transform: translateX(100%);
  }
}
.stats {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 10px;
  margin-top: 22px;
}
.stats > div {
  padding: 12px 14px;
  border-radius: var(--r-md);
  background: var(--surface-2);
  border: 1px solid var(--border-1);
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.k {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.stats b {
  font-size: var(--fs-lg);
  font-weight: 600;
}
.current {
  margin-top: 16px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
