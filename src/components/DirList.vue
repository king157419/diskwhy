<script setup lang="ts">
import type { DirListing, NodeView } from "../api";
import { humanBytes, pct } from "../format";

defineProps<{ listing: DirListing; selectedPath: string | null }>();
const emit = defineEmits<{ open: [path: string]; select: [node: NodeView] }>();

function icon(n: NodeView): string {
  if (n.kind === "Rest") return "ico-rest";
  if (n.kind === "Dir") {
    if (n.reparse_skipped) return "ico-link";
    if (n.inaccessible) return "ico-lock";
    return "ico-dir";
  }
  if (n.cloud_placeholder) return "ico-cloud";
  return "ico-file";
}

function canOpen(n: NodeView): boolean {
  return n.kind === "Dir" && !n.inaccessible && !n.reparse_skipped && n.child_count > 0;
}

function onClick(n: NodeView) {
  if (canOpen(n)) emit("open", n.path);
  else emit("select", n);
}

function rowKey(n: NodeView): string {
  return n.kind === "Rest" ? n.path + "|rest" : n.path;
}
</script>

<template>
  <section class="list card">
    <nav class="crumbs" aria-label="路径">
      <template v-for="(c, i) in listing.breadcrumb" :key="c.path">
        <span v-if="i > 0" class="sep">›</span>
        <button class="crumb" :class="{ current: i === listing.breadcrumb.length - 1 }" @click="emit('open', c.path)">
          {{ c.name }}
        </button>
      </template>
      <span class="muted crumb-total">
        {{ humanBytes(listing.node.allocated) }} · {{ listing.total_children }} 项
      </span>
    </nav>
    <div class="table">
      <div class="thead">
        <span class="c-name">名称</span>
        <span class="c-bar">占父目录</span>
        <span class="c-size">实际占用</span>
        <span class="c-pct">占比</span>
        <span class="c-type">类型</span>
      </div>
      <div class="tbody">
        <div
          v-for="n in listing.children"
          :key="rowKey(n)"
          class="row"
          :class="{
            selected: selectedPath === rowKey(n),
            clickable: canOpen(n),
            dim: n.inaccessible || n.reparse_skipped || n.cloud_placeholder,
          }"
          role="button"
          tabindex="0"
          @click="onClick(n)"
          @keydown.enter="onClick(n)"
          @contextmenu.prevent="emit('select', n)"
          :title="canOpen(n) ? '点击进入这个文件夹' : '点击查看详情'"
        >
          <span class="c-name">
            <span class="ico" :class="icon(n)"></span>
            <span class="nm">{{ n.name }}</span>
            <span v-if="n.system_reserved" class="tag tag-sys">系统保留</span>
            <span v-if="n.inaccessible" class="tag tag-inacc">无权限</span>
            <span v-if="n.reparse_skipped" class="tag">未跟随</span>
          </span>
          <span class="c-bar"><i :style="{ width: Math.max(0.5, n.percent_of_parent) + '%' }"></i></span>
          <span class="c-size">{{ humanBytes(n.allocated) }}</span>
          <span class="c-pct">{{ pct(n.allocated, listing.node.allocated) }}</span>
          <span class="c-type">{{ n.type_label }}</span>
        </div>
        <div v-if="listing.children.length === 0" class="empty muted">这个文件夹是空的</div>
        <div v-if="listing.total_children > listing.children.length" class="empty muted">
          只显示前 {{ listing.children.length }} 项（共 {{ listing.total_children }} 项）
        </div>
      </div>
    </div>
  </section>
</template>
