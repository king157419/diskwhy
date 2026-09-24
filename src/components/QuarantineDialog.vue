<script setup lang="ts">
import { onMounted, ref } from "vue";
import { listQuarantine, purgeBatch, restoreBatch, type BatchInfo, type ScanSummary } from "../api";
import { fmtInt, fmtTime, humanBytes } from "../format";

const emit = defineEmits<{ close: []; changed: [summary: ScanSummary | null] }>();

const batches = ref<BatchInfo[]>([]);
const loading = ref(true);
const busy = ref("");
const msg = ref("");
const err = ref("");
/** 彻底清空的二次确认：第一次点击记下批次，第二次点“确定彻底删除”才执行 */
const confirmPurge = ref<string | null>(null);

async function load() {
  loading.value = true;
  err.value = "";
  try {
    batches.value = await listQuarantine();
  } catch (e) {
    err.value = String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

async function doRestore(b: BatchInfo) {
  busy.value = b.manifest_path;
  msg.value = "";
  err.value = "";
  confirmPurge.value = null;
  try {
    const r = await restoreBatch(b.manifest_path);
    const rep = r.report;
    msg.value = `已还原 ${rep.restored} 个文件（${humanBytes(rep.restored_bytes)}）到原来的位置${rep.skipped ? `，跳过 ${rep.skipped}` : ""}${rep.failed ? `，失败 ${rep.failed}` : ""}。`;
    if (rep.problems.length) err.value = rep.problems.slice(0, 5).map(([p, w]) => `${p}：${w}`).join("\n");
    emit("changed", r.summary);
    await load();
  } catch (e) {
    err.value = `还原失败：${e}`;
  } finally {
    busy.value = "";
  }
}

async function doPurge(b: BatchInfo) {
  if (confirmPurge.value !== b.manifest_path) {
    confirmPurge.value = b.manifest_path;
    return;
  }
  busy.value = b.manifest_path;
  msg.value = "";
  err.value = "";
  try {
    const r = await purgeBatch(b.manifest_path);
    msg.value = r.removed
      ? `已彻底删除这一批 ${r.deleted_entries} 个文件，${b.dest_root} 腾出约 ${humanBytes(r.freed_bytes)}。`
      : `没能全部删掉：${r.error ?? ""}`;
    confirmPurge.value = null;
    emit("changed", null);
    await load();
  } catch (e) {
    err.value = `清空失败：${e}`;
  } finally {
    busy.value = "";
  }
}
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal card quarantine" role="dialog" aria-label="隔离区">
      <h2>隔离区</h2>
      <p class="muted small">
        选“隔离到其他盘”清理的文件放在这里（每个盘的 DiskWhy-Quarantine 文件夹），按批次保存，<b>保留 7 天</b>。DiskWhy 不会自动删除，超过 7 天只会提醒你。
      </p>
      <p v-if="loading" class="muted">正在读取…</p>
      <p v-else-if="!batches.length" class="muted">隔离区是空的。</p>
      <div v-for="b in batches" :key="b.manifest_path" class="q-batch" :class="{ expired: b.expired }">
        <div class="q-head">
          <b>{{ fmtTime(b.created_at) }}</b>
          <span class="muted small">批次 {{ b.batch_id }}</span>
          <span v-if="b.expired" class="tag warn-tag">已超过 7 天</span>
          <span v-else class="muted small">第 {{ b.age_days + 1 }} 天</span>
          <span class="spacer"></span>
          <b>{{ humanBytes(b.bytes) }}</b>
          <span class="muted small">{{ fmtInt(b.stored_entries) }} 个文件</span>
        </div>
        <div class="small muted">位置：{{ b.batch_dir }}</div>
        <div class="small">原来在：
          <span v-for="p in b.items" :key="p" class="q-item">{{ p }}</span>
        </div>
        <div class="q-actions">
          <button class="btn" :disabled="!!busy" @click="doRestore(b)">{{ busy === b.manifest_path ? "请稍候…" : "还原整批" }}</button>
          <template v-if="confirmPurge === b.manifest_path">
            <span class="ai-error small">彻底删除后<b>无法恢复</b>（不进回收站）。确定吗？</span>
            <button class="btn danger" :disabled="!!busy" @click="doPurge(b)">确定彻底删除</button>
            <button class="btn" :disabled="!!busy" @click="confirmPurge = null">算了</button>
          </template>
          <button v-else class="btn" :disabled="!!busy" @click="doPurge(b)">彻底清空…</button>
        </div>
      </div>
      <p v-if="msg" class="ok-line small">{{ msg }}</p>
      <p v-if="err" class="ai-error small" style="white-space: pre-line">{{ err }}</p>
      <div class="modal-actions">
        <button class="btn" :disabled="loading" @click="load">刷新</button>
        <span class="spacer"></span>
        <button class="btn primary" @click="emit('close')">关闭</button>
      </div>
    </div>
  </div>
</template>
