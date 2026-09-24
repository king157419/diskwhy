<script setup lang="ts">
// “四种颜色是什么意思”：欢迎页和扫描中就常驻，让人还没扫完就知道结果会怎么标（第 1 轮冷启动评审）。
import type { TierKey } from "../api";
import { TIER_HINT } from "../format";
import TierChip from "./TierChip.vue";

withDefaults(defineProps<{ compact?: boolean }>(), { compact: false });
const tiers: TierKey[] = ["safe", "review", "keep", "unknown"];
</script>

<template>
  <section class="guide card" :class="{ 'guide-compact': compact }" aria-label="四种颜色是什么意思">
    <div class="guide-title">扫完后，每个文件夹都会标上下面四种颜色之一</div>
    <ul class="guide-list">
      <li v-for="t in tiers" :key="t">
        <TierChip :tier="t" />
        <span>{{ TIER_HINT[t] }}</span>
      </li>
    </ul>
  </section>
</template>
