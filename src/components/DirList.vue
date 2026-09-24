<script setup lang="ts">
import { ChevronRight, Cloud, File, Folder, Info, Layers, Link2, Lock } from "lucide-vue-next";
import { tierKey, tmKey, type DirListing, type NodeView } from "../api";
import { fmtInt, humanBytes, pct } from "../format";
import TierChip from "./ui/TierChip.vue";

defineProps<{ listing: DirListing; selectedPath: string | null }>();
const emit = defineEmits<{ open: [path: string]; select: [node: NodeView] }>();

function icon(n: NodeView) {
  if (n.kind === "Rest") return Layers;
  if (n.kind === "Dir") {
    if (n.reparse_skipped) return Link2;
    if (n.inaccessible) return Lock;
    return Folder;
  }
  if (n.cloud_placeholder) return Cloud;
  return File;
}

function canOpen(n: NodeView): boolean {
  // 无权限但量了已知子目录的（WindowsApps）也能进
  return n.kind === "Dir" && !n.reparse_skipped && !n.mapped && n.child_count > 0;
}

function onClick(n: NodeView) {
  if (canOpen(n)) emit("open", n.path);
  else emit("select", n);
}

function sub(n: NodeView): string {
  if (n.kind === "Rest") return `其余 ${fmtInt(n.files)} 个小文件合计`;
  if (n.kind === "Dir") {
    if (n.reparse_skipped) return "链接 / 挂载点，未跟随";
    if (n.inaccessible && !n.child_count) return "无法读取";
    return `${fmtInt(n.files)} 个文件`;
  }
  return n.type_label;
}
</script>

<template>
  <section class="list card">
    <div class="thead">
      <span class="h-name">名称 · 这是什么</span>
      <span class="h-bar">占当前目录</span>
      <span class="h-size">实际占用</span>
      <span class="h-act"></span>
    </div>
    <div class="tbody">
      <div
        v-for="n in listing.children"
        :key="tmKey(n)"
        class="row"
        :class="{
          selected: selectedPath === tmKey(n),
          clickable: canOpen(n),
          dim: n.inaccessible || n.reparse_skipped || n.cloud_placeholder || n.mapped || n.hardlink_dup,
        }"
        role="button"
        tabindex="0"
        :aria-label="n.name + ' ' + humanBytes(n.allocated)"
        :title="(canOpen(n) ? '点击进入这个文件夹 · ' : '') + n.type_label"
        @click="onClick(n)"
        @keydown.enter="onClick(n)"
        @contextmenu.prevent="emit('select', n)"
      >
        <span class="ico" :class="'ico-' + tierKey(n.explanation)">
          <component :is="icon(n)" :size="18" :stroke-width="1.7" />
        </span>
        <span class="main">
          <span class="line1">
            <span class="nm">{{ n.name }}</span>
            <span v-if="n.system_reserved" class="tag">系统保留</span>
            <span v-if="n.inaccessible" class="tag warn">无权限</span>
            <span v-if="n.reparse_skipped" class="tag">未跟随</span>
            <span v-if="n.mapped" class="tag accent" :title="'和 ' + (n.mapped_to ?? '另一个路径') + ' 是同一份数据，空间记在那边'">映射</span>
            <span v-if="n.hardlink_dup" class="tag accent" title="硬链接：这个文件还有别的名字，空间记在另一个路径上">硬链接</span>
            <span class="count">{{ sub(n) }}</span>
          </span>
          <span class="line2" :title="n.explanation?.what">
            <TierChip :tier="tierKey(n.explanation)" />
            <span class="what">{{ n.explanation?.what ?? "" }}</span>
          </span>
        </span>
        <span class="bar">
          <span class="meter"><i :style="{ width: Math.max(0.8, n.percent_of_parent) + '%' }"></i></span>
          <span class="pct num">{{ pct(n.allocated, listing.node.allocated) }}</span>
        </span>
        <span class="size num">{{ humanBytes(n.allocated) }}</span>
        <span class="act">
          <button class="info-btn" :aria-label="'详情：' + n.name" title="看这一项的说明（不进入）" @click.stop="emit('select', n)">
            <Info :size="15" :stroke-width="1.8" />
          </button>
          <ChevronRight v-if="canOpen(n)" class="chev" :size="16" :stroke-width="1.8" />
          <span v-else class="chev-space"></span>
        </span>
      </div>
      <div v-if="listing.children.length === 0" class="empty-row muted">这个文件夹是空的</div>
      <div v-if="listing.total_children > listing.children.length" class="empty-row muted small">
        只显示前 {{ listing.children.length }} 项（共 {{ fmtInt(listing.total_children) }} 项）
      </div>
    </div>
  </section>
</template>

<style scoped>
.list {
  display: flex;
  flex-direction: column;
  min-height: 0;
  flex: 1;
  overflow: hidden;
}
.thead,
.row {
  display: grid;
  grid-template-columns: 38px minmax(0, 1fr) clamp(96px, 14%, 170px) 92px 58px;
  align-items: center;
  column-gap: 14px;
}
.thead {
  padding: 11px 18px 9px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  letter-spacing: 0.02em;
  border-bottom: 1px solid var(--border-1);
}
.h-name {
  grid-column: 1 / 3;
}
.h-size {
  text-align: right;
}
.tbody {
  overflow-y: auto;
  flex: 1;
  padding: 6px 8px 10px;
}
.row {
  position: relative;
  min-height: 60px;
  padding: 8px 10px;
  border-radius: var(--r-md);
  cursor: default;
  transition: background var(--dur-1) var(--ease);
  animation: rise-in var(--dur-2) var(--ease) both;
}
.row.clickable {
  cursor: pointer;
}
.row:hover {
  background: var(--surface-2);
}
.row:focus-visible {
  outline-offset: -2px;
}
.row.selected {
  background: var(--accent-softer);
  box-shadow: inset 0 0 0 1px var(--accent-soft);
}
.row.selected::before {
  content: "";
  position: absolute;
  left: 0;
  top: 14px;
  bottom: 14px;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background: var(--accent);
}
.row.dim .nm {
  color: var(--text-2);
}
.ico {
  width: 38px;
  height: 38px;
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--surface-2);
  color: var(--text-2);
  transition: transform var(--dur-1) var(--ease);
}
.row:hover .ico {
  transform: scale(1.04);
}
.ico-safe {
  background: var(--t-safe-bg);
  color: var(--t-safe-fg);
}
.ico-review {
  background: var(--t-review-bg);
  color: var(--t-review-fg);
}
.ico-keep {
  background: var(--t-keep-bg);
  color: var(--t-keep-fg);
}
.ico-unknown {
  background: var(--t-unknown-bg);
  color: var(--t-unknown-fg);
}
.main {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}
.line1 {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}
.nm {
  font-weight: 550;
  font-size: var(--fs-md);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}
.count {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  white-space: nowrap;
  flex: none;
}
.line2 {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.what {
  font-size: var(--fs-xs);
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}
.bar {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.bar .meter {
  height: 4px;
}
.pct {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.size {
  text-align: right;
  font-size: var(--fs-md);
  font-weight: 550;
}
.act {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
}
.info-btn {
  width: 26px;
  height: 26px;
  border: 0;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--text-3);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  opacity: 0.55;
  transition:
    opacity var(--dur-1) var(--ease),
    background var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease);
}
.row:hover .info-btn,
.row.selected .info-btn,
.info-btn:focus-visible {
  opacity: 1;
}
.info-btn:hover {
  background: var(--surface-3);
  color: var(--text-1);
}
.chev {
  color: var(--text-3);
  transition:
    transform var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease);
}
.row:hover .chev {
  color: var(--text-1);
  transform: translateX(2px);
}
.chev-space {
  width: 16px;
}
.empty-row {
  padding: 18px;
  text-align: center;
}
</style>
