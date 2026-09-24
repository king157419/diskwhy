<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { pickFolder, type CartItem, type DriveInfo, type Strategy } from "../api";
import { TIER_SHORT, humanBytes } from "../format";

const props = defineProps<{ cart: CartItem[]; drives: DriveInfo[]; scanRoot: string; busy: boolean; open: boolean }>();
const emit = defineEmits<{ remove: [path: string]; clear: []; preview: [strategy: Strategy]; toggle: [open: boolean] }>();

const total = computed(() => props.cart.reduce((s, c) => s + c.allocated, 0));
const mode = ref<"recycle" | "quarantine">("recycle");
const srcDrive = computed(() => (props.scanRoot.slice(0, 2) + "\\").toUpperCase());
/** 隔离区只能选源所在盘以外的固定盘 */
const otherDrives = computed(() => props.drives.filter((d) => d.root.toUpperCase() !== srcDrive.value));
const dest = ref("");
/** “其他文件夹…”：很多机器上普通用户不能在盘的根目录（如 D:\）建文件夹，可以换成盘上的某个文件夹 */
const CUSTOM = "__custom__";
const customDest = ref("");
const pickErr = ref("");
async function browse() {
  pickErr.value = "";
  try {
    const p = await pickFolder("选择隔离区放在哪个文件夹");
    if (p) customDest.value = p;
  } catch (e) {
    pickErr.value = String(e);
  }
}
watch(
  otherDrives,
  (v) => {
    if (dest.value !== CUSTOM && !v.find((d) => d.root === dest.value)) dest.value = v[0]?.root ?? CUSTOM;
  },
  { immediate: true },
);

/** 外部（预览对话框里“切换到隔离区”）调用 */
function useQuarantine(root: string) {
  mode.value = "quarantine";
  if (otherDrives.value.find((d) => d.root === root)) dest.value = root;
  else {
    dest.value = CUSTOM;
    customDest.value = root;
  }
}
defineExpose({ useQuarantine });

function strategy(): Strategy {
  if (mode.value === "recycle") return { kind: "recycle_bin" };
  return { kind: "quarantine", dest_root: dest.value === CUSTOM ? customDest.value.trim() : dest.value };
}
const canPreview = computed(() => mode.value === "recycle" || (dest.value === CUSTOM ? !!customDest.value.trim() : !!dest.value));
</script>

<template>
  <div class="drawer" :class="{ open }">
    <button class="drawer-tab" :aria-expanded="open" @click="emit('toggle', !open)">
      🧹 清理清单 <b>{{ cart.length }}</b> 项
      <span v-if="cart.length"> · 约 {{ humanBytes(total) }}</span>
      <span class="chev">{{ open ? "▾" : "▴" }}</span>
    </button>
    <div v-if="open" class="drawer-body card" role="region" aria-label="清理清单">
      <div class="drawer-head">
        <b>清理清单</b>
        <span class="muted small">合计最多能腾出约 <b>{{ humanBytes(total) }}</b>（按扫描结果；预览时按实际文件重新算）</span>
        <span class="spacer"></span>
        <button v-if="cart.length" class="mini" @click="emit('clear')">清空清单</button>
      </div>
      <p v-if="!cart.length" class="muted small">
        还没有东西。在右侧详情里，“可以放心清理”和“需要你来判断”的项目下面有“加入清理清单”按钮；“建议保留”和“未知”的项目不能加入。
      </p>
      <ul v-else class="cart-list">
        <li v-for="c in cart" :key="c.path">
          <span class="chip" :class="'chip-' + c.tier">{{ TIER_SHORT[c.tier] }}</span>
          <span class="cart-name" :title="c.path">{{ c.name }}</span>
          <span class="cart-size">{{ humanBytes(c.allocated) }}</span>
          <button class="mini" :title="'从清单移出 ' + c.path" @click="emit('remove', c.path)">移出</button>
          <div class="cart-path muted small" :title="c.path">{{ c.path }}</div>
        </li>
      </ul>

      <fieldset class="strategy" :disabled="busy">
        <legend class="small">删到哪里</legend>
        <label class="radio-row">
          <input type="radio" value="recycle" v-model="mode" />
          <span><b>回收站</b>（默认）：可以在回收站里还原。回收站在同一个盘上，<b>清空回收站之后</b>才真正腾出空间。</span>
        </label>
        <label class="radio-row">
          <input type="radio" value="quarantine" v-model="mode" />
          <span>
            <b>隔离到其他盘</b>：挪到别的盘的 DiskWhy-Quarantine 文件夹，{{ scanRoot.slice(0, 2) }} 立刻腾出空间；保留 7 天（不会自动删），可以一键整批还原。
            <span v-if="!otherDrives.length" class="muted">（本机没有别的固定盘，只能选文件夹；同一个盘上不会腾出空间）</span>
          </span>
        </label>
        <div v-if="mode === 'quarantine'" class="dest-row">
          <label for="qdest" class="small">隔离到</label>
          <select id="qdest" v-model="dest">
            <option v-for="d in otherDrives" :key="d.root" :value="d.root">
              {{ d.label || "本地磁盘" }} ({{ d.root.replace("\\", "") }}) · 剩余 {{ humanBytes(d.free_bytes) }} / {{ humanBytes(d.total_bytes) }}
            </option>
            <option :value="CUSTOM">其他文件夹…</option>
          </select>
          <span v-if="otherDrives.find((d) => d.root === dest)?.free_bytes! < total" class="ai-error small">剩余空间可能不够</span>
        </div>
        <div v-if="mode === 'quarantine' && dest === CUSTOM" class="dest-row">
          <label for="qdest-custom" class="small">文件夹</label>
          <input id="qdest-custom" v-model="customDest" placeholder="例如 D:\DiskWhy" spellcheck="false" />
          <button class="btn" type="button" @click="browse">浏览…</button>
          <span v-if="pickErr" class="ai-error small">{{ pickErr }}</span>
        </div>
        <p v-if="mode === 'quarantine'" class="muted small">预览时会实际试一下能不能在那里建文件夹；不能写的位置会被拦下，不会执行。</p>
      </fieldset>

      <div class="drawer-actions">
        <button class="btn" @click="emit('toggle', false)">收起</button>
        <span class="spacer"></span>
        <button class="btn primary" :disabled="!cart.length || busy || !canPreview" @click="emit('preview', strategy())">
          {{ busy ? "正在检查…" : "预览要删的内容" }}
        </button>
      </div>
    </div>
  </div>
</template>
