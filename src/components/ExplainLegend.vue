<script setup lang="ts">
// 顶部小图例：颜色代表什么档位、说明从哪来（规则 / 推测 / 未知），并数一数当前文件夹里各有几项。
import { computed } from "vue";
import { sourceKey, tierKey, type DirListing, type SourceKey, type TierKey } from "../api";
import { SOURCE_HINT, SOURCE_SHORT } from "../format";
import TierChip from "./TierChip.vue";

const props = defineProps<{ listing: DirListing | null }>();

const counts = computed(() => {
  const src: Record<SourceKey, number> = { rule: 0, heuristic: 0, llm: 0, unknown: 0 };
  const tier: Record<TierKey, number> = { safe: 0, review: 0, keep: 0, unknown: 0 };
  for (const n of props.listing?.children ?? []) {
    if (!n.explanation) continue;
    src[sourceKey(n.explanation)]++;
    tier[tierKey(n.explanation)]++;
  }
  return { src, tier };
});

const tiers: { key: TierKey; hint: string }[] = [
  { key: "safe", hint: "缓存、临时文件一类，删了会自己长回来" },
  { key: "review", hint: "删了有代价，看说明再决定" },
  { key: "keep", hint: "系统或你的数据，别删；可以迁走或用自带工具" },
  { key: "unknown", hint: "没认出来，先别删" },
];
const sources: SourceKey[] = ["rule", "heuristic", "unknown"];
</script>

<template>
  <section class="legend card" aria-label="图例">
    <div class="lg-group">
      <span class="lg-title">颜色</span>
      <span v-for="t in tiers" :key="t.key" class="lg-item" :title="t.hint">
        <TierChip :tier="t.key" />
        <span class="lg-n" v-if="listing">{{ counts.tier[t.key] }}</span>
      </span>
    </div>
    <div class="lg-group">
      <span class="lg-title">解释来源</span>
      <span v-for="s in sources" :key="s" class="lg-item" :title="SOURCE_HINT[s]">
        <span class="src-tag" :class="'src-' + s">{{ SOURCE_SHORT[s] }}</span>
        <span class="lg-desc">{{ SOURCE_HINT[s] }}</span>
        <span class="lg-n" v-if="listing">{{ counts.src[s] }}</span>
      </span>
      <span v-if="counts.src.llm" class="lg-item" :title="SOURCE_HINT.llm">
        <span class="src-tag src-llm">{{ SOURCE_SHORT.llm }}</span>
        <span class="lg-n">{{ counts.src.llm }}</span>
      </span>
    </div>
    <span v-if="listing" class="lg-foot muted small">数字是当前文件夹里的项数</span>
  </section>
</template>
