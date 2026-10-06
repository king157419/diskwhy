<script setup lang="ts">
// 暂存区（界面文案，阶段 J；后端标识仍叫 quarantine）：暂存到其他盘的文件按批次放着，可以整批还原；彻底删除要再确认一次。
// DiskWhy 不会自动删暂存区，超过保留天数只提醒。窄窗口时每批的“位置 / 大小 / 操作”换行排。
import { computed, onMounted, ref } from "vue";
import { listQuarantine, purgeBatch, restoreBatch, type BatchInfo } from "../api";
import { gbNum, sizeParts } from "../capacity";
import { shortPath } from "../labels";
import { actEnv, onQuarantineChanged } from "../store";

const batches = ref<BatchInfo[]>([]);
const loading = ref(true);
const busy = ref("");
const msg = ref("");
const err = ref("");
/** 彻底删除的二次确认：第一次点击记下批次，第二次点“确定删除”才执行 */
const confirmPurge = ref<string | null>(null);

const keepDays = computed(() => actEnv.value?.keep_days ?? 7);
const totalBytes = computed(() => batches.value.reduce((s, b) => s + b.bytes, 0));

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
    msg.value = `已还原 ${rep.restored} 个文件（${sizeParts(rep.restored_bytes).join(" ")}）${rep.skipped ? `，跳过 ${rep.skipped}` : ""}${rep.failed ? `，失败 ${rep.failed}` : ""}`;
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
    msg.value = r.removed ? `已彻底删除 ${r.deleted_entries} 个文件，${b.dest_root} 腾出 ${sizeParts(r.freed_bytes).join(" ")}` : `没能全部删掉：${r.error ?? ""}`;
    confirmPurge.value = null;
    await onQuarantineChanged(null);
    await load();
  } catch (e) {
    err.value = `删除失败：${e}`;
  } finally {
    busy.value = "";
  }
}

function day(unix: number) {
  const t = new Date(unix * 1000);
  return `${t.getMonth() + 1} 月 ${t.getDate()} 日`;
}
</script>

<template>
  <div class="q">
    <div class="top">
      <div class="k">暂存区</div>
      <div class="big"><span class="v num">{{ gbNum(totalBytes) }}</span><span class="u">GB</span></div>
      <div class="meta">
        <span><b class="num">{{ batches.length }}</b> 批</span><span>保留 <b class="num">{{ keepDays }}</b> 天</span>
        <button class="quiet" :disabled="loading" @click="load">刷新</button>
      </div>
    </div>

    <div class="list">
      <div v-if="msg" class="note ok">{{ msg }}</div>
      <div v-if="err" class="note err">{{ err }}</div>
      <div v-if="!loading && !batches.length" class="empty">空的</div>
      <div v-for="b in batches" :key="b.manifest_path" class="row" :class="{ expired: b.expired }">
        <div class="when">
          <b>{{ day(b.created_at) }}</b>
          <span v-if="b.expired" class="tag">已过 {{ keepDays }} 天</span>
          <span v-else class="d">第 {{ b.age_days + 1 }} 天</span>
        </div>
        <div class="what">
          <span v-for="p in b.items.slice(0, 3)" :key="p" class="mono" :title="p">{{ shortPath(p) }}</span>
          <span v-if="b.items.length > 3" class="more">等 {{ b.items.length }} 处</span>
          <span class="loc mono" :title="b.batch_dir">{{ b.batch_dir }}</span>
        </div>
        <div class="sz num">{{ sizeParts(b.bytes)[0] }}<small>{{ sizeParts(b.bytes)[1] }}</small></div>
        <div class="acts">
          <template v-if="confirmPurge === b.manifest_path">
            <span class="warn">删了找不回来</span>
            <button class="quiet danger" :disabled="!!busy" @click="doPurge(b)">确定删除</button>
            <button class="quiet" :disabled="!!busy" @click="confirmPurge = null">算了</button>
          </template>
          <template v-else>
            <button class="link" :disabled="!!busy" @click="doRestore(b)">{{ busy === b.manifest_path ? "请稍候" : "还原" }}</button>
            <button class="quiet" :disabled="!!busy" @click="doPurge(b)">彻底删除</button>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.top {
  position: absolute;
  left: var(--gutter);
  top: 92px;
  right: var(--gutter);
}
.k {
  font-size: var(--fs-group);
  font-weight: var(--w-med);
  color: var(--ink2);
}
[data-theme="C"] .k {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 22px;
  color: var(--ink);
}
.big {
  display: flex;
  align-items: baseline;
  gap: 12px;
  margin-top: 12px;
}
.big .v {
  font-family: var(--display);
  font-weight: var(--display-w);
  letter-spacing: var(--display-track);
  font-variation-settings: var(--display-vs);
  font-size: 120px;
  line-height: 0.85;
}
[data-theme="B"] .big .v {
  color: var(--accent);
}
.big .u {
  font-size: 28px;
  color: var(--ink2);
  font-weight: var(--w-med);
}
[data-theme="A"] .big .u {
  font-family: var(--display);
  font-style: italic;
  font-size: 40px;
}
.meta {
  margin-top: 22px;
  display: flex;
  align-items: center;
  gap: 18px;
  font-size: var(--fs-sub);
  color: var(--ink3);
}
.meta b {
  color: var(--ink);
  font-weight: var(--w-med);
}
.list {
  position: absolute;
  left: var(--gutter);
  right: var(--gutter);
  top: 330px;
  bottom: 40px;
  overflow-y: auto;
  border-top: var(--bw) solid var(--line2);
  scrollbar-width: thin;
}
.row {
  display: grid;
  grid-template-columns: 160px minmax(0, 1fr) 110px 210px;
  align-items: center;
  gap: 16px;
  min-height: 64px;
  padding: 10px 0;
  border-bottom: 1px solid var(--line);
}
.when {
  display: flex;
  flex-direction: column;
  gap: 4px;
  align-items: flex-start;
}
.when b {
  font-weight: var(--w-semi);
}
.when .d {
  font-size: var(--fs-meta);
  color: var(--ink3);
}
.what {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.what .mono {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--ink2);
}
.what .loc {
  color: var(--ink3);
  font-size: 11px;
}
.what .more {
  font-size: var(--fs-meta);
  color: var(--ink3);
}
.sz {
  text-align: right;
  font-size: var(--fn-row);
  font-weight: var(--w-med);
}
.sz small {
  font-size: 12px;
  color: var(--ink3);
  margin-left: 3px;
}
.acts {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 16px;
}
.warn {
  font-size: var(--fs-meta);
  color: var(--danger);
}
.quiet.danger {
  color: var(--danger);
  font-weight: var(--w-semi);
}
.note {
  padding: 10px 0;
  font-size: var(--fs-sub);
  border-bottom: 1px solid var(--line);
  white-space: pre-line;
  user-select: text;
}
.note.ok {
  color: var(--safe-label);
}
.note.err {
  color: var(--danger);
}
.empty {
  padding: 48px 0;
  color: var(--ink3);
  font-size: var(--fs-body);
}

/* ───────── 窄窗口 / 竖屏（阶段 J §3.7） ───────── */
@media (max-width: 1099px), (max-aspect-ratio: 1/1) {
  .top {
    top: 72px;
  }
  .big .v {
    font-size: 96px;
  }
  .list {
    top: 268px;
    bottom: 20px;
  }
  .row {
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-areas:
      "when sz"
      "what what"
      "acts acts";
    row-gap: 8px;
    padding: 14px 0;
  }
  .when {
    grid-area: when;
    flex-direction: row;
    align-items: baseline;
    gap: 10px;
  }
  .what {
    grid-area: what;
  }
  .sz {
    grid-area: sz;
  }
  .acts {
    grid-area: acts;
    justify-content: flex-start;
  }
}
@media (max-width: 1099px) and (max-height: 759px), (max-aspect-ratio: 1/1) and (max-height: 759px) {
  .big .v {
    font-size: 72px;
  }
  .meta {
    margin-top: 14px;
  }
  .list {
    top: 222px;
  }
}
</style>
