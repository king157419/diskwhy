<script setup lang="ts">
// 清理页：左边分类是纯文字行（名字 + 大小 + 细条），右边是表格式列表（名称 / 位置 / 大小），
// 按“可放心清理”“需你判断”分组，行高 50px、细线分隔；需你判断的行只挂一个后果标签（来自规则的 lose_kind）；
// 可放心清理的行在规则写了“要重新下载”这类后果时也挂（“下次会慢一点”不挂）。
// 点一行在行内展开说明（这是什么 / 删了会怎样 / 建议，阶段 H）。底部通栏 + 一个主按钮。
// 窄窗口 / 竖屏（阶段 J §3.7）：左边分类变成顶上一行可横向滚动的标签，表格只留 勾选 / 名字（带标签）/ 大小。
// 行上的标签是规则写的具体后果（lose_short，比如“不能休眠”），颜色按 lose_kind（找不回来实心、其余浅底）。
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { getGuidance, openSystemSettings, type CandidateGroup } from "../api";
import { gbNum, sizeParts } from "../capacity";
import { advice, CLEAN_CATS, driveName, isHardTag, loseTag, shortPath, type CleanCat } from "../labels";
import {
  canCheck,
  candidates,
  checked,
  checkedBytes,
  checkState,
  freesByDefault,
  checkedGroups,
  doPreview,
  groupCat,
  hardWhy,
  previewBusy,
  scanRoot,
  setGroups,
  summary,
  toggleGroup,
} from "../store";

const cats = computed(() => {
  const list = CLEAN_CATS.map((c) => {
    const gs = candidates.value.filter((g) => groupCat(g) === c.id);
    const safe = gs.filter((g) => g.tier === "safe").reduce((a, g) => a + g.bytes, 0);
    const review = gs.filter((g) => g.tier !== "safe").reduce((a, g) => a + g.bytes, 0);
    return { ...c, groups: gs, safe, review, total: safe + review };
  }).filter((c) => c.groups.length > 0);
  return list;
});
const maxCat = computed(() => Math.max(1, ...cats.value.map((c) => c.total)));

const cat = ref<CleanCat | null>(null);
watch(
  cats,
  (list) => {
    if (!list.find((c) => c.id === cat.value)) cat.value = list[0]?.id ?? null;
  },
  { immediate: true },
);
const current = computed(() => cats.value.find((c) => c.id === cat.value) ?? null);

function ordered(gs: CandidateGroup[]) {
  // 能直接删的在前，按大小；只能手动 / 用工具处理的排后面
  return [...gs].sort((a, b) => Number(canCheck(b)) - Number(canCheck(a)) || b.bytes - a.bytes);
}
type Sec = "safe" | "review" | "mine";
/** 列表分段：可放心清理 / 需你判断；「你加的」分类里只有一段 */
const sections = computed<{ t: Sec; rows: CandidateGroup[] }[]>(() => {
  const gs = current.value?.groups ?? [];
  if (current.value?.id === "mine") return [{ t: "mine", rows: ordered(gs) }];
  return [
    { t: "safe", rows: ordered(gs.filter((g) => g.tier === "safe")) },
    { t: "review", rows: ordered(gs.filter((g) => g.tier !== "safe")) },
  ];
});
const SEC_NAME: Record<Sec, string> = { safe: "可放心清理", review: "需你判断", mine: "你加的" };
const SEC_HINT: Record<Sec, string> = { safe: "不丢数据", review: "看过再勾", mine: "可找回" };
const SEC_DOT: Record<Sec, string> = { safe: "safe", review: "review", mine: "keep" };

// ── 全选：整页 / 本类（表头）/ 一段（段标题）都走同一套：全勾满了 → 全取消；否则勾满（要强制的先二次确认一次）──
function selState(gs: CandidateGroup[]): "" | "on" | "part" {
  const cs = gs.filter(canCheck);
  if (!cs.length) return "";
  const st = cs.map(checkState);
  if (st.every((s) => s === "on")) return "on";
  return st.some((s) => s) ? "part" : "";
}
function toggleAll(gs: CandidateGroup[]) {
  const keys = gs.filter(canCheck).map((g) => g.key);
  setGroups(keys, selState(gs) !== "on");
}
const headState = computed(() => selState(current.value?.groups ?? []));
const pageState = computed(() => selState(candidates.value));

const open = ref<string | null>(null);
function toggleOpen(k: string) {
  open.value = open.value === k ? null : k;
}

// 建议是“用 Windows 自带工具清”时，能一键打开对应的系统设置页就给个按钮（阶段 E 的 get_guidance / open_system_settings）
const toolUri = ref<Record<string, string | null>>({});
watch(open, async (k) => {
  const g = k ? candidates.value.find((x) => x.key === k) : null;
  if (!g || !g.paths.length || k! in toolUri.value || advice(g.tier, g.how, g.how_detail, g.group).kind !== "tool") return;
  try {
    const gs = await getGuidance(g.paths[0].path, "Dir");
    toolUri.value = { ...toolUri.value, [g.key]: gs.find((x) => x.settings_uri)?.settings_uri ?? null };
  } catch {
    toolUri.value = { ...toolUri.value, [g.key]: null };
  }
});

function loc(g: CandidateGroup): { text: string; mono: boolean } {
  if (g.paths.length === 1) return { text: shortPath(g.paths[0].path), mono: true };
  if (g.heuristic) return { text: `${g.paths.length} 个项目`, mono: false };
  return { text: `${g.paths.length} 个位置`, mono: false };
}

/** 行上的后果标签：需你判断的一律挂；可放心清理的只在规则写了比“下次慢一点”更重的后果（比如要重新下载）时挂 */
function rowTag(g: CandidateGroup): string | null {
  if (g.tier === "review") return loseTag(g);
  return g.lose_kind && g.lose_kind !== "slower" ? loseTag(g) : null;
}

function manualLabel(g: CandidateGroup): string {
  if (g.how.includes("builtin_tool")) return "用自带工具";
  if (g.how.includes("migrate") && !g.how.includes("manual")) return "搬到别的盘";
  return "要手动删";
}

// 点「清理」后先做快速计划（不到 1 秒），确认弹层马上出现，逐个文件的检查在弹层里显示进度（阶段 G）；
// 这段很短的等待期间按钮上显示已经等了多久
const elapsed = ref(0);
let timer: ReturnType<typeof setInterval> | undefined;
watch(previewBusy, (busy) => {
  clearInterval(timer);
  elapsed.value = 0;
  if (busy) timer = setInterval(() => elapsed.value++, 1000);
});
onBeforeUnmount(() => clearInterval(timer));

const free = computed(() => summary.value?.free_bytes ?? 0);
const dname = computed(() => driveName(summary.value?.root ?? scanRoot.value));
const keepBytes = computed(() => summary.value?.tiers.keep_bytes ?? 0);
const totalChecked = computed(() => checkedGroups.value.length);
const ck = (s: string) => s === "on";
</script>

<template>
  <div class="cleanup">
    <nav class="rail" aria-label="分类">
      <h6>分类</h6>
      <button
        v-for="c in cats"
        :key="c.id"
        class="cat"
        :class="{ on: c.id === cat }"
        :aria-label="'分类：' + c.name"
        :aria-pressed="c.id === cat"
        @click="cat = c.id; open = null"
      >
        <span class="nm">{{ c.name }}</span>
        <span class="sz num">{{ gbNum(c.total) }} GB</span>
        <span class="cbar">
          <i class="s" :style="{ width: (c.safe / maxCat) * 100 + '%' }"></i>
          <i class="r" :style="{ width: (c.review / maxCat) * 100 + '%' }"></i>
        </span>
      </button>
      <div class="rail-foot"><span class="dot keep"></span>先别动 <b class="num">{{ gbNum(keepBytes) }} GB</b></div>
    </nav>

    <main v-if="current" class="main">
      <div class="head">
        <h1>{{ current.name }}</h1>
        <span class="meta"><b class="num">{{ gbNum(current.total) }}</b> GB</span>
        <button class="all" role="checkbox" :aria-checked="ck(pageState)" aria-label="全选：所有分类" @click="toggleAll(candidates)">
          <span class="cb" :class="pageState"><svg v-if="ck(pageState)" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" /></svg></span>全选
        </button>
      </div>

      <div class="grid cols">
        <button class="cb" :class="headState" role="checkbox" :aria-checked="ck(headState)" :aria-label="'全选：' + current.name" @click="toggleAll(current.groups)">
          <svg v-if="ck(headState)" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" /></svg>
        </button>
        <span>名称</span><span>位置</span><span class="r">大小</span><span></span>
      </div>

      <div class="scroll">
        <template v-for="grp in sections" :key="grp.t">
          <template v-if="grp.rows.length">
            <div class="grp">
              <button
                class="cb"
                :class="selState(grp.rows)"
                role="checkbox"
                :aria-checked="ck(selState(grp.rows))"
                :aria-label="'全选：' + SEC_NAME[grp.t]"
                :disabled="!grp.rows.some(canCheck)"
                @click="toggleAll(grp.rows)"
              >
                <svg v-if="ck(selState(grp.rows))" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" /></svg>
              </button>
              <span class="dot" :class="SEC_DOT[grp.t]"></span>{{ SEC_NAME[grp.t] }}
              <span class="hint">{{ SEC_HINT[grp.t] }}</span>
            </div>
            <template v-for="g in grp.rows" :key="g.key">
              <div class="grid row" :class="{ off: !checked.has(g.key), open: open === g.key }">
                <button
                  v-if="canCheck(g)"
                  class="cb"
                  :class="checkState(g)"
                  role="checkbox"
                  :aria-checked="ck(checkState(g))"
                  :aria-label="'选中：' + g.name"
                  @click="toggleGroup(g.key)"
                >
                  <svg v-if="ck(checkState(g))" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" /></svg>
                </button>
                <span v-else class="cb na" :title="hardWhy(g) ?? ''"></span>
                <button class="nm" :aria-label="'展开：' + g.name" :aria-expanded="open === g.key" @click="toggleOpen(g.key)">
                  <span class="nm-t">{{ g.name }}</span>
                  <span v-if="hardWhy(g)" class="tag plain">{{ hardWhy(g) }}</span>
                  <template v-else>
                    <span v-if="rowTag(g)" class="tag" :class="{ hard: isHardTag(g.lose_kind) }">{{ rowTag(g) }}</span>
                    <span v-if="g.deletable_bytes <= 0 && g.tier === 'safe'" class="tag plain">{{ manualLabel(g) }}</span>
                  </template>
                </button>
                <span class="loc" :class="{ cjk: !loc(g).mono }" :title="g.paths.map((p) => p.path).join('\n')" @click="toggleOpen(g.key)">{{ loc(g).text }}</span>
                <span class="sz num" @click="toggleOpen(g.key)">{{ sizeParts(g.bytes)[0] }}<small>{{ sizeParts(g.bytes)[1] }}</small></span>
                <span class="more" @click="toggleOpen(g.key)">
                  <svg class="i" viewBox="0 0 24 24"><path :d="open === g.key ? 'm18 15-6-6-6 6' : 'm6 9 6 6 6-6'" /></svg>
                </span>
              </div>
              <div v-if="open === g.key" class="detail" :class="g.tier">
                <dl>
                  <dt>这是什么</dt>
                  <dd>{{ g.what }}</dd>
                  <dt>删了会怎样</dt>
                  <dd><b v-if="rowTag(g)" class="lead">{{ rowTag(g) }}。</b>{{ g.lose }}</dd>
                  <dt>建议</dt>
                  <dd>
                    <b class="lead">{{ advice(g.tier, g.how, g.how_detail, g.group).head }}。</b>{{ advice(g.tier, g.how, g.how_detail, g.group).detail }}
                    <button v-if="toolUri[g.key]" class="link small tool" @click="openSystemSettings(toolUri[g.key]!)">打开存储设置</button>
                  </dd>
                  <template v-if="g.paths.length > 1">
                    <dt>位置</dt>
                    <dd class="paths">
                      <span v-for="p in g.paths.slice(0, 6)" :key="p.path"><code>{{ shortPath(p.path) }}</code><i class="num">{{ sizeParts(p.bytes).join(" ") }}</i></span>
                      <span v-if="g.paths.length > 6" class="more-n">…共 {{ g.paths.length }} 个</span>
                    </dd>
                  </template>
                </dl>
              </div>
            </template>
          </template>
        </template>
      </div>
    </main>
    <main v-else class="main empty">
      <div class="head"><h1>没有可清理的</h1></div>
    </main>

    <footer class="bar">
      <div class="big"><span class="k">已选</span><span class="v num">{{ gbNum(checkedBytes) }}<small>GB</small></span></div>
      <div class="sep"></div>
      <div class="free">
        <span class="k">{{ dname }}剩余</span><span class="fv" title="按默认去向估算，回收站不腾空间"><b class="num">{{ gbNum(free) }}</b><span class="arr">→</span><b class="num">{{ gbNum(free + freesByDefault) }} GB</b></span>
      </div>
      <button class="btn" :disabled="!totalChecked || previewBusy" @click="doPreview()">
        <template v-if="previewBusy">正在检查 <span class="num">{{ elapsed }} 秒</span></template>
        <template v-else>清理 <span class="num">{{ gbNum(checkedBytes) }} GB</span></template>
      </button>
    </footer>
  </div>
</template>

<style scoped>
/* ── 左：分类（纯文字行，不是卡片） ── */
.rail {
  position: absolute;
  left: 24px;
  top: 60px;
  width: 272px;
  bottom: 88px;
  padding-right: 24px;
  border-right: 1px solid var(--line);
  display: flex;
  flex-direction: column;
}
.rail h6 {
  font-size: var(--fs-meta);
  font-weight: var(--w-med);
  color: var(--ink3);
  letter-spacing: 0.08em;
  padding: 12px 16px 10px;
}
.cat {
  position: relative;
  padding: 12px 16px 13px;
  display: grid;
  grid-template-columns: 1fr auto;
  row-gap: 8px;
  align-items: baseline;
  border: var(--bw) solid transparent;
  border-radius: var(--r);
  transition: background-color var(--dur-1) var(--ease);
}
.cat:hover:not(.on) {
  background: var(--hover);
}
.cat .nm {
  font-size: var(--fs-body);
  color: var(--ink2);
  font-weight: var(--w-med);
}
.cat .sz {
  font-size: 14px;
  color: var(--ink3);
  font-weight: var(--w-med);
}
.cat .cbar {
  grid-column: 1/3;
  height: 3px;
  background: var(--sunk);
  border-radius: 2px;
  display: flex;
  overflow: hidden;
}
.cat .cbar i {
  display: block;
  height: 100%;
}
.cat .cbar .s {
  background: var(--safe);
}
.cat .cbar .r {
  background: var(--review);
  opacity: 0.75;
}
.cat.on .nm {
  color: var(--ink);
  font-weight: var(--w-bold);
}
.cat.on .sz {
  color: var(--ink);
}
.cat.on::before {
  content: "";
  position: absolute;
  left: -1px;
  top: 12px;
  bottom: 12px;
  width: 2px;
  background: var(--accent);
  border-radius: 2px;
}
[data-theme="C"] .cat {
  margin: 2px 0;
}
[data-theme="C"] .cat.on {
  background: var(--accent);
  border-color: #141412;
  border-radius: 12px;
  box-shadow: 3px 3px 0 #141412;
}
[data-theme="C"] .cat.on::before {
  display: none;
}
[data-theme="C"] .cat.on .cbar {
  background: rgba(20, 20, 18, 0.12);
}
[data-theme="C"] .cat.on .cbar .s {
  background: #141412;
}
[data-theme="C"] .cat .nm {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 16px;
}
[data-theme="C"] .rail h6 {
  font-family: var(--heading);
  font-weight: 700;
}
.rail-foot {
  margin-top: auto;
  padding: 0 16px 4px;
  font-size: var(--fs-meta);
  color: var(--ink3);
  display: flex;
  align-items: center;
  gap: 8px;
}
.rail-foot b {
  font-weight: var(--w-med);
  color: var(--ink2);
}

/* ── 右：列表 ── */
.main {
  position: absolute;
  left: 320px;
  right: 40px;
  top: 60px;
  bottom: 88px;
  display: flex;
  flex-direction: column;
}
.head {
  display: flex;
  align-items: baseline;
  gap: 16px;
  padding: 6px 0 18px;
}
.head h1 {
  font-family: var(--heading);
  font-size: var(--fs-title);
  font-weight: var(--heading-w);
  letter-spacing: 0.01em;
}
[data-theme="C"] .head h1 {
  font-size: 30px;
}
.head .meta {
  font-size: var(--fs-sub);
  color: var(--ink3);
}
.head .meta b {
  font-weight: var(--w-med);
  color: var(--ink2);
}
/* 整页全选：一个小勾选框 + 两个字，靠右；不用三档颜色 */
.head .all {
  margin-left: auto;
  align-self: center;
  display: flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 12px 0 8px;
  border-radius: var(--r);
  font-size: var(--fs-sub);
  font-weight: var(--w-med);
  color: var(--ink2);
  transition: background-color var(--dur-1) var(--ease);
}
.head .all:hover {
  background: var(--hover);
  color: var(--ink);
}
.head .all:focus-visible {
  outline: 2px solid var(--ink3);
  outline-offset: 1px;
}

.grid {
  display: grid;
  grid-template-columns: 40px minmax(0, 1fr) minmax(160px, 330px) 96px 28px;
  align-items: center;
}
.cols {
  height: 34px;
  flex: none;
  border-bottom: var(--bw) solid var(--line2);
  font-size: var(--fs-meta);
  color: var(--ink3);
  letter-spacing: 0.04em;
}
.cols .r {
  text-align: right;
}
.scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  scrollbar-width: thin;
  scrollbar-color: var(--line2) transparent;
}
.grp {
  height: 44px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: var(--fs-sub);
  font-weight: var(--w-semi);
  color: var(--ink);
  border-bottom: 1px solid var(--line);
}
/* 段标题前的勾选框 = 这一段全选（和行上的勾选框对齐） */
.grp .cb {
  margin-right: 12px;
}
.grp .cb:disabled {
  visibility: hidden;
}
[data-theme="C"] .grp .cb {
  margin-right: 10px;
}
.grp .c {
  font-weight: var(--w-reg);
  color: var(--ink3);
}
.grp .hint {
  margin-left: auto;
  margin-right: 28px;
  font-weight: var(--w-reg);
  font-size: var(--fs-meta);
  color: var(--ink3);
}
[data-theme="C"] .grp {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 16px;
}
[data-theme="C"] .grp .hint,
[data-theme="C"] .grp .c {
  font-family: var(--ui);
  font-weight: var(--w-reg);
}
.row {
  height: 50px;
  border-bottom: 1px solid var(--line);
  transition: background-color var(--dur-1) var(--ease);
}
.row:hover {
  background: linear-gradient(90deg, transparent, var(--hover) 6%, var(--hover) 94%, transparent);
}
[data-theme="C"] .row:hover {
  background: #fff;
}
.row .nm {
  display: flex;
  align-items: center;
  gap: 10px;
  padding-right: 14px;
  font-size: var(--fs-body);
  font-weight: var(--w-med);
  color: var(--ink);
  min-width: 0;
  height: 100%;
}
.row .nm:focus-visible {
  outline: none;
}
.row:has(.nm:focus-visible) {
  background: var(--hover);
  box-shadow: inset 2px 0 0 var(--accent);
}
.row .nm-t {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.row .loc {
  font-family: var(--mono);
  font-size: var(--fn-path);
  color: var(--ink3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
  padding-right: 12px;
}
.row .loc.cjk {
  font-family: var(--ui);
  font-size: var(--fs-sub);
}
.row .sz {
  text-align: right;
  font-size: var(--fn-row);
  font-weight: var(--w-med);
  color: var(--ink);
  cursor: pointer;
}
.row .sz small {
  font-size: 12px;
  color: var(--ink3);
  margin-left: 3px;
  font-weight: var(--w-med);
}
.row.off .nm,
.row.off .sz {
  color: var(--ink2);
}
.row .more {
  justify-self: end;
  color: var(--ink3);
  display: grid;
  place-items: center;
  opacity: 0;
  cursor: pointer;
  transition: opacity var(--dur-1) var(--ease);
}
.row:hover .more,
.row.open .more {
  opacity: 1;
}
.row .more svg {
  width: 16px;
  height: 16px;
}
.row.open {
  border-bottom: 0;
}
.row.open .more {
  color: var(--ink);
}

/* 行内展开的说明：左边一条竖线，三行 */
.detail {
  padding: 2px 124px 16px 40px;
  border-bottom: 1px solid var(--line);
  user-select: text;
  animation: drop var(--dur-2) var(--ease);
}
@keyframes drop {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}
.detail dl {
  border-left: 2px solid var(--review);
  padding: 4px 0 4px 16px;
  display: grid;
  grid-template-columns: 72px 1fr;
  row-gap: 8px;
  column-gap: 16px;
  font-size: var(--fs-sub);
  line-height: 1.6;
}
.detail.safe dl {
  border-left-color: var(--safe);
}
.detail dt {
  color: var(--ink3);
}
.detail dd {
  color: var(--ink2);
}
.detail dd .lead {
  color: var(--ink);
  font-weight: var(--w-semi);
}
.detail dd .tool {
  margin-left: 10px;
  font-size: var(--fs-sub);
}
.detail code {
  font-family: var(--mono);
  font-size: 12px;
  color: var(--ink);
  background: var(--sunk);
  padding: 1px 6px;
  border-radius: 4px;
}
.detail .paths {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.detail .paths span {
  display: flex;
  gap: 10px;
  align-items: baseline;
}
.detail .paths i {
  font-style: normal;
  color: var(--ink3);
  font-size: 12px;
}
.detail .more-n {
  color: var(--ink3);
}
[data-theme="C"] .detail dl {
  border-left: 3px solid #141412;
}

/* ── 底部固定栏：通栏细线，不是浮动卡片 ── */
.bar {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 88px;
  border-top: 1px solid var(--line2);
  display: flex;
  align-items: center;
  padding: 0 40px;
  gap: 40px;
  background: var(--bg);
}
.bar .k {
  font-size: var(--fs-sub);
  color: var(--ink3);
  display: block;
  margin-bottom: 2px;
}
.bar .v {
  font-size: 26px;
  font-weight: var(--w-semi);
  color: var(--ink);
  line-height: 1.1;
}
.bar .v small {
  font-size: 14px;
  color: var(--ink3);
  margin-left: 4px;
  font-weight: var(--w-med);
}
.bar .big .v {
  font-family: var(--display);
  font-weight: var(--display-w);
  letter-spacing: var(--display-track);
  font-variation-settings: var(--display-vs);
  font-size: 44px;
  line-height: 0.9;
}
.bar .big .v small {
  font-family: var(--ui);
  letter-spacing: 0;
}
[data-theme="A"] .bar .big .v {
  font-size: 50px;
  color: var(--accent);
}
[data-theme="A"] .bar .big .v small {
  font-family: var(--display);
  font-style: italic;
}
[data-theme="B"] .bar .big .v {
  font-weight: 300;
  color: var(--accent);
}
.bar .sep {
  width: 1px;
  height: 40px;
  background: var(--line);
}
.bar .free {
  font-size: var(--fs-sub);
  color: var(--ink3);
  display: flex;
  align-items: center;
  gap: 10px;
  white-space: nowrap;
}
.bar .free .k {
  margin-bottom: 0;
}
.bar .free .fv {
  display: flex;
  align-items: center;
  gap: 10px;
}
.bar .free b {
  font-weight: var(--w-semi);
  color: var(--ink);
  font-size: 15px;
}
.bar .btn {
  margin-left: auto;
  height: 52px;
  min-width: 200px;
}
[data-theme="C"] .bar {
  background: #141412;
  border-top: 0;
  color: #f4f0e6;
}
[data-theme="C"] .bar .k {
  color: rgba(244, 240, 230, 0.6);
}
[data-theme="C"] .bar .v {
  color: #f4f0e6;
}
[data-theme="C"] .bar .big .v {
  color: var(--accent);
}
[data-theme="C"] .bar .sep {
  background: rgba(244, 240, 230, 0.2);
}
[data-theme="C"] .bar .free {
  color: rgba(244, 240, 230, 0.6);
}
[data-theme="C"] .bar .free b {
  color: #f4f0e6;
}
[data-theme="C"] .bar .btn {
  background: var(--accent);
  color: #141412;
  border-color: var(--accent);
  box-shadow: none;
}

/* ───────── 窄窗口 / 竖屏（阶段 J §3.7）：分类变成顶上一行横向标签；表格只留 勾选 / 名字 / 大小 ───────── */
@media (max-width: 1099px), (max-aspect-ratio: 1/1) {
  .rail {
    left: var(--gutter);
    right: var(--gutter);
    top: 52px;
    bottom: auto;
    width: auto;
    height: 52px;
    padding: 0 6px 0 0;
    border-right: 0;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    mask-image: linear-gradient(90deg, #000 calc(100% - 28px), transparent);
  }
  .rail::-webkit-scrollbar {
    display: none;
  }
  .rail h6,
  .rail-foot {
    display: none;
  }
  .cat {
    flex: none;
    display: flex;
    align-items: baseline;
    gap: 8px;
    height: 38px;
    padding: 9px 14px 0;
    margin: 0;
    border: var(--bw) solid var(--line);
    border-radius: var(--r-pill);
    white-space: nowrap;
  }
  .cat .cbar,
  .cat.on::before {
    display: none;
  }
  .cat .nm {
    font-size: 14px;
  }
  .cat .sz {
    font-size: 12px;
  }
  .cat.on {
    background: var(--ink);
    border-color: var(--ink);
  }
  .cat.on .nm,
  .cat.on .sz {
    color: var(--bg);
  }
  [data-theme="B"] .cat.on {
    background: var(--line2);
    border-color: var(--line2);
  }
  [data-theme="B"] .cat.on .nm,
  [data-theme="B"] .cat.on .sz {
    color: var(--ink);
  }
  [data-theme="C"] .cat {
    margin: 0;
    border: 2px solid #141412;
    background: #fff;
    border-radius: 999px;
  }
  [data-theme="C"] .cat:hover:not(.on) {
    background: var(--safe-soft);
  }
  [data-theme="C"] .cat.on {
    background: var(--accent);
    border-radius: 999px;
    box-shadow: 3px 3px 0 #141412;
  }
  [data-theme="C"] .cat.on .nm,
  [data-theme="C"] .cat.on .sz {
    color: #141412;
  }
  [data-theme="C"] .cat .nm {
    font-size: 15px;
  }

  .main {
    left: var(--gutter);
    right: var(--gutter);
    top: 116px;
  }
  .head {
    padding: 0 0 12px;
    gap: 12px;
    flex-wrap: wrap;
  }
  .grid {
    grid-template-columns: 36px minmax(0, 1fr) 84px 26px;
  }
  .cols > span:nth-of-type(2),
  .row .loc {
    display: none;
  }
  .grp .cb {
    margin-right: 8px;
  }
  .grp .hint {
    margin-right: 26px;
  }
  .detail {
    padding: 2px 4px 16px 36px;
  }
  .detail dl {
    grid-template-columns: 72px minmax(0, 1fr);
    column-gap: 12px;
  }
  .detail .paths span {
    min-width: 0;
  }
  .detail code {
    overflow-wrap: anywhere;
  }

  .bar {
    padding: 0 var(--gutter);
    gap: 24px;
  }
  .bar .free {
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
  }
  .bar .free .k {
    margin-bottom: 2px;
  }
  .bar .btn {
    min-width: 0;
    padding: 0 24px;
  }
}
@media (max-width: 819px) {
  .bar .cnt,
  .bar .cnt-sep {
    display: none;
  }
  .bar {
    gap: 20px;
  }
  .bar .big .v {
    font-size: 38px;
  }
}
</style>
