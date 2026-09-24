<script setup lang="ts">
import { computed } from "vue";
import { Archive, FolderTree, LayoutDashboard, ListChecks, Settings, ShieldCheck, ShieldAlert } from "lucide-vue-next";
import BrandMark from "./ui/BrandMark.vue";
import { actEnv, cart, doRelaunch, env, page, phase, relaunching, type Page } from "../store";
import { humanBytes } from "../format";

defineProps<{ compact: boolean }>();

const cartBytes = computed(() => cart.value.reduce((s, c) => s + c.allocated, 0));
const batches = computed(() => actEnv.value?.batches.length ?? 0);
const expired = computed(() => actEnv.value?.expired_batches ?? 0);

interface Item {
  page: Page;
  label: string;
  icon: unknown;
  badge?: string;
  warn?: boolean;
  hint?: string;
}
const groups = computed<{ label: string; items: Item[] }[]>(() => [
  {
    label: "分析",
    items: [
      { page: "overview", label: "概览", icon: LayoutDashboard, hint: "归属、能腾多少、最大的目录" },
      { page: "analysis", label: "空间分析", icon: FolderTree, hint: "列表 / 方块图，逐层看每一块是什么" },
    ],
  },
  {
    label: "清理",
    items: [
      {
        page: "cleanup",
        label: "清理清单",
        icon: ListChecks,
        badge: cart.value.length ? String(cart.value.length) : undefined,
        hint: cart.value.length ? `${cart.value.length} 项 · 约 ${humanBytes(cartBytes.value)}` : "加入清单的项目在这里预览、确认",
      },
      {
        page: "quarantine",
        label: "隔离区",
        icon: Archive,
        badge: batches.value ? String(batches.value) : undefined,
        warn: expired.value > 0,
        hint: expired.value ? `${expired.value} 批已超过 ${actEnv.value?.keep_days ?? 7} 天` : "隔离到其他盘的文件，可整批还原",
      },
    ],
  },
]);
</script>

<template>
  <aside class="sidebar" :class="{ compact }">
    <div class="brand">
      <BrandMark :size="compact ? 34 : 32" />
      <div v-if="!compact" class="brand-text">
        <div class="brand-name">DiskWhy</div>
        <div class="brand-sub">每一块空间都说清楚</div>
      </div>
    </div>

    <nav class="nav" aria-label="主导航">
      <template v-for="g in groups" :key="g.label">
        <div class="group-label" :class="{ hide: compact }">{{ g.label }}</div>
        <button
          v-for="it in g.items"
          :key="it.page"
          class="nav-item"
          :class="{ active: page === it.page }"
          :aria-current="page === it.page ? 'page' : undefined"
          :aria-label="it.label + (it.badge ? `（${it.badge}）` : '')"
          :title="compact ? `${it.label} · ${it.hint}` : it.hint"
          @click="page = it.page"
        >
          <component :is="it.icon" :size="18" :stroke-width="1.8" />
          <span v-if="!compact" class="nav-label">{{ it.label }}</span>
          <span v-if="it.badge" class="nav-badge num" :class="{ warn: it.warn }">{{ it.badge }}</span>
        </button>
      </template>
    </nav>

    <div class="side-foot">
      <div
        v-if="env && !env.elevated"
        class="priv"
        :class="{ compact }"
        title="当前不是管理员身份：System Volume Information、WindowsApps、其他用户的文件夹等系统目录读不了，会记到“无权限”里。以管理员身份重新启动可以看到这些目录。"
      >
        <ShieldAlert :size="16" :stroke-width="1.8" class="priv-ico" />
        <div v-if="!compact" class="priv-text">
          <div class="priv-title">普通权限</div>
          <button class="priv-link" :disabled="relaunching || phase === 'scanning'" @click="doRelaunch">
            {{ relaunching ? "等待授权…" : "以管理员重新启动" }}
          </button>
        </div>
      </div>
      <div v-else-if="env" class="priv ok" :class="{ compact }" title="已经是管理员身份，系统目录也能读到">
        <ShieldCheck :size="16" :stroke-width="1.8" class="priv-ico" />
        <div v-if="!compact" class="priv-text"><div class="priv-title">管理员身份</div></div>
      </div>
      <button
        class="nav-item"
        :class="{ active: page === 'settings' }"
        aria-label="设置"
        :title="env ? `设置 · 规则库 ${env.rules_total} 条` : '设置'"
        @click="page = 'settings'"
      >
        <Settings :size="18" :stroke-width="1.8" />
        <span v-if="!compact" class="nav-label">设置</span>
        <span v-if="!compact && env" class="nav-meta num">v{{ env.version }}</span>
      </button>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: var(--sidebar-w);
  flex: none;
  display: flex;
  flex-direction: column;
  padding: 18px 12px 14px;
  background: var(--bg-0);
  border-right: 1px solid var(--border-1);
  transition: width var(--dur-2) var(--ease);
  overflow: hidden;
}
.sidebar.compact {
  width: var(--sidebar-w-compact);
  align-items: center;
}
.brand {
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 2px 8px 22px;
}
.compact .brand {
  padding: 2px 0 22px;
}
.brand-name {
  font-weight: 700;
  font-size: 15.5px;
  letter-spacing: -0.02em;
  line-height: 1.15;
}
.brand-sub {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  margin-top: 2px;
  white-space: nowrap;
}
.nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
}
.group-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  letter-spacing: 0.06em;
  color: var(--text-3);
  padding: 14px 12px 6px;
}
.group-label:first-child {
  padding-top: 0;
}
.group-label.hide {
  height: 14px;
  padding: 0;
  overflow: hidden;
  color: transparent;
}
.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 11px;
  width: 100%;
  height: 38px;
  padding: 0 12px;
  border: 0;
  border-radius: var(--r-md);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-sm);
  font-weight: 500;
  cursor: pointer;
  text-align: left;
  transition:
    background var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease);
}
.compact .nav-item {
  justify-content: center;
  padding: 0;
  width: 44px;
  margin: 0 auto;
}
.nav-item:hover {
  background: var(--surface-1);
  color: var(--text-1);
}
.nav-item:focus-visible {
  outline: 1.5px solid var(--accent-ring);
  outline-offset: -1.5px;
}
.nav-item.active {
  background: var(--surface-2);
  color: var(--text-1);
}
.nav-item.active::before {
  content: "";
  position: absolute;
  left: -12px;
  top: 10px;
  bottom: 10px;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background: var(--accent);
}
.compact .nav-item.active::before {
  left: -14px;
}
.nav-item.active svg {
  color: var(--accent);
}
.nav-label {
  flex: 1;
  white-space: nowrap;
}
.nav-badge {
  min-width: 20px;
  height: 18px;
  padding: 0 6px;
  border-radius: var(--r-pill);
  background: var(--accent-soft);
  color: var(--accent);
  font-size: 10.5px;
  font-weight: 600;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.nav-badge.warn {
  background: var(--warn-bg);
  color: var(--warn);
  box-shadow: inset 0 0 0 1px var(--warn-border);
}
.compact .nav-badge {
  position: absolute;
  top: 3px;
  right: 1px;
  min-width: 16px;
  height: 15px;
  padding: 0 4px;
  font-size: 9.5px;
}
.nav-meta {
  font-size: 10.5px;
  color: var(--text-3);
}
.side-foot {
  margin-top: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
}
.priv {
  display: flex;
  gap: 10px;
  align-items: flex-start;
  padding: 10px 12px;
  border-radius: var(--r-md);
  background: var(--surface-1);
  border: 1px solid var(--border-1);
}
.priv.compact {
  width: 44px;
  justify-content: center;
  padding: 10px 0;
  margin: 0 auto;
}
.priv-ico {
  color: var(--warn);
  margin-top: 1px;
  flex: none;
}
.priv.ok .priv-ico {
  color: var(--ok);
}
.priv-title {
  font-size: var(--fs-xs);
  font-weight: 600;
  line-height: 1.3;
}
.priv-link {
  padding: 0;
  margin-top: 3px;
  border: 0;
  background: none;
  color: var(--accent);
  font-size: var(--fs-2xs);
  cursor: pointer;
}
.priv-link:hover:not(:disabled) {
  text-decoration: underline;
}
.priv-link:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
