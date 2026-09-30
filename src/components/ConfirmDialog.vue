<script setup lang="ts">
// 确认弹层（阶段 F）：一行标题、一个总数 + C 盘剩余、可收起的清单（需你判断的置顶高亮，其余按分类一行）、
// 「跳过 N 项 · 正在使用 ›」、只有清单里有需你判断的项时才出现的勾选、一个主按钮。
// 清单数据全部来自后端预检（按磁盘上的真实内容重新枚举），护栏逻辑和阶段 D 一样：
// 被拦下的项不会删、阻止性警告时按钮不可用、被占用的文件跳过、执行后只重扫受影响的子树。
// 阶段 G：点「清理」后弹层立刻出现（report.pending = true，数字是扫描时的大小），逐个文件的检查在后台进行，
// 「跳过」一行先显示检查进度、主按钮显示“正在检查”且不可用；检查完换成完整报告（同一个 plan_id，勾选状态保留）。
import { computed, ref } from "vue";
import { executePlan, pickFolder, type ExecOutcome, type ExecProgress, type ItemReport, type LoseKind, type PreflightReport } from "../api";
import { gbNum, sizeParts } from "../capacity";
import { checkLine, CLEAN_CAT_NAME, isHardTag, loseLabel, shortPath, type CleanCat } from "../labels";
import { candidates, checkProgress, groupCat, onExecuted, page, report as reportRef, requote, summary } from "../store";

const props = defineProps<{ report: PreflightReport }>();
const emit = defineEmits<{ close: [] }>();

type Phase = "preview" | "running" | "done";
const phase = ref<Phase>("preview");
const progress = ref<ExecProgress | null>(null);
const outcome = ref<ExecOutcome | null>(null);
const error = ref("");
const ack = ref(false);
const listOpen = ref(true);
const skipOpen = ref(false);
const openCat = ref<string | null>(null);
const destOpen = ref(false);

/** 预检项 → 清理页里的那一组（拿名字、分类、后果类型） */
const groupOf = (p: string) => {
  const k = p.toLowerCase();
  return candidates.value.find((g) => g.paths.some((x) => x.path.toLowerCase() === k)) ?? null;
};

const pending = computed(() => props.report.pending === true);
const live = computed(() => props.report.items.filter((i) => !i.blocked && (pending.value ? i.will_delete_bytes > 0 : i.will_delete_count > 0)));
/** 检查进度一行：列文件 → 逐个检查 → 问是哪些程序在占用 */
const check = computed(() => checkLine(checkProgress.value));
const blocked = computed(() => props.report.items.filter((i) => i.blocked));

/** 需你判断的项：按候选组合并（一组可能有好几个位置） */
const reviewRows = computed(() => {
  const m = new Map<string, { key: string; name: string; lose: LoseKind | null; bytes: number }>();
  for (const it of live.value.filter((i) => i.tier === "review")) {
    const g = groupOf(it.path);
    const key = g?.key ?? it.path;
    const cur = m.get(key) ?? { key, name: g?.name ?? it.what, lose: g?.lose_kind ?? null, bytes: 0 };
    cur.bytes += it.will_delete_bytes;
    m.set(key, cur);
  }
  return [...m.values()].sort((a, b) => b.bytes - a.bytes);
});

/** 可放心清理：按分类一行 */
const safeCats = computed(() => {
  const m = new Map<CleanCat, { cat: CleanCat; items: { name: string; bytes: number; key: string }[]; bytes: number }>();
  for (const it of live.value.filter((i) => i.tier === "safe")) {
    const g = groupOf(it.path);
    const cat: CleanCat = g ? groupCat(g) : "sys";
    const cur = m.get(cat) ?? { cat, items: [], bytes: 0 };
    const key = g?.key ?? it.path;
    const ex = cur.items.find((x) => x.key === key);
    if (ex) ex.bytes += it.will_delete_bytes;
    else cur.items.push({ key, name: g?.name ?? it.what, bytes: it.will_delete_bytes });
    cur.bytes += it.will_delete_bytes;
    m.set(cat, cur);
  }
  return [...m.values()].sort((a, b) => b.bytes - a.bytes);
});

const itemCount = computed(() => reviewRows.value.length + safeCats.value.reduce((a, c) => a + c.items.length, 0));
const hasReview = computed(() => reviewRows.value.length > 0);
const blocking = computed(() => props.report.warnings.filter((w) => w.blocking));
/** 其余不阻止执行、又没在“跳过”一行里说到的提示（隔离区和要清理的在同一个盘、有几项合并了等），原文照列 */
const notes = computed(() =>
  props.report.warnings.filter((w) => !w.blocking && !["locked", "hot", "cloud", "recycle_low_space"].includes(w.code)),
);
const lowSpace = computed(() => props.report.warnings.find((w) => w.code === "recycle_low_space") ?? null);
const isRecycle = computed(() => props.report.strategy.kind === "recycle_bin");
const destLabel = computed(() => {
  const s = props.report.strategy;
  return s.kind === "recycle_bin" ? "回收站" : `隔离到 ${s.dest_root}`;
});
const otherDrives = computed(() => props.report.quarantine_choices);

const skipCount = computed(() => props.report.total_skipped + blocked.value.length);
const skipReason = computed(() => {
  const r = props.report;
  if (r.locked_count > 0) return "正在使用";
  if (r.hot_count > 0) return "刚改过";
  if (r.cloud_count > 0) return "云端文件";
  if (blocked.value.length) return "被拦下";
  return "读不到";
});
const REASON: Record<string, string> = {
  hot: "刚改过",
  cloud: "云端文件",
  locked: "正在使用",
  no_permission: "没权限",
  protected: "受保护",
  changed: "被改过",
  missing: "已不在",
  unreadable: "读不到",
};
function skipText(it: ItemReport): string {
  return Object.entries(it.skipped_by_reason)
    .map(([k, n]) => `${REASON[k] ?? k} ${n}`)
    .join(" · ");
}
function nameOf(p: string) {
  return groupOf(p)?.name ?? p.slice(p.lastIndexOf("\\") + 1);
}

const free = computed(() => summary.value?.free_bytes ?? 0);
const canRun = computed(
  () =>
    phase.value === "preview" &&
    !pending.value &&
    props.report.can_execute &&
    !blocking.value.length &&
    live.value.length > 0 &&
    (!hasReview.value || ack.value),
);

async function run() {
  if (!canRun.value) return;
  phase.value = "running";
  error.value = "";
  progress.value = { done: 0, total: props.report.total_entries, bytes_done: 0, current: "" };
  try {
    const o = await executePlan(props.report.plan_id, props.report.locked_count > 0, (p) => (progress.value = p));
    outcome.value = o;
    phase.value = "done";
    await onExecuted(o);
  } catch (e) {
    error.value = String(e);
    phase.value = "preview";
  }
}

async function changeDest(kind: "recycle" | "quarantine", root?: string) {
  destOpen.value = false;
  const s = kind === "recycle" ? ({ kind: "recycle_bin" } as const) : ({ kind: "quarantine", dest_root: root! } as const);
  reportRef.value = null;
  await requote(s);
}

/** 盘的根目录普通用户常常写不进去：让用户挑一个别的盘上的文件夹做隔离区 */
async function pickDest() {
  destOpen.value = false;
  try {
    const p = await pickFolder("选择隔离区放在哪个文件夹");
    if (p) await changeDest("quarantine", p);
  } catch (e) {
    error.value = String(e);
  }
}

function close() {
  if (phase.value === "running") return;
  emit("close");
}
function goQuarantine() {
  emit("close");
  page.value = "quarantine";
}
const pct = computed(() => {
  const p = progress.value;
  return p && p.total ? Math.min(100, (p.done * 100) / p.total) : 0;
});
</script>

<template>
  <div class="scrim" @click.self="close">
    <div class="modal" role="dialog" aria-modal="true" :aria-label="phase === 'done' ? '清理完成' : '确认清理'">
      <!-- ───── 完成 ───── -->
      <template v-if="phase === 'done' && outcome">
        <div class="m-top">
          <h2>清理完成</h2>
          <button class="x" aria-label="关闭" @click="close"><svg class="i" viewBox="0 0 24 24"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg></button>
        </div>
        <div class="m-total">
          <span class="v num">{{ gbNum(outcome.report.ok_bytes) }}<small>GB</small></span>
          <span class="after">{{ isRecycle ? "已进回收站" : "已隔离" }}</span>
        </div>
        <div class="m-list">
          <div class="m-row"><span class="n">成功</span><span class="s num">{{ outcome.report.ok.toLocaleString("zh-CN") }}</span></div>
          <div class="m-row"><span class="n">跳过</span><span class="s num">{{ outcome.report.skipped.toLocaleString("zh-CN") }}</span></div>
          <div v-if="outcome.report.failed" class="m-row"><span class="n">失败</span><span class="s num">{{ outcome.report.failed }}</span></div>
          <div v-if="isRecycle" class="m-skip">清空回收站后才腾出 C 盘空间</div>
          <div v-else class="m-skip"><button class="quiet" @click="goQuarantine">隔离区<svg class="i" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6" /></svg></button></div>
        </div>
        <button class="btn" @click="close">完成</button>
      </template>

      <!-- ───── 执行中 ───── -->
      <template v-else-if="phase === 'running'">
        <div class="m-top"><h2>正在清理</h2></div>
        <div class="m-total">
          <span class="v num">{{ gbNum(progress?.bytes_done ?? 0) }}<small>GB</small></span>
          <span class="after num">{{ (progress?.done ?? 0).toLocaleString("zh-CN") }} / {{ (progress?.total ?? 0).toLocaleString("zh-CN") }}</span>
        </div>
        <div class="prog"><i :style="{ width: pct + '%' }"></i></div>
        <div class="cur mono">{{ progress?.current ? shortPath(progress.current) : "" }}</div>
      </template>

      <!-- ───── 确认 ───── -->
      <template v-else>
        <div class="m-top">
          <h2>清理 <span class="num">{{ itemCount }}</span> 项</h2>
          <button class="x" aria-label="取消" @click="close"><svg class="i" viewBox="0 0 24 24"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg></button>
        </div>
        <div class="m-total">
          <span class="v num" :class="{ est: pending }">{{ gbNum(report.total_bytes) }}<small>GB</small></span>
          <span class="after">
            <em v-if="isRecycle">清空回收站后</em>C 盘剩余 <b class="num">{{ gbNum(free) }} → {{ gbNum(free + report.total_bytes) }} GB</b>
          </span>
        </div>

        <div class="m-list">
          <div class="m-lh">
            <span>清单</span>
            <span class="dest">
              <button class="quiet" :aria-expanded="destOpen" aria-label="删到哪里" @click="destOpen = !destOpen">
                {{ destLabel }}<svg class="i" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6" /></svg>
              </button>
              <span v-if="destOpen" class="menu">
                <button :class="{ on: isRecycle }" @click="changeDest('recycle')">回收站</button>
                <button
                  v-for="d in otherDrives"
                  :key="d.root"
                  :class="{ on: !isRecycle && report.strategy.kind === 'quarantine' && report.strategy.dest_root === d.root }"
                  @click="changeDest('quarantine', d.root)"
                >
                  隔离到 {{ d.root }}<i class="num">{{ gbNum(d.free_bytes) }} GB</i>
                </button>
                <button @click="pickDest">其他文件夹…</button>
              </span>
            </span>
            <button class="quiet" :aria-expanded="listOpen" @click="listOpen = !listOpen">
              {{ listOpen ? "收起" : "展开" }}<svg class="i" viewBox="0 0 24 24"><path :d="listOpen ? 'm18 15-6-6-6 6' : 'm6 9 6 6 6-6'" /></svg>
            </button>
          </div>
          <div v-if="listOpen" class="m-rows">
            <div v-for="r in reviewRows" :key="r.key" class="m-row rv">
              <span class="dot review"></span><span class="n">{{ r.name }}</span>
              <span v-if="loseLabel(r.lose)" class="tag" :class="{ hard: isHardTag(r.lose) }">{{ loseLabel(r.lose) }}</span>
              <span class="s num">{{ sizeParts(r.bytes).join(" ") }}</span>
            </div>
            <template v-for="c in safeCats" :key="c.cat">
              <button class="m-row" :aria-expanded="openCat === c.cat" @click="openCat = openCat === c.cat ? null : c.cat">
                <span class="dot safe"></span><span class="n">{{ CLEAN_CAT_NAME[c.cat] }}</span><span class="c num">{{ c.items.length }} 项</span>
                <span class="s num">{{ sizeParts(c.bytes).join(" ") }}</span>
                <span class="chev"><svg class="i" viewBox="0 0 24 24"><path :d="openCat === c.cat ? 'm6 9 6 6 6-6' : 'm9 18 6-6-6-6'" /></svg></span>
              </button>
              <div v-if="openCat === c.cat" class="m-sub">
                <span v-for="it in c.items" :key="it.key"><i>{{ it.name }}</i><b class="num">{{ sizeParts(it.bytes).join(" ") }}</b></span>
              </div>
            </template>
          </div>
          <div v-if="pending" class="m-skip checking" role="status" aria-live="polite">
            <span class="spin" aria-hidden="true"></span><span class="num">{{ check.text }}</span>
            <span class="cbar"><i :style="{ width: check.pct + '%' }"></i></span>
          </div>
          <button v-else-if="skipCount" class="m-skip" :aria-expanded="skipOpen" @click="skipOpen = !skipOpen">
            <b>跳过 <span class="num">{{ skipCount.toLocaleString("zh-CN") }}</span> 个</b>{{ skipReason }}
            <svg class="i" viewBox="0 0 24 24"><path :d="skipOpen ? 'm6 9 6 6 6-6' : 'm9 18 6-6-6-6'" /></svg>
          </button>
          <div v-if="skipOpen" class="m-sub skips">
            <span v-for="it in blocked" :key="it.path"><i>{{ nameOf(it.path) }}</i><em>{{ it.blocked }}</em></span>
            <span v-for="it in live.filter((x) => x.skipped_count > 0)" :key="'s' + it.path"><i>{{ nameOf(it.path) }}</i><em>{{ skipText(it) }}</em></span>
            <span v-if="report.locked_processes.length" class="procs"><i>占用的程序</i><em>{{ report.locked_processes.map((p) => p.name).join("、") }}</em></span>
          </div>
        </div>

        <div v-if="lowSpace && otherDrives.length" class="low">
          <span>回收站在 C 盘，不腾空间</span>
          <button v-for="d in otherDrives.slice(0, 2)" :key="d.root" class="chip" @click="changeDest('quarantine', d.root)">改放 {{ d.root }}</button>
        </div>
        <div v-for="w in notes" :key="w.code + w.message" class="note">{{ w.message }}</div>
        <div v-for="w in blocking" :key="w.code + w.message" class="warn">{{ w.message }}</div>
        <div v-if="error" class="warn">{{ error }}</div>

        <label v-if="hasReview" class="m-ack">
          <button class="cb" :class="{ on: ack }" role="checkbox" :aria-checked="ack" aria-label="我知道这些删了的后果" @click="ack = !ack">
            <svg v-if="ack" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" /></svg>
          </button>
          <span @click="ack = !ack">我知道这些删了的后果</span>
          <span class="c"><span class="num">{{ reviewRows.length }}</span> 项需你判断</span>
        </label>
        <button class="btn" :disabled="!canRun" :aria-busy="pending" @click="run">
          <template v-if="pending">正在检查…</template>
          <template v-else>清理 <span class="num">{{ gbNum(report.total_bytes) }} GB</span></template>
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: absolute;
  inset: 0;
  background: var(--overlay);
  z-index: 80;
  display: grid;
  place-items: center;
  animation: fade var(--dur-2) var(--ease);
}
[data-theme="A"] .scrim {
  backdrop-filter: blur(2px);
}
@keyframes fade {
  from {
    opacity: 0;
  }
}
.modal {
  width: 540px;
  max-height: calc(100% - 80px);
  overflow-y: auto;
  background: var(--surface);
  border-radius: var(--r-lg);
  padding: 26px 28px 24px;
  box-shadow: var(--shadow-pop);
  animation: rise var(--dur-3) var(--ease);
  scrollbar-width: thin;
}
@keyframes rise {
  from {
    opacity: 0;
    transform: translateY(8px) scale(0.99);
  }
}
[data-theme="B"] .modal {
  background: #1e1a15;
}
[data-theme="C"] .modal {
  border: 2px solid #141412;
  border-radius: 22px;
}
.m-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.m-top h2 {
  font-family: var(--heading);
  font-size: var(--fs-group);
  font-weight: var(--heading-w);
}
[data-theme="C"] .m-top h2 {
  font-size: 22px;
}
.x {
  color: var(--ink3);
  width: 28px;
  height: 28px;
  display: grid;
  place-items: center;
  border-radius: 4px;
}
.x:hover {
  color: var(--ink);
  background: var(--hover);
}
.m-total {
  margin-top: 14px;
  display: flex;
  align-items: baseline;
  gap: 16px;
}
.m-total .v {
  font-family: var(--display);
  font-weight: var(--display-w);
  letter-spacing: var(--display-track);
  font-variation-settings: var(--display-vs);
  font-size: 64px;
  line-height: 0.9;
  color: var(--ink);
}
.m-total .v small {
  font-family: var(--ui);
  font-size: 20px;
  color: var(--ink3);
  letter-spacing: 0;
  margin-left: 6px;
  font-weight: var(--w-med);
  font-variation-settings: normal;
}
[data-theme="B"] .m-total .v {
  font-weight: 300;
  color: var(--accent);
}
.m-total .after {
  font-size: var(--fs-sub);
  color: var(--ink3);
  margin-left: auto;
  text-align: right;
}
.m-total .after em {
  font-style: normal;
  display: block;
  font-size: var(--fs-meta);
}
.m-total .after b {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.m-list {
  margin-top: 20px;
  border-top: 1px solid var(--line);
}
.m-lh {
  height: 38px;
  display: flex;
  align-items: center;
  gap: 14px;
  font-size: var(--fs-meta);
  color: var(--ink3);
  letter-spacing: 0.04em;
}
.m-lh .dest {
  margin-left: auto;
  position: relative;
}
.m-lh .quiet {
  font-size: var(--fs-meta);
}
.m-lh .quiet .i,
.m-skip .i,
.quiet .i {
  width: 14px;
  height: 14px;
}
.menu {
  position: absolute;
  right: 0;
  top: 26px;
  z-index: 5;
  min-width: 180px;
  display: flex;
  flex-direction: column;
  padding: 4px;
  background: var(--surface);
  border-radius: var(--r);
  box-shadow: var(--shadow-pop);
}
[data-theme="B"] .menu {
  background: #26211b;
}
[data-theme="C"] .menu {
  border: 2px solid #141412;
  box-shadow: var(--shadow);
}
.menu button {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 7px 10px;
  border-radius: 4px;
  font-size: var(--fs-sub);
  color: var(--ink2);
}
.menu button i {
  font-style: normal;
  color: var(--ink3);
}
.menu button:hover {
  background: var(--hover);
  color: var(--ink);
}
.menu button.on {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.m-rows {
  max-height: 280px;
  overflow-y: auto;
  overflow-x: hidden;
  margin: 0 -12px;
  padding: 0 12px;
  scrollbar-width: thin;
}
.m-row {
  width: 100%;
  height: 40px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
  color: var(--ink2);
  border-top: 1px solid var(--line);
}
.m-row .n {
  color: var(--ink);
  font-weight: var(--w-med);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.m-row .c {
  color: var(--ink3);
  font-size: var(--fs-sub);
}
.m-row .s {
  margin-left: auto;
  font-weight: var(--w-med);
  color: var(--ink);
  font-size: 15px;
  white-space: nowrap;
}
.m-row .chev {
  color: var(--ink3);
  display: grid;
}
.m-row .chev svg {
  width: 14px;
  height: 14px;
}
.m-row.rv {
  background: var(--review-soft);
  margin: 0 -12px;
  padding: 0 12px;
  width: calc(100% + 24px);
  border-radius: 6px;
  border-top-color: transparent;
}
.m-row.rv + .m-row.rv {
  margin-top: 4px;
}
[data-theme="C"] .m-row.rv {
  border: 2px solid #141412;
  border-radius: 10px;
}
.m-sub {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 2px 0 10px 18px;
  font-size: var(--fs-meta);
  color: var(--ink3);
  user-select: text;
}
.m-sub span {
  display: flex;
  gap: 12px;
  justify-content: space-between;
}
.m-sub i {
  font-style: normal;
  color: var(--ink2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.m-sub b {
  font-weight: var(--w-med);
  color: var(--ink2);
}
.m-sub em {
  font-style: normal;
  text-align: right;
  max-width: 70%;
}
.m-sub.skips {
  max-height: 140px;
  overflow-y: auto;
}
.m-skip {
  width: 100%;
  height: 40px;
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-sub);
  color: var(--ink3);
  border-top: 1px solid var(--line);
}
.m-skip b {
  color: var(--ink2);
  font-weight: var(--w-med);
}
.m-skip.checking {
  cursor: default;
}
.m-skip .cbar {
  margin-left: auto;
  width: 120px;
  height: 3px;
  border-radius: 2px;
  background: var(--sunk);
  overflow: hidden;
}
.m-skip .cbar i {
  display: block;
  height: 100%;
  background: var(--ink3);
  transition: width var(--dur-2) linear;
}
.spin {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  border: 2px solid var(--line2);
  border-top-color: var(--ink2);
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.m-total .v.est {
  opacity: 0.55;
  transition: opacity var(--dur-2) var(--ease);
}
.low {
  margin-top: 12px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: var(--fs-sub);
  color: var(--review-label);
}
.low span {
  margin-right: auto;
}
.chip {
  height: 28px;
  padding: 0 12px;
  border-radius: var(--r-pill);
  border: var(--bw) solid var(--line2);
  font-size: var(--fs-sub);
  font-weight: var(--w-med);
  color: var(--ink);
}
.chip:hover {
  background: var(--hover);
}
.note {
  margin-top: 12px;
  font-size: var(--fs-meta);
  color: var(--review-label);
  line-height: 1.5;
  user-select: text;
}
.warn {
  margin-top: 12px;
  font-size: var(--fs-sub);
  color: var(--danger);
  line-height: 1.5;
  user-select: text;
}
.m-ack {
  margin-top: 14px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
  color: var(--ink);
  font-weight: var(--w-med);
  cursor: pointer;
}
.m-ack .c {
  margin-left: auto;
  font-size: var(--fs-meta);
  color: var(--ink3);
  font-weight: var(--w-reg);
}
.modal .btn {
  margin-top: 18px;
  width: 100%;
  height: 50px;
}
.prog {
  margin-top: 20px;
  height: 4px;
  border-radius: 2px;
  background: var(--sunk);
  overflow: hidden;
}
.prog i {
  display: block;
  height: 100%;
  background: var(--safe);
  transition: width var(--dur-2) linear;
}
[data-theme="C"] .prog {
  height: 10px;
  border: 2px solid #141412;
  border-radius: 6px;
  background: #fff;
}
.cur {
  margin-top: 10px;
  height: 18px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
