<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Archive, Clock, FolderOpen, RefreshCw, RotateCcw, Trash2, TriangleAlert } from "lucide-vue-next";
import { listQuarantine, purgeBatch, restoreBatch, type BatchInfo } from "../api";
import { fmtInt, fmtTime, humanBytes } from "../format";
import PageHeader from "../components/ui/PageHeader.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import { actEnv, onQuarantineChanged } from "../store";

const batches = ref<BatchInfo[]>([]);
const loading = ref(true);
const busy = ref("");
const msg = ref("");
const err = ref("");
/** 彻底清空的二次确认：第一次点击记下批次，第二次点“确定彻底删除”才执行 */
const confirmPurge = ref<string | null>(null);

const keepDays = computed(() => actEnv.value?.keep_days ?? 7);
const totalBytes = computed(() => batches.value.reduce((s, b) => s + b.bytes, 0));
const expiredCount = computed(() => batches.value.filter((b) => b.expired).length);

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
    await onQuarantineChanged(r.summary);
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
    await onQuarantineChanged(null);
    await load();
  } catch (e) {
    err.value = `清空失败：${e}`;
  } finally {
    busy.value = "";
  }
}

function dayOf(unix: number): { d: string; m: string } {
  const t = new Date(unix * 1000);
  return { d: String(t.getDate()).padStart(2, "0"), m: `${t.getFullYear()}.${String(t.getMonth() + 1).padStart(2, "0")}` };
}
</script>

<template>
  <div class="page">
    <PageHeader title="隔离区" :subtitle="`选“隔离到其他盘”清理的文件放在这里，按批次保存，保留 ${keepDays} 天；DiskWhy 不会自动删除，超过 ${keepDays} 天只会提醒你`">
      <button class="btn" :disabled="loading" @click="load"><RefreshCw :size="15" /> 刷新</button>
    </PageHeader>

    <div class="page-body">
      <div v-if="msg" class="callout ok"><RotateCcw :size="16" /><span>{{ msg }}</span></div>
      <div v-if="err" class="callout danger"><TriangleAlert :size="16" /><span style="white-space: pre-line">{{ err }}</span></div>

      <p v-if="loading && !batches.length" class="muted loading">正在读取…</p>
      <div v-else-if="!batches.length" class="center">
        <EmptyState
          kind="quarantine"
          title="隔离区是空的"
          :text="`清理时选“隔离到其他盘”，文件会先挪到别的盘的 DiskWhy-Quarantine 文件夹，C 盘立刻腾出空间；${keepDays} 天内可以一键整批还原。`"
        />
      </div>

      <template v-else>
        <div class="stats">
          <div class="stat card">
            <span class="s-ico"><Archive :size="18" /></span>
            <div><div class="s-k">批次</div><div class="s-v num">{{ batches.length }}</div></div>
          </div>
          <div class="stat card">
            <span class="s-ico"><FolderOpen :size="18" /></span>
            <div><div class="s-k">占用其他盘</div><div class="s-v num">{{ humanBytes(totalBytes) }}</div></div>
          </div>
          <div class="stat card" :class="{ warn: expiredCount > 0 }">
            <span class="s-ico"><Clock :size="18" /></span>
            <div><div class="s-k">超过 {{ keepDays }} 天</div><div class="s-v num">{{ expiredCount }}</div></div>
          </div>
        </div>

        <TransitionGroup tag="div" name="batch" class="batches">
          <article v-for="b in batches" :key="b.manifest_path" class="batch card" :class="{ expired: b.expired }">
            <div class="date">
              <div class="date-d num">{{ dayOf(b.created_at).d }}</div>
              <div class="date-m num">{{ dayOf(b.created_at).m }}</div>
            </div>
            <div class="b-main">
              <div class="b-head">
                <span class="b-size num">{{ humanBytes(b.bytes) }}</span>
                <span class="faint small"><span class="num">{{ fmtInt(b.stored_entries) }}</span> 个文件 · {{ fmtTime(b.created_at) }}</span>
                <span v-if="b.expired" class="tag warn">已超过 {{ keepDays }} 天</span>
                <span v-else class="tag">第 {{ b.age_days + 1 }} 天</span>
              </div>
              <div class="b-items">
                <span class="faint small">原来在</span>
                <span v-for="p in b.items" :key="p" class="b-item mono" :title="p">{{ p }}</span>
              </div>
              <div class="b-loc faint small mono" :title="b.batch_dir">位置：{{ b.batch_dir }} · 批次 {{ b.batch_id }}</div>
              <div v-if="confirmPurge === b.manifest_path" class="confirm callout danger">
                <TriangleAlert :size="16" />
                <span class="grow">彻底删除后<b>无法恢复</b>（不进回收站）。确定吗？</span>
                <button class="btn danger sm" :disabled="!!busy" @click="doPurge(b)">确定彻底删除</button>
                <button class="btn sm" :disabled="!!busy" @click="confirmPurge = null">算了</button>
              </div>
            </div>
            <div class="b-actions">
              <button class="btn primary" :disabled="!!busy" @click="doRestore(b)">
                <RotateCcw :size="15" /> {{ busy === b.manifest_path ? "请稍候…" : "还原整批" }}
              </button>
              <button v-if="confirmPurge !== b.manifest_path" class="btn subtle-danger" :disabled="!!busy" @click="doPurge(b)">
                <Trash2 :size="15" /> 彻底清空…
              </button>
            </div>
          </article>
        </TransitionGroup>
      </template>
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
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.page-body > * {
  flex-shrink: 0;
}
.center {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.loading {
  padding: 40px;
  text-align: center;
}
.stats {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 14px;
}
.stat {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px 18px;
}
.s-ico {
  width: 38px;
  height: 38px;
  border-radius: 11px;
  background: var(--accent-soft);
  color: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
}
.stat.warn .s-ico {
  background: var(--warn-bg);
  color: var(--warn);
}
.s-k {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.s-v {
  font-size: var(--fs-xl);
  font-weight: 600;
  letter-spacing: -0.02em;
}
.batches {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.batch {
  display: flex;
  gap: 18px;
  padding: 18px 20px;
  align-items: flex-start;
  transition: border-color var(--dur-1) var(--ease);
}
.batch:hover {
  border-color: var(--border-2);
}
.batch.expired {
  border-color: var(--warn-border);
}
.date {
  width: 64px;
  flex: none;
  text-align: center;
  padding: 8px 0 9px;
  border-radius: var(--r-md);
  background: var(--surface-2);
}
.date-d {
  font-size: var(--fs-2xl);
  font-weight: 600;
  line-height: 1.05;
}
.date-m {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.b-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.b-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  flex-wrap: wrap;
}
.b-size {
  font-size: var(--fs-xl);
  font-weight: 600;
  letter-spacing: -0.02em;
}
.b-items {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
}
.b-item {
  font-size: var(--fs-2xs);
  padding: 3px 8px;
  border-radius: var(--r-sm);
  background: var(--surface-2);
  border: 1px solid var(--border-1);
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.b-loc {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.confirm {
  align-items: center;
}
.confirm .grow {
  flex: 1;
}
.b-actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: none;
}
.batch-enter-active,
.batch-leave-active {
  transition:
    opacity var(--dur-2) var(--ease),
    transform var(--dur-2) var(--ease);
}
.batch-enter-from,
.batch-leave-to {
  opacity: 0;
  transform: translateY(6px);
}
</style>
