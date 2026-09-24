<script setup lang="ts">
// 图例：每种颜色一句大白话常驻显示（不再只放在悬停提示里），后面是“这一层有几项”；
// 说明从哪来只分“规则库 / 推测”（未认出已经是一种颜色，不再重复当来源）。
// 很小的未认出项（< 1 MB）单独数成“小文件夹”，不把第一印象变成“认不全”（第 1 轮评审）。
import { computed } from "vue";
import { sourceKey, tierKey, type DirListing, type SourceKey, type TierKey } from "../api";
import { SOURCE_HINT, SOURCE_SHORT, TIER_HINT } from "../format";
import TierChip from "./TierChip.vue";

const props = defineProps<{ listing: DirListing | null }>();

const SMALL = 1024 * 1024;

const counts = computed(() => {
  const src: Record<SourceKey, number> = { rule: 0, heuristic: 0, llm: 0, unknown: 0 };
  const tier: Record<TierKey, number> = { safe: 0, review: 0, keep: 0, unknown: 0, zero: 0 };
  let smallUnknown = 0;
  for (const n of props.listing?.children ?? []) {
    if (!n.explanation) continue;
    src[sourceKey(n.explanation)]++;
    const t = tierKey(n.explanation);
    if (t === "unknown" && n.allocated < SMALL) smallUnknown++;
    else tier[t]++;
  }
  return { src, tier, smallUnknown };
});

const tiers: TierKey[] = ["safe", "review", "keep", "unknown"];
const sources: SourceKey[] = ["rule", "heuristic"];
</script>

<template>
  <section class="legend card" aria-label="图例">
    <div class="lg-group lg-tiers">
      <span v-for="t in tiers" :key="t" class="lg-item">
        <TierChip :tier="t" />
        <span class="lg-desc">{{ TIER_HINT[t] }}</span>
        <span class="lg-n" v-if="listing">这一层 {{ counts.tier[t] }} 项</span>
      </span>
      <span v-if="listing && counts.smallUnknown" class="lg-item" title="小于 1 MB、还没认出来的项。占的地方很小，先不用管。">
        <span class="lg-small">另有 {{ counts.smallUnknown }} 个没认出来的小文件夹（都不到 1 MB）</span>
      </span>
      <span v-if="listing && counts.tier.zero" class="lg-item">
        <TierChip tier="zero" />
        <span class="lg-n">{{ counts.tier.zero }} 项</span>
      </span>
    </div>
    <div class="lg-group">
      <span class="lg-title">说明从哪来</span>
      <span v-for="s in sources" :key="s" class="lg-item" :title="SOURCE_HINT[s]">
        <span class="src-tag" :class="'src-' + s">{{ SOURCE_SHORT[s] }}</span>
        <span class="lg-desc">{{ SOURCE_HINT[s] }}</span>
      </span>
      <span v-if="counts.src.llm" class="lg-item" :title="SOURCE_HINT.llm">
        <span class="src-tag src-llm">{{ SOURCE_SHORT.llm }}</span>
        <span class="lg-desc">{{ SOURCE_HINT.llm }}</span>
      </span>
    </div>
  </section>
</template>
