<script setup lang="ts">
// 阶段 M：分析页工具栏（图例旁边）的「AI」小标签——后台自动问 AI 的进度，没有一句话：
//   进行中「AI 3/12」+ 小转圈（系统要求减少动画时换成静止的小星）；已问的数按总数的位数留宽（等宽数字、右对齐）：
//   「9/40」→「10/40」标签不变宽，旁边面包屑的截断位置不跟着跳；
//   出错「AI 出错」：悬停 / 点一下弹出原因（一句短话）+「重试」「设置」；
//   全部问完闪一下「AI ✓」约 3 秒（store.aiFlash），然后消失；不写个数（跳过的、以前问过的也算在进度里，写出来对不上）；
//   别的时候不显示。
// 中性配色（墨色 / 灰），不用三档颜色、不用 --accent。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { aiChip } from "../ai";
import { aiFlash, aiRetry, aiStatus, page } from "../store";

const st = computed(() => aiChip(aiStatus.value, aiFlash.value));
const hover = ref(false);
const pinned = ref(false);
const err = computed(() => (st.value?.kind === "err" ? st.value : null));
const open = computed(() => !!err.value && (hover.value || pinned.value));
watch(err, (e) => {
  if (!e) pinned.value = false;
});
// 小标签在鼠标底下消失（进行中 → 闪一下 ✓ → 不显示）时收不到 mouseleave：hover 会一直是 true，
// 下次出错时弹层不等指上去就自己打开。消失时一并复位。
watch(st, (v) => {
  if (!v) {
    hover.value = false;
    pinned.value = false;
  }
});

function retry() {
  pinned.value = false;
  hover.value = false;
  void aiRetry();
}
function goSettings() {
  pinned.value = false;
  page.value = "settings";
}
function onDocDown(e: MouseEvent) {
  if (pinned.value && !(e.target as Element | null)?.closest?.(".aic-wrap")) pinned.value = false;
}
/** Esc 先收起钉住的弹层（捕获阶段先拿到，不让分析页再把方块图全部收起） */
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape" && pinned.value) {
    pinned.value = false;
    e.stopPropagation();
  }
}
onMounted(() => {
  document.addEventListener("mousedown", onDocDown);
  window.addEventListener("keydown", onKey, true);
});
onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onDocDown);
  window.removeEventListener("keydown", onKey, true);
});
</script>

<template>
  <Transition name="aic">
    <div v-if="st" class="aic-wrap" @mouseenter="hover = true" @mouseleave="hover = false">
      <span v-if="st.kind === 'run'" class="aic run" role="status" :aria-label="`AI 识别 ${st.done}/${st.total}`">
        <i class="spin" aria-hidden="true"></i
        ><svg class="ai-star still" viewBox="0 0 12 12" aria-hidden="true"><path d="M6 .6C6.35 3.6 8.4 5.65 11.4 6 8.4 6.35 6.35 8.4 6 11.4 5.65 8.4 3.6 6.35.6 6 3.6 5.65 5.65 3.6 6 .6Z" /></svg
        ><b>AI</b><span class="num"><i class="dn" :style="{ minWidth: String(st.total).length + 'ch' }">{{ st.done }}</i><i class="sl">/</i>{{ st.total }}</span>
      </span>
      <span v-else-if="st.kind === 'done'" class="aic done" role="status" aria-label="AI 识别完成">
        <b>AI</b><svg class="ok" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 6 9 17l-5-5" /></svg>
      </span>
      <template v-else-if="err">
        <button class="aic err" :class="{ on: open }" :aria-expanded="open" aria-label="AI 出错" @click="pinned = !pinned">
          <svg class="warn" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M12 7.5v5.5" /><path d="M12 16.4v.1" /></svg><b>AI</b><span>出错</span>
        </button>
        <div v-if="open" class="aic-pop" role="dialog" aria-label="AI 出错">
          <div class="msg">{{ err.message }}</div>
          <div class="acts">
            <button class="redo" @click="retry">重试</button>
            <button class="quiet" @click="goSettings">设置</button>
          </div>
        </div>
      </template>
    </div>
  </Transition>
</template>

<style scoped>
.aic-wrap {
  position: relative;
  flex: none;
  display: flex;
  align-items: center;
}
.aic {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 11px 0 9px;
  border-radius: var(--r-pill);
  font-family: var(--ui);
  font-size: 12px;
  line-height: 1;
  color: var(--ink2);
  background: var(--sunk);
  border: 1px solid transparent;
  white-space: nowrap;
}
.aic b {
  font-weight: var(--w-semi);
  color: var(--ink);
  letter-spacing: 0.02em;
}
.aic .num {
  font-weight: var(--w-med);
  color: var(--ink2);
}
.aic .sl {
  font-style: normal;
  margin: 0 1px;
  color: var(--ink3);
}
/* 已问的数：按总数的位数留宽（ch = 一个数字宽；.num 是等宽数字），右对齐 */
.aic .dn {
  display: inline-block;
  font-style: normal;
  text-align: right;
}
/* 小转圈：一圈细线，头上一段深一点 */
.spin {
  width: 11px;
  height: 11px;
  border-radius: 50%;
  border: 1.5px solid var(--line2);
  border-top-color: var(--ink2);
  animation: aic-spin 0.85s linear infinite;
  flex: none;
}
@keyframes aic-spin {
  to {
    transform: rotate(360deg);
  }
}
.aic .still {
  display: none;
  color: var(--ink3);
}
@media (prefers-reduced-motion: reduce) {
  .spin {
    display: none;
    animation: none;
  }
  .aic .still {
    display: block;
  }
}
.aic .ok {
  width: 12px;
  height: 12px;
  fill: none;
  stroke: currentColor;
  stroke-width: 2.6;
  stroke-linecap: round;
  stroke-linejoin: round;
  margin: 0 -1px;
}
.aic.err {
  background: transparent;
  border-color: var(--line2);
  cursor: pointer;
  transition: background-color var(--dur-1) var(--ease);
}
.aic.err:hover,
.aic.err.on {
  background: var(--hover);
}
.aic.err span {
  color: var(--ink2);
}
.warn {
  width: 13px;
  height: 13px;
  fill: none;
  stroke: currentColor;
  stroke-width: 2;
  stroke-linecap: round;
  flex: none;
}

/* 深色：底比页面亮一点点 */
[data-theme="B"] .aic {
  background: rgba(255, 255, 255, 0.045);
  border-color: var(--line);
}
[data-theme="B"] .aic.err {
  background: transparent;
  border-color: var(--line2);
}
[data-theme="B"] .aic.err:hover,
[data-theme="B"] .aic.err.on {
  background: rgba(255, 255, 255, 0.06);
}

/* 卡通：和「AI 推测」标签同一种虚线墨框（= AI 的东西）；出错是能点的按钮，实线墨框 */
[data-theme="C"] .aic {
  height: 30px;
  padding: 0 12px 0 10px;
  border: 2px dashed #141412;
  background: transparent;
  color: #141412;
  font-size: 12.5px;
}
[data-theme="C"] .aic b,
[data-theme="C"] .aic .num,
[data-theme="C"] .aic.err span {
  color: #141412;
}
[data-theme="C"] .aic .sl {
  color: #86817a;
}
[data-theme="C"] .spin {
  border: 2px solid rgba(20, 20, 18, 0.18);
  border-top-color: #141412;
}
[data-theme="C"] .aic .still {
  color: #141412;
}
[data-theme="C"] .aic.err {
  border-style: solid;
  background: #fff;
  transition:
    transform var(--dur-1) var(--ease),
    box-shadow var(--dur-1) var(--ease);
}
[data-theme="C"] .aic.err:hover,
[data-theme="C"] .aic.err.on {
  background: #fff;
  transform: translate(-1px, -1px);
  box-shadow: 3px 3px 0 #141412;
}

/* ── 出错的原因 + 重试：悬停 / 点一下出现 ── */
.aic-pop {
  position: absolute;
  right: 0;
  top: calc(100% + 8px);
  z-index: 21;
  width: max-content;
  min-width: 164px;
  max-width: 260px;
  padding: 11px 12px 10px 14px;
  background: var(--surface);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-pop);
  animation: aic-pop-in var(--dur-2) var(--ease);
}
/* 鼠标从小标签挪到弹层上要跨过 8px 的空隙：垫一条看不见的桥，弹层不会半路收起 */
.aic-pop::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: -10px;
  height: 10px;
}
@keyframes aic-pop-in {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}
[data-theme="C"] .aic-pop {
  border: 2px solid #141412;
  box-shadow: 4px 4px 0 #141412;
}
.aic-pop .msg {
  font-size: var(--fs-sub);
  font-weight: var(--w-med);
  line-height: 1.5;
  color: var(--ink);
  overflow-wrap: anywhere;
  user-select: text;
}
.aic-pop .acts {
  margin-top: 10px;
  display: flex;
  align-items: center;
  gap: 14px;
}
.redo {
  height: 28px;
  padding: 0 14px;
  border-radius: var(--r-pill);
  background: var(--ink);
  color: var(--bg);
  font-size: var(--fs-sub);
  font-weight: var(--w-semi);
  transition: filter var(--dur-1) var(--ease);
}
.redo:hover {
  filter: brightness(1.15);
}
[data-theme="B"] .redo {
  background: var(--line2);
  color: var(--ink);
}
[data-theme="C"] .redo {
  border: 2px solid #141412;
  background: #141412;
  color: #fff;
  font-family: var(--heading);
  font-weight: 700;
  font-size: 14px;
}

.aic-enter-active,
.aic-leave-active {
  transition:
    opacity var(--dur-2) var(--ease),
    transform var(--dur-2) var(--ease);
}
.aic-enter-from,
.aic-leave-to {
  opacity: 0;
  transform: scale(0.94);
}

/* 窄窗口：工具栏换到面包屑下面一行、靠左，弹层也朝右展开 */
@media (max-width: 819px) {
  .aic-pop {
    right: auto;
    left: 0;
  }
}
</style>
