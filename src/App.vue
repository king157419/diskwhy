<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ArrowRight, CircleAlert, Info, ListChecks, X } from "lucide-vue-next";
import AppSidebar from "./components/AppSidebar.vue";
import PreviewDialog from "./components/PreviewDialog.vue";
import OverviewPage from "./pages/OverviewPage.vue";
import AnalysisPage from "./pages/AnalysisPage.vue";
import CleanupPage from "./pages/CleanupPage.vue";
import QuarantinePage from "./pages/QuarantinePage.vue";
import SettingsPage from "./pages/SettingsPage.vue";
import { init, message, notice, onExecuted, onRequote, page, report, toast } from "./store";

const pages = {
  overview: OverviewPage,
  analysis: AnalysisPage,
  cleanup: CleanupPage,
  quarantine: QuarantinePage,
  settings: SettingsPage,
};

// 窗口窄于 1200px 时侧栏收成图标
const compact = ref(false);
const mq = window.matchMedia("(max-width: 1199px)");
const syncCompact = () => (compact.value = mq.matches);

onMounted(() => {
  syncCompact();
  mq.addEventListener?.("change", syncCompact);
  void init();
});
onBeforeUnmount(() => mq.removeEventListener?.("change", syncCompact));

// 普通提示（快照已加载 / 已保存）12 秒后自动收起；错误提示要用户自己关
let noticeTimer: ReturnType<typeof setTimeout> | undefined;
watch(notice, (v) => {
  clearTimeout(noticeTimer);
  if (v) noticeTimer = setTimeout(() => (notice.value = ""), 12000);
});
</script>

<template>
  <div class="shell">
    <AppSidebar :compact="compact" />
    <main class="main" :class="{ 'has-bar': page === 'cleanup' }">
      <Transition name="page" mode="out-in">
        <component :is="pages[page]" :key="page" />
      </Transition>

      <div class="banners" aria-live="polite">
        <TransitionGroup name="banner">
          <div v-if="message" key="msg" class="banner danger" role="alert">
            <CircleAlert :size="16" />
            <span class="b-text">{{ message }}</span>
            <button class="b-close" aria-label="关闭提示" @click="message = ''"><X :size="14" /></button>
          </div>
          <div v-if="notice" key="notice" class="banner info" :title="notice">
            <Info :size="16" />
            <span class="b-text">{{ notice }}</span>
            <button class="b-close" aria-label="关闭提示" @click="notice = ''"><X :size="14" /></button>
          </div>
          <div v-if="toast" key="toast" class="banner">
            <ListChecks :size="16" />
            <span class="b-text">{{ toast.text }}</span>
            <button v-if="toast.action" class="btn sm" @click="page = toast.action!.page; toast = null">
              {{ toast.action.label }} <ArrowRight :size="13" />
            </button>
            <button class="b-close" aria-label="关闭提示" @click="toast = null"><X :size="14" /></button>
          </div>
        </TransitionGroup>
      </div>
    </main>

    <PreviewDialog v-if="report" :key="report.plan_id" :report="report" @close="report = null" @requote="onRequote" @executed="onExecuted" />
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100%;
  min-width: 0;
  background: var(--bg-0);
}
.main {
  position: relative;
  flex: 1;
  min-width: 0;
  min-height: 0;
  background: var(--bg-1);
  border-radius: 14px 0 0 0;
  border-top: 1px solid var(--border-1);
  border-left: 1px solid var(--border-1);
  margin-top: 6px;
  overflow: hidden;
}
.main > :deep(.page) {
  height: 100%;
}
.banners {
  position: absolute;
  right: 24px;
  bottom: 22px;
  z-index: 50;
  display: flex;
  flex-direction: column-reverse;
  gap: 8px;
  width: min(520px, calc(100% - 48px));
  pointer-events: none;
}
/* 清理清单页底部有操作栏，提示往上挪 */
.main.has-bar .banners {
  bottom: 104px;
}
.banner {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 10px 9px 14px;
  border-radius: var(--r-md);
  background: var(--surface-2);
  border: 1px solid var(--border-2);
  box-shadow: var(--shadow-pop);
  font-size: var(--fs-sm);
}
.banner > svg {
  flex: none;
  color: var(--accent);
}
.banner.danger {
  background: color-mix(in srgb, var(--danger) 14%, var(--surface-1));
  border-color: var(--danger-border);
}
.banner.danger > svg {
  color: var(--danger);
}
.banner.info > svg {
  color: var(--accent);
}
.b-text {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  line-height: 1.5;
}
.b-close {
  width: 26px;
  height: 26px;
  flex: none;
  border: 0;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--text-3);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.b-close:hover {
  background: var(--surface-3);
  color: var(--text-1);
}
.banner-enter-active,
.banner-leave-active {
  transition:
    opacity var(--dur-2) var(--ease),
    transform var(--dur-2) var(--ease);
}
.banner-enter-from,
.banner-leave-to {
  opacity: 0;
  transform: translateY(6px);
}
</style>
