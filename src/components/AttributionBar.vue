<script setup lang="ts">
import { computed } from "vue";
import type { ScanSummary } from "../api";
import { humanBytes, pct } from "../format";

const props = defineProps<{ summary: ScanSummary }>();

interface Seg {
  key: string;
  label: string;
  bytes: number;
  cls: string;
  note: string;
}

const segs = computed<Seg[]>(() => {
  const b = props.summary.buckets;
  const inaccDirs = b.inaccessible_dirs_other + b.inaccessible_dirs_system;
  return [
    { key: "scanned", label: "已扫描", bytes: b.scanned, cls: "seg-scanned", note: "扫到并量出大小的普通文件" },
    {
      key: "sys",
      label: "系统保留",
      bytes: b.system_reserved,
      cls: "seg-sys",
      note:
        "回收站、卷影副本、pagefile、hiberfil、Recovery 等" +
        (b.system_reserved_estimated > 0 ? `（其中 ${humanBytes(b.system_reserved_estimated)} 由差额推算）` : ""),
    },
    {
      key: "inacc",
      label: "无权限",
      bytes: b.inaccessible,
      cls: "seg-inacc",
      note: `${inaccDirs} 个目录读不了，大小只能用卷已用空间的差额推算（含 NTFS 元数据）`,
    },
    {
      key: "cloud",
      label: "云端占位",
      bytes: b.cloud_placeholder,
      cls: "seg-cloud",
      note: `${b.cloud_placeholder_files} 个文件只在云端，逻辑大小 ${humanBytes(b.cloud_placeholder_logical)}，不占本地空间，也不会被打开`,
    },
    {
      key: "unattr",
      label: "未归属",
      bytes: b.unattributed,
      cls: "seg-unattr",
      note: b.scope_is_volume_root ? "说不清的部分" : "扫描根不是卷根，这里是扫描范围之外的内容",
    },
  ];
});

const denom = computed(() => segs.value.reduce((s, x) => s + x.bytes, 0) || 1);
</script>

<template>
  <section class="attr card">
    <div class="attr-head">
      <div>
        <span class="attr-title">已用空间归属</span>
        <span class="muted">
          {{ summary.root }} 已用 {{ humanBytes(summary.used_bytes) }} / 共 {{ humanBytes(summary.total_bytes) }}，
          可用 {{ humanBytes(summary.free_bytes) }}
        </span>
      </div>
    </div>
    <div class="stack" role="img" aria-label="已用空间归属堆叠条">
      <div
        v-for="s in segs.filter((x) => x.bytes > 0)"
        :key="s.key"
        class="seg"
        :class="s.cls"
        :style="{ width: (s.bytes * 100) / denom + '%' }"
        :title="`${s.label}：${humanBytes(s.bytes)}（${pct(s.bytes, summary.used_bytes)}）`"
      ></div>
    </div>
    <ul class="legend">
      <li v-for="s in segs" :key="s.key" :title="s.note">
        <i class="dot" :class="s.cls"></i>
        <b>{{ s.label }}</b>
        <span>{{ humanBytes(s.bytes) }}</span>
        <span class="muted">{{ pct(s.bytes, summary.used_bytes) }}</span>
      </li>
    </ul>
    <p v-if="summary.buckets.overcount > 0" class="warn-line">
      注意：扫到的文件合计比卷已用空间多 {{ humanBytes(summary.buckets.overcount) }}。这是同一份数据被多个路径看到而重复计数
      （硬链接、应用目录映射等，常见于 WinSxS 和商店应用），阶段 C 会去重。在此之前，无权限目录的真实大小无法从差额推算，暂按 0 显示。
    </p>
    <p v-else-if="summary.buckets.inaccessible > 0" class="muted small">
      “无权限”一项是推算值：卷已用空间减去所有量得到的部分。以管理员身份扫描可以把它变小。
    </p>
  </section>
</template>
