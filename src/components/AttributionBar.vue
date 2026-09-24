<script setup lang="ts">
import { computed } from "vue";
import { Link2, TriangleAlert } from "lucide-vue-next";
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
      note: `${fmtInt(inaccDirs)} 个目录读不了，大小用卷已用空间的差额推算（含 NTFS 元数据）`,
    },
    {
      key: "cloud",
      label: "云端占位",
      bytes: b.cloud_placeholder,
      cls: "seg-cloud",
      note: `${fmtInt(b.cloud_placeholder_files)} 个文件只在云端（逻辑 ${humanBytes(b.cloud_placeholder_logical)}），不占本地空间，也不会被打开`,
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
    <div class="card-head">
      <div>
        <div class="card-title">已用空间归属</div>
        <div class="muted small">每个字节都有去处：量到的、系统保留的、读不到的，都明确标出来</div>
      </div>
    </div>
    <div class="stack" role="img" :aria-label="'已用空间归属：' + segs.map((s) => s.label + ' ' + humanBytes(s.bytes)).join('，')">
      <div
        v-for="s in segs.filter((x) => x.bytes > 0)"
        :key="s.key"
        class="seg"
        :class="s.cls"
        :style="{ flexGrow: s.bytes / denom }"
        :title="`${s.label}：${humanBytes(s.bytes)}（${pct(s.bytes, summary.used_bytes)}）`"
      ></div>
    </div>
    <ul class="legend">
      <li v-for="s in segs" :key="s.key" :class="{ zero: s.bytes === 0 }">
        <i class="sw" :class="s.cls"></i>
        <span class="lg-label">{{ s.label }}</span>
        <span class="lg-note">{{ s.note }}</span>
        <span class="lg-size num">{{ humanBytes(s.bytes) }}</span>
        <span class="lg-pct num">{{ pct(s.bytes, summary.used_bytes) }}</span>
      </li>
    </ul>
    <p
      v-if="summary.buckets.dedup_enabled"
      class="dedup"
      title="同一份数据只算一次。硬链接主要在 WinSxS 与 System32 等目录之间，空间记在 WinSxS 以外的那个名字上；商店应用目录映射是 AppData 下的同名文件夹和 Packages\…\LocalCache 里是同一份数据（两边目录的文件 ID 相同），空间记在 Packages 那一侧。“无权限”是去重之后用卷已用空间差额推算的。"
    >
      <Link2 :size="14" :stroke-width="1.8" />
      <span>
        已去重 <b class="num">{{ humanBytes(summary.buckets.hardlink_dedup + summary.buckets.mapped_dedup) }}</b>：硬链接
        <span class="num">{{ fmtInt(summary.buckets.hardlink_dedup_files) }}</span> 个多余名字 <span class="num">{{ humanBytes(summary.buckets.hardlink_dedup) }}</span
        ><template v-if="summary.buckets.mapped_dirs > 0">
          · 商店应用目录映射 <span class="num">{{ summary.buckets.mapped_dirs }}</span> 处 <span class="num">{{ humanBytes(summary.buckets.mapped_dedup) }}</span></template
        >（同一份数据只算一次）
      </span>
    </p>
    <div v-if="summary.buckets.overcount > 0" class="callout warn small-callout">
      <TriangleAlert :size="15" />
      <span>
        注意：去重之后扫到的合计仍比卷已用空间多 {{ humanBytes(summary.buckets.overcount) }}，说明还有没识别出来的重复计数（或扫描期间盘上数据在变）。这时“无权限”的真实大小无法从差额推算，暂按 0 显示。
      </span>
    </div>
    <div v-if="!summary.buckets.dedup_enabled" class="callout warn small-callout">
      <TriangleAlert :size="15" />
      <span>这份结果没有做硬链接 / 目录映射去重（旧版本的快照），重复计数会让各项之和偏大。</span>
    </div>
  </section>
</template>

<style scoped>
.attr {
  padding: 20px 22px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.card-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
}
.card-head .small {
  margin-top: 2px;
}
.stack {
  display: flex;
  gap: 3px;
  height: 14px;
}
.seg {
  min-width: 4px;
  border-radius: 4px;
  transition: filter var(--dur-1) var(--ease);
}
.seg:first-child {
  border-radius: 7px 4px 4px 7px;
}
.seg:last-child {
  border-radius: 4px 7px 7px 4px;
}
.seg:hover {
  filter: brightness(1.15);
}
.seg-scanned {
  background: var(--c-scanned);
}
.seg-sys {
  background: var(--c-sys);
}
.seg-inacc {
  background: repeating-linear-gradient(135deg, var(--c-inacc) 0 3px, transparent 3px 6px), var(--surface-3);
}
.seg-cloud {
  background: var(--c-cloud);
}
.seg-unattr {
  background: var(--c-unattr);
}
.legend {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
}
.legend li {
  display: grid;
  grid-template-columns: 12px 70px 1fr auto 62px;
  align-items: center;
  gap: 10px;
  padding: 7px 0;
  border-top: 1px solid var(--border-1);
  font-size: var(--fs-sm);
}
.legend li:first-child {
  border-top: 0;
}
.legend li.zero {
  opacity: 0.5;
}
.sw {
  width: 10px;
  height: 10px;
  border-radius: 3px;
}
.lg-label {
  font-weight: 550;
}
.lg-note {
  color: var(--text-3);
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.lg-size {
  text-align: right;
  font-weight: 550;
}
.lg-pct {
  text-align: right;
  color: var(--text-2);
  font-size: var(--fs-xs);
}
.dedup {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  font-size: var(--fs-xs);
  color: var(--text-2);
  padding: 10px 12px;
  border-radius: var(--r-md);
  background: var(--surface-2);
}
.dedup svg {
  flex: none;
  margin-top: 2px;
  color: var(--accent);
}
.small-callout {
  font-size: var(--fs-xs);
}
</style>
