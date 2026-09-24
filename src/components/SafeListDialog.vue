<script setup lang="ts">
// 全盘可清理清单（第 2 轮冷启动评审）：点“可清理”卡片打开。
// “可清理 23.83 GB”大多在深层（Temp 里的缓存、各个程序的 Cache），在根目录一层看不到；
// 这里按大小列出每一处（带路径），点一行跳到它所在的文件夹并选中它。
// 整体标“需判断”的文件夹（里面混着像是你的数据的东西）照实写：可清理的只是其中一部分。
import { computed, onBeforeUnmount, onMounted } from "vue";
import type { SafeItem, TierSummary } from "../api";
import { humanBytes } from "../format";

const props = defineProps<{ tiers: TierSummary }>();
const emit = defineEmits<{ close: []; reveal: [item: SafeItem] }>();

const items = computed(() => props.tiers.safe_items ?? []);
const total = computed(() => props.tiers.safe_items_total ?? items.value.length);
const shownBytes = computed(() => items.value.reduce((a, b) => a + b.bytes, 0));

function lastSeg(p: string): string {
  const s = p.replace(/\\+$/, "").split("\\");
  return s[s.length - 1] || p;
}
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal card safe-list" role="dialog" aria-label="全盘可清理清单">
      <h2>全盘可清理清单</h2>
      <p class="muted small">
        一共 {{ humanBytes(tiers.safe_bytes) }}，分在 {{ total }} 处{{ total > items.length ? `（这里列出最大的 ${items.length} 处，共 ${humanBytes(shownBytes)}）` : "" }}。
        大多在深层文件夹里，所以在根目录那一层看不到。点一行跳到它所在的文件夹。
      </p>
      <ol class="safe-rows">
        <li v-for="it in items" :key="it.path + '|' + it.rule_id">
          <button type="button" class="safe-row" :title="it.path" @click="emit('reveal', it)">
            <span class="sr-size">{{ humanBytes(it.bytes) }}</span>
            <span class="sr-main">
              <span class="sr-name">
                {{ it.mixed ? `${lastSeg(it.path)} 里的缓存` : lastSeg(it.path) }}
                <span v-if="it.mixed" class="sr-tag sr-mixed">整个文件夹标“需判断”</span>
                <span v-if="it.tool_only" class="sr-tag">用写明的工具清</span>
              </span>
              <span class="sr-what muted small">{{ it.owner }} · {{ it.what }}</span>
              <span class="sr-path muted small">{{ it.path }}</span>
              <span v-if="it.mixed" class="sr-note small">里面还混着看起来是你的数据的东西，别整个删；可清理的是其中这 {{ humanBytes(it.bytes) }}，点进去逐项看。</span>
            </span>
          </button>
        </li>
      </ol>
      <p v-if="!items.length" class="muted">这次扫描没有找到能放心清理的地方。</p>
      <div class="modal-actions">
        <span class="spacer"></span>
        <button class="btn" type="button" @click="emit('close')">关闭</button>
      </div>
    </div>
  </div>
</template>
