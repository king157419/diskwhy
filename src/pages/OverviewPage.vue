<script setup lang="ts">
import { computed } from "vue";
import { Archive, ArrowRight, ChevronRight, Cloud, Eye, File, Folder, HardDrive, Layers, Link2, Lock, ShieldAlert, ShieldCheck } from "lucide-vue-next";
import { tierKey, type DriveInfo, type NodeView } from "../api";
import { fmtDuration, fmtInt, humanBytes, pct } from "../format";
import PageHeader from "../components/ui/PageHeader.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import TierChip from "../components/ui/TierChip.vue";
import ScanControls from "../components/ScanControls.vue";
import ScanProgressPanel from "../components/ScanProgressPanel.vue";
import AttributionBar from "../components/AttributionBar.vue";
import TierSummaryCard from "../components/TierSummaryCard.vue";
import DiskRing from "../components/DiskRing.vue";
import {
  actEnv,
  cancelling,
  doCancel,
  doRelaunch,
  doScan,
  drives,
  env,
  openInAnalysis,
  page,
  phase,
  progress,
  relaunching,
  rootListing,
  selectedDrive,
  summary,
} from "../store";

const subtitle = computed(() => {
  const s = summary.value;
  if (phase.value === "scanning") return `正在扫描 ${selectedDrive.value}…`;
  if (!s) return "选一个盘开始扫描：DiskWhy 只读属性，不改动、不上传任何东西";
  return `${s.snapshot ? "快照 · " : ""}${s.root} · 用时 ${fmtDuration(s.duration_ms)} · ${fmtInt(s.files)} 个文件 · ${fmtInt(s.dirs)} 个文件夹`;
});

const topDirs = computed(() => (rootListing.value?.children ?? []).filter((c) => c.allocated > 0).slice(0, 7));
const rootTotal = computed(() => rootListing.value?.node.allocated ?? 1);

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

function openTop(n: NodeView) {
  const canOpen = n.kind === "Dir" && !n.reparse_skipped && !n.mapped && n.child_count > 0;
  if (canOpen) void openInAnalysis(n.path);
  else if (summary.value) void openInAnalysis(summary.value.root, n);
}

function scanDrive(d: DriveInfo) {
  selectedDrive.value = d.root;
  void doScan();
}
</script>

<template>
  <div class="page">
    <PageHeader title="概览" :subtitle="subtitle">
      <ScanControls />
    </PageHeader>

    <div class="page-body">
      <!-- 还没扫描 -->
      <div v-if="phase === 'idle' && !summary" class="idle">
        <EmptyState
          kind="disk"
          :title="`看看 ${selectedDrive || '磁盘'} 的空间都去哪了`"
          text="扫描只读取文件夹和文件的属性。只有你把东西加入清理清单、看过预览并确认之后才会删除（默认进回收站）。也可以用右上角的按钮打开以前保存的快照复核。"
        />
        <div class="drives">
          <button v-for="d in drives" :key="d.root" class="drive card" :class="{ on: selectedDrive === d.root }" @click="scanDrive(d)">
            <span class="drive-ico"><HardDrive :size="20" :stroke-width="1.7" /></span>
            <span class="drive-main">
              <span class="drive-name">{{ d.label || "本地磁盘" }} <span class="faint">({{ d.root.replace("\\", "") }})</span></span>
              <span class="meter"><i :style="{ width: pct(d.total_bytes - d.free_bytes, d.total_bytes) }"></i></span>
              <span class="drive-sub num">{{ humanBytes(d.free_bytes) }} 可用 · 共 {{ humanBytes(d.total_bytes) }}</span>
            </span>
            <span class="drive-go">扫描 <ArrowRight :size="14" /></span>
          </button>
        </div>
        <ul class="principles">
          <li><b>大小按“实际占用”算</b><span>按簇取整，压缩文件按压缩后算。</span></li>
          <li><b>不重复、不漏算</b><span>链接和挂载点不跟进去；硬链接、商店应用目录映射只算一次；云端占位文件只看属性，不触发下载。</span></li>
          <li><b>读不到也要说清楚</b><span>量不到的地方会明确标成“无权限 / 系统保留 / 未归属”，不会悄悄少算。</span></li>
        </ul>
      </div>

      <!-- 扫描中 -->
      <div v-else-if="phase === 'scanning'" class="scanning">
        <ScanProgressPanel :progress="progress" :root="selectedDrive" :cancelling="cancelling" @cancel="doCancel" />
      </div>

      <!-- 结果 -->
      <div v-else-if="summary" class="grid">
        <section class="disk card">
          <div class="card-title">磁盘</div>
          <div class="disk-body">
            <DiskRing :summary="summary" :size="168" />
            <div class="disk-nums">
              <div>
                <div class="eyebrow">已用</div>
                <div class="disk-big num">{{ humanBytes(summary.used_bytes) }}</div>
                <div class="faint small num">共 {{ humanBytes(summary.total_bytes) }}</div>
              </div>
              <div>
                <div class="eyebrow">可用</div>
                <div class="disk-mid num">{{ humanBytes(summary.free_bytes) }}</div>
              </div>
            </div>
          </div>
          <ul class="ring-legend">
            <li><i class="dot dot-safe"></i>可清理</li>
            <li><i class="dot dot-review"></i>需判断</li>
            <li><i class="dot dot-keep"></i>保留</li>
            <li><i class="dot dot-unknown"></i>未覆盖</li>
            <li><i class="dot inacc"></i>量不到</li>
          </ul>
        </section>

        <TierSummaryCard class="tiers" :tiers="summary.tiers" :used-bytes="summary.used_bytes" />

        <AttributionBar class="attr" :summary="summary" />

        <section class="top card">
          <div class="top-head">
            <div>
              <div class="card-title">最大的目录</div>
              <div class="muted small">{{ summary.root }} 下占用最多的几项</div>
            </div>
            <button class="btn sm" @click="openInAnalysis(summary.root)">空间分析 <ArrowRight :size="14" /></button>
          </div>
          <ul class="top-list">
            <li v-for="n in topDirs" :key="n.path + n.kind">
              <button class="top-row" :title="n.explanation?.what" @click="openTop(n)">
                <span class="t-ico" :class="'ico-' + tierKey(n.explanation)"><component :is="icon(n)" :size="16" :stroke-width="1.7" /></span>
                <span class="t-main">
                  <span class="t-name">{{ n.name }}</span>
                  <span class="t-what">
                    <TierChip :tier="tierKey(n.explanation)" />
                    <span class="ellipsis">{{ n.explanation?.what ?? n.type_label }}</span>
                  </span>
                </span>
                <span class="t-right">
                  <span class="t-size num">{{ humanBytes(n.allocated) }}</span>
                  <span class="meter"><i :style="{ width: Math.max(1, (n.allocated * 100) / rootTotal) + '%' }"></i></span>
                </span>
                <ChevronRight :size="15" class="t-chev" />
              </button>
            </li>
          </ul>
        </section>

        <section v-if="env && !env.elevated" class="note card">
          <span class="note-ico"><ShieldAlert :size="18" :stroke-width="1.8" /></span>
          <div class="note-text">
            <div class="note-title">当前是普通权限，有 <span class="num">{{ humanBytes(summary.buckets.inaccessible) }}</span> 量不到</div>
            <div class="muted small">System Volume Information、WindowsApps、其他用户的文件夹等系统目录读不了，已记到“无权限”里。以管理员身份重新启动可以看到这些目录。</div>
          </div>
          <button class="btn" :disabled="relaunching || phase !== 'done'" @click="doRelaunch">
            <ShieldCheck :size="15" /> {{ relaunching ? "等待授权…" : "以管理员重新启动" }}
          </button>
        </section>

        <section v-if="actEnv && actEnv.expired_batches > 0" class="note card">
          <span class="note-ico"><Archive :size="18" :stroke-width="1.8" /></span>
          <div class="note-text">
            <div class="note-title">隔离区里有 {{ actEnv.expired_batches }} 批文件已经放了超过 {{ actEnv.keep_days }} 天</div>
            <div class="muted small">确认用不到的话可以彻底清空了（DiskWhy 不会自动删）。</div>
          </div>
          <button class="btn" @click="page = 'quarantine'"><Eye :size="15" /> 打开隔离区</button>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.page-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 6px var(--sp-8) var(--sp-8);
}
.idle {
  max-width: 900px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  align-items: stretch;
}
.idle :deep(.empty) {
  padding-top: var(--sp-6);
}
.drives {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
}
.drive {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px 18px;
  text-align: left;
  cursor: pointer;
  color: var(--text-1);
  transition:
    border-color var(--dur-1) var(--ease),
    background var(--dur-1) var(--ease),
    transform var(--dur-1) var(--ease);
}
.drive:hover {
  border-color: var(--border-3);
  background: var(--surface-2);
}
.drive:active {
  transform: scale(0.99);
}
.drive.on {
  border-color: var(--accent-ring);
}
.drive-ico {
  width: 40px;
  height: 40px;
  border-radius: 11px;
  background: var(--accent-soft);
  color: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
}
.drive-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}
.drive-name {
  font-weight: 600;
}
.drive-sub {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.drive-go {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xs);
  color: var(--accent);
  opacity: 0;
  transform: translateX(-4px);
  transition:
    opacity var(--dur-1) var(--ease),
    transform var(--dur-1) var(--ease);
}
.drive:hover .drive-go,
.drive.on .drive-go {
  opacity: 1;
  transform: none;
}
.principles {
  list-style: none;
  padding: 0;
  margin: 28px 0 0;
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
}
.principles li {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 14px 16px;
  border-radius: var(--r-md);
  border: 1px dashed var(--border-2);
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.6;
}
.principles b {
  color: var(--text-1);
  font-size: var(--fs-sm);
}
.scanning {
  padding-top: var(--sp-8);
}
.grid {
  display: grid;
  grid-template-columns: repeat(12, minmax(0, 1fr));
  gap: 16px;
  max-width: 1400px;
  margin: 0 auto;
  animation: rise-in var(--dur-3) var(--ease);
}
.disk {
  grid-column: span 4;
  padding: 20px 22px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.tiers {
  grid-column: span 8;
}
.attr {
  grid-column: span 7;
}
.top {
  grid-column: span 5;
}
.note {
  grid-column: span 12;
}
@media (max-width: 1300px) {
  .disk {
    grid-column: span 5;
  }
  .tiers {
    grid-column: span 7;
  }
  .attr,
  .top {
    grid-column: span 6;
  }
}
.disk-body {
  display: flex;
  align-items: center;
  gap: 20px;
  flex: 1;
}
.disk-nums {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-width: 0;
}
.disk-big {
  font-size: var(--fs-2xl);
  font-weight: 600;
  letter-spacing: -0.03em;
  line-height: 1.15;
}
.disk-mid {
  font-size: var(--fs-xl);
  font-weight: 600;
  letter-spacing: -0.02em;
}
.ring-legend {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 6px 14px;
  font-size: var(--fs-2xs);
  color: var(--text-2);
}
.ring-legend li {
  display: flex;
  align-items: center;
  gap: 6px;
}
.dot.inacc {
  background: var(--c-inacc);
}
.top {
  padding: 20px 14px 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.top-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  padding: 0 8px;
  gap: 10px;
}
.top-head .small {
  margin-top: 2px;
}
.top-list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.top-row {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 9px 8px;
  border: 0;
  border-radius: var(--r-md);
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background var(--dur-1) var(--ease);
}
.top-row:hover {
  background: var(--surface-2);
}
.t-ico {
  width: 32px;
  height: 32px;
  border-radius: 9px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--surface-2);
  color: var(--text-2);
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
.t-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.t-name {
  font-weight: 550;
  font-size: var(--fs-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.t-what {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  min-width: 0;
}
.t-right {
  width: 92px;
  flex: none;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 5px;
}
.t-right .meter {
  width: 100%;
  height: 3px;
}
.t-size {
  font-size: var(--fs-sm);
  font-weight: 550;
}
.t-chev {
  color: var(--text-3);
  flex: none;
  transition: transform var(--dur-1) var(--ease);
}
.top-row:hover .t-chev {
  transform: translateX(2px);
  color: var(--text-1);
}
.note {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 18px;
}
.note-ico {
  width: 36px;
  height: 36px;
  border-radius: 10px;
  background: var(--warn-bg);
  color: var(--warn);
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
}
.note-text {
  flex: 1;
  min-width: 0;
}
.note-title {
  font-weight: 600;
  font-size: var(--fs-sm);
  margin-bottom: 2px;
}
</style>
