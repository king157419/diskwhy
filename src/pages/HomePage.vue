<script setup lang="ts">
// 首页：左重右轻。左边“可放心清理”大数字 + 分类小字 + 主按钮 + 文字链接；右边所扫盘的竖向容量柱。
// 可点的只有：清理、看看都是什么、设置（标题栏）+ 底栏的主题切换 + 盘名下拉（有不止一个固定盘时，阶段 J）。
// 一打开就自动扫描 C 盘，扫描时同一个布局显示进度。窄窗口 / 竖屏（阶段 J §3.7）：大数字在上，容量柱横放在下面。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import CapacityColumn, { type ColSeg } from "../components/CapacityColumn.vue";
import { gbNum } from "../capacity";
import { CLEAN_CAT_NAME, driveName } from "../labels";
import { NARROW_QUERY, useMedia } from "../layout";
import type { DriveInfo } from "../api";
import { elapsedText, filesCount, scanSegs } from "../scanview";
import { THEMES, setTheme, theme } from "../theme";
import {
  actEnv,
  cancelling,
  doCancel,
  doScan,
  drive,
  drives,
  goExplore,
  message,
  page,
  phase,
  progress,
  reviewBytes,
  safeByCat,
  safeBytes,
  safeGroups,
  scanFinishedAt,
  scanRoot,
  summary,
  switchDrive,
} from "../store";

const narrow = useMedia(NARROW_QUERY);

// ── 盘名 + 换盘下拉（只有固定盘 > 1 个时才是下拉） ──
const curRoot = computed(() => summary.value?.root ?? scanRoot.value);
const dname = computed(() => driveName(curRoot.value));
const canSwitch = computed(() => drives.value.length > 1);
const isCur = (d: DriveInfo) => curRoot.value.toUpperCase().startsWith(d.root.toUpperCase());
const menu = ref(false);
function pick(d: DriveInfo) {
  menu.value = false;
  if (!isCur(d)) void switchDrive(d.root);
}
function onDoc(e: Event) {
  if (!menu.value) return;
  if (e instanceof KeyboardEvent && e.key !== "Escape") return;
  if (e instanceof MouseEvent && (e.target as Element | null)?.closest?.(".drv-wrap")) return;
  menu.value = false;
}
onMounted(() => {
  document.addEventListener("mousedown", onDoc);
  document.addEventListener("keydown", onDoc);
});
onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onDoc);
  document.removeEventListener("keydown", onDoc);
});

// 还没开始（启动那一瞬间）也按“扫描中”显示；扫描失败 / 取消（有错误提示）时显示“重新扫描”
const scanning = computed(() => phase.value === "scanning" || (phase.value === "idle" && !summary.value && !message.value));

const total = computed(() => summary.value?.total_bytes ?? drive.value?.total_bytes ?? 0);
const free = computed(() => summary.value?.free_bytes ?? drive.value?.free_bytes ?? 0);
const used = computed(() => summary.value?.used_bytes ?? Math.max(0, total.value - free.value));
const usedPct = computed(() => (total.value ? Math.round((used.value * 100) / total.value) : 0));

// 扫描中不显示扫描字节（去重前的原始数，可能比整个盘还大）：大数字是看过的文件数，容量柱只画剩余 / 已用（阶段 H）
const scanCount = computed(() => filesCount(progress.value?.files ?? 0));
const bigNumber = computed(() => (scanning.value ? scanCount.value.n : gbNum(safeBytes.value)));
const bigUnit = computed(() => (scanning.value ? scanCount.value.u : "GB"));

const segs = computed<ColSeg[]>(() => {
  if (scanning.value) return scanSegs(free.value, used.value);
  const safe = safeBytes.value;
  const review = Math.min(reviewBytes.value, Math.max(0, used.value - safe));
  return [
    { key: "free", kind: "free", label: "剩余", bytes: free.value },
    { key: "safe", kind: "safe", label: "可放心清理", bytes: safe },
    { key: "review", kind: "review", label: "需你判断", bytes: review },
    { key: "keep", kind: "keep", label: "其他", bytes: Math.max(0, used.value - safe - review), note: "系统 · 程序 · 你的文件" },
  ];
});

const split = computed(() => safeByCat.value.slice(0, 4).map(([c, b]) => ({ name: CLEAN_CAT_NAME[c], v: gbNum(b) })));

const foot = computed(() => {
  if (scanning.value) return null;
  const s = summary.value;
  if (!s) return null;
  const secs = Math.max(1, Math.round(s.duration_ms / 1000));
  const d = scanFinishedAt.value ? new Date(scanFinishedAt.value) : new Date();
  const today = new Date().toDateString() === d.toDateString();
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  return { dur: secs >= 60 ? `${Math.floor(secs / 60)} 分 ${secs % 60} 秒` : `${secs} 秒`, when: today ? `今天 ${hm}` : `${d.getMonth() + 1} 月 ${d.getDate()} 日 ${hm}` };
});
const expired = computed(() => actEnv.value?.expired_batches ?? 0);
</script>

<template>
  <div class="home" :class="{ scanning }">
    <div class="panel"></div>
    <section class="hero">
      <div class="hero-in">
        <!-- C 主题插画：文件块从柱顶被弹出去 -->
        <svg class="deco" width="230" height="104" viewBox="0 0 200 92" fill="none" stroke="#141412" stroke-width="2" stroke-linejoin="round" aria-hidden="true">
          <rect x="18" y="44" width="30" height="24" rx="5" fill="#FF8A76" transform="rotate(-18 33 56)" />
          <rect x="70" y="14" width="34" height="26" rx="5" fill="#fff" transform="rotate(12 87 27)" />
          <path d="M78 24h18M78 30h12" transform="rotate(12 87 27)" />
          <rect x="128" y="34" width="26" height="22" rx="5" fill="#D4F24C" transform="rotate(-8 141 45)" />
          <path d="M58 70l8-10M112 60l10-8M40 30l4-8" stroke-linecap="round" />
          <path d="M176 18l3 7 7 3-7 3-3 7-3-7-7-3 7-3z" fill="#141412" />
        </svg>
        <div class="eyebrow">
          <span class="dot safe"></span>
          <template v-if="scanning">正在扫描<span class="cnt num">{{ elapsedText(progress?.elapsed_ms ?? 0) }}</span></template>
          <template v-else>可放心清理<span class="cnt num">{{ safeGroups.length }} 项</span></template>
        </div>
        <div class="big">
          <span class="n display num">{{ bigNumber }}</span><span class="u" :class="{ cjk: scanning }">{{ bigUnit }}</span>
        </div>
        <div v-if="scanning" class="split scan-path mono">{{ progress?.current_path || curRoot }}</div>
        <div v-else class="split">
          <span v-for="s in split" :key="s.name">{{ s.name }}<b class="num">{{ s.v }}</b></span>
        </div>
        <div class="actions">
          <template v-if="scanning">
            <button class="link" :disabled="cancelling" @click="doCancel">{{ cancelling ? "正在取消" : "取消" }}</button>
          </template>
          <template v-else-if="summary">
            <button class="btn" :disabled="safeBytes <= 0 && reviewBytes <= 0" @click="page = 'cleanup'">
              清理 <span class="num">{{ gbNum(safeBytes) }} GB</span>
            </button>
            <button class="link" @click="goExplore">
              看看都是什么<svg class="i" viewBox="0 0 24 24" style="width: 16px; height: 16px"><path d="M5 12h14" /><path d="m12 5 7 7-7 7" /></svg>
            </button>
          </template>
          <template v-else>
            <button class="btn" @click="doScan">重新扫描</button>
          </template>
        </div>
      </div>
    </section>

    <section class="disk">
      <div class="disk-h">
        <div class="drv-wrap">
          <button
            v-if="canSwitch"
            class="lbl drv"
            :class="{ open: menu }"
            aria-haspopup="menu"
            :aria-expanded="menu"
            :aria-label="'换盘：' + dname"
            @click="menu = !menu"
          >
            {{ dname }}<svg class="i chev" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6" /></svg>
          </button>
          <span v-else class="lbl">{{ dname }}</span>
          <div v-if="menu" class="drv-menu" role="menu">
            <button
              v-for="d in drives"
              :key="d.root"
              class="drv-row"
              :class="{ on: isCur(d) }"
              role="menuitemradio"
              :aria-checked="isCur(d)"
              @click="pick(d)"
            >
              <b>{{ driveName(d.root) }}</b><span class="num">剩 {{ gbNum(d.free_bytes) }} GB</span>
              <svg v-if="isCur(d)" class="i ok" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" /></svg>
            </button>
          </div>
        </div>
        <span class="pct">已用 <b class="num">{{ usedPct }}%</b></span>
      </div>
      <div class="disk-read">
        <span class="a num">{{ gbNum(used) }}</span><span class="b num">/ {{ gbNum(total) }} GB</span>
      </div>
      <div class="colwrap">
        <CapacityColumn
          :segs="segs"
          :horizontal="narrow"
          :after-bytes="scanning ? undefined : free + safeBytes"
          :after-value="gbNum(free + safeBytes) + ' GB'"
        />
      </div>
    </section>

    <div class="foot">
      <template v-if="foot">
        <span>扫描完成</span><i>/</i><span class="num">{{ foot.dur }}</span><i>/</i><span class="num">{{ foot.when }}</span>
        <template v-if="expired"><i>/</i><button class="foot-link" @click="page = 'quarantine'">暂存区 {{ expired }} 批到期</button></template>
      </template>
      <span v-else-if="scanning">约需 1 分钟</span>
      <span class="themes" role="radiogroup" aria-label="主题">
        <button
          v-for="t in THEMES"
          :key="t.id"
          class="sw"
          :class="['sw-' + t.id, { on: theme === t.id }]"
          role="radio"
          :aria-checked="theme === t.id"
          :aria-label="'主题：' + t.name"
          :title="t.name"
          @click="setTheme(t.id)"
        ></button>
      </span>
    </div>

  </div>
</template>

<style scoped>
.home {
  --hero-top: clamp(150px, 23.5vh, 212px);
  --big: min(300px, 21vw, 33vh);
}
/* ── 左：主角 ── */
.hero {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 62%;
}
.hero-in {
  position: absolute;
  left: var(--gutter);
  top: var(--hero-top);
}
.eyebrow {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: var(--fs-group);
  font-weight: var(--w-med);
  color: var(--ink2);
}
.eyebrow .cnt {
  font-size: var(--fs-sub);
  color: var(--ink3);
  font-weight: var(--w-reg);
  margin-left: 4px;
}
.big {
  display: flex;
  align-items: baseline;
  gap: 18px;
  margin-top: 22px;
  color: var(--ink);
  white-space: nowrap;
}
.big .n {
  font-size: var(--big);
}
.big .u {
  font-family: var(--ui);
  font-size: 40px;
  font-weight: var(--w-med);
  color: var(--ink2);
  letter-spacing: 0.02em;
}
.scanning .big .n {
  color: var(--ink3);
}
.split {
  margin-top: 34px;
  display: flex;
  font-size: var(--fs-sub);
  color: var(--ink3);
  min-height: 21px;
}
.split span {
  padding: 0 clamp(10px, 1.25vw, 18px);
  border-left: 1px solid var(--line);
}
.split span:first-child {
  padding-left: 0;
  border-left: 0;
}
.split b {
  font-weight: var(--w-med);
  color: var(--ink);
  font-size: 15px;
  margin-left: 6px;
}
.split.scan-path {
  display: block;
  max-width: 560px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 12px;
}
.actions {
  margin-top: 48px;
  display: flex;
  align-items: center;
  gap: 32px;
  min-height: 52px;
}
.foot {
  position: absolute;
  left: var(--gutter);
  bottom: 30px;
  font-size: var(--fs-meta);
  color: var(--ink3);
  display: flex;
  align-items: center;
  gap: 14px;
  white-space: nowrap;
}
.foot i {
  font-style: normal;
  color: var(--line2);
}
.foot-link {
  color: var(--review-label);
  text-decoration: underline;
  text-underline-offset: 3px;
}
.themes {
  margin-left: 22px;
  display: flex;
  gap: 8px;
}
.sw {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: 1px solid var(--line2);
  position: relative;
  transition: transform var(--dur-1) var(--ease);
}
.sw:hover {
  transform: scale(1.12);
}
.sw.on {
  box-shadow: 0 0 0 2px var(--bg), 0 0 0 3.5px var(--ink2);
}
.sw-A {
  background: linear-gradient(135deg, #fbfaf7 50%, #1f6b50 50%);
}
.sw-B {
  background: linear-gradient(135deg, #1b1713 50%, #ebc47a 50%);
}
.sw-C {
  background: linear-gradient(135deg, #d4f24c 50%, #141412 50%);
  border-color: #141412;
}

/* ── 右：所扫盘的容量柱 ── */
.disk {
  position: absolute;
  left: max(560px, 61.4%);
  right: 56px;
  top: 96px;
  bottom: 92px;
  display: flex;
  flex-direction: column;
}
.disk-h {
  display: flex;
  align-items: baseline;
  gap: 14px;
}
.disk-h .lbl {
  font-size: var(--fs-group);
  font-weight: var(--w-semi);
  color: var(--ink);
}
.disk-h .pct {
  margin-left: auto;
  font-size: var(--fs-sub);
  color: var(--ink2);
}
.disk-h .pct b {
  font-weight: var(--w-semi);
  color: var(--review-ink);
}
[data-theme="A"] .disk-h .pct b {
  color: var(--review-ink);
}
.disk-read {
  margin-top: 6px;
  display: flex;
  align-items: baseline;
  gap: 10px;
  color: var(--ink);
}
.disk-read .a {
  font-family: var(--display);
  font-weight: var(--display-w);
  letter-spacing: var(--display-track);
  font-variation-settings: var(--display-vs);
  font-size: 80px;
  line-height: 1;
}
.disk-read .b {
  font-size: 26px;
  color: var(--ink3);
  font-weight: var(--w-med);
}
[data-theme="B"] .disk-read .a {
  font-weight: 300;
}
.colwrap {
  flex: 1;
  min-height: 240px;
  margin-top: 34px;
}

/* ── C 主题装饰：左侧大色块 + 右上插画 ── */
.panel,
.deco {
  display: none;
}
[data-theme="C"] .panel {
  display: block;
  position: absolute;
  left: 20px;
  top: 56px;
  bottom: 20px;
  width: calc(max(560px, 61.4%) - 184px);
  max-width: 700px;
  background: var(--accent);
  border: 2px solid #141412;
  border-radius: 28px;
  box-shadow: 6px 6px 0 #141412;
}
[data-theme="C"] .panel::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: 26px;
  opacity: 0.5;
  background: radial-gradient(rgba(20, 20, 18, 0.18) 1px, transparent 1.3px) 0 0 / 16px 16px;
}
/* 插画在 hero-in 里面（窄窗口时跟着内容走）；宽窗口时位置按整页算：.home 占满视口宽，61.4% = 61.4vw */
[data-theme="C"] .deco {
  display: block;
  position: absolute;
  left: calc(min(470px, max(560px, 61.4vw) - 414px) - var(--gutter));
  top: calc(112px - var(--hero-top));
}
[data-theme="C"] .eyebrow {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 22px;
  color: #141412;
}
[data-theme="C"] .eyebrow .cnt {
  font-family: var(--ui);
  color: #141412;
  opacity: 0.6;
}
[data-theme="C"] .eyebrow .dot {
  width: 12px;
  height: 12px;
  background: #fff;
}
[data-theme="C"] .big .u {
  font-family: var(--display);
  font-weight: 800;
  font-variation-settings: "wdth" 75;
  color: #141412;
  font-size: 56px;
}
[data-theme="C"] .split {
  color: rgba(20, 20, 18, 0.62);
}
[data-theme="C"] .split span {
  border-color: rgba(20, 20, 18, 0.22);
}
[data-theme="C"] .split b {
  color: #141412;
}
[data-theme="C"] .disk-h .lbl {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 22px;
}
[data-theme="C"] .disk-h .pct b {
  color: #141412;
}
[data-theme="C"] .foot {
  color: rgba(20, 20, 18, 0.55);
}
[data-theme="C"] .foot i {
  color: rgba(20, 20, 18, 0.3);
}
[data-theme="C"] .scanning .big .n,
[data-theme="C"] .home.scanning .big .n {
  color: rgba(20, 20, 18, 0.45);
}

/* 扫描中的单位是中文（“万个文件”）：展示字体只有数字，统一用界面字体 */
.home .big .u.cjk {
  font-family: var(--ui);
  font-style: normal;
  font-weight: var(--w-med);
  font-variation-settings: normal;
  font-size: 40px;
  color: var(--ink3);
  letter-spacing: 0.02em;
}

/* A：衬线大数字 + 斜体 GB；B：金色细体 + 柔光 */
[data-theme="A"] .big .n {
  font-size: calc(var(--big) * 1.1);
  margin-left: -8px;
}
[data-theme="A"] .big .u {
  font-family: var(--display);
  font-style: italic;
  font-weight: 400;
  font-size: 64px;
  color: var(--ink2);
  letter-spacing: 0;
}
[data-theme="B"] .big .n {
  font-size: calc(var(--big) * 0.893);
  color: var(--accent);
  text-shadow: 0 0 80px rgba(235, 196, 122, 0.22);
}
[data-theme="B"] .scanning .big .n,
[data-theme="B"] .home.scanning .big .n {
  color: var(--ink3);
  text-shadow: none;
}
[data-theme="B"] .big .u {
  color: #c99b4a;
  font-weight: 300;
  font-size: 44px;
}
[data-theme="B"] .hero::before {
  content: "";
  position: absolute;
  left: -200px;
  top: 80px;
  width: 1000px;
  height: 700px;
  background: radial-gradient(closest-side, rgba(235, 196, 122, 0.07), transparent);
  pointer-events: none;
}

/* ── 盘名下拉（阶段 J §3.6） ── */
.drv-wrap {
  position: relative;
}
.drv {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: -8px;
  padding: 2px 6px 2px 8px;
  border-radius: var(--r);
  border: var(--bw) solid transparent;
  transition: background-color var(--dur-1) var(--ease);
}
.drv:hover,
.drv.open {
  background: var(--hover);
}
.drv .chev {
  width: 16px;
  height: 16px;
  color: var(--ink3);
  transition: transform var(--dur-2) var(--ease);
}
.drv.open .chev {
  transform: rotate(180deg);
}
[data-theme="C"] .drv {
  border-color: #141412;
  border-radius: 999px;
  padding: 0 8px 0 12px;
  margin-left: 0;
  background: #fff;
  box-shadow: 2px 2px 0 #141412;
}
[data-theme="C"] .drv:hover,
[data-theme="C"] .drv.open {
  background: var(--accent);
}
[data-theme="C"] .drv .chev {
  color: #141412;
}
.drv-menu {
  position: absolute;
  left: -8px;
  top: calc(100% + 8px);
  z-index: 30;
  min-width: 196px;
  padding: 6px;
  background: var(--surface);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-pop);
  display: flex;
  flex-direction: column;
  animation: menu-in var(--dur-2) var(--ease);
}
@keyframes menu-in {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}
[data-theme="C"] .drv-menu {
  left: 0;
  border: 2px solid #141412;
  box-shadow: 4px 4px 0 #141412;
}
.drv-row {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 40px;
  padding: 0 12px;
  border-radius: var(--r);
  font-size: var(--fs-body);
}
.drv-row b {
  font-weight: var(--w-semi);
  color: var(--ink);
}
.drv-row span {
  font-size: var(--fs-meta);
  color: var(--ink3);
}
.drv-row .ok {
  margin-left: auto;
  width: 16px;
  height: 16px;
  color: var(--ink2);
}
.drv-row:hover {
  background: var(--hover);
}
[data-theme="C"] .drv-row:hover {
  background: var(--accent);
}
[data-theme="C"] .drv-row b {
  font-family: var(--heading);
  font-weight: 700;
}

/* ───────── 窄窗口 / 竖屏（阶段 J §3.7）：大数字在上，容量柱横放在下面，整页上下排 ───────── */
@media (max-width: 1099px), (max-aspect-ratio: 1/1) {
  .home {
    --big: min(250px, 29vw, 22vh);
    display: flex;
    flex-direction: column;
    gap: 28px;
    padding: calc(var(--titlebar) + 12px) var(--gutter) 22px;
    overflow-x: hidden;
    overflow-y: auto;
    scrollbar-width: thin;
  }
  .hero {
    position: relative;
    left: auto;
    top: auto;
    bottom: auto;
    width: auto;
    flex: 1 0 auto;
    display: flex;
    align-items: center;
    padding: 8px 0;
  }
  .hero-in {
    position: relative;
    left: auto;
    top: auto;
    z-index: 1;
    min-width: 0;
  }
  .big {
    margin-top: 16px;
  }
  .split {
    margin-top: 26px;
    flex-wrap: wrap;
    row-gap: 6px;
  }
  .actions {
    margin-top: 34px;
    gap: 28px;
  }
  .disk {
    position: relative;
    left: auto;
    right: auto;
    top: auto;
    bottom: auto;
    flex: none;
  }
  .disk-read .a {
    font-size: 60px;
  }
  .disk-read .b {
    font-size: 22px;
  }
  .colwrap {
    flex: none;
    min-height: 0;
    margin-top: 6px;
  }
  .foot {
    position: relative;
    left: auto;
    bottom: auto;
    flex: none;
  }
  .themes {
    margin-left: auto;
  }
  [data-theme="C"] .panel {
    display: none;
  }
  /* C：大色块就是 hero 自己 */
  [data-theme="C"] .hero {
    padding: 36px 40px;
    background: var(--accent);
    border: 2px solid #141412;
    border-radius: 28px;
    box-shadow: 6px 6px 0 #141412;
    margin-right: 6px;
  }
  [data-theme="C"] .hero::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 26px;
    opacity: 0.5;
    pointer-events: none;
    background: radial-gradient(rgba(20, 20, 18, 0.18) 1px, transparent 1.3px) 0 0 / 16px 16px;
  }
  [data-theme="C"] .deco {
    left: auto;
    right: -24px;
    top: -34px;
    width: 160px;
    height: 72px;
  }
  [data-theme="C"] .foot {
    color: var(--ink3);
  }
  [data-theme="B"] .hero::before {
    left: -120px;
    top: -40px;
    width: 760px;
    height: 520px;
  }
}
/* 窄而且矮（720×640 这类）：收紧上下间距，插画让位 */
@media (max-width: 1099px) and (max-height: 759px), (max-aspect-ratio: 1/1) and (max-height: 759px) {
  .home {
    gap: 18px;
    padding-bottom: 14px;
  }
  .big {
    margin-top: 10px;
  }
  .split {
    margin-top: 16px;
  }
  .actions {
    margin-top: 20px;
  }
  .disk-read .a {
    font-size: 46px;
  }
  [data-theme="C"] .hero {
    padding: 22px 32px;
  }
  [data-theme="C"] .deco {
    display: none;
  }
  /* 盘名、读数、已用并成一行 */
  .disk {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    column-gap: 18px;
  }
  .disk-h {
    display: contents;
  }
  .disk-h .pct {
    order: 3;
  }
  .disk-read {
    order: 2;
    margin-top: 0;
  }
  .colwrap {
    order: 4;
    grid-column: 1 / -1;
    --hbar: 50px;
  }
  .disk-read .a {
    font-size: 40px;
  }
  .disk-read .b {
    font-size: 18px;
  }
}
</style>
