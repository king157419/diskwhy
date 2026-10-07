<script setup lang="ts">
// 首页容量柱。竖放（宽窗口，右栏）：每段右边直接写名字和数字（不做图例），细段的标签用 capacity.ts 挤开 + 引线。
// 横放（阶段 J：窄窗口 / 竖屏，horizontal）：段从左到右，标签在柱子下面，同样一维挤开 + 引线；“清理后剩余”是一条竖虚线。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { afterOffset, gbNum, layoutHorizontal, layoutLabels, layoutSegments, type Seg } from "../capacity";

export interface ColSeg extends Seg {
  /** free / safe / review / keep / scanned */
  kind: string;
  label: string;
  note?: string;
}

const props = defineProps<{
  segs: ColSeg[];
  /** 画一条虚线水位：从柱顶（横放时柱子左端）往里这么多字节处（“清理后剩余”） */
  afterBytes?: number;
  afterValue?: string;
  horizontal?: boolean;
}>();

const col = ref<HTMLDivElement | null>(null);
/** 竖放 = 柱高，横放 = 柱宽 */
const length = ref(572);
let ro: ResizeObserver | null = null;
const measure = () => {
  if (col.value) length.value = (props.horizontal ? col.value.clientWidth : col.value.clientHeight) || length.value;
};
onMounted(() => {
  if (!col.value) return;
  measure();
  ro = new ResizeObserver(measure);
  ro.observe(col.value);
});
// 竖放 ↔ 横放是 v-if / v-else 两个不同的 .col：换了元素就要重新观察，否则之后改窗口大小量不到新柱子
watch(
  () => props.horizontal,
  () =>
    requestAnimationFrame(() => {
      ro?.disconnect();
      if (col.value) ro?.observe(col.value);
      measure();
    }),
);
onBeforeUnmount(() => ro?.disconnect());

const SEP = 2;
/** 横放时一个标签框的宽度（名字一行 + 数字一行） */
const LABEL_W = 112;
const kindOf = (key: string) => props.segs.find((s) => s.key === key)!;

const placed = computed(() => layoutSegments(props.segs, length.value, 14, SEP));

const labels = computed(() => {
  const live = placed.value.filter((p) => p.height > 0);
  const pos = layoutLabels(
    live.map((p) => p.top + p.height / 2),
    36,
    10,
    length.value - 10,
  );
  return live.map((p, i) => ({ seg: kindOf(p.key), ...pos[i] }));
});

const hz = computed(() => {
  const l = layoutHorizontal(props.segs, length.value, LABEL_W, { minSeg: 14, sep: SEP, space: 10 });
  return { segs: l.segs, labels: l.labels.map((x) => ({ ...x, seg: kindOf(x.key) })) };
});

/** 虚线位置：累计字节落在第几段、段内多少 */
const afterPos = computed(() => afterOffset(placed.value, props.afterBytes));
/** 横放时虚线上方那行字：靠近左端就向右展开，靠近右端就向左展开 */
const afterAlign = computed(() => ((afterPos.value ?? 0) > length.value / 2 ? "end" : "start"));
</script>

<template>
  <div v-if="!horizontal" class="cap">
    <div ref="col" class="col">
      <div
        v-for="p in placed"
        v-show="p.height > 0"
        :key="p.key"
        class="seg"
        :class="kindOf(p.key).kind"
        :style="{ top: p.top + 'px', height: p.height + 'px' }"
      ></div>
    </div>
    <svg class="leaders" :style="{ height: length + 'px' }" aria-hidden="true">
      <path
        v-for="l in labels"
        :key="l.seg.key"
        :d="l.leader ? `M150 ${l.want} L158 ${l.want} L170 ${l.y} L174 ${l.y}` : `M154 ${l.y} L172 ${l.y}`"
      />
    </svg>
    <div v-for="l in labels" :key="'l' + l.seg.key" class="lab" :class="l.seg.kind" :style="{ top: l.y + 'px' }">
      <span class="k">{{ l.seg.label }}<span v-if="l.seg.note" class="note">{{ l.seg.note }}</span></span>
      <span class="v num">{{ gbNum(l.seg.bytes) }}<small>GB</small></span>
    </div>
    <div v-if="afterPos != null" class="after" :style="{ top: afterPos + 'px' }">
      <span>清理后剩余 <b class="num">{{ afterValue }}</b></span>
    </div>
  </div>

  <div v-else class="cap hz">
    <div ref="col" class="col">
      <div
        v-for="p in hz.segs"
        v-show="p.width > 0"
        :key="p.key"
        class="seg"
        :class="kindOf(p.key).kind"
        :style="{ left: p.left + 'px', width: p.width + 'px' }"
      ></div>
    </div>
    <svg class="leaders" :style="{ width: length + 'px' }" aria-hidden="true">
      <path
        v-for="l in hz.labels"
        :key="l.key"
        :d="l.leader ? `M${l.want} 0 L${l.want} 6 L${l.x} 16 L${l.x} 20` : `M${l.x} 0 L${l.x} 20`"
      />
    </svg>
    <div class="labs">
      <div v-for="l in hz.labels" :key="'l' + l.key" class="lab" :class="l.seg.kind" :style="{ left: l.left + 'px' }" :title="l.seg.note">
        <span class="k">{{ l.seg.label }}</span>
        <span class="v num">{{ gbNum(l.seg.bytes) }}<small>GB</small></span>
      </div>
    </div>
    <div v-if="afterPos != null" class="after" :class="afterAlign" :style="{ left: afterPos + 'px' }">
      <span>清理后剩余 <b class="num">{{ afterValue }}</b></span>
    </div>
  </div>
</template>

<style scoped>
.cap {
  position: relative;
  height: 100%;
}
.col {
  position: absolute;
  left: 0;
  top: 0;
  width: 148px;
  height: 100%;
}
.seg {
  position: absolute;
  left: 0;
  right: 0;
  transition:
    top var(--dur-3) var(--ease),
    height var(--dur-3) var(--ease);
}
.seg.free {
  background: repeating-linear-gradient(135deg, transparent 0 6px, var(--free-line) 6px 7px);
  border: 1px solid var(--free-line);
  border-bottom: 0;
}
.seg.safe {
  background: var(--safe);
}
.seg.review {
  background: var(--review);
}
.seg.keep {
  background: var(--keep);
}
.seg.scanned {
  background: var(--keep);
  background-image: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.18), transparent);
  background-size: 200% 100%;
  animation: sweep 1.6s linear infinite;
}
.seg.pending {
  background: var(--keep2);
}
@keyframes sweep {
  from {
    background-position: 200% 0;
  }
  to {
    background-position: -200% 0;
  }
}
[data-theme="A"] .col {
  border-radius: 4px;
  overflow: hidden;
}
[data-theme="B"] .col {
  border-radius: 6px;
  overflow: hidden;
}
[data-theme="B"] .seg.safe {
  box-shadow: 0 0 40px rgba(235, 196, 122, 0.35);
  background: linear-gradient(180deg, #f1cf8c, #e5b865);
}
[data-theme="C"] .col {
  border: 2px solid #141412;
  border-radius: 16px;
  overflow: hidden;
  box-shadow: 6px 6px 0 #141412;
  background: #fff;
  box-sizing: content-box;
  left: -2px;
  top: -2px;
}
[data-theme="C"] .seg + .seg {
  border-top: 2px solid #141412;
}
[data-theme="C"] .seg.free {
  border: 0;
  background:
    radial-gradient(#141412 1.2px, transparent 1.4px) 0 0 / 10px 10px,
    #fff;
}

.leaders {
  position: absolute;
  left: 0;
  top: 0;
  width: 180px;
  overflow: visible;
  pointer-events: none;
}
.leaders path {
  fill: none;
  stroke: var(--line2);
  stroke-width: 1;
}
[data-theme="C"] .leaders path {
  stroke: #141412;
  stroke-width: 1.5;
}

.lab {
  position: absolute;
  left: 180px;
  display: flex;
  align-items: center;
  gap: 14px;
  transform: translateY(-50%);
  white-space: nowrap;
  transition: top var(--dur-3) var(--ease);
}
.lab .k {
  font-size: var(--fs-body);
  color: var(--ink2);
  font-weight: var(--w-med);
  min-width: 96px;
}
.lab .v {
  font-size: var(--fn-label);
  font-weight: var(--w-med);
  color: var(--ink);
  letter-spacing: -0.01em;
}
.lab .v small {
  font-size: 15px;
  color: var(--ink3);
  margin-left: 4px;
  font-weight: var(--w-med);
}
.lab.safe .v {
  color: var(--safe-label);
}
.lab.review .v {
  color: var(--review-label);
}
[data-theme="C"] .lab.safe .v {
  color: #141412;
  background: var(--safe);
  padding: 0 8px;
  border: 2px solid #141412;
  border-radius: 8px;
}
/* “其他”的小注（系统 · 程序 · 你的文件）写在名字下面，窄的右栏里也不会被裁 */
.lab .k {
  position: relative;
}
.lab .note {
  position: absolute;
  left: 0;
  top: calc(100% + 1px);
  font-family: var(--ui);
  font-size: 11px;
  font-weight: var(--w-reg);
  color: var(--ink3);
  white-space: nowrap;
}
[data-theme="C"] .lab .k {
  font-family: var(--heading);
  font-weight: 700;
  font-size: 16px;
  color: var(--ink);
}

.after {
  position: absolute;
  left: -150px;
  width: 298px;
  border-top: 1.5px dashed var(--ink2);
  pointer-events: none;
}
.after span {
  position: absolute;
  right: 160px;
  top: -26px;
  font-size: var(--fs-meta);
  color: var(--ink2);
  white-space: nowrap;
}
.after b {
  font-weight: var(--w-semi);
  color: var(--ink);
  font-size: 13px;
}
[data-theme="C"] .after {
  border-top: 2px dashed #141412;
}

/* ───── 横放（窄窗口 / 竖屏） ───── */
.cap.hz {
  height: auto;
  padding-top: 26px;
}
.hz .col {
  position: relative;
  left: 0;
  top: 0;
  width: 100%;
  height: var(--hbar, 64px);
}
.hz .seg {
  top: 0;
  bottom: 0;
  left: auto;
  right: auto;
  transition:
    left var(--dur-3) var(--ease),
    width var(--dur-3) var(--ease);
}
.hz .seg.free {
  border: 1px solid var(--free-line);
  border-right: 0;
}
[data-theme="C"] .hz .col {
  left: 0;
  top: 0;
  box-sizing: border-box;
  border-radius: 14px;
}
[data-theme="C"] .hz .seg + .seg {
  border-top: 0;
  border-left: 2px solid #141412;
}
[data-theme="C"] .hz .seg.free {
  border: 0;
}
.hz .leaders {
  position: relative;
  display: block;
  height: 20px;
  left: 0;
  top: 0;
}
.hz .labs {
  position: relative;
  height: 50px;
}
.hz .lab {
  top: 0;
  width: 112px;
  transform: none;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  text-align: center;
  transition: left var(--dur-3) var(--ease);
}
.hz .lab .k {
  min-width: 0;
  font-size: var(--fs-sub);
}
.hz .lab .v {
  font-size: 22px;
  line-height: 1.15;
}
.hz .lab .v small {
  font-size: 12px;
}
[data-theme="C"] .hz .lab .k {
  font-size: 14px;
}
[data-theme="C"] .hz .lab.safe .v {
  border-width: 2px;
  padding: 0 7px;
  border-radius: 8px;
}
.hz .after {
  left: 0;
  top: 8px;
  width: 0;
  height: calc(var(--hbar, 64px) + 18px);
  border-top: 0;
  border-left: 1.5px dashed var(--ink2);
}
.hz .after span {
  right: auto;
  top: -2px;
  left: 8px;
}
.hz .after.end span {
  left: auto;
  right: 8px;
}
[data-theme="C"] .hz .after {
  border-top: 0;
  border-left: 2px dashed #141412;
}
</style>
