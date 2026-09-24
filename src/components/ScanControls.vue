<script setup lang="ts">
// 页头右侧：盘符选择 + 扫描文件夹 / 快照 + 主按钮。
import { FolderOpen, FolderSearch, Save, ScanSearch, RotateCw } from "lucide-vue-next";
import type { DriveInfo } from "../api";
import { humanBytes } from "../format";
import { customRoot, doOpenSnapshot, doPickFolder, doSaveSnapshot, doScan, drives, phase, selectedDrive, snapBusy } from "../store";

function driveLabel(d: DriveInfo): string {
  const name = d.label || "本地磁盘";
  const used = d.total_bytes - d.free_bytes;
  return `${d.root.replace("\\", "")}  ${name} · ${humanBytes(used)} / ${humanBytes(d.total_bytes)}`;
}
</script>

<template>
  <div class="scan-controls">
    <label class="sr-only" for="drive">选择磁盘</label>
    <select id="drive" v-model="selectedDrive" class="input drive" :disabled="phase === 'scanning'">
      <option v-for="d in drives" :key="d.root" :value="d.root">{{ driveLabel(d) }}</option>
      <option v-if="customRoot" :value="customRoot" :title="customRoot">文件夹：…\{{ customRoot.split("\\").slice(-2).join("\\") }}</option>
    </select>
    <div class="tool-group">
      <button class="btn icon ghost" aria-label="扫描文件夹…" title="只扫描某个文件夹（而不是整个盘）" :disabled="phase === 'scanning'" @click="doPickFolder">
        <FolderSearch :size="17" :stroke-width="1.8" />
      </button>
      <button
        class="btn icon ghost"
        aria-label="打开快照"
        title="打开之前保存的快照（.dwsnap）或 diskwhy-cli --json 输出"
        :disabled="phase === 'scanning' || snapBusy"
        @click="doOpenSnapshot()"
      >
        <FolderOpen :size="17" :stroke-width="1.8" />
      </button>
      <button
        class="btn icon ghost"
        aria-label="保存快照"
        title="把这次扫描结果存成文件，之后可以再打开复核"
        :disabled="phase !== 'done' || snapBusy"
        @click="doSaveSnapshot"
      >
        <Save :size="17" :stroke-width="1.8" />
      </button>
    </div>
    <button class="btn primary" :disabled="!selectedDrive || phase === 'scanning'" @click="doScan">
      <RotateCw v-if="phase === 'done'" :size="16" :stroke-width="2" />
      <ScanSearch v-else :size="16" :stroke-width="2" />
      {{ phase === "done" ? "重新扫描" : "开始扫描" }}
    </button>
  </div>
</template>

<style scoped>
.scan-controls {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.drive {
  width: 300px;
  font-family: var(--font-sans);
}
.tool-group {
  display: flex;
  gap: 2px;
  padding: 0 2px;
}
@media (max-width: 1360px) {
  .drive {
    width: 240px;
  }
}
</style>
