<script setup lang="ts">
// 自绘标题栏（窗口无系统边框）：首页是品牌，其他页是「‹ 页名」；右边设置 + 最小化 / 最大化 / 关闭。
// 整条可拖动（data-tauri-drag-region 只对直接标了的元素生效，所以空白处单独标）。
import { computed } from "vue";
import { kaiSafe } from "../labels";
import { execPhase, execStopping, onCloseBlocked, page, type Page } from "../store";

/** 确认弹层开着：左边（返回 / 设置）不可用，右边的窗口按钮照常可用 */
defineProps<{ modal?: boolean }>();

const TITLES: Record<Page, string> = {
  home: "",
  cleanup: "清理",
  explore: "空间分析",
  quarantine: "暂存区",
  settings: "设置",
};
const title = computed(() => TITLES[page.value]);
/** C 主题的标题字体缺“暂”等字：这种页名整个用界面字体，不在一个词里混两种字体 */
const uiFont = computed(() => !kaiSafe(title.value));

async function win(action: "min" | "max" | "close") {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const w = getCurrentWindow();
    if (action === "min") await w.minimize();
    else if (action === "max") await w.toggleMaximize();
    // 清理中还没按停止：直接提示“先停止再关”。按了停止之后照样发关窗请求，由后端决定：还在收尾就拦下并提示，
    // 停了 20 秒还停不下来（某个系统调用卡住）就放行（安全阀）
    else if (execPhase.value === "running" && !execStopping.value) onCloseBlocked();
    else await w.close();
  } catch {
    /* 浏览器预览里没有窗口 API */
  }
}
</script>

<template>
  <header class="titlebar" data-tauri-drag-region>
    <div v-if="page === 'home'" class="brand" data-tauri-drag-region><span class="mark"></span>DiskWhy</div>
    <button v-else class="back" :class="{ 'ui-font': uiFont }" aria-label="返回首页" :inert="modal" @click="page = 'home'">
      <svg class="i" viewBox="0 0 24 24"><path d="m15 18-6-6 6-6" /></svg>{{ title }}
    </button>
    <div class="tb-spacer" data-tauri-drag-region></div>
    <button v-if="page === 'home'" class="tb-btn" title="设置" aria-label="设置" :inert="modal" @click="page = 'settings'">
      <svg class="i" viewBox="0 0 24 24"><path d="M20 7h-9" /><path d="M14 17H5" /><circle cx="17" cy="17" r="3" /><circle cx="7" cy="7" r="3" /></svg>
    </button>
    <div class="win">
      <button aria-label="最小化" @click="win('min')"><svg viewBox="0 0 10 10"><path d="M0 5h10" /></svg></button>
      <button aria-label="最大化" @click="win('max')"><svg viewBox="0 0 10 10"><rect x=".5" y=".5" width="9" height="9" /></svg></button>
      <button class="close" aria-label="关闭" @click="win('close')"><svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" /></svg></button>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  position: absolute;
  left: 0;
  right: 0;
  top: 0;
  height: var(--titlebar);
  display: flex;
  align-items: center;
  padding-left: 20px;
  z-index: 40;
}
.brand {
  display: flex;
  align-items: center;
  gap: 9px;
  font-size: 13px;
  font-weight: var(--w-med);
  color: var(--ink2);
  letter-spacing: 0.01em;
}
.brand .mark {
  width: 16px;
  height: 16px;
  border-radius: 4px;
  background: var(--ink);
  position: relative;
  overflow: hidden;
}
.brand .mark::after {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: 0;
  height: 6px;
  background: var(--safe);
}
[data-theme="C"] .brand {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 15px;
  color: var(--ink);
}
.back {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 15px;
  font-weight: var(--w-semi);
  color: var(--ink);
  height: 32px;
  padding-right: 8px;
  border-radius: var(--r);
}
.back .i {
  color: var(--ink2);
  transition: transform var(--dur-1) var(--ease);
}
.back:hover .i {
  transform: translateX(-2px);
}
[data-theme="C"] .back {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 16px;
}
[data-theme="C"] .back.ui-font {
  font-family: var(--ui);
  font-size: 15px;
}
.tb-spacer {
  flex: 1;
  align-self: stretch;
}
.tb-btn {
  width: 36px;
  height: 36px;
  display: grid;
  place-items: center;
  color: var(--ink2);
  border-radius: var(--r);
}
.tb-btn:hover {
  color: var(--ink);
  background: var(--hover);
}
.win {
  display: flex;
  margin-left: 6px;
  align-self: stretch;
}
.win button {
  width: 46px;
  height: var(--titlebar);
  display: grid;
  place-items: center;
  color: var(--ink3);
  transition: background-color var(--dur-1) var(--ease);
}
.win button:hover {
  background: var(--hover);
  color: var(--ink);
}
.win button.close:hover {
  background: #c42b1c;
  color: #fff;
}
.win svg {
  width: 10px;
  height: 10px;
  stroke: currentColor;
  stroke-width: 1;
  fill: none;
}
</style>
