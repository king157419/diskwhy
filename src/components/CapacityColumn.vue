<script setup lang="ts">
// 首页右侧：C 盘竖向容量柱。每段右边直接写名字和数字（不做图例），细段的标签用 capacity.ts 挤开 + 引线。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { gbNum, layoutLabels, layoutSegments, type Seg } from "../capacity";

export interface ColSeg extends Seg {
  /** free / safe / review / keep / scanned */
  kind: string;
  label: string;
  note?: string;
}

const props = defineProps<{
  segs: ColSeg[];
  /** 画一条虚线水位：从柱顶往下这么多字节处（“清理后剩余”） */
  afterBytes?: number;
  afterValue?: string;
}>();

const col = ref<HTMLDivElement | null>(null);
const height = ref(572);
let ro: ResizeObserver | null = null;
onMounted(() => {
  if (!col.value) return;
  height.value = col.value.clientHeight || 572;
  ro = new ResizeObserver(() => {
    if (col.value) height.value = col.value.clientHeight;
  });
  ro.observe(col.value);
});
onBeforeUnmount(() => ro?.disconnect());

const SEP = 2;
const placed = computed(() => layoutSegments(props.segs, height.value, 14, SEP));
const kindOf = (key: string) => props.segs.find((s) => s.key === key)!;

const labels = computed(() => {
  const live = placed.value.filter((p) => p.height > 0);
  const pos = layoutLabels(
    live.map((p) => p.top + p.height / 2),
    36,
    10,
    height.value - 10,
  );
  return live.map((p, i) => ({ seg: kindOf(p.key), ...pos[i] }));
});

/** 虚线位置：累计字节落在第几段、段内多少 */
const afterY = computed(() => {
  if (props.afterBytes == null || props.afterBytes <= 0) return null;
  let acc = 0;
  for (const p of placed.value) {
    if (p.height <= 0) continue;
    if (props.afterBytes <= acc + p.bytes + 1) {
      const f = p.bytes > 0 ? (props.afterBytes - acc) / p.bytes : 1;
      return Math.round(p.top + p.height * Math.min(1, Math.max(0, f)));
    }
    acc += p.bytes;
  }
  return null;
});
</script>

<template>
  <div class="cap">
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
    <svg class="leaders" :style="{ height: height + 'px' }" aria-hidden="true">
      <path
        v-for="l in labels"
        :key="l.seg.key"
        :d="l.leader ? `M150 ${l.want} L158 ${l.want} L170 ${l.y} L174 ${l.y}` : `M154 ${l.y} L172 ${l.y}`"
      />
    </svg>
    <div v-for="l in labels" :key="'l' + l.seg.key" class="lab" :class="l.seg.kind" :style="{ top: l.y + 'px' }">
      <span class="k">{{ l.seg.label }}</span>
      <span class="v num">{{ gbNum(l.seg.bytes) }}<small>GB</small></span>
      <span v-if="l.seg.note" class="note">{{ l.seg.note }}</span>
    </div>
    <div v-if="afterY != null" class="after" :style="{ top: afterY + 'px' }">
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
.lab .note {
  font-size: var(--fs-meta);
  color: var(--ink3);
  margin-left: -4px;
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
</style>
