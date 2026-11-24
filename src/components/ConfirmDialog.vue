<script setup lang="ts">
// 确认弹层（阶段 F → J）：确认 → 执行中 → 完成 / 已停止，三个状态都在这一个弹层里。
// 确认：一行标题、真正要处理的总数 + 所扫盘剩余、去向一行（可放心清理 直接删 · 需你判断 暂存到 D 盘 ▾）、
//   可收起的清单（需你判断的置顶高亮，其余按分类一行）、「跳过 N 个 · X GB · 主要原因 ›」（展开按原因带字节，
//   解释数字为什么比首页小）、只有清单里有需你判断的项时才出现的勾选、主按钮 + 预计用时。
// 执行中：大数字 = 按估计耗时加权的百分比（和进度条同一个值，单调；大文件先做完，按 GB 算会和进度条对不上），
//   条下一行小字「已处理 12.3 / 29.3 GB · 4.1 万 / 39.6 万个」，再下面「剩约 2 分 · 停止」；执行中关窗被拦下时「停止」闪一下。
// 完成：大数字 = 所扫盘实测多出来的空间，下面只列非零的去向 / 跳过 / 没处理。
// 护栏逻辑和阶段 D 一样，全部在后端：被拦下的项不会删、阻止性警告时按钮不可用、被占用的文件跳过、执行后只重扫受影响的子树。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { pickFolder, type ItemReport, type LoseKind, type PreflightReport } from "../api";
import { gbNum, sizeParts } from "../capacity";
import {
  checkLine,
  CLEAN_CAT_NAME,
  destLine,
  doneRows,
  driveName,
  driveOf,
  estText,
  isDriveRoot,
  isHardTag,
  loseTag,
  pctText,
  processedLine,
  pushSample,
  remainSecs,
  remainText,
  shortPath,
  skipRows,
  skipSummary,
  stashFolder,
  type CleanCat,
  type RateSample,
} from "../labels";
import {
  authorizeStash,
  candidates,
  checkedBytes,
  checkProgress,
  chooseDest,
  closeBlockedAt,
  closeReport,
  dest,
  elevating,
  execError,
  execEstSecs,
  execPhase,
  execProg,
  execResult,
  execStopping,
  finishExec,
  groupCat,
  page,
  rechecking,
  runExec,
  scanRoot,
  okStashDrives,
  stashDrives,
  stopExec,
  summary,
} from "../store";

const props = defineProps<{ report: PreflightReport | null }>();

const ack = ref(false);
const listOpen = ref(true);
const skipOpen = ref(false);
const doneSkipOpen = ref(false);
const openCat = ref<string | null>(null);
const destOpen = ref(false);

const size = (b: number) => sizeParts(b).join(" ");
const n = (x: number) => x.toLocaleString("zh-CN");
/** 所扫的盘：「C 盘」 */
const dname = computed(() => driveName(summary.value?.root ?? scanRoot.value));

// ───────────────────────── 确认 ─────────────────────────
const rep = computed(() => props.report);
/** 预览数据还没回来（换了去向）时给模板一个空壳，避免到处判空 */
const R = computed<PreflightReport>(
  () =>
    rep.value ?? {
      plan_id: "",
      strategy: { kind: "recycle_bin" },
      items: [],
      total_entries: 0,
      total_bytes: 0,
      total_skipped: 0,
      warnings: [],
      locked_processes: [],
      locked_count: 0,
      hot_count: 0,
      cloud_count: 0,
      link_count: 0,
      review_items: [],
      can_execute: false,
      quarantine_choices: [],
      pending: true,
      timing_ms: {},
    },
);

/** 预检项 → 清理页里的那一组（拿名字、分类、后果类型） */
const groupOf = (p: string) => {
  const k = p.toLowerCase();
  return candidates.value.find((g) => g.paths.some((x) => x.path.toLowerCase() === k)) ?? null;
};

const pending = computed(() => R.value.pending === true);
const live = computed(() => R.value.items.filter((i) => !i.blocked && (pending.value ? i.will_delete_bytes > 0 : i.will_delete_count > 0)));
/** 检查进度一行：列文件 → 逐个检查 → 问是哪些程序在占用 */
const check = computed(() => checkLine(checkProgress.value));
const blocked = computed(() => R.value.items.filter((i) => i.blocked));

/** 走「需你判断」去向的：需你判断的项 + 强制加入的（keep / 认不出来 / 不建议直接删的，只进回收站 / 暂存区） */
const isReviewLike = (i: ItemReport) => i.tier !== "safe" || !!i.forced;
/** 需你判断的项：按候选组合并（一组可能有好几个位置） */
const reviewRows = computed(() => {
  const m = new Map<string, { key: string; name: string; lose: LoseKind | null; short: string | null; bytes: number }>();
  for (const it of live.value.filter(isReviewLike)) {
    const g = groupOf(it.path);
    const key = g?.key ?? it.path;
    const cur = m.get(key) ?? { key, name: g?.name ?? it.what, lose: g?.lose_kind ?? null, short: g ? loseTag(g) : null, bytes: 0 };
    cur.bytes += it.will_delete_bytes;
    m.set(key, cur);
  }
  return [...m.values()].sort((a, b) => b.bytes - a.bytes);
});

/** 可放心清理：按分类一行 */
const safeCats = computed(() => {
  const m = new Map<CleanCat, { cat: CleanCat; items: { name: string; bytes: number; key: string }[]; bytes: number }>();
  for (const it of live.value.filter((i) => !isReviewLike(i))) {
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

const hasReview = computed(() => reviewRows.value.length > 0);

// ── 去向（§3.1）──
const isStash = computed(() => dest.value.mode === "stash");
/** 暂存区还不能用（要授权 / 建不了 / 正在准备），而且确实有东西要去暂存区 */
const stashBlocked = computed(() => isStash.value && dest.value.status !== "ok" && dest.value.status !== "" && hasReview.value);
const stashDriveName = computed(() => {
  const d = dest.value;
  if (d.mode === "stash") return driveName(d.drive || d.dest);
  if (d.mode === "folder") return driveName(d.dest);
  return undefined;
});
const destParts = computed(() => {
  const d = dest.value;
  const reviewDest = d.mode === "permanent" ? "permanent" : d.mode === "recycle" ? "recycle" : "quarantine";
  // 可放心清理一律直接删；只有明确选了「回收站」时也进回收站（和 currentPlanArgs 的 safeDirect 一致）
  const safeDirect = !(d.mode === "recycle" && d.explicit);
  return destLine({
    safe: safeCats.value.length > 0,
    review: hasReview.value,
    safeDest: safeDirect ? "permanent" : "recycle",
    reviewDest,
    drive: stashDriveName.value,
  });
});
/** 可放心清理这次进回收站（只有明确选了「回收站」时），否则直接删（和 currentPlanArgs 一致） */
const safeToBin = computed(() => dest.value.mode === "recycle" && dest.value.explicit);
/**
 * 去向一行悬停：暂存区不在默认的「X:\DiskWhy暂存区」（盘根建不了、用的是以前选过的文件夹），
 * 或选了「其他文件夹…」时，给出真正放到哪里
 */
const destTip = computed(() => {
  const d = dest.value;
  if (!hasReview.value || !d.dest) return undefined;
  if (d.mode === "folder" || (d.mode === "stash" && d.status === "ok" && !isDriveRoot(d.dest))) return stashFolder(d.dest);
  return undefined;
});
/** 下拉里「其他文件夹…」选中的文件夹（显示在那一行） */
const folderShort = computed(() => (dest.value.mode === "folder" ? shortPath(dest.value.dest) : ""));

// 换了去向就要重新勾“我知道后果”（直接删除和暂存的后果不一样）
watch(
  () => dest.value.mode + dest.value.drive + dest.value.dest,
  () => (ack.value = false),
);

async function pick(mode: "stash" | "recycle" | "permanent", root?: string) {
  destOpen.value = false;
  await chooseDest(mode, root);
}
/** 盘根常常写不进去：让用户挑一个别的盘上的文件夹 */
async function pickDest() {
  destOpen.value = false;
  try {
    const p = await pickFolder("暂存到哪个文件夹");
    if (p) await chooseDest("folder", p);
  } catch (e) {
    execError.value = String(e);
  }
}
function onDoc(e: Event) {
  if (e instanceof KeyboardEvent && e.key === "Escape" && !destOpen.value) {
    close(); // Esc 关弹层（清理中 close 自己会忽略）
    return;
  }
  if (!destOpen.value) return;
  if (e instanceof KeyboardEvent && e.key !== "Escape") return;
  if (e instanceof MouseEvent && (e.target as Element | null)?.closest?.(".dest")) return;
  destOpen.value = false;
}
// 打开时把键盘焦点放进弹层（页面和标题栏左边在弹层开着时是 inert 的，Tab 不会跑到后面去）
const modalEl = ref<HTMLElement | null>(null);
onMounted(() => void nextTick(() => modalEl.value?.focus({ preventScroll: true })));
document.addEventListener("mousedown", onDoc);
document.addEventListener("keydown", onDoc);
onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onDoc);
  document.removeEventListener("keydown", onDoc);
});

// ── 提示 / 警告 ──
/** 暂存区要授权 / 建不了时，后端那条“建不了暂存区文件夹”的长警告由「授权」/「选文件夹…」一行代替 */
const blocking = computed(() =>
  R.value.warnings
    .filter((w) => w.blocking && !(stashBlocked.value && w.code === "quarantine_bad_dest"))
    .map((w) => ({ key: w.code + w.message, code: w.code, text: BLOCK_SHORT[w.code] ?? w.message, tip: w.message })),
);
/** 阻止执行的提示：短字 + 「换个文件夹」，原文悬停看 */
const BLOCK_SHORT: Record<string, string> = {
  quarantine_no_space: "暂存区放不下",
  quarantine_bad_dest: "暂存区用不了",
};
const NOTE_SHORT: Record<string, string> = {
  recycle_over_capacity: "回收站会超上限",
  quarantine_same_volume: "暂存区在同一个盘，不腾空间",
};
/** 其余不阻止执行、又没在“跳过”一行里说到的提示：已知的给短字（悬停看原文），其余原文 */
const notes = computed(() =>
  R.value.warnings
    .filter((w) => !w.blocking && !["locked", "hot", "cloud", "recycle_low_space", "merged"].includes(w.code))
    .map((w) => ({ key: w.code + w.message, text: NOTE_SHORT[w.code] ?? w.message, tip: w.message })),
);
/** 有东西进回收站、所扫盘又快满了：回收站不腾空间 → 一键改放别的盘（走暂存区回退链，不会是死路） */
const lowSpace = computed(() => R.value.warnings.find((w) => w.code === "recycle_low_space") ?? null);

// ── 跳过（解释 30.1 → 29.3）──
const skip = computed(() => skipSummary(R.value));
/** 展开后按项列（同名的组合并，跳过字节多的在前，最多 12 行）：「Chrome 缓存  正在使用 4 · 刚改过 2」 */
const skipItems = computed(() => {
  const m = new Map<string, { name: string; bytes: number; reasons: Record<string, number> }>();
  for (const it of live.value.filter((x: ItemReport) => x.skipped_count > 0)) {
    const name = nameOf(it.path);
    const cur = m.get(name) ?? { name, bytes: 0, reasons: {} };
    cur.bytes += it.skipped_bytes;
    for (const [k, v] of Object.entries(it.skipped_by_reason)) cur.reasons[k] = (cur.reasons[k] ?? 0) + v;
    m.set(name, cur);
  }
  return [...m.values()]
    .sort((a, b) => b.bytes - a.bytes)
    .slice(0, 12)
    .map((x) => ({
      name: x.name,
      text: skipRows(x.reasons)
        .map((r) => `${r.label} ${n(r.count)}`)
        .join(" · "),
    }));
});
function nameOf(p: string) {
  return groupOf(p)?.name ?? p.slice(p.lastIndexOf("\\") + 1);
}

// ── 总数 / 剩余 / 预计时间 ──
const free = computed(() => summary.value?.free_bytes ?? 0);
/** 真能腾出所扫盘空间的部分：直接删 + 暂存到别的盘（回收站、同盘暂存都不腾） */
const freesBytes = computed(() => {
  const bd = R.value.by_dest;
  if (!bd) return R.value.strategy.kind === "recycle_bin" ? 0 : R.value.total_bytes;
  const src = driveOf(summary.value?.root ?? scanRoot.value);
  const q = R.value.strategy.kind === "quarantine" && driveOf(R.value.strategy.dest_root) !== src ? bd.quarantine.bytes : 0;
  return bd.permanent.bytes + q;
});
const est = computed(() => (pending.value || !rep.value ? "" : estText(R.value.est_secs ?? 0)));
/** 大数字 = 清理页的「已选」（同一个数）；界面重载后重新附着、没有勾选时退回预检的总数 */
const selTotal = computed(() => checkedBytes.value || R.value.total_bytes);
/** 检查完真能处理的比已选少（有在用 / 刚改过的会跳过）：一行小字「实际 28.1 GB · 正在使用」，不再出第二个大数字 */
const actualText = computed(() => {
  if (pending.value || !rep.value) return "";
  const a = gbNum(R.value.total_bytes);
  if (a === gbNum(selTotal.value) && !skip.value.bytes) return "";
  return [`实际 ${a} GB`, skip.value.main].filter(Boolean).join(" · ");
});

const canRun = computed(
  () =>
    execPhase.value === "idle" &&
    !!rep.value &&
    !pending.value &&
    R.value.can_execute &&
    !rechecking.value &&
    !blocking.value.length &&
    !stashBlocked.value &&
    live.value.length > 0 &&
    (!hasReview.value || ack.value),
);

async function run() {
  if (!canRun.value || !rep.value) return;
  await runExec(rep.value);
}

// ───────────────────────── 执行中 ─────────────────────────
const samples = ref<RateSample[]>([]);
const pct = ref(0);
watch(
  execProg,
  (p) => {
    if (!p) return;
    samples.value = pushSample(samples.value, { t: p.elapsed_ms, c: p.cost_done });
    const v = p.cost_total > 0 ? Math.min(100, (p.cost_done * 100) / p.cost_total) : 0;
    // 单调：重新附着 / 收尾时偶尔回报小一点的值也不往回走
    pct.value = Math.max(pct.value, v);
  },
  { immediate: true },
);
/** 剩余时间：按滑窗速率算；刚开始（前 2 秒、还算不出速率）先用预检估的用时减去已经过去的 */
const remain = computed(() => {
  const p = execProg.value;
  if (!p) return "";
  const r = remainSecs(samples.value, p.cost_total);
  if (r !== null) return remainText(r, pct.value);
  if (execEstSecs.value > 0) return remainText(Math.max(5, execEstSecs.value - p.elapsed_ms / 1000), pct.value);
  return "";
});
/** 条下一行：已处理 12.3 / 29.3 GB · 4.1 万 / 39.6 万个 */
const doneLine = computed(() => {
  const p = execProg.value;
  return p && (p.total_bytes > 0 || p.total > 0) ? processedLine(p) : "";
});

// 执行中关窗被拦下：「停止」闪一下 + 一行「先停止再关」，几秒后收起
const nudge = ref(false);
let nudgeT: ReturnType<typeof setTimeout> | undefined;
watch(closeBlockedAt, (t) => {
  if (!t || execPhase.value !== "running") return;
  nudge.value = false;
  requestAnimationFrame(() => (nudge.value = true));
  clearTimeout(nudgeT);
  nudgeT = setTimeout(() => (nudge.value = false), 3600);
});
onBeforeUnmount(() => clearTimeout(nudgeT));

// ───────────────────────── 完成 / 已停止 ─────────────────────────
const res = computed(() => execResult.value);
const doneStash = computed(() => driveOf(res.value?.strategy_label) ?? stashDriveName.value);
const rows = computed(() => (res.value ? doneRows(res.value, doneStash.value) : []));
const doneSkips = computed(() => skipRows(res.value?.skipped_by_reason));
/** 「没成功 N 个」悬停：失败原因（去重，最多 6 条） */
const failTip = computed(() => {
  const why = new Set<string>();
  for (const it of res.value?.items ?? []) for (const d of it.details) if (d.result === "failed" && d.reason) why.add(d.reason);
  return [...why].slice(0, 6).join("\n");
});
/** 后端的补充说明（例如 Windows 直接删了某个本该进回收站的文件）：一个小标记，悬停看原文 */
const doneNotes = computed(() => (res.value?.notes ?? []).join("\n"));

function goQuarantine() {
  finishExec();
  page.value = "quarantine";
}

function close() {
  if (execPhase.value === "running") return;
  if (execPhase.value === "done") finishExec();
  else closeReport();
}
</script>

<template>
  <div class="scrim" @click.self="close">
    <div
      ref="modalEl"
      class="modal"
      tabindex="-1"
      role="dialog"
      aria-modal="true"
      :aria-label="execPhase === 'done' ? (res?.cancelled ? '已停止' : '清理完成') : execPhase === 'running' ? '正在清理' : '确认清理'"
    >
      <!-- ───── 完成 / 已停止 ───── -->
      <template v-if="execPhase === 'done' && res">
        <div class="m-top">
          <h2>{{ res.cancelled ? "已停止" : "清理完成" }}</h2>
          <button class="x" aria-label="关闭" @click="close"><svg class="i" viewBox="0 0 24 24"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg></button>
        </div>
        <div class="m-total">
          <span class="v num" title="按清理前后剩余空间实测">{{ res.freed_bytes ? gbNum(res.freed_bytes) : "0" }}<small>GB</small></span>
          <span class="after"><b>{{ dname }} 多出</b></span>
        </div>
        <div class="m-list done">
          <template v-for="r in rows" :key="r.key">
            <button v-if="r.key === 'skipped' && doneSkips.length" class="m-row" :aria-expanded="doneSkipOpen" @click="doneSkipOpen = !doneSkipOpen">
              <span class="n">{{ r.label }}</span><span v-if="r.note" class="c">{{ r.note }}</span><span class="s num">{{ r.value }}</span>
              <span class="chev"><svg class="i" viewBox="0 0 24 24"><path :d="doneSkipOpen ? 'm6 9 6 6 6-6' : 'm9 18 6-6-6-6'" /></svg></span>
            </button>
            <button v-else-if="r.key === 'quarantine'" class="m-row" title="去暂存区" @click="goQuarantine">
              <span class="dot q"></span><span class="n">{{ r.label }}</span><span class="s num">{{ r.value }}</span>
              <span class="chev"><svg class="i" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6" /></svg></span>
            </button>
            <div v-else class="m-row" :class="r.key" :title="r.key === 'failed' && failTip ? failTip : undefined">
              <span v-if="r.key === 'permanent'" class="dot safe"></span>
              <span v-else-if="r.key === 'recycle' || r.key === 'nuked'" class="dot bin"></span>
              <span class="n">{{ r.label }}</span>
              <span v-if="r.note" class="c">{{ r.note }}</span>
              <span class="s num">{{ r.value }}</span>
            </div>
            <div v-if="r.key === 'skipped' && doneSkipOpen" class="m-sub reasons">
              <span v-for="s in doneSkips" :key="s.code" :title="s.tip">
                <i>{{ s.label }}</i><em class="num">{{ n(s.count) }} 个</em><b class="num">{{ s.bytes ? size(s.bytes) : "" }}</b>
              </span>
            </div>
          </template>
          <div v-if="!rows.length" class="m-row"><span class="n">没有动任何文件</span></div>
        </div>
        <div v-if="doneNotes" class="note" :title="doneNotes">注意 {{ res.notes.length }} 条</div>
        <button class="btn" @click="close">完成</button>
      </template>

      <!-- ───── 执行中 ───── -->
      <template v-else-if="execPhase === 'running'">
        <div class="m-top"><h2>正在清理</h2></div>
        <div class="m-total">
          <span class="v num pct">{{ pctText(pct) }}<small>%</small></span>
          <span v-if="remain && !execStopping" class="after"><b class="remain-big num">{{ remain }}</b></span>
        </div>
        <div class="prog" role="progressbar" :aria-valuenow="Math.floor(pct)" aria-valuemin="0" aria-valuemax="100" :aria-valuetext="pctText(pct) + '%'">
          <i :style="{ width: pct + '%' }"></i>
        </div>
        <div class="done-line num">{{ doneLine }}</div>
        <div class="run-line">
          <template v-if="execStopping">
            <span class="spin" aria-hidden="true"></span><span class="stopping">正在停止</span>
            <span v-if="nudge" class="hint" role="status">稍等再关</span>
          </template>
          <template v-else>
            <button class="stop" :class="{ nudge }" @click="stopExec">停止</button>
            <span v-if="nudge" class="hint" role="status">先停止再关</span>
          </template>
        </div>
        <div class="cur mono">{{ execProg?.current ? shortPath(execProg.current) : "" }}</div>
      </template>

      <!-- ───── 确认 ───── -->
      <template v-else>
        <div class="m-top">
          <h2>确认清理</h2>
          <button class="x" aria-label="取消" @click="close"><svg class="i" viewBox="0 0 24 24"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg></button>
        </div>
        <div class="m-total">
          <span class="v num">{{ gbNum(selTotal) }}<small>GB</small></span>
          <span class="after">
            {{ dname }}剩余
            <b class="num">{{ gbNum(free) }}<template v-if="freesBytes > 0"> → {{ gbNum(free + freesBytes) }}</template> GB</b>
          </span>
        </div>

        <div class="m-list">
          <div class="m-lh">
            <span>清单</span>
            <span class="dest">
              <button
                class="dline"
                :class="{ tipped: destTip }"
                :aria-expanded="destOpen"
                aria-haspopup="menu"
                aria-label="删到哪里"
                :title="destOpen ? undefined : destTip"
                @click="destOpen = !destOpen"
              >
                <template v-for="(p, i) in destParts" :key="p.tier">
                  <i v-if="i" class="sep">·</i>
                  <span class="dp"><span class="dot" :class="p.tier"></span>{{ p.label }} <b>{{ p.dest }}</b></span>
                </template>
                <svg class="i" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6" /></svg>
              </button>
              <!-- 只有可放心清理的：去向只有「直接删」和「回收站」两种，暂存区对它们不起作用 -->
              <span v-if="destOpen && !hasReview" class="menu" role="menu">
                <button role="menuitemradio" :aria-checked="!safeToBin" :class="{ on: !safeToBin }" @click="pick('permanent')">
                  <span>直接删除</span><i>真腾空间</i>
                </button>
                <button role="menuitemradio" :aria-checked="safeToBin" :class="{ on: safeToBin }" title="回收站满了，Windows 会清掉最早的" @click="pick('recycle')">
                  <span>回收站</span><i>不腾空间</i>
                </button>
              </span>
              <span v-else-if="destOpen" class="menu" role="menu">
                <button
                  v-for="d in stashDrives"
                  :key="d.root"
                  role="menuitemradio"
                  :aria-checked="isStash && dest.drive === d.root"
                  :class="{ on: isStash && dest.drive === d.root }"
                  @click="pick('stash', d.root)"
                >
                  <span>{{ driveName(d.root) }}暂存区</span><i class="num">剩 {{ gbNum(d.free_bytes) }} GB</i>
                </button>
                <button
                  role="menuitemradio"
                  :aria-checked="dest.mode === 'recycle'"
                  :class="{ on: dest.mode === 'recycle' }"
                  title="回收站满了，Windows 会清掉最早的"
                  @click="pick('recycle')"
                >
                  <span>回收站</span><i>不腾空间</i>
                </button>
                <button role="menuitemradio" :aria-checked="dest.mode === 'permanent'" :class="{ on: dest.mode === 'permanent' }" @click="pick('permanent')">
                  <span>直接删除</span><i>找不回</i>
                </button>
                <button role="menuitemradio" :aria-checked="dest.mode === 'folder'" :class="{ on: dest.mode === 'folder' }" @click="pickDest">
                  <span>其他文件夹…</span><i v-if="folderShort" class="mono" :title="dest.dest">{{ folderShort }}</i>
                </button>
              </span>
            </span>
            <button class="quiet" :aria-expanded="listOpen" @click="listOpen = !listOpen">
              {{ listOpen ? "收起" : "展开" }}<svg class="i" viewBox="0 0 24 24"><path :d="listOpen ? 'm18 15-6-6-6 6' : 'm6 9 6 6 6-6'" /></svg>
            </button>
          </div>
          <div v-if="listOpen" class="m-rows">
            <div v-for="r in reviewRows" :key="r.key" class="m-row rv">
              <span class="dot review"></span><span class="n">{{ r.name }}</span>
              <span v-if="r.short" class="tag" :class="{ hard: isHardTag(r.lose) }">{{ r.short }}</span>
              <span class="s num">{{ size(r.bytes) }}</span>
            </div>
            <template v-for="c in safeCats" :key="c.cat">
              <button class="m-row" :aria-expanded="openCat === c.cat" @click="openCat = openCat === c.cat ? null : c.cat">
                <span class="dot safe"></span><span class="n">{{ CLEAN_CAT_NAME[c.cat] }}</span>
                <span class="s num">{{ size(c.bytes) }}</span>
                <span class="chev"><svg class="i" viewBox="0 0 24 24"><path :d="openCat === c.cat ? 'm6 9 6 6 6-6' : 'm9 18 6-6-6-6'" /></svg></span>
              </button>
              <div v-if="openCat === c.cat" class="m-sub">
                <span v-for="it in c.items" :key="it.key"><i>{{ it.name }}</i><b class="num">{{ size(it.bytes) }}</b></span>
              </div>
            </template>
          </div>
          <div v-if="pending" class="m-skip checking" role="status" aria-live="polite">
            <span class="spin" aria-hidden="true"></span><span class="num">{{ check.text }}</span>
            <span class="cbar"><i :style="{ width: check.pct + '%' }"></i></span>
          </div>
          <button v-else-if="actualText" class="m-skip" :aria-expanded="skipOpen" @click="skipOpen = !skipOpen">
            <b class="num">{{ actualText }}</b>
            <svg class="i" viewBox="0 0 24 24"><path :d="skipOpen ? 'm6 9 6 6 6-6' : 'm9 18 6-6-6-6'" /></svg>
          </button>
          <div v-if="skipOpen && !pending" class="m-sub reasons">
            <span v-for="s in skip.rows" :key="s.code" :title="s.tip">
              <i>{{ s.label }}</i><em></em><b class="num">{{ s.bytes ? size(s.bytes) : "" }}</b>
            </span>
            <div v-if="R.locked_processes.length || blocked.length || skipItems.length" class="details">
              <span v-if="R.locked_processes.length" class="procs"><i>占用的程序</i><em>{{ [...new Set(R.locked_processes.map((p) => p.name))].join("、") }}</em></span>
              <span v-for="it in blocked" :key="it.path" class="item"><i>{{ nameOf(it.path) }}</i><em :title="it.blocked ?? ''">被拦下</em></span>
              <span v-for="it in skipItems" :key="'s' + it.name" class="item"><i>{{ it.name }}</i><em>{{ it.text }}</em></span>
            </div>
          </div>
        </div>

        <!-- 暂存区要授权 / 建不了 / 正在准备 -->
        <div v-if="isStash && hasReview && dest.status === 'needs_elevation'" class="act-row">
          <span>{{ stashDriveName }}暂存区要授权</span>
          <button class="chip" :disabled="elevating" @click="authorizeStash">{{ elevating ? "等你确认" : "授权" }}</button>
        </div>
        <div v-else-if="isStash && hasReview && dest.status === 'failed'" class="act-row" :title="dest.failMsg">
          <span>{{ stashDriveName }}建不了暂存区</span>
          <button class="chip" @click="pickDest">选文件夹…</button>
        </div>
        <div v-else-if="isStash && dest.status === 'pending'" class="act-row quiet-row">
          <span class="spin" aria-hidden="true"></span><span>正在准备 {{ stashDriveName }}暂存区</span>
        </div>
        <!-- 回收站在所扫的盘上、这个盘又快满了：「改放 D 盘」只对需你判断的有意义（可放心清理不进暂存区） -->
        <div v-if="lowSpace && !isStash && (hasReview || safeToBin)" class="act-row" :title="lowSpace.message">
          <span>回收站不腾空间</span>
          <button v-if="hasReview && okStashDrives.length" class="chip" @click="pick('stash', okStashDrives[0].root)">改放 {{ driveName(okStashDrives[0].root) }}</button>
        </div>
        <div v-for="w in notes" :key="w.key" class="note" :title="w.tip">{{ w.text }}</div>
        <div v-for="w in blocking" :key="w.key" class="act-row warn-row" :title="w.tip">
          <span>{{ w.text }}</span>
          <button v-if="w.code.startsWith('quarantine_')" class="chip" @click="pickDest">换个文件夹</button>
        </div>
        <div v-if="execError" class="warn">{{ execError }}</div>

        <!-- 整行可点，只在这一层切换一次（原来是 <label>：点文字时 label 又替里面的按钮点一次，等于没点） -->
        <div v-if="hasReview" class="m-ack" @click="ack = !ack">
          <button class="cb" :class="{ on: ack }" role="checkbox" :aria-checked="ack" aria-label="我知道这些删了的后果">
            <svg v-if="ack" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" /></svg>
          </button>
          <span>我知道这些删了的后果</span>
        </div>
        <button class="btn go" :disabled="!canRun" :aria-busy="pending || rechecking" @click="run">
          <template v-if="pending">正在检查…</template>
          <template v-else>
            开始清理
            <span v-if="rechecking" class="est-t"><span class="spin" aria-hidden="true"></span></span>
            <span v-else-if="est" class="est-t" :title="'预计用时' + est"
              ><svg class="clk" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></svg>预计{{ est }}</span
            >
          </template>
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
/* 从标题栏下面开始：清理很久时还能最小化 / 拖动窗口（标题栏左边在弹层开着时是 inert 的） */
.scrim {
  position: absolute;
  inset: var(--titlebar) 0 0 0;
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
  outline: none;
  width: min(540px, calc(100vw - 32px));
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
  min-height: 28px;
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
  white-space: nowrap;
}
.m-total .after b {
  color: var(--ink);
  font-weight: var(--w-semi);
  margin-left: 4px;
}
.m-total .v[title] {
  cursor: help;
}
.m-list {
  margin-top: 20px;
  border-top: 1px solid var(--line);
}
.m-lh {
  height: 40px;
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
  min-width: 0;
}
.m-lh .quiet {
  font-size: var(--fs-meta);
  flex: none;
}
.m-lh .quiet .i,
.m-skip .i,
.quiet .i {
  width: 14px;
  height: 14px;
}
/* 去向一行：两段短字，整行是下拉 */
.dline {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  max-width: 100%;
  height: 28px;
  padding: 0 6px 0 10px;
  border-radius: var(--r-pill);
  border: 1px solid var(--line);
  font-size: var(--fs-meta);
  letter-spacing: 0;
  color: var(--ink3);
  white-space: nowrap;
  transition:
    border-color var(--dur-1) var(--ease),
    background-color var(--dur-1) var(--ease);
}
.dline:hover,
.dline[aria-expanded="true"] {
  border-color: var(--line2);
  background: var(--hover);
}
.dline b {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.dline .dp {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}
.dline .dp .dot {
  width: 7px;
  height: 7px;
}
.dline .sep,
.run-line .sep {
  font-style: normal;
  color: var(--line2);
}
.dline .i {
  width: 14px;
  height: 14px;
  color: var(--ink3);
}
[data-theme="C"] .dline {
  border: 2px solid #141412;
  background: #fff;
  box-shadow: 2px 2px 0 #141412;
}
[data-theme="C"] .dline:hover,
[data-theme="C"] .dline[aria-expanded="true"] {
  background: var(--accent);
}
[data-theme="C"] .dline .i {
  color: #141412;
}
/* C 主题：下拉展开 / 悬停时底色是亮黄绿，和“可放心清理”的点同色——给点描一圈黑边 */
[data-theme="C"] .dline .dp .dot {
  box-shadow: 0 0 0 1.5px #141412;
}
[data-theme="C"] .dline .sep {
  color: rgba(20, 20, 18, 0.35);
}
.menu {
  position: absolute;
  right: 0;
  top: 34px;
  z-index: 5;
  min-width: 220px;
  display: flex;
  flex-direction: column;
  padding: 4px;
  background: var(--surface);
  border-radius: var(--r);
  box-shadow: var(--shadow-pop);
  animation: menu-in var(--dur-2) var(--ease);
}
@keyframes menu-in {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}
[data-theme="B"] .menu {
  background: #26211b;
}
[data-theme="C"] .menu {
  border: 2px solid #141412;
  box-shadow: 4px 4px 0 #141412;
}
.menu button {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  height: 36px;
  padding: 0 10px;
  border-radius: 4px;
  font-size: var(--fs-sub);
  color: var(--ink2);
  letter-spacing: 0;
}
.menu button i {
  font-style: normal;
  font-size: var(--fs-meta);
  color: var(--ink3);
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.menu button:hover {
  background: var(--hover);
  color: var(--ink);
}
[data-theme="C"] .menu button:hover {
  background: var(--accent);
}
.menu button.on {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.menu button.on span::before {
  content: "";
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--ink);
  margin: 0 8px 2px -2px;
  vertical-align: middle;
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
  white-space: nowrap;
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
.dot.q {
  background: var(--ink2);
}
.dot.bin {
  background: transparent;
  border: 1.5px solid var(--ink3);
}
[data-theme="C"] .dot.q {
  background: #141412;
}
/* 完成页：回收站那一行“还没腾出”带点警示色 */
.m-row.recycle .c {
  color: var(--review-label);
}
.m-list.done .m-row:first-child {
  border-top: 0;
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
/* 跳过原因：名字 | 个数 | 字节 三列对齐 */
.m-sub.reasons {
  max-height: 180px;
  overflow-y: auto;
  scrollbar-width: thin;
  padding-top: 4px;
}
.m-sub.reasons span:not(.item):not(.procs) {
  display: grid;
  grid-template-columns: 1fr 90px 76px;
  gap: 12px;
  line-height: 22px;
}
.m-sub.reasons span[title] {
  cursor: help;
}
.m-sub.reasons span:not(.item):not(.procs) b {
  text-align: right;
}
.m-sub.reasons .procs,
.m-sub.reasons .item {
  line-height: 20px;
  color: var(--ink3);
}
.m-sub.reasons .details {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-top: 6px;
  padding-top: 8px;
  border-top: 1px dashed var(--line);
}
.m-sub.reasons .procs i,
.m-sub.reasons .item i {
  color: var(--ink3);
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
.m-skip:not(.checking):hover b {
  color: var(--ink);
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
  flex: none;
  border-radius: 50%;
  border: 2px solid var(--line2);
  border-top-color: var(--ink2);
  animation: spin 0.9s linear infinite;
}
[data-theme="C"] .spin {
  border-color: rgba(20, 20, 18, 0.2);
  border-top-color: #141412;
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
/* 一行短字 + 一个小动作（授权 / 选文件夹… / 改放 D 盘） */
.act-row {
  margin-top: 12px;
  min-height: 30px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: var(--fs-sub);
  color: var(--review-label);
}
.act-row > span:first-child {
  margin-right: auto;
}
.act-row.quiet-row {
  color: var(--ink3);
}
.act-row.quiet-row > span:first-child {
  margin-right: 0;
}
.chip {
  height: 28px;
  padding: 0 14px;
  border-radius: var(--r-pill);
  border: var(--bw) solid var(--line2);
  font-family: var(--ui);
  font-size: var(--fs-sub);
  font-weight: var(--w-semi);
  color: var(--ink);
  flex: none;
}
.chip:hover:not(:disabled) {
  background: var(--hover);
}
.chip:disabled {
  opacity: 0.6;
  cursor: progress;
}
[data-theme="C"] .chip {
  background: #fff;
  box-shadow: 2px 2px 0 #141412;
}
[data-theme="C"] .chip:hover:not(:disabled) {
  background: var(--accent);
}
.note {
  margin-top: 12px;
  font-size: var(--fs-meta);
  color: var(--review-label);
  line-height: 1.5;
  user-select: text;
}
.note[title] {
  cursor: help;
}
.act-row.warn-row {
  color: var(--danger);
  cursor: help;
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
/* 主按钮里的预计用时：「⏱ 预计约 2 分钟」，一条细竖线隔开，比按钮字小一号但看得清 */
.btn .est-t {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: var(--ui);
  font-size: 15px;
  font-weight: var(--w-semi);
  opacity: 0.9;
  margin-left: 14px;
  padding-left: 14px;
  border-left: 1px solid currentColor;
  line-height: 1;
  letter-spacing: 0;
}
.btn .est-t .clk {
  width: 15px;
  height: 15px;
  stroke: currentColor;
  stroke-width: 2.2;
  fill: none;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.btn .est-t .spin {
  border-color: currentColor;
  border-top-color: transparent;
  opacity: 0.8;
}
[data-theme="C"] .btn .est-t {
  border-left-color: rgba(212, 242, 76, 0.45);
}

/* ── 执行中 ── */
.prog {
  margin-top: 22px;
  height: 6px;
  border-radius: 3px;
  background: var(--sunk);
  overflow: hidden;
}
.prog i {
  display: block;
  height: 100%;
  background: var(--safe);
  border-radius: inherit;
  transition: width 220ms linear;
}
[data-theme="C"] .prog {
  height: 14px;
  border: 2px solid #141412;
  border-radius: 8px;
  background: #fff;
}
[data-theme="C"] .prog i {
  border-radius: 0;
  background: repeating-linear-gradient(-45deg, #d4f24c 0 8px, #c3e335 8px 16px);
  border-right: 2px solid #141412;
}
/* 条下的小字：已处理多少（字节 + 个数），等宽数字不跳 */
.done-line {
  margin-top: 10px;
  height: 18px;
  font-size: var(--fs-meta);
  color: var(--ink3);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.m-total .v.pct {
  font-variant-numeric: tabular-nums;
}
/* 展示字体只有数字：% 用界面字体，比 GB 单位大一号，贴着数字 */
.m-total .v.pct small {
  margin-left: 3px;
  font-size: 30px;
  font-weight: var(--w-semi);
}
[data-theme="B"] .m-total .v.pct small {
  color: #c99b4a;
  font-weight: 300;
}
[data-theme="C"] .m-total .v.pct small {
  color: #141412;
}
.run-line {
  margin-top: 10px;
  height: 30px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: var(--fs-body);
  color: var(--ink2);
}
/* 执行中大数字右边的剩余时间（刚开始用预检估的用时，之后按实际速率） */
.m-total .after .remain-big {
  font-size: 20px;
  font-weight: var(--w-semi);
  color: var(--ink);
  margin-left: 0;
}
.run-line .stopping {
  color: var(--ink2);
}
.stop {
  height: 30px;
  padding: 0 14px;
  border-radius: var(--r-pill);
  border: var(--bw) solid var(--line2);
  font-family: var(--ui);
  font-size: var(--fs-sub);
  font-weight: var(--w-semi);
  color: var(--ink);
  transition:
    background-color var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease),
    border-color var(--dur-1) var(--ease);
}
.stop:hover {
  border-color: var(--danger);
  color: var(--danger);
}
/* 「剩约 2 分 · 停止」：点后面的按钮收紧一点；前 2 秒没有剩余时间时按钮和上面的字左对齐 */
.run-line .sep + .stop {
  margin-left: -4px;
}
[data-theme="C"] .stop {
  background: #fff;
  box-shadow: 2px 2px 0 #141412;
}
[data-theme="C"] .stop:hover {
  background: var(--review);
  color: #141412;
  border-color: #141412;
}
/* 关窗被拦下：停止按钮闪三下 */
.stop.nudge {
  animation: nudge 0.5s var(--ease) 3;
  border-color: var(--danger);
  color: var(--danger);
}
[data-theme="C"] .stop.nudge {
  background: var(--review);
  color: #141412;
  border-color: #141412;
}
@keyframes nudge {
  0%,
  100% {
    transform: none;
    box-shadow: 0 0 0 0 transparent;
  }
  40% {
    transform: scale(1.08);
    box-shadow: 0 0 0 6px color-mix(in srgb, var(--danger) 22%, transparent);
  }
}
.hint {
  font-size: var(--fs-sub);
  color: var(--danger);
  font-weight: var(--w-med);
  animation: fade var(--dur-2) var(--ease);
}
[data-theme="C"] .hint {
  color: #141412;
  background: var(--review-soft);
  border-radius: 999px;
  padding: 2px 10px;
}
.cur {
  margin-top: 8px;
  height: 18px;
  font-size: var(--fn-path);
  color: var(--ink3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
@media (max-width: 600px) {
  .modal {
    padding: 22px 18px 20px;
  }
  .m-total .v {
    font-size: 52px;
  }
}
</style>
