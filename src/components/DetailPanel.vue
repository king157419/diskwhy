<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ArrowRight,
  Bot,
  Check,
  ChevronDown,
  CircleHelp,
  Cloud,
  Copy,
  File,
  Folder,
  HardDrive,
  Info,
  Layers,
  Link2,
  Lock,
  Package,
  Plus,
  Settings2,
  ShieldAlert,
  TriangleAlert,
  Wrench,
} from "lucide-vue-next";
import { askAi, cartEligibility, getGuidance, openSystemSettings, tierKey, type Explanation, type Guidance, type NodeView } from "../api";
import { HOW_LABEL, TIER_BIG, TIER_HINT, attrText, fmtInt, fmtTime, humanBytes, sourceLabel } from "../format";
import EmptyState from "./ui/EmptyState.vue";

const props = defineProps<{ node: NodeView | null; llmReady: boolean; inCart: boolean; canAct: boolean }>();
const emit = defineEmits<{ openSettings: []; openPath: [path: string]; addToCart: [node: NodeView]; removeFromCart: [path: string] }>();

// ───── 阶段 D：动作区 ─────
const elig = computed(() => cartEligibility(props.node));
const guidance = ref<Guidance[]>([]);
const guidanceError = ref("");
const settingsError = ref("");
watch(
  () => (props.node ? props.node.path + "|" + props.node.kind + "|" + (props.node.explanation?.tier ?? "") : ""),
  async () => {
    guidance.value = [];
    guidanceError.value = "";
    settingsError.value = "";
    const n = props.node;
    if (!n || n.pseudo || n.kind === "Rest" || !n.explanation || n.explanation.source.kind === "unknown") return;
    try {
      guidance.value = await getGuidance(n.path, n.kind);
    } catch (e) {
      guidanceError.value = String(e);
    }
  },
  { immediate: true },
);
async function openSettingsPage(uri: string) {
  settingsError.value = "";
  try {
    await openSystemSettings(uri);
  } catch (e) {
    settingsError.value = String(e);
  }
}

const aiResult = ref<Explanation | null>(null);
const aiLoading = ref(false);
const aiError = ref("");
const copied = ref("");
const linkError = ref("");
const showProps = ref(false);

watch(
  () => (props.node ? props.node.path + "|" + props.node.kind : ""),
  () => {
    aiResult.value = null;
    aiError.value = "";
    aiLoading.value = false;
  },
);

const ex = computed(() => props.node?.explanation ?? null);
const tk = computed(() => tierKey(ex.value));
const canAsk = computed(() => {
  const e = ex.value;
  if (!e || !props.node || props.node.kind === "Rest" || props.node.pseudo) return false;
  return e.source.kind === "unknown" || e.source.kind === "heuristic";
});

const nodeIcon = computed(() => {
  const n = props.node;
  if (!n) return Folder;
  if (n.pseudo) return ShieldAlert;
  if (n.kind === "Rest") return Layers;
  if (n.reparse_skipped) return Link2;
  if (n.inaccessible) return Lock;
  if (n.cloud_placeholder) return Cloud;
  return n.kind === "Dir" ? Folder : File;
});

const status = computed(() => {
  const n = props.node;
  if (!n) return [] as string[];
  const s: string[] = [];
  if (n.pseudo) return s;
  if (n.inaccessible && n.child_count > 0)
    s.push(`列不出目录（Win32 错误 ${n.error_code}），但从注册表里知道其中 ${n.child_count} 个子目录，已经量上；其余部分算在“无权限”里`);
  else if (n.inaccessible) s.push(`无法读取（Win32 错误 ${n.error_code}${n.error_code === 5 ? "：拒绝访问" : ""}）`);
  if (n.reparse_skipped) s.push("这是链接/挂载点，指向别处，没有跟进去统计，避免重复计算");
  if (n.cloud_placeholder) s.push("云端占位文件：内容在云上，本地几乎不占空间；DiskWhy 只读属性，没有打开它");
  if (n.system_reserved) s.push("属于系统保留项（回收站 / 卷影 / 页面文件等）");
  if (n.kind === "Rest") s.push(`为了省内存，这个文件夹里除最大的若干个文件外，其余 ${fmtInt(n.files)} 个文件合并显示`);
  return s;
});

function howText(e: Explanation): string {
  return e.how.map((h) => HOW_LABEL[h]).join("，或");
}

async function doAsk() {
  if (!props.node) return;
  if (!props.llmReady) {
    emit("openSettings");
    return;
  }
  aiLoading.value = true;
  aiError.value = "";
  try {
    aiResult.value = await askAi(props.node.path);
  } catch (e) {
    aiError.value = String(e);
  } finally {
    aiLoading.value = false;
  }
}

async function openLink(url: string) {
  linkError.value = "";
  try {
    await openUrl(url);
  } catch (e) {
    linkError.value = `打不开链接（${e}），可以点“复制”后粘到浏览器里`;
  }
}

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    copied.value = text;
    setTimeout(() => (copied.value = ""), 1500);
  } catch {
    copied.value = "";
  }
}
</script>

<template>
  <aside class="detail card" aria-label="详情">
    <Transition name="swap" mode="out-in">
      <div v-if="node" :key="node.path + '|' + node.kind + '|' + (node.pseudo ?? '')" class="d-scroll">
        <!-- 头部 -->
        <header class="d-head">
          <span class="d-ico" :class="'ico-' + tk"><component :is="nodeIcon" :size="20" :stroke-width="1.7" /></span>
          <div class="d-title">
            <h2 class="d-name" :title="node.name">{{ node.name }}</h2>
            <div class="d-type">{{ node.type_label }}</div>
          </div>
        </header>
        <div class="d-size-row">
          <span class="d-size num">{{ humanBytes(node.allocated) }}</span>
          <span v-if="ex" class="chip lg" :class="'chip-' + tk">{{ TIER_BIG[tk] }}</span>
          <span v-if="ex?.source.kind === 'heuristic'" class="tag">推测</span>
        </div>
        <p v-if="ex" class="d-hint">{{ TIER_HINT[tk] }}</p>

        <div v-if="node.mapped" class="callout info">
          <Link2 :size="16" />
          <div>
            <b>这是商店应用的目录映射</b>：和「{{ node.mapped_to ?? "另一个路径" }}」是同一份数据（两边目录的文件 ID 相同），空间已经记在那边，这里按 0 算，避免重复。
            <div v-if="node.mapped_to" class="mt8">
              <button class="btn sm" @click="emit('openPath', node.mapped_to)">跳到真正记账的位置 <ArrowRight :size="14" /></button>
            </div>
          </div>
        </div>
        <div v-if="node.hardlink_dup" class="callout info">
          <Link2 :size="16" />
          <div><b>这是硬链接</b>：这个文件还有别的名字（同一个文件 ID），空间已经记在另一个路径上，这里按 0 算。删掉这个名字腾不出空间。</div>
        </div>

        <!-- 六项说明 -->
        <section v-if="ex" class="six">
          <div class="six-row lead">
            <span class="six-ico"><Info :size="15" /></span>
            <div class="six-body">
              <div class="six-k">这是什么</div>
              <div class="six-v what">{{ ex.what }}</div>
            </div>
          </div>
          <div class="six-row">
            <span class="six-ico"><Package :size="15" /></span>
            <div class="six-body">
              <div class="six-k">属于</div>
              <div class="six-v">{{ ex.owner ?? "不确定" }}</div>
            </div>
          </div>
          <div class="six-row">
            <span class="six-ico"><CircleHelp :size="15" /></span>
            <div class="six-body">
              <div class="six-k">为什么会有</div>
              <div class="six-v">{{ ex.why }}</div>
            </div>
          </div>
          <div class="six-row" :class="{ cost: tk === 'review', costkeep: tk === 'keep' }">
            <span class="six-ico"><TriangleAlert :size="15" /></span>
            <div class="six-body">
              <div class="six-k">删了会失去什么</div>
              <div class="six-v">{{ ex.lose }}</div>
            </div>
          </div>
          <div class="six-row">
            <span class="six-ico"><HardDrive :size="15" /></span>
            <div class="six-body">
              <div class="six-k">能腾出</div>
              <div class="six-v">{{ ex.gain_note }}</div>
            </div>
          </div>
          <div class="six-row">
            <span class="six-ico"><Wrench :size="15" /></span>
            <div class="six-body">
              <div class="six-k">该怎么腾</div>
              <div class="six-v">
                <div>{{ howText(ex) }}</div>
                <div v-if="ex.how_detail" class="how-detail">{{ ex.how_detail }}</div>
              </div>
            </div>
          </div>
        </section>

        <!-- 动作区：safe/review 且可以直接删 → 加入清理清单；keep → 该怎么腾；未知 → 先搞清楚 -->
        <section v-if="ex && !node.pseudo && node.kind !== 'Rest'" class="act" :class="'act-' + (elig.ok ? tk : elig.reason || 'none')">
          <template v-if="elig.ok">
            <button v-if="!inCart" class="btn primary lg block" :disabled="!canAct" @click="emit('addToCart', node)">
              <Plus :size="17" :stroke-width="2.2" /> 加入清理清单 <span class="btn-meta num">{{ humanBytes(node.allocated) }}</span>
            </button>
            <button v-else class="btn lg block in-cart" title="点此移出清理清单" @click="emit('removeFromCart', node.path)">
              <Check :size="17" :stroke-width="2.2" /> 已在清理清单 · 点此移出
            </button>
            <p class="act-note">
              加入清单不会马上删除。在“清理清单”里预览时，会逐条列出真正要删的文件、跳过的文件和原因，确认后才执行；默认进回收站。
            </p>
            <div v-if="tk === 'review'" class="callout warn">
              <TriangleAlert :size="15" />
              <span>这一项删了有代价：{{ ex.lose }}</span>
            </div>
          </template>
          <template v-else-if="elig.reason === 'unknown'">
            <div class="act-title"><CircleHelp :size="16" /> 先搞清楚它是什么再说</div>
            <p class="act-note">规则库和启发式都没认出来，DiskWhy 不会让你删它。下面列了看到的线索；也可以问问 AI——但 AI 的回答只是推测，同样不能作为删除依据。</p>
          </template>
          <template v-else-if="elig.reason === 'keep' || elig.reason === 'tool'">
            <div class="act-title">
              <ShieldAlert v-if="elig.reason === 'keep'" :size="16" />
              <Wrench v-else :size="16" />
              {{ elig.reason === "keep" ? "这个不能删，想腾空间这样做" : "这一项不建议直接删，这样清理" }}
            </div>
            <p v-if="!guidance.length && ex.how_detail" class="act-note">{{ ex.how_detail }}</p>
          </template>
          <p v-else-if="elig.reason" class="act-note">{{ elig.reason }}</p>

          <div v-if="guidance.length" class="guides">
            <div v-for="g in guidance" :key="g.title" class="guide">
              <div class="guide-title">
                {{ g.title }}
                <span v-if="g.needs_admin" class="tag warn">需要管理员</span>
              </div>
              <div v-if="g.command" class="guide-cmd">
                <code>{{ g.command }}</code>
                <button class="mini" @click="copy(g.command!)">
                  <Check v-if="copied === g.command" :size="12" /><Copy v-else :size="12" />{{ copied === g.command ? "已复制" : "复制" }}
                </button>
              </div>
              <div v-if="g.shell" class="muted small">在哪运行：{{ g.shell }}（DiskWhy 不会替你运行）</div>
              <p class="guide-text">{{ g.explain }}</p>
              <button v-if="g.settings_uri" class="btn sm" @click="openSettingsPage(g.settings_uri!)"><Settings2 :size="14" /> 打开系统设置</button>
            </div>
            <p v-if="settingsError" class="err-text small">{{ settingsError }}</p>
          </div>
          <p v-if="guidanceError" class="err-text small">{{ guidanceError }}</p>
        </section>

        <!-- 备注 / 线索 / 来源 -->
        <section v-if="ex" class="meta-block">
          <ul v-if="ex.notes.length" class="notes">
            <li v-for="n in ex.notes" :key="n">{{ n }}</li>
          </ul>
          <div v-if="ex.clues.length" class="clues">
            <div class="clue-title">{{ ex.source.kind === "unknown" ? "我也不确定，这是我看到的线索：" : "依据：" }}</div>
            <ul>
              <li v-for="c in ex.clues" :key="c">{{ c }}</li>
            </ul>
          </div>
          <div class="source">
            <span class="faint">来源</span>
            <span>{{ sourceLabel(ex) }}<span v-if="ex.verified_on" class="faint"> · 核对于 {{ ex.verified_on }}</span></span>
          </div>
          <div v-if="ex.refs.length" class="refs">
            <span class="faint">参考</span>
            <div class="ref-list">
              <div v-for="r in ex.refs" :key="r" class="ref">
                <a class="ref-url" :href="r" :title="'用默认浏览器打开 ' + r" @click.prevent="openLink(r)">{{ r }}</a>
                <button class="mini" @click="copy(r)">{{ copied === r ? "已复制" : "复制" }}</button>
              </div>
              <p v-if="linkError" class="err-text small">{{ linkError }}</p>
            </div>
          </div>

          <div v-if="canAsk" class="ask">
            <button class="btn" :class="{ primary: llmReady }" :disabled="aiLoading" @click="doAsk">
              <Bot :size="16" />
              {{ aiLoading ? "AI 正在推测…" : llmReady ? "问 AI 这是什么" : "问 AI（先去设置里启用）" }}
            </button>
            <p class="faint small">只发送名字、位置（用户名换成 ~）、前 10 大子项名、文件类型分布、大小和修改日期，不发送任何文件内容。</p>
            <p v-if="aiError" class="err-text small">AI 这次没答上来：{{ aiError }}（不影响其他功能，可以稍后再试或检查设置）</p>
          </div>
        </section>

        <section v-if="aiResult" class="ai-block">
          <div class="ai-head">
            <span class="chip chip-review">AI 推测 · 需要你来判断</span>
            <span class="faint small">可信度 {{ Math.round(aiResult.confidence * 100) }}%</span>
          </div>
          <dl class="ai-grid">
            <dt>这是什么</dt>
            <dd><b>{{ aiResult.what }}</b></dd>
            <dt>属于</dt>
            <dd>{{ aiResult.owner ?? "AI 也说不准" }}</dd>
            <dt>为什么会有</dt>
            <dd>{{ aiResult.why }}</dd>
            <dt>删了会失去什么</dt>
            <dd>{{ aiResult.lose }}</dd>
            <dt>该怎么腾</dt>
            <dd>{{ aiResult.how_detail }}</dd>
          </dl>
          <ul class="notes">
            <li v-for="n in aiResult.notes" :key="n">{{ n }}</li>
          </ul>
          <div class="source"><span class="faint">来源</span><span>{{ sourceLabel(aiResult) }}</span></div>
        </section>

        <!-- 文件属性 -->
        <section v-if="!node.pseudo" class="props">
          <button class="props-toggle" :aria-expanded="showProps" @click="showProps = !showProps">
            文件属性
            <ChevronDown :size="15" class="chev" :class="{ open: showProps }" />
          </button>
          <div class="props-path mono" :title="node.path">{{ node.path }}</div>
          <dl v-if="showProps" class="props-grid">
            <dt>实际占用</dt>
            <dd><b class="num">{{ humanBytes(node.allocated) }}</b> <span class="faint num">（{{ fmtInt(node.allocated) }} 字节）</span></dd>
            <dt>逻辑大小</dt>
            <dd class="num">{{ humanBytes(node.logical) }}</dd>
            <dt>修改时间</dt>
            <dd class="num">{{ node.kind === "Rest" ? "（最新）" : "" }}{{ fmtTime(node.mtime) }}</dd>
            <template v-if="node.kind === 'Dir'">
              <dt>子项数</dt>
              <dd><span class="num">{{ fmtInt(node.child_count) }}</span> 项（内含 <span class="num">{{ fmtInt(node.files) }}</span> 个文件、<span class="num">{{ fmtInt(node.dirs) }}</span> 个文件夹）</dd>
            </template>
            <template v-if="node.kind !== 'Rest'">
              <dt>属性</dt>
              <dd>{{ attrText(node.attrs) }}</dd>
            </template>
          </dl>
        </section>
        <ul v-if="status.length" class="status">
          <li v-for="s in status" :key="s">{{ s }}</li>
        </ul>
      </div>
      <div v-else key="empty" class="d-empty">
        <EmptyState kind="select" title="点一项看它是什么" text="在左侧列表或方块图里点一下，这里会写清楚：这是什么、属于谁、为什么会有、删了会失去什么、能腾出多少、该怎么腾。" compact />
      </div>
    </Transition>
  </aside>
</template>

<style scoped>
.detail {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}
.d-scroll {
  overflow-y: auto;
  padding: 22px 22px 26px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-height: 0;
  flex: 1;
}
/* 内容多时整块滚动，不要把某一节压扁 */
.d-scroll > * {
  flex-shrink: 0;
}
.d-empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.swap-enter-active,
.swap-leave-active {
  transition:
    opacity var(--dur-1) var(--ease),
    transform var(--dur-1) var(--ease);
}
.swap-enter-from {
  opacity: 0;
  transform: translateY(3px);
}
.swap-leave-to {
  opacity: 0;
}
.d-head {
  display: flex;
  gap: 12px;
  align-items: center;
}
.d-ico {
  width: 42px;
  height: 42px;
  border-radius: 12px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--surface-2);
  color: var(--text-2);
}
.ico-safe {
  background: var(--t-safe-bg);
  color: var(--t-safe-fg);
}
.ico-review {
  background: var(--t-review-bg);
  color: var(--t-review-fg);
}
.ico-keep {
  background: var(--t-keep-bg);
  color: var(--t-keep-fg);
}
.ico-unknown {
  background: var(--t-unknown-bg);
  color: var(--t-unknown-fg);
}
.d-title {
  min-width: 0;
}
.d-name {
  font-size: var(--fs-lg);
  font-weight: 650;
  letter-spacing: -0.01em;
  line-height: 1.3;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.d-type {
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.d-size-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.d-size {
  font-size: var(--fs-2xl);
  font-weight: 600;
  letter-spacing: -0.03em;
  line-height: 1.1;
  margin-right: 2px;
}
.d-hint {
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.6;
  margin-top: -4px;
}
.mt8 {
  margin-top: 8px;
}
.six {
  display: flex;
  flex-direction: column;
  border-radius: var(--r-md);
  border: 1px solid var(--border-1);
  background: var(--surface-inset);
  overflow: hidden;
}
.six-row {
  display: flex;
  gap: 11px;
  padding: 11px 14px;
  border-top: 1px solid var(--border-1);
}
.six-row:first-child {
  border-top: 0;
}
.six-ico {
  width: 26px;
  height: 26px;
  border-radius: 8px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--surface-2);
  color: var(--text-2);
}
.six-row.lead .six-ico {
  background: var(--accent-soft);
  color: var(--accent);
}
.six-row.cost .six-ico {
  background: var(--t-review-bg);
  color: var(--t-review-fg);
}
.six-row.costkeep .six-ico {
  background: var(--t-keep-bg);
  color: var(--t-keep-fg);
}
.six-body {
  min-width: 0;
  flex: 1;
}
.six-k {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  font-weight: 550;
  letter-spacing: 0.02em;
  margin-bottom: 2px;
}
.six-v {
  font-size: var(--fs-sm);
  line-height: 1.6;
  word-break: break-word;
}
.six-v.what {
  font-weight: 600;
  font-size: var(--fs-md);
}
.how-detail {
  margin-top: 4px;
  color: var(--text-2);
}
.act {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.btn.block {
  width: 100%;
}
.btn-meta {
  margin-left: auto;
  opacity: 0.8;
  font-size: var(--fs-xs);
}
.btn.in-cart {
  color: var(--ok);
  border-color: var(--ok-border);
  background: var(--ok-bg);
}
.act-note {
  font-size: var(--fs-xs);
  color: var(--text-3);
  line-height: 1.6;
}
.act-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  font-size: var(--fs-sm);
}
.act-keep .act-title svg {
  color: var(--t-keep-fg);
}
.act-tool .act-title svg,
.act-unknown .act-title svg {
  color: var(--accent);
}
.guides {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.guide {
  padding: 12px 14px;
  border-radius: var(--r-md);
  background: var(--surface-2);
  border: 1px solid var(--border-1);
  display: flex;
  flex-direction: column;
  gap: 7px;
  align-items: flex-start;
}
.guide-title {
  font-weight: 600;
  font-size: var(--fs-sm);
  display: flex;
  gap: 8px;
  align-items: center;
}
.guide-cmd {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  width: 100%;
}
.guide-cmd code {
  flex: 1;
  padding: 6px 8px;
  font-size: var(--fs-2xs);
}
.guide-text {
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.6;
}
.meta-block {
  display: flex;
  flex-direction: column;
  gap: 10px;
  font-size: var(--fs-xs);
}
.notes {
  margin: 0;
  padding-left: 18px;
  color: var(--text-2);
  line-height: 1.6;
}
.clues {
  padding: 10px 12px;
  border-radius: var(--r-md);
  background: var(--surface-2);
}
.clue-title {
  font-weight: 600;
  margin-bottom: 4px;
}
.clues ul {
  margin: 0;
  padding-left: 18px;
  color: var(--text-2);
  word-break: break-all;
}
.source,
.refs {
  display: grid;
  grid-template-columns: 34px 1fr;
  gap: 8px;
  color: var(--text-2);
  line-height: 1.55;
}
.ref-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}
.ref {
  display: flex;
  gap: 6px;
  align-items: center;
  min-width: 0;
}
.ref-url {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
  flex: 1;
}
.ask {
  display: flex;
  flex-direction: column;
  gap: 6px;
  align-items: flex-start;
  padding-top: 4px;
}
.ai-block {
  padding: 14px;
  border-radius: var(--r-md);
  border: 1px dashed var(--warn-border);
  background: var(--warn-bg);
  display: flex;
  flex-direction: column;
  gap: 10px;
  font-size: var(--fs-sm);
}
.ai-head {
  display: flex;
  gap: 10px;
  align-items: center;
}
.ai-grid {
  display: grid;
  grid-template-columns: 96px 1fr;
  gap: 6px 10px;
  margin: 0;
}
.ai-grid dt {
  color: var(--text-3);
  font-size: var(--fs-xs);
}
.ai-grid dd {
  margin: 0;
  line-height: 1.55;
}
.props {
  border-top: 1px solid var(--border-1);
  padding-top: 12px;
}
.props-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 0;
  border: 0;
  background: none;
  color: var(--text-2);
  font-size: var(--fs-xs);
  font-weight: 600;
  cursor: pointer;
}
.props-toggle:hover {
  color: var(--text-1);
}
.chev {
  transition: transform var(--dur-2) var(--ease);
}
.chev.open {
  transform: rotate(180deg);
}
.props-path {
  margin-top: 6px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  word-break: break-all;
}
.props-grid {
  display: grid;
  grid-template-columns: 66px 1fr;
  gap: 6px 10px;
  margin: 10px 0 0;
  font-size: var(--fs-xs);
}
.props-grid dt {
  color: var(--text-3);
}
.props-grid dd {
  margin: 0;
}
.status {
  margin: 0;
  padding: 10px 12px 10px 28px;
  border-radius: var(--r-md);
  background: var(--surface-2);
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.6;
}
</style>
