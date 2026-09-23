<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  cancelScan,
  getEnv,
  listDir,
  listDrives,
  relaunchAsAdmin,
  startScan,
  type DirListing,
  type DriveInfo,
  type EnvInfo,
  type LlmSettings,
  type NodeView,
  type ScanProgress,
  type ScanSummary,
} from "./api";
import { fmtDuration, fmtInt, humanBytes } from "./format";
import AttributionBar from "./components/AttributionBar.vue";
import ScanProgressPanel from "./components/ScanProgressPanel.vue";
import DirList from "./components/DirList.vue";
import DetailPanel from "./components/DetailPanel.vue";
import TierSummaryCard from "./components/TierSummaryCard.vue";
import SettingsDialog from "./components/SettingsDialog.vue";

type Phase = "idle" | "scanning" | "done";

const env = ref<EnvInfo | null>(null);
const drives = ref<DriveInfo[]>([]);
const selectedDrive = ref("");
const phase = ref<Phase>("idle");
const progress = ref<ScanProgress | null>(null);
const summary = ref<ScanSummary | null>(null);
const listing = ref<DirListing | null>(null);
const selected = ref<NodeView | null>(null);
const selectedKey = ref<string | null>(null);
const message = ref("");
const cancelling = ref(false);
const relaunching = ref(false);
const showSettings = ref(false);
const llmReady = computed(() => env.value?.llm_ready ?? false);

function onSettingsSaved(s: LlmSettings) {
  if (env.value) env.value.llm_ready = s.enabled && !!s.base_url && !!s.model;
}

const detailNode = computed(() => selected.value ?? listing.value?.node ?? null);

onMounted(async () => {
  try {
    env.value = await getEnv();
    drives.value = await listDrives();
    const c = drives.value.find((d) => d.root.toUpperCase() === "C:\\");
    selectedDrive.value = (c ?? drives.value[0])?.root ?? "";
  } catch (e) {
    message.value = `初始化失败：${e}`;
  }
});

function driveLabel(d: DriveInfo): string {
  const name = d.label ? `${d.label} ` : "本地磁盘 ";
  const used = d.total_bytes - d.free_bytes;
  return `${name}(${d.root.replace("\\", "")})  已用 ${humanBytes(used)} / ${humanBytes(d.total_bytes)}`;
}

async function doScan() {
  if (!selectedDrive.value || phase.value === "scanning") return;
  message.value = "";
  phase.value = "scanning";
  progress.value = null;
  summary.value = null;
  listing.value = null;
  selected.value = null;
  cancelling.value = false;
  try {
    const s = await startScan(selectedDrive.value, (p) => (progress.value = p));
    summary.value = s;
    phase.value = "done";
    await open(s.root);
  } catch (e) {
    phase.value = "idle";
    message.value = String(e);
  } finally {
    cancelling.value = false;
  }
}

async function doCancel() {
  cancelling.value = true;
  await cancelScan();
}

async function open(path: string) {
  try {
    listing.value = await listDir(path);
    selected.value = null;
    selectedKey.value = null;
  } catch (e) {
    message.value = String(e);
  }
}

function select(n: NodeView) {
  selected.value = n;
  selectedKey.value = n.kind === "Rest" ? n.path + "|rest" : n.path;
}

async function doRelaunch() {
  relaunching.value = true;
  try {
    await relaunchAsAdmin();
  } catch (e) {
    message.value = `没有以管理员身份重启：${e}`;
  } finally {
    relaunching.value = false;
  }
}
</script>

<template>
  <div class="app">
    <header class="topbar">
      <div class="brand">
        <span class="logo">D</span>
        <div>
          <div class="brand-name">DiskWhy</div>
          <div class="brand-sub">每一块空间都说清楚</div>
        </div>
      </div>
      <div class="controls">
        <label class="sr-only" for="drive">选择磁盘</label>
        <select id="drive" v-model="selectedDrive" :disabled="phase === 'scanning'">
          <option v-for="d in drives" :key="d.root" :value="d.root">{{ driveLabel(d) }}</option>
        </select>
        <button class="btn primary" :disabled="!selectedDrive || phase === 'scanning'" @click="doScan">
          {{ phase === "done" ? "重新扫描" : "开始扫描" }}
        </button>
      </div>
      <div class="meta" v-if="summary">
        用时 {{ fmtDuration(summary.duration_ms) }} · {{ fmtInt(summary.files) }} 个文件 · {{ fmtInt(summary.dirs) }} 个文件夹
      </div>
      <div class="meta-right">
        <span v-if="env" class="muted small" :title="env.rule_warnings.join('；')">
          规则库 {{ env.rules_total }} 条{{ env.rule_warnings.length ? `（${env.rule_warnings.length} 条警告）` : "" }}
        </span>
        <button class="btn" @click="showSettings = true">设置</button>
      </div>
    </header>

    <div v-if="env && !env.elevated" class="admin-banner">
      <span>
        当前不是管理员身份：System Volume Information、其他用户的文件夹等系统目录读不了，会记到“无权限”里。以管理员身份重新启动可看到系统目录。
      </span>
      <button class="btn warn" :disabled="relaunching || phase === 'scanning'" @click="doRelaunch">
        {{ relaunching ? "等待授权…" : "以管理员重新启动" }}
      </button>
    </div>

    <div v-if="message" class="msg" @click="message = ''">{{ message }}</div>

    <main class="content">
      <div v-if="phase === 'idle' && !summary" class="welcome card">
        <h1>看看 {{ selectedDrive || "磁盘" }} 的空间都去哪了</h1>
        <p>选好磁盘，点“开始扫描”。DiskWhy 只读取文件夹和文件的属性，不会改动、不会上传任何东西。</p>
        <ul class="muted">
          <li>大小按“实际占用”算：按簇取整，压缩文件按压缩后算。</li>
          <li>链接和挂载点不会跟进去，避免重复计算；云端占位文件只看属性，绝不打开（不会触发下载）。</li>
          <li>读不到的地方会明确标成“无权限 / 系统保留 / 未归属”，不会悄悄少算。</li>
        </ul>
      </div>

      <ScanProgressPanel
        v-if="phase === 'scanning'"
        :progress="progress"
        :root="selectedDrive"
        :cancelling="cancelling"
        @cancel="doCancel"
      />

      <template v-if="phase === 'done' && summary">
        <div class="summary-row">
          <AttributionBar :summary="summary" />
          <TierSummaryCard :tiers="summary.tiers" :used-bytes="summary.used_bytes" />
        </div>
        <div class="split">
          <DirList v-if="listing" :listing="listing" :selected-path="selectedKey" @open="open" @select="select" />
          <DetailPanel :node="detailNode" :llm-ready="llmReady" @open-settings="showSettings = true" />
        </div>
      </template>
    </main>
    <SettingsDialog v-if="showSettings" @close="showSettings = false" @saved="onSettingsSaved" />
  </div>
</template>
