<script setup lang="ts">
import { computed } from "vue";
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
  <section class="progress card">
    <div class="progress-head">
      <div>
        <span class="attr-title">正在扫描 {{ root }}</span>
        <span class="muted">只读取目录和文件属性，不会修改任何东西</span>
      </div>
      <button class="btn" :disabled="cancelling" @click="$emit('cancel')">
        {{ cancelling ? "正在取消…" : "取消扫描" }}
      </button>
    </div>
    <div class="bar"><div class="bar-fill" :style="{ width: ratio * 100 + '%' }"></div></div>
    <div class="progress-stats">
      <div><span class="muted">已扫文件</span><b>{{ fmtInt(progress?.files ?? 0) }}</b></div>
      <div><span class="muted">已扫目录</span><b>{{ fmtInt(progress?.dirs ?? 0) }}</b></div>
      <div>
        <span class="muted">已量出</span>
        <b>{{ humanBytes(progress?.bytes ?? 0) }}</b>
        <span class="muted" v-if="progress?.used_bytes"> / 已用 {{ humanBytes(progress.used_bytes) }}</span>
      </div>
      <div><span class="muted">无权限目录</span><b>{{ fmtInt(progress?.inaccessible ?? 0) }}</b></div>
      <div><span class="muted">用时</span><b>{{ fmtDuration(progress?.elapsed_ms ?? 0) }}</b></div>
    </div>
    <div class="current" :title="progress?.current_path">当前：{{ progress?.current_path || "准备中…" }}</div>
  </section>
</template>
