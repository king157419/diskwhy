<script setup lang="ts">
import { onMounted, watch } from "vue";
import TitleBar from "./components/TitleBar.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import HomePage from "./pages/HomePage.vue";
import CleanupPage from "./pages/CleanupPage.vue";
import ExplorePage from "./pages/ExplorePage.vue";
import QuarantinePage from "./pages/QuarantinePage.vue";
import SettingsPage from "./pages/SettingsPage.vue";
import { init, message, notice, page, report } from "./store";

const pages = {
  home: HomePage,
  cleanup: CleanupPage,
  explore: ExplorePage,
  quarantine: QuarantinePage,
  settings: SettingsPage,
};

onMounted(() => void init());

// 普通提示 8 秒后自动收起；错误提示要手动关
let t: ReturnType<typeof setTimeout> | undefined;
watch(notice, (v) => {
  clearTimeout(t);
  if (v) t = setTimeout(() => (notice.value = ""), 8000);
});
</script>

<template>
  <div class="app">
    <TitleBar />
    <component :is="pages[page]" :key="page" class="page" />

    <div class="banners" aria-live="polite">
      <div v-if="message" class="banner err" role="alert">
        <span class="b-text">{{ message }}</span>
        <button class="b-x" aria-label="关闭提示" @click="message = ''">
          <svg class="i" viewBox="0 0 24 24"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg>
        </button>
      </div>
      <div v-if="notice" class="banner">
        <span class="b-text">{{ notice }}</span>
        <button class="b-x" aria-label="关闭提示" @click="notice = ''">
          <svg class="i" viewBox="0 0 24 24"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg>
        </button>
      </div>
    </div>

    <ConfirmDialog v-if="report" :key="report.plan_id" :report="report" @close="report = null" />
  </div>
</template>

<style scoped>
.app {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
}
.page {
  position: absolute;
  inset: 0;
  animation: page-in var(--dur-2) var(--ease);
}
@keyframes page-in {
  from {
    transform: translateY(4px);
  }
}
.banners {
  position: absolute;
  left: 50%;
  top: 52px;
  transform: translateX(-50%);
  z-index: 60;
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: min(560px, calc(100% - 48px));
  pointer-events: none;
}
.banner {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 8px 8px 14px;
  border-radius: var(--r);
  background: var(--surface);
  box-shadow: var(--shadow-pop);
  font-size: var(--fs-sub);
  color: var(--ink2);
}
[data-theme="C"] .banner {
  border: 2px solid #141412;
  box-shadow: var(--shadow);
}
.banner.err {
  color: var(--ink);
  border-left: 3px solid var(--danger);
}
.b-text {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  user-select: text;
}
.b-x {
  width: 26px;
  height: 26px;
  display: grid;
  place-items: center;
  color: var(--ink3);
  border-radius: 4px;
  flex: none;
}
.b-x .i {
  width: 14px;
  height: 14px;
}
.b-x:hover {
  color: var(--ink);
  background: var(--hover);
}
</style>
