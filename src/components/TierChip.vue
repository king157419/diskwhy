<script setup lang="ts">
// 档位小标签：可清理（绿）/ 需判断（琥珀）/ 保留（灰 + 锁）/ 未知（淡紫 + 问号）。
import type { TierKey } from "../api";
import { TIER_SHORT } from "../format";

withDefaults(defineProps<{ tier: TierKey; big?: boolean; text?: string }>(), { big: false, text: "" });
</script>

<template>
  <span class="chip" :class="['chip-' + tier, { 'chip-big': big }]">
    <svg v-if="tier === 'keep'" class="chip-ico" viewBox="0 0 16 16" aria-hidden="true">
      <path
        d="M5 7V5a3 3 0 0 1 6 0v2h.5A1.5 1.5 0 0 1 13 8.5v5A1.5 1.5 0 0 1 11.5 15h-7A1.5 1.5 0 0 1 3 13.5v-5A1.5 1.5 0 0 1 4.5 7H5Zm1.5 0h3V5a1.5 1.5 0 0 0-3 0v2Z"
        fill="currentColor"
      />
    </svg>
    <svg v-else-if="tier === 'unknown'" class="chip-ico" viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.5" />
      <path d="M6.2 6.3a1.9 1.9 0 1 1 2.6 1.8c-.5.2-.8.6-.8 1.1v.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      <circle cx="8" cy="11.6" r=".9" fill="currentColor" />
    </svg>
    <svg v-else-if="tier === 'safe'" class="chip-ico" viewBox="0 0 16 16" aria-hidden="true">
      <path d="M3.5 8.5 6.5 11.5 12.5 5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
    <svg v-else class="chip-ico" viewBox="0 0 16 16" aria-hidden="true">
      <path d="M8 2.5 14 13H2Z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
      <path d="M8 6.5v3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      <circle cx="8" cy="11.2" r=".8" fill="currentColor" />
    </svg>
    <span>{{ text || TIER_SHORT[tier] }}</span>
  </span>
</template>
