<script setup lang="ts">
defineProps<{ title: string; subtitle?: string }>();
</script>

<template>
  <header class="page-header">
    <div class="ph-text">
      <h1 class="ph-title">{{ title }}</h1>
      <p v-if="subtitle || $slots.subtitle" class="ph-sub">
        <slot name="subtitle">{{ subtitle }}</slot>
      </p>
    </div>
    <div class="ph-actions"><slot /></div>
  </header>
</template>

<style scoped>
.page-header {
  display: flex;
  align-items: center;
  gap: var(--sp-4);
  min-height: var(--header-h);
  padding: 16px var(--sp-8) 12px;
  flex: none;
}
.ph-text {
  min-width: 0;
  flex: 1;
}
.ph-title {
  font-size: var(--fs-xl);
  font-weight: 650;
  letter-spacing: -0.015em;
  line-height: var(--lh-tight);
}
.ph-sub {
  margin-top: 4px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ph-actions {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex: none;
}
</style>
