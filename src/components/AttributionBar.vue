<script setup lang="ts">
import { computed } from "vue";
import type { ScanSummary } from "../api";
import { fmtInt, humanBytes, pct } from "../format";

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
    <p
      v-if="summary.buckets.dedup_enabled"
      class="muted small dedup-line"
      title="同一份数据只算一次。硬链接主要在 WinSxS 与 System32 等目录之间，空间记在 WinSxS 以外的那个名字上；商店应用目录映射是 AppData 下的同名文件夹和 Packages\…\LocalCache 里是同一份数据（两边目录的文件 ID 相同），空间记在 Packages 那一侧。“无权限”是去重之后用卷已用空间差额推算的。"
    >
      已去重 {{ humanBytes(summary.buckets.hardlink_dedup + summary.buckets.mapped_dedup) }}：硬链接
      {{ fmtInt(summary.buckets.hardlink_dedup_files) }} 个多余名字 {{ humanBytes(summary.buckets.hardlink_dedup) }}<template
        v-if="summary.buckets.mapped_dirs > 0"
      >
        · 商店应用目录映射 {{ summary.buckets.mapped_dirs }} 处 {{ humanBytes(summary.buckets.mapped_dedup) }}</template
      >（同一份数据只算一次）
    </p>
    <p v-if="summary.buckets.overcount > 0" class="warn-line">
      注意：去重之后扫到的合计仍比卷已用空间多 {{ humanBytes(summary.buckets.overcount) }}，说明还有没识别出来的重复计数
      （或扫描期间盘上数据在变）。这时“无权限”的真实大小无法从差额推算，暂按 0 显示。
    </p>
    <p v-if="!summary.buckets.dedup_enabled" class="warn-line">这份结果没有做硬链接 / 目录映射去重（旧版本的快照），重复计数会让各项之和偏大。</p>
  </section>
</template>
