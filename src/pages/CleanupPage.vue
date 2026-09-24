<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { ArrowRight, Archive, CircleCheck, FolderOpen, Layers, ScanEye, Trash2, TriangleAlert, X } from "lucide-vue-next";
import { pickFolder, type Strategy, type Tier } from "../api";
import { fmtInt, humanBytes } from "../format";
import PageHeader from "../components/ui/PageHeader.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import TierChip from "../components/ui/TierChip.vue";
import {
  actEnv,
  cart,
  checkedCart,
  cleanupMode,
  clearCart,
  CUSTOM_DEST,
  doPreview,
  drives,
  page,
  previewBusy,
  qCustom,
  qDest,
  removeFromCart,
  summary,
  toggleChecked,
  unchecked,
} from "../store";

type Cat = "all" | Tier;
const cat = ref<Cat>("all");

const cats = computed(() => {
  const by = (t: Tier | null) => cart.value.filter((c) => !t || c.tier === t);
  const mk = (key: Cat, label: string, sub: string) => {
    const list = by(key === "all" ? null : (key as Tier));
    return { key, label, sub, count: list.length, bytes: list.reduce((s, c) => s + c.allocated, 0) };
  };
  return [
    mk("all", "全部", "清单里的所有项"),
    mk("safe", "可以放心清理", "缓存类，删了会自动重建"),
    mk("review", "需要你判断", "删了有代价，逐项看清楚"),
  ];
});

const shown = computed(() => (cat.value === "all" ? cart.value : cart.value.filter((c) => c.tier === cat.value)));
const checkedBytes = computed(() => checkedCart.value.reduce((s, c) => s + c.allocated, 0));
const allShownChecked = computed(() => shown.value.length > 0 && shown.value.every((c) => !unchecked.value.has(c.path.toLowerCase())));
function toggleAll() {
  const s = new Set(unchecked.value);
  const on = allShownChecked.value;
  for (const c of shown.value) {
    const k = c.path.toLowerCase();
    if (on) s.add(k);
    else s.delete(k);
  }
  unchecked.value = s;
}

// ───── 删到哪里 ─────
const scanRoot = computed(() => summary.value?.root ?? "C:\\");
const allDrives = computed(() => actEnv.value?.drives ?? drives.value);
const srcDrive = computed(() => (scanRoot.value.slice(0, 2) + "\\").toUpperCase());
/** 隔离区只能选源所在盘以外的固定盘 */
const otherDrives = computed(() => allDrives.value.filter((d) => d.root.toUpperCase() !== srcDrive.value));
const pickErr = ref("");
async function browse() {
  pickErr.value = "";
  try {
    const p = await pickFolder("选择隔离区放在哪个文件夹");
    if (p) qCustom.value = p;
  } catch (e) {
    pickErr.value = String(e);
  }
}
watch(
  otherDrives,
  (v) => {
    if (qDest.value !== CUSTOM_DEST && !v.find((d) => d.root === qDest.value)) qDest.value = v[0]?.root ?? CUSTOM_DEST;
  },
  { immediate: true },
);
function strategy(): Strategy {
  if (cleanupMode.value === "recycle") return { kind: "recycle_bin" };
  return { kind: "quarantine", dest_root: qDest.value === CUSTOM_DEST ? qCustom.value.trim() : qDest.value };
}
const canPreview = computed(
  () => checkedCart.value.length > 0 && (cleanupMode.value === "recycle" || (qDest.value === CUSTOM_DEST ? !!qCustom.value.trim() : !!qDest.value)),
);
const destFree = computed(() => otherDrives.value.find((d) => d.root === qDest.value)?.free_bytes);
const destLabel = computed(() => {
  if (cleanupMode.value === "recycle") return "回收站";
  const d = qDest.value === CUSTOM_DEST ? qCustom.value.trim() || "（还没选文件夹）" : qDest.value;
  return `隔离到 ${d}`;
});
</script>

<template>
  <div class="page">
    <PageHeader title="清理清单" subtitle="加入清单不会马上删除：预览会列出真正要删的每个文件、跳过的文件和原因，确认后才执行">
      <button v-if="cart.length" class="btn ghost sm" @click="clearCart"><X :size="14" /> 清空清单</button>
    </PageHeader>

    <div v-if="!cart.length" class="page-pad center">
      <EmptyState
        kind="cart"
        title="清单还是空的"
        text="在空间分析里点开一项，“可以放心清理”和“需要你来判断”的项目下面有“加入清理清单”按钮；“建议保留”和“未知”的项目不能加入。"
      >
        <button class="btn primary" @click="page = 'analysis'">去空间分析 <ArrowRight :size="15" /></button>
      </EmptyState>
    </div>

    <template v-else>
      <div class="work">
        <!-- 左：分类 + 删到哪里 -->
        <aside class="side">
          <div class="cats card">
            <button v-for="c in cats" :key="c.key" class="cat" :class="{ on: cat === c.key }" @click="cat = c.key">
              <span class="cat-ico" :class="'ico-' + c.key">
                <Layers v-if="c.key === 'all'" :size="17" :stroke-width="1.7" />
                <CircleCheck v-else-if="c.key === 'safe'" :size="17" :stroke-width="1.7" />
                <TriangleAlert v-else :size="17" :stroke-width="1.7" />
              </span>
              <span class="cat-main">
                <span class="cat-name">{{ c.label }}</span>
                <span class="cat-sub">{{ c.count }} 项 · {{ c.sub }}</span>
              </span>
              <span class="cat-size num">{{ humanBytes(c.bytes) }}</span>
            </button>
          </div>

          <fieldset class="strategy card" :disabled="previewBusy">
            <legend class="card-title">删到哪里</legend>
            <label class="opt" :class="{ on: cleanupMode === 'recycle' }">
              <input type="radio" value="recycle" v-model="cleanupMode" />
              <span class="opt-ico"><Trash2 :size="16" /></span>
              <span class="opt-text">
                <b>回收站</b><span class="tag accent">默认</span>
                <span class="opt-sub">可以在回收站里还原。回收站在同一个盘上，<b>清空回收站之后</b>才真正腾出空间。</span>
              </span>
            </label>
            <label class="opt" :class="{ on: cleanupMode === 'quarantine' }">
              <input type="radio" value="quarantine" v-model="cleanupMode" />
              <span class="opt-ico"><Archive :size="16" /></span>
              <span class="opt-text">
                <b>隔离到其他盘</b>
                <span class="opt-sub">
                  挪到别的盘的 DiskWhy-Quarantine 文件夹，{{ scanRoot.slice(0, 2) }} 立刻腾出空间；保留 7 天（不会自动删），可以一键整批还原。
                  <template v-if="!otherDrives.length">本机没有别的固定盘，只能选文件夹；同一个盘上不会腾出空间。</template>
                </span>
              </span>
            </label>
            <div v-if="cleanupMode === 'quarantine'" class="dest">
              <label for="qdest" class="faint small">隔离到</label>
              <select id="qdest" v-model="qDest" class="input">
                <option v-for="d in otherDrives" :key="d.root" :value="d.root">
                  {{ d.label || "本地磁盘" }} ({{ d.root.replace("\\", "") }}) · 剩余 {{ humanBytes(d.free_bytes) }}
                </option>
                <option :value="CUSTOM_DEST">其他文件夹…</option>
              </select>
              <span v-if="destFree !== undefined && destFree < checkedBytes" class="err-text small">剩余空间可能不够</span>
              <div v-if="qDest === CUSTOM_DEST" class="dest-row">
                <input id="qdest-custom" v-model="qCustom" class="input" placeholder="例如 D:\DiskWhy" spellcheck="false" aria-label="隔离区文件夹" />
                <button class="btn icon" type="button" aria-label="浏览…" title="浏览…" @click="browse"><FolderOpen :size="16" /></button>
              </div>
              <span v-if="pickErr" class="err-text small">{{ pickErr }}</span>
              <p class="faint small">预览时会实际试一下能不能在那里建文件夹；不能写的位置会被拦下，不会执行。</p>
            </div>
          </fieldset>
        </aside>

        <!-- 右：条目 -->
        <section class="items card">
          <div class="items-head">
            <label class="check-all">
              <input type="checkbox" :checked="allShownChecked" aria-label="全选" @change="toggleAll" />
              <span>{{ cats.find((c) => c.key === cat)?.label }}</span>
              <span class="faint small">{{ shown.length }} 项</span>
            </label>
            <span class="faint small">取消勾选的项不会进入预览</span>
          </div>
          <TransitionGroup tag="ul" name="item" class="item-list">
            <li v-for="c in shown" :key="c.path" class="item" :class="{ off: unchecked.has(c.path.toLowerCase()) }">
              <input type="checkbox" :checked="!unchecked.has(c.path.toLowerCase())" :aria-label="'勾选 ' + c.name" @change="toggleChecked(c.path)" />
              <span class="it-ico" :class="'ico-' + c.tier"><component :is="c.tier === 'safe' ? CircleCheck : TriangleAlert" :size="16" :stroke-width="1.8" /></span>
              <span class="it-main">
                <span class="it-line1">
                  <span class="it-name">{{ c.name }}</span>
                  <TierChip :tier="c.tier" :label="c.tier === 'safe' ? '低风险' : '有代价'" />
                </span>
                <span class="it-what">{{ c.what }}</span>
                <span v-if="c.tier === 'review'" class="it-lose">删了会失去：{{ c.lose }}</span>
                <span class="it-path mono" :title="c.path">{{ c.path }}</span>
              </span>
              <span class="it-size num">{{ humanBytes(c.allocated) }}</span>
              <button class="btn icon sm ghost" :aria-label="'移出 ' + c.name" :title="'从清单移出 ' + c.path" @click="removeFromCart(c.path)"><X :size="15" /></button>
            </li>
          </TransitionGroup>
        </section>
      </div>

      <!-- 底部操作栏 -->
      <footer class="actionbar">
        <div class="ab-stat">
          <div class="ab-k">已选</div>
          <div class="ab-v num">{{ fmtInt(checkedCart.length) }} <span class="ab-u">/ {{ cart.length }} 项</span></div>
        </div>
        <div class="ab-div"></div>
        <div class="ab-stat">
          <div class="ab-k">预计释放</div>
          <div class="ab-v num">{{ humanBytes(checkedBytes) }}</div>
        </div>
        <div class="ab-div"></div>
        <div class="ab-stat grow">
          <div class="ab-k">删到</div>
          <div class="ab-dest">{{ destLabel }}</div>
        </div>
        <span class="ab-note faint small">按扫描结果估算；预览时按实际文件重新算</span>
        <button class="btn primary lg" :disabled="previewBusy || !canPreview" @click="doPreview(strategy())">
          <ScanEye :size="17" /> {{ previewBusy ? "正在检查…" : "预览要删的内容" }}
        </button>
      </footer>
    </template>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.page-pad {
  flex: 1;
  overflow-y: auto;
  padding: var(--sp-8);
}
.page-pad.center {
  display: flex;
  align-items: center;
  justify-content: center;
}
.work {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 16px;
  padding: 0 var(--sp-8) var(--sp-4);
}
.side {
  width: 320px;
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 14px;
  overflow-y: auto;
  min-height: 0;
}
.cats {
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.cat {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px;
  border: 0;
  border-radius: var(--r-md);
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background var(--dur-1) var(--ease);
}
.cat:hover {
  background: var(--surface-2);
}
.cat.on {
  background: var(--surface-2);
  box-shadow: inset 0 0 0 1px var(--border-2);
}
.cat-ico,
.it-ico {
  width: 34px;
  height: 34px;
  border-radius: 10px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--accent-soft);
  color: var(--accent);
}
.it-ico {
  width: 32px;
  height: 32px;
  border-radius: 9px;
}
.ico-safe {
  background: var(--t-safe-bg);
  color: var(--t-safe-fg);
}
.ico-review {
  background: var(--t-review-bg);
  color: var(--t-review-fg);
}
.cat-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.cat-name {
  font-weight: 600;
  font-size: var(--fs-sm);
}
.cat-sub {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cat-size {
  font-size: var(--fs-sm);
  font-weight: 550;
}
.strategy {
  margin: 0;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}
.strategy legend {
  float: left;
  width: 100%;
  padding: 0;
  margin-bottom: 6px;
}
.opt {
  display: flex;
  gap: 10px;
  align-items: flex-start;
  padding: 11px 12px;
  border-radius: var(--r-md);
  border: 1px solid var(--border-1);
  cursor: pointer;
  transition:
    border-color var(--dur-1) var(--ease),
    background var(--dur-1) var(--ease);
}
.opt:hover {
  border-color: var(--border-3);
}
.opt.on {
  border-color: var(--accent-ring);
  background: var(--accent-softer);
}
.opt input {
  margin-top: 3px;
}
.opt-ico {
  color: var(--text-2);
  margin-top: 1px;
}
.opt.on .opt-ico {
  color: var(--accent);
}
.opt-text {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 8px;
  font-size: var(--fs-sm);
  min-width: 0;
}
.opt-sub {
  flex-basis: 100%;
  font-size: var(--fs-2xs);
  color: var(--text-2);
  line-height: 1.6;
}
.dest {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 4px 2px 0;
}
.dest .input {
  width: 100%;
}
.dest-row {
  display: flex;
  gap: 6px;
}
.items {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.items-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 13px 18px;
  border-bottom: 1px solid var(--border-1);
}
.check-all {
  display: flex;
  align-items: center;
  gap: 10px;
  font-weight: 600;
  font-size: var(--fs-sm);
  cursor: pointer;
}
.item-list {
  list-style: none;
  margin: 0;
  padding: 6px 8px 10px;
  overflow-y: auto;
  flex: 1;
}
.item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 10px;
  border-radius: var(--r-md);
  transition:
    background var(--dur-1) var(--ease),
    opacity var(--dur-2) var(--ease);
}
.item:hover {
  background: var(--surface-2);
}
.item.off {
  opacity: 0.5;
}
.it-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.it-line1 {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.it-name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.it-what {
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.it-lose {
  font-size: var(--fs-xs);
  color: var(--t-review-fg);
}
.it-path {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.it-size {
  font-size: var(--fs-md);
  font-weight: 600;
  white-space: nowrap;
}
.item-enter-active,
.item-leave-active {
  transition:
    opacity var(--dur-2) var(--ease),
    transform var(--dur-2) var(--ease);
}
.item-enter-from,
.item-leave-to {
  opacity: 0;
  transform: translateX(8px);
}
.actionbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 18px;
  margin: 0 var(--sp-8) var(--sp-6);
  padding: 12px 14px 12px 22px;
  border-radius: var(--r-lg);
  background: var(--surface-2);
  border: 1px solid var(--border-2);
  box-shadow: var(--shadow-2);
}
.ab-stat {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.ab-stat.grow {
  flex: 1;
}
.ab-k {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.ab-v {
  font-size: var(--fs-xl);
  font-weight: 600;
  letter-spacing: -0.02em;
  line-height: 1.2;
}
.ab-u {
  font-size: var(--fs-xs);
  color: var(--text-3);
  font-weight: 500;
}
.ab-dest {
  font-size: var(--fs-sm);
  font-weight: 550;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ab-div {
  width: 1px;
  height: 30px;
  background: var(--border-2);
}
.ab-note {
  max-width: 180px;
  line-height: 1.4;
}
@media (max-width: 1280px) {
  .side {
    width: 280px;
  }
  .ab-note {
    display: none;
  }
}
</style>
