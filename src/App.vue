<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import {
  cancelScan,
  getEnv,
  getTreemap,
  listDir,
  listDrives,
  nodeInfo,
  openSnapshot,
  relaunchAsAdmin,
  saveSnapshot,
  startScan,
  startupSnapshot,
  tmKey,
  type DirListing,
  type DriveInfo,
  type EnvInfo,
  type LlmSettings,
  type NodeView,
  type PseudoKey,
  type ScanProgress,
  type ScanSummary,
  type TmSelect,
  type TreemapNode,
  actionEnv,
  pickScanFolder,
  previewPlan,
  startupScanDir,
  type ActionEnv,
  type CartItem,
  type ExecOutcome,
  type PreflightReport,
  type Strategy,
} from "./api";
import { fmtDuration, fmtInt, fmtTime, humanBytes } from "./format";
import AttributionBar from "./components/AttributionBar.vue";
import ScanProgressPanel from "./components/ScanProgressPanel.vue";
import DirList from "./components/DirList.vue";
import DetailPanel from "./components/DetailPanel.vue";
import TierSummaryCard from "./components/TierSummaryCard.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import TreemapView from "./components/TreemapView.vue";
import { pseudoView } from "./pseudo";
import CleanupDrawer from "./components/CleanupDrawer.vue";
import PreviewDialog from "./components/PreviewDialog.vue";
import QuarantineDialog from "./components/QuarantineDialog.vue";

type Phase = "idle" | "scanning" | "done";
type ViewMode = "split" | "map" | "list";

const env = ref<EnvInfo | null>(null);
const drives = ref<DriveInfo[]>([]);
const selectedDrive = ref("");
const phase = ref<Phase>("idle");
const progress = ref<ScanProgress | null>(null);
const summary = ref<ScanSummary | null>(null);
const listing = ref<DirListing | null>(null);
const tm = ref<TreemapNode | null>(null);
const tmLoading = ref(false);
const selected = ref<NodeView | null>(null);
const selectedKey = ref<string | null>(null);
const message = ref("");
const notice = ref("");
const cancelling = ref(false);
const relaunching = ref(false);
const showSettings = ref(false);
const snapBusy = ref(false);
const llmReady = computed(() => env.value?.llm_ready ?? false);

// ───── 阶段 D：清理清单 / 预览 / 隔离区 ─────
const actEnv = ref<ActionEnv | null>(null);
const cart = ref<CartItem[]>([]);
const drawerOpen = ref(false);
const drawerRef = ref<InstanceType<typeof CleanupDrawer> | null>(null);
const previewBusy = ref(false);
const report = ref<PreflightReport | null>(null);
const showQuarantine = ref(false);
/** 用户选的文件夹（不是整个盘） */
const customRoot = ref("");
const cartPaths = computed(() => new Set(cart.value.map((c) => c.path.toLowerCase())));

async function refreshActionEnv() {
  try {
    actEnv.value = await actionEnv();
  } catch (e) {
    message.value = `读取隔离区信息失败：${e}`;
  }
}

function addToCart(n: NodeView) {
  const e = n.explanation;
  if (!e || cartPaths.value.has(n.path.toLowerCase())) return;
  cart.value.push({
    path: n.path,
    name: n.name,
    kind: n.kind,
    allocated: n.allocated,
    tier: e.tier,
    rule_id: e.source.kind === "rule" ? e.source.id : null,
    what: e.what,
    lose: e.lose,
  });
  drawerOpen.value = true;
}

function removeFromCart(path: string) {
  cart.value = cart.value.filter((c) => c.path.toLowerCase() !== path.toLowerCase());
}

async function doPreview(strategy: Strategy) {
  previewBusy.value = true;
  message.value = "";
  try {
    report.value = await previewPlan(
      cart.value.map((c) => ({ path: c.path, expect_rule: c.rule_id })),
      strategy,
    );
  } catch (e) {
    message.value = `预览失败：${e}`;
  } finally {
    previewBusy.value = false;
  }
}

async function onRequote(s: Strategy) {
  if (s.kind === "quarantine") drawerRef.value?.useQuarantine(s.dest_root);
  report.value = null;
  await doPreview(s);
}

async function onExecuted(o: ExecOutcome) {
  if (o.summary) summary.value = o.summary;
  const done = new Set(o.report.items.filter((i) => !i.blocked && i.failed === 0).map((i) => i.path.toLowerCase()));
  cart.value = cart.value.filter((c) => !done.has(c.path.toLowerCase()));
  if (listing.value) await open(listing.value.node.path);
  await refreshActionEnv();
}

async function onQuarantineChanged(s: ScanSummary | null) {
  if (s) summary.value = s;
  if (listing.value && s) await open(listing.value.node.path);
  await refreshActionEnv();
}

async function doPickFolder() {
  try {
    const p = await pickScanFolder();
    if (!p) return;
    customRoot.value = p;
    selectedDrive.value = p;
    await doScan();
  } catch (e) {
    message.value = `选择文件夹失败：${e}`;
  }
}

function loadViewMode(): ViewMode {
  try {
    const v = localStorage.getItem("diskwhy.view");
    if (v === "split" || v === "map" || v === "list") return v;
  } catch {
    /* 存储不可用时用默认 */
  }
  return "split";
}
const viewMode = ref<ViewMode>(loadViewMode());
const overviewCollapsed = ref(false);
watch(viewMode, (v) => {
  try {
    localStorage.setItem("diskwhy.view", v);
  } catch {
    /* 忽略 */
  }
});

function onSettingsSaved(s: LlmSettings) {
  if (env.value) env.value.llm_ready = s.enabled && !!s.base_url && !!s.model;
}

const detailNode = computed(() => selected.value ?? listing.value?.node ?? null);
const isScanRoot = computed(() => !!summary.value && !!listing.value && listing.value.node.path === summary.value.root);

onMounted(async () => {
  try {
    env.value = await getEnv();
    drives.value = await listDrives();
    const c = drives.value.find((d) => d.root.toUpperCase() === "C:\\");
    selectedDrive.value = (c ?? drives.value[0])?.root ?? "";
    // 用 “diskwhy.exe 某个快照.dwsnap” 启动（或双击关联的快照文件）时直接打开
    await refreshActionEnv();
    const snap = await startupSnapshot();
    if (snap) await doOpenSnapshot(snap);
    const dir = await startupScanDir();
    if (dir && !snap) {
      customRoot.value = dir;
      selectedDrive.value = dir;
      await doScan();
    }
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
  notice.value = "";
  phase.value = "scanning";
  progress.value = null;
  summary.value = null;
  listing.value = null;
  tm.value = null;
  selected.value = null;
  cancelling.value = false;
  cart.value = [];
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
    tmLoading.value = true;
    const [l, t] = await Promise.all([listDir(path), getTreemap(path, 3)]);
    listing.value = l;
    tm.value = t;
    selected.value = null;
    selectedKey.value = null;
  } catch (e) {
    message.value = String(e);
  } finally {
    tmLoading.value = false;
  }
}

function goUp() {
  const bc = listing.value?.breadcrumb;
  if (bc && bc.length > 1) open(bc[bc.length - 2].path);
}

function select(n: NodeView) {
  selected.value = n;
  selectedKey.value = tmKey(n);
}

async function onTmSelect(s: TmSelect) {
  selectedKey.value = s.key;
  if (s.pseudo) {
    selected.value = pseudoView(s.pseudo as PseudoKey, summary.value!);
    return;
  }
  if (!s.node) return; // “其他”块：只高亮
  const direct = listing.value?.children.find((c) => tmKey(c) === s.key);
  if (direct) {
    selected.value = direct;
    return;
  }
  try {
    selected.value = await nodeInfo(s.node.path, s.node.kind);
  } catch (e) {
    message.value = String(e);
  }
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

async function doSaveSnapshot() {
  snapBusy.value = true;
  try {
    const p = await saveSnapshot();
    if (p) notice.value = `快照已保存：${p}`;
  } catch (e) {
    message.value = `保存快照失败：${e}`;
  } finally {
    snapBusy.value = false;
  }
}

async function doOpenSnapshot(path?: string) {
  snapBusy.value = true;
  message.value = "";
  try {
    const s = await openSnapshot(path);
    if (!s) return;
    summary.value = s;
    phase.value = "done";
    const sn = s.snapshot;
    notice.value = sn
      ? `正在查看快照：${sn.path}（${sn.meta.kind === "snapshot" ? `保存于 ${fmtTime(sn.meta.saved_at)}` : "CLI --json 输出，目录树可能只有几层"}，读取用时 ${fmtDuration(sn.load_ms)}）。说明是按当前规则库现算的。`
      : "";
    await open(s.root);
  } catch (e) {
    message.value = `打开快照失败：${e}`;
  } finally {
    snapBusy.value = false;
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
          <option v-if="customRoot" :value="customRoot" :title="customRoot">文件夹：…\{{ customRoot.split("\\").slice(-2).join("\\") }}</option>
        </select>
        <button class="btn" :disabled="phase === 'scanning'" title="只扫描某个文件夹（而不是整个盘）" @click="doPickFolder">扫描文件夹…</button>
        <button class="btn primary" :disabled="!selectedDrive || phase === 'scanning'" @click="doScan">
          {{ phase === "done" ? "重新扫描" : "开始扫描" }}
        </button>
        <button class="btn" :disabled="phase !== 'done' || snapBusy" title="把这次扫描结果存成文件，之后可以再打开复核" @click="doSaveSnapshot">保存快照</button>
        <button class="btn" :disabled="phase === 'scanning' || snapBusy" title="打开之前保存的快照（.dwsnap）或 diskwhy-cli --json 输出" @click="doOpenSnapshot()">
          {{ snapBusy ? "请稍候…" : "打开快照" }}
        </button>
      </div>
      <div class="meta" v-if="summary" :title="env ? `规则库 ${env.rules_total} 条` : ''">
        {{ summary.snapshot ? "快照 · " : "" }}用时 {{ fmtDuration(summary.duration_ms) }} · {{ fmtInt(summary.files) }} 个文件 · {{ fmtInt(summary.dirs) }} 个文件夹
      </div>
      <div class="meta-right">
        <button class="btn" title="隔离到其他盘的文件：可以整批还原或彻底清空" @click="showQuarantine = true">
          隔离区{{ actEnv && actEnv.batches.length ? ` (${actEnv.batches.length})` : "" }}
        </button>
        <button
          class="btn"
          :title="env ? `规则库 ${env.rules_total} 条${env.rule_warnings.length ? '，警告：' + env.rule_warnings.join('；') : ''}` : ''"
          @click="showSettings = true"
        >
          设置
        </button>
      </div>
    </header>

    <div v-if="env && !env.elevated" class="admin-banner">
      <span>
        当前不是管理员身份：System Volume Information、WindowsApps、其他用户的文件夹等系统目录读不了，会记到“无权限”里。以管理员身份重新启动可看到系统目录。
      </span>
      <button class="btn warn" :disabled="relaunching || phase === 'scanning'" @click="doRelaunch">
        {{ relaunching ? "等待授权…" : "以管理员重新启动" }}
      </button>
    </div>

    <div v-if="actEnv && actEnv.expired_batches > 0" class="admin-banner">
      <span>隔离区里有 {{ actEnv.expired_batches }} 批文件已经放了超过 {{ actEnv.keep_days }} 天，确认用不到的话可以彻底清空了（DiskWhy 不会自动删）。</span>
      <button class="btn warn" @click="showQuarantine = true">打开隔离区</button>
    </div>
    <div v-if="message" class="msg" @click="message = ''">{{ message }}</div>
    <div v-if="notice" class="notice" :title="notice + '（点一下关闭）'" @click="notice = ''">{{ notice }}</div>

    <main class="content">
      <div v-if="phase === 'idle' && !summary" class="welcome card">
        <h1>看看 {{ selectedDrive || "磁盘" }} 的空间都去哪了</h1>
        <p>选好磁盘，点“开始扫描”。扫描只读取文件夹和文件的属性，不会改动、不会上传任何东西；只有你把东西加入清理清单、看过预览并确认之后才会删除（默认进回收站）。也可以“打开快照”复核以前的扫描结果。</p>
        <ul class="muted">
          <li>大小按“实际占用”算：按簇取整，压缩文件按压缩后算。</li>
          <li>链接和挂载点不会跟进去；硬链接、商店应用的目录映射只算一次；云端占位文件只看属性，绝不打开（不会触发下载）。</li>
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
        <div v-if="!overviewCollapsed" class="summary-row">
          <AttributionBar :summary="summary" />
          <TierSummaryCard :tiers="summary.tiers" :used-bytes="summary.used_bytes" />
        </div>
        <div class="split">
          <div class="left" :class="'mode-' + viewMode">
            <div class="view-switch" role="group" aria-label="视图">
              <button :aria-pressed="viewMode === 'split'" :class="{ on: viewMode === 'split' }" @click="viewMode = 'split'">方块图 + 列表</button>
              <button :aria-pressed="viewMode === 'map'" :class="{ on: viewMode === 'map' }" @click="viewMode = 'map'">只看方块图</button>
              <button :aria-pressed="viewMode === 'list'" :class="{ on: viewMode === 'list' }" @click="viewMode = 'list'">只看列表</button>
              <button class="collapse-btn" :title="overviewCollapsed ? '显示顶部的归属条和各档合计' : '收起顶部的归属条和各档合计，给方块图和列表腾地方'" @click="overviewCollapsed = !overviewCollapsed">
                {{ overviewCollapsed ? "展开概览 ▾" : "收起概览 ▴" }}
              </button>
            </div>
            <TreemapView
              v-if="viewMode !== 'list' && listing"
              :data="tm"
              :summary="summary"
              :is-scan-root="isScanRoot"
              :selected-key="selectedKey"
              :crumbs="listing.breadcrumb"
              :loading="tmLoading"
              @select="onTmSelect"
              @open="open"
              @up="goUp"
            />
            <DirList v-if="viewMode !== 'map' && listing" :listing="listing" :selected-path="selectedKey" @open="open" @select="select" />
          </div>
          <DetailPanel
            :node="detailNode"
            :llm-ready="llmReady"
            :in-cart="!!detailNode && cartPaths.has(detailNode.path.toLowerCase())"
            :can-act="phase === 'done'"
            @open-settings="showSettings = true"
            @open-path="open"
            @add-to-cart="addToCart"
            @remove-from-cart="removeFromCart"
          />
        </div>
      </template>
    </main>
    <SettingsDialog v-if="showSettings" @close="showSettings = false" @saved="onSettingsSaved" />
    <CleanupDrawer
      v-if="phase === 'done' && summary"
      ref="drawerRef"
      :cart="cart"
      :drives="actEnv?.drives ?? drives"
      :scan-root="summary.root"
      :busy="previewBusy"
      :open="drawerOpen"
      @toggle="(v: boolean) => (drawerOpen = v)"
      @remove="removeFromCart"
      @clear="cart = []"
      @preview="doPreview"
    />
    <PreviewDialog v-if="report" :key="report.plan_id" :report="report" @close="report = null" @requote="onRequote" @executed="onExecuted" />
    <QuarantineDialog v-if="showQuarantine" @close="showQuarantine = false" @changed="onQuarantineChanged" />
  </div>
</template>
