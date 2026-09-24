<script setup lang="ts">
import { computed } from "vue";
import { ArrowUp, ChevronRight, LayoutGrid, List } from "lucide-vue-next";
import { fmtInt, humanBytes } from "../format";
import PageHeader from "../components/ui/PageHeader.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ScanControls from "../components/ScanControls.vue";
import ScanProgressPanel from "../components/ScanProgressPanel.vue";
import DirList from "../components/DirList.vue";
import TreemapView from "../components/TreemapView.vue";
import DetailPanel from "../components/DetailPanel.vue";
import {
  addToCart,
  cancelling,
  cartPaths,
  detailNode,
  doCancel,
  doScan,
  goUp,
  isScanRoot,
  listing,
  llmReady,
  onTmSelect,
  open,
  page,
  phase,
  progress,
  removeFromCart,
  select,
  selectedDrive,
  selectedKey,
  summary,
  tm,
  tmLoading,
  viewMode,
} from "../store";

const subtitle = computed(() => {
  const l = listing.value;
  if (!l) return "逐层看每一块是什么";
  return `${l.node.path} · ${humanBytes(l.node.allocated)} · ${fmtInt(l.total_children)} 项 · 点文件夹进入，点 ⓘ 或右键看说明`;
});
</script>

<template>
  <div class="page">
    <PageHeader title="空间分析" :subtitle="subtitle">
      <ScanControls />
    </PageHeader>

    <div v-if="phase === 'scanning'" class="page-pad">
      <ScanProgressPanel :progress="progress" :root="selectedDrive" :cancelling="cancelling" @cancel="doCancel" />
    </div>

    <div v-else-if="!summary || !listing" class="page-pad center">
      <EmptyState kind="folder" title="还没有扫描结果" text="扫描完成后，这里可以用列表或方块图逐层查看每个文件夹：多大、是什么、能不能清理。">
        <button class="btn primary" :disabled="!selectedDrive" @click="doScan">开始扫描 {{ selectedDrive }}</button>
        <button class="btn" @click="page = 'overview'">回到概览</button>
      </EmptyState>
    </div>

    <template v-else>
      <div class="toolbar">
        <button class="btn icon sm" :disabled="listing.breadcrumb.length <= 1" aria-label="上一级" title="上一级（Backspace / Alt+← / 鼠标后退键）" @click="goUp">
          <ArrowUp :size="16" :stroke-width="2" />
        </button>
        <nav class="crumbs" aria-label="路径">
          <template v-for="(c, i) in listing.breadcrumb" :key="c.path">
            <ChevronRight v-if="i > 0" :size="14" class="sep" />
            <button class="crumb" :class="{ current: i === listing.breadcrumb.length - 1 }" :title="c.path" @click="open(c.path)">{{ c.name }}</button>
          </template>
        </nav>
        <span class="spacer"></span>
        <div class="segmented" role="group" aria-label="视图">
          <button :class="{ on: viewMode === 'list' }" :aria-pressed="viewMode === 'list'" @click="viewMode = 'list'"><List :size="15" /> 列表</button>
          <button :class="{ on: viewMode === 'map' }" :aria-pressed="viewMode === 'map'" @click="viewMode = 'map'"><LayoutGrid :size="15" /> 方块图</button>
        </div>
      </div>

      <div class="work">
        <div class="left">
          <Transition name="view" mode="out-in">
            <DirList v-if="viewMode === 'list'" key="list" :listing="listing" :selected-path="selectedKey" @open="open" @select="select" />
            <TreemapView
              v-else
              key="map"
              :data="tm"
              :summary="summary"
              :is-scan-root="isScanRoot"
              :selected-key="selectedKey"
              :loading="tmLoading"
              @select="onTmSelect"
              @open="open"
              @up="goUp"
            />
          </Transition>
        </div>
        <DetailPanel
          class="right"
          :node="detailNode"
          :llm-ready="llmReady"
          :in-cart="!!detailNode && cartPaths.has(detailNode.path.toLowerCase())"
          :can-act="phase === 'done'"
          @open-settings="page = 'settings'"
          @open-path="open"
          @add-to-cart="addToCart"
          @remove-from-cart="removeFromCart"
        />
      </div>
    </template>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.page-pad {
  flex: 1;
  overflow-y: auto;
  padding: var(--sp-8);
}
.page-pad.center {
  display: flex;
  align-items: center;
  justify-content: center;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 2px var(--sp-8) 12px;
  flex: none;
}
.crumbs {
  display: flex;
  align-items: center;
  gap: 2px;
  min-width: 0;
  overflow: hidden;
}
.sep {
  color: var(--text-3);
  flex: none;
}
.crumb {
  border: 0;
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-sm);
  padding: 4px 8px;
  border-radius: var(--r-sm);
  cursor: pointer;
  white-space: nowrap;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  transition:
    background var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease);
}
.crumb:hover {
  background: var(--surface-2);
  color: var(--text-1);
}
.crumb.current {
  color: var(--text-1);
  font-weight: 600;
  background: var(--surface-1);
  box-shadow: inset 0 0 0 1px var(--border-1);
}
.work {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 16px;
  padding: 0 var(--sp-8) var(--sp-6);
}
.left {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.right {
  width: var(--detail-w);
  flex: none;
}
@media (max-width: 1360px) {
  .right {
    width: 340px;
  }
}
.view-enter-active,
.view-leave-active {
  transition: opacity var(--dur-2) var(--ease);
}
.view-enter-from,
.view-leave-to {
  opacity: 0;
}
</style>
