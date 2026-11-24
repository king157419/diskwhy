<script setup lang="ts">
// 二次确认（清理页 / 看看都是什么）：勾 keep、认不出来、规则说要手动 / 用自带工具删的东西时问一次。
// 只写：是什么（名字）+ 删了丢什么（后果标签）+ 大小；一行「可找回」（这些只进回收站 / 暂存区，不直接删）。
// 两个按钮：「仍要删」（实心）/「只选能删的」（全选时，里面还有能直接删的部分）或「算了」。
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import type { CandidateGroup } from "../api";
import { sizeParts } from "../capacity";
import { loseTag } from "../labels";
import { answerForce, candidates, forceAsk, forceBytes } from "../store";

const MAX_ROWS = 4;
const groups = computed(() =>
  (forceAsk.value?.keys ?? []).map((k) => candidates.value.find((g) => g.key === k)).filter((g): g is CandidateGroup => !!g),
);
const rows = computed(() => [...groups.value].sort((a, b) => forceBytes(b) - forceBytes(a)).slice(0, MAX_ROWS));
const more = computed(() => groups.value.length > MAX_ROWS);
const single = computed(() => groups.value.length === 1);
const total = computed(() => groups.value.reduce((a, g) => a + forceBytes(g), 0));
const canPlain = computed(() => (forceAsk.value?.plainKeys.length ?? 0) > 0);
/** 删了丢什么：规则写的短后果；没有时按档位给一个兜底（keep / 认不出来 = 可能丢数据） */
const lose = (g: CandidateGroup) => loseTag(g) ?? (g.tier === "keep" ? "可能丢数据" : "要手动删");
const dot = (g: CandidateGroup) => (g.tier === "safe" ? "safe" : g.tier === "review" ? "review" : "keep");
const size = (b: number) => sizeParts(b).join(" ");

const el = ref<HTMLElement | null>(null);
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") answerForce(null);
}
onMounted(() => {
  document.addEventListener("keydown", onKey);
  void nextTick(() => el.value?.querySelector<HTMLElement>(".btn")?.focus({ preventScroll: true }));
});
onBeforeUnmount(() => document.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="fc-scrim" @click.self="answerForce(null)">
    <div ref="el" class="fc" role="alertdialog" aria-modal="true" :aria-label="single ? '仍要删：' + rows[0]?.name : '仍要删？'">
      <h3>{{ single ? "仍要删？" : "有些不建议删" }}</h3>
      <div class="fc-rows">
        <div v-for="g in rows" :key="g.key" class="fc-row">
          <span class="dot" :class="dot(g)"></span>
          <span class="n">{{ g.name }}</span>
          <span class="tag" :class="{ plain: g.tier === 'keep' }">{{ lose(g) }}</span>
          <span class="s num">{{ size(forceBytes(g)) }}</span>
        </div>
        <div v-if="more" class="fc-more">…</div>
      </div>
      <div class="fc-note">
        <svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 12a9 9 0 1 0 3-6.7" /><path d="M3 4v5h5" /></svg>
        <span>进回收站 / 暂存区，可找回</span>
        <b v-if="!single" class="num">{{ size(total) }}</b>
      </div>
      <div class="fc-btns">
        <button v-if="canPlain" class="quiet" @click="answerForce('plain')">只选能删的</button>
        <button v-else class="quiet" @click="answerForce(null)">算了</button>
        <button class="btn" @click="answerForce('all')">仍要删</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fc-scrim {
  position: absolute;
  inset: var(--titlebar) 0 0 0;
  background: var(--overlay);
  z-index: 90;
  display: grid;
  place-items: center;
  animation: fade var(--dur-2) var(--ease);
}
@keyframes fade {
  from {
    opacity: 0;
  }
}
.fc {
  width: min(400px, calc(100vw - 32px));
  background: var(--surface);
  border-radius: var(--r-lg);
  padding: 22px 24px 20px;
  box-shadow: var(--shadow-pop);
  animation: rise var(--dur-3) var(--ease);
}
@keyframes rise {
  from {
    opacity: 0;
    transform: translateY(6px);
  }
}
[data-theme="C"] .fc {
  border: 2px solid #141412;
  border-radius: 22px;
  box-shadow: 4px 4px 0 #141412;
}
h3 {
  font-family: var(--heading);
  font-size: var(--fs-group);
  font-weight: var(--heading-w);
}
.fc-rows {
  margin-top: 14px;
  border-top: 1px solid var(--line);
}
.fc-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 42px;
  border-bottom: 1px solid var(--line);
  font-size: var(--fs-body);
}
.fc-row .n {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--ink);
  font-weight: var(--w-med);
}
.fc-row .tag {
  flex: none;
}
.fc-row .s {
  flex: none;
  min-width: 64px;
  text-align: right;
  color: var(--ink2);
  font-size: var(--fs-sub);
}
.fc-more {
  padding: 4px 0 0 18px;
  color: var(--ink3);
}
.fc-note {
  margin-top: 12px;
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-sub);
  color: var(--ink2);
}
.fc-note svg {
  width: 15px;
  height: 15px;
  stroke: currentColor;
  stroke-width: 2;
  fill: none;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.fc-note b {
  margin-left: auto;
  color: var(--ink);
  font-weight: var(--w-semi);
}
.fc-btns {
  margin-top: 18px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 18px;
}
.fc-btns .btn {
  height: 44px;
  min-width: 120px;
}
</style>
