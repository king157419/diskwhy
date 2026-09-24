<script setup lang="ts">
import type { TierKey } from "../../api";
import { TIER_SHORT } from "../../format";
defineProps<{ tier: TierKey; label?: string; size?: "sm" | "lg" }>();
</script>

<template>
  <span class="chip" :class="['chip-' + tier, size === 'lg' ? 'lg' : '']">{{ label ?? TIER_SHORT[tier] }}</span>
</template>
