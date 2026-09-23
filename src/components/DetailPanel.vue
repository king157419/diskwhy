<script setup lang="ts">
import { computed } from "vue";
import type { NodeView } from "../api";
import { attrText, fmtInt, fmtTime, humanBytes } from "../format";

const props = defineProps<{ node: NodeView | null }>();

const kindText = computed(() => {
  const n = props.node;
  if (!n) return "";
  return n.type_label;
});

const status = computed(() => {
  const n = props.node;
  if (!n) return [] as string[];
  const s: string[] = [];
  if (n.inaccessible) s.push(`无法读取（Win32 错误 ${n.error_code}${n.error_code === 5 ? "：拒绝访问" : ""}）`);
  if (n.reparse_skipped) s.push("这是链接/挂载点，指向别处，没有跟进去统计，避免重复计算");
  if (n.cloud_placeholder) s.push("云端占位文件：内容在云上，本地几乎不占空间；DiskWhy 只读属性，没有打开它");
  if (n.system_reserved) s.push("属于系统保留项（回收站 / 卷影 / 页面文件等）");
  if (n.kind === "Rest") s.push(`为了省内存，这个文件夹里除最大的若干个文件外，其余 ${fmtInt(n.files)} 个文件合并显示`);
  return s;
});
</script>

<template>
  <aside class="detail card">
    <template v-if="node">
      <h2 class="d-name" :title="node.name">{{ node.name }}</h2>
      <div class="d-type">{{ kindText }}</div>
      <dl class="d-grid">
        <dt>路径</dt>
        <dd class="path">{{ node.path }}</dd>
        <dt>实际占用</dt>
        <dd><b>{{ humanBytes(node.allocated) }}</b> <span class="muted">（{{ fmtInt(node.allocated) }} 字节）</span></dd>
        <dt>逻辑大小</dt>
        <dd>{{ humanBytes(node.logical) }} <span class="muted">（{{ fmtInt(node.logical) }} 字节）</span></dd>
        <dt>修改时间</dt>
        <dd>{{ node.kind === "Rest" ? "（最新）" : "" }}{{ fmtTime(node.mtime) }}</dd>
        <template v-if="node.kind === 'Dir'">
          <dt>子项数</dt>
          <dd>{{ fmtInt(node.child_count) }} 项（内含 {{ fmtInt(node.files) }} 个文件、{{ fmtInt(node.dirs) }} 个文件夹）</dd>
        </template>
        <template v-if="node.kind !== 'Rest'">
          <dt>属性</dt>
          <dd>{{ attrText(node.attrs) }}</dd>
        </template>
      </dl>
      <ul v-if="status.length" class="d-status">
        <li v-for="s in status" :key="s">{{ s }}</li>
      </ul>
      <section class="explain">
        <h3>说明</h3>
        <p class="muted">
          阶段 B 接入：这里将用大白话告诉你——这是什么、属于哪个软件、为什么会有、删了会失去什么、能腾出多少、该怎么腾。
        </p>
      </section>
      <p class="muted small">“实际占用”是文件在磁盘上真正占的空间（按簇取整，压缩文件按压缩后算）；“逻辑大小”是文件本身的字节数。</p>
    </template>
    <template v-else>
      <p class="muted">点击左侧列表中的一项查看详情。</p>
    </template>
  </aside>
</template>
