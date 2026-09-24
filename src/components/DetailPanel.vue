<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { askAi, cartEligibility, getGuidance, openSystemSettings, tierKey, type Explanation, type Guidance, type NodeView } from "../api";
import { HOW_LABEL, TIER_BIG, TIER_HINT, attrText, fmtInt, fmtTime, humanBytes, sourceLabel } from "../format";

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
  <aside class="detail card">
    <template v-if="node">
      <h2 class="d-name" :title="node.name">{{ node.name }}</h2>
      <div class="d-type">{{ node.type_label }} · {{ humanBytes(node.allocated) }}</div>
      <div v-if="node.mapped" class="dup-callout">
        <b>这是商店应用的目录映射</b>：和「{{ node.mapped_to ?? "另一个路径" }}」是同一份数据（两边目录的文件 ID 相同），
        空间已经记在那边，这里按 0 算，避免重复。
        <div v-if="node.mapped_to"><button class="btn" @click="emit('openPath', node.mapped_to)">跳到真正记账的位置</button></div>
      </div>
      <div v-if="node.hardlink_dup" class="dup-callout">
        <b>这是硬链接</b>：这个文件还有别的名字（同一个文件 ID），空间已经记在另一个路径上，这里按 0 算。删掉这个名字腾不出空间。
      </div>

      <section v-if="ex" class="explain" :class="'ex-' + tk">
        <div class="ex-head">
          <span class="ex-badge" :class="'chip-' + tk">{{ TIER_BIG[tk] }}</span>
          <span v-if="ex.source.kind === 'heuristic'" class="tag">推测</span>
        </div>
        <p class="ex-hint muted small">{{ TIER_HINT[tk] }}</p>
        <dl class="ex-grid">
          <dt>这是什么</dt>
          <dd><b>{{ ex.what }}</b></dd>
          <dt>属于</dt>
          <dd>{{ ex.owner ?? "不确定" }}</dd>
          <dt>为什么会有</dt>
          <dd>{{ ex.why }}</dd>
          <dt>删了会失去什么</dt>
          <dd>{{ ex.lose }}</dd>
          <dt>能腾出</dt>
          <dd>{{ ex.gain_note }}</dd>
          <dt>该怎么腾</dt>
          <dd>
            <div>{{ howText(ex) }}</div>
            <div v-if="ex.how_detail" class="how-detail">{{ ex.how_detail }}</div>
          </dd>
        </dl>
        <ul v-if="ex.notes.length" class="ex-notes small">
          <li v-for="n in ex.notes" :key="n">{{ n }}</li>
        </ul>
        <div v-if="ex.clues.length" class="ex-clues">
          <div class="ex-sub">{{ ex.source.kind === "unknown" ? "我也不确定，这是我看到的线索：" : "依据：" }}</div>
          <ul>
            <li v-for="c in ex.clues" :key="c">{{ c }}</li>
          </ul>
        </div>
        <div class="ex-source small">
          <span class="muted">来源：</span>{{ sourceLabel(ex) }}
          <span v-if="ex.verified_on" class="muted"> · 核对于 {{ ex.verified_on }}</span>
        </div>
        <div v-if="ex.refs.length" class="ex-refs small">
          <span class="muted">参考：</span>
          <div v-for="r in ex.refs" :key="r" class="ref">
            <a class="ref-url ref-link" :href="r" :title="'用默认浏览器打开 ' + r" @click.prevent="openLink(r)">{{ r }}</a>
            <button class="mini" @click="copy(r)">{{ copied === r ? "已复制" : "复制" }}</button>
          </div>
          <p v-if="linkError" class="ai-error small">{{ linkError }}</p>
        </div>

        <div v-if="canAsk" class="ask">
          <button class="btn" :class="{ primary: llmReady }" :disabled="aiLoading" @click="doAsk">
            {{ aiLoading ? "AI 正在推测…" : llmReady ? "问 AI 这是什么" : "问 AI（先去设置里启用）" }}
          </button>
          <p class="muted small">
            只发送名字、位置（用户名换成 ~）、前 10 大子项名、文件类型分布、大小和修改日期，不发送任何文件内容。
          </p>
          <p v-if="aiError" class="ai-error small">AI 这次没答上来：{{ aiError }}（不影响其他功能，可以稍后再试或检查设置）</p>
        </div>
      </section>

      <!-- 阶段 D：动作区。safe/review 且可以直接删 → 加入清理清单；keep → 该怎么腾；未知 → 先搞清楚 -->
      <section v-if="ex && !node.pseudo && node.kind !== 'Rest'" class="act-box" :class="'act-' + (elig.ok ? tk : elig.reason || 'none')">
        <template v-if="elig.ok">
          <div class="act-row">
            <button v-if="!inCart" class="btn primary" :disabled="!canAct" @click="emit('addToCart', node)">＋ 加入清理清单</button>
            <button v-else class="btn" @click="emit('removeFromCart', node.path)">✓ 已在清理清单（点此移出）</button>
            <span class="muted small">{{ humanBytes(node.allocated) }}</span>
          </div>
          <p class="muted small">
            加入清单不会马上删除。点右下角“清理清单 → 预览”后，会逐条列出真正要删的文件、跳过的文件和原因，确认后才执行；默认进回收站。
          </p>
          <p v-if="tk === 'review'" class="act-review small">这一项删了有代价：{{ ex.lose }}</p>
        </template>
        <template v-else-if="elig.reason === 'unknown'">
          <div class="act-title">先搞清楚它是什么再说</div>
          <p class="small">
            规则库和启发式都没认出来，DiskWhy 不会让你删它。上面列了看到的线索；也可以问问 AI——但 AI 的回答只是推测，同样不能作为删除依据。
          </p>
        </template>
        <template v-else-if="elig.reason === 'keep' || elig.reason === 'tool'">
          <div class="act-title">{{ elig.reason === "keep" ? "这个不能删，想腾空间这样做" : "这一项不建议直接删，这样清理" }}</div>
          <p v-if="!guidance.length && ex.how_detail" class="small">{{ ex.how_detail }}</p>
        </template>
        <p v-else-if="elig.reason" class="small muted">{{ elig.reason }}</p>

        <div v-if="guidance.length" class="guides">
          <div v-for="g in guidance" :key="g.title" class="guide">
            <div class="guide-title">
              {{ g.title }}
              <span v-if="g.needs_admin" class="tag">需要管理员</span>
            </div>
            <div v-if="g.command" class="guide-cmd">
              <code>{{ g.command }}</code>
              <button class="mini" @click="copy(g.command!)">{{ copied === g.command ? "已复制" : "复制" }}</button>
            </div>
            <div v-if="g.shell" class="muted small">在哪运行：{{ g.shell }}（DiskWhy 不会替你运行）</div>
            <p class="small">{{ g.explain }}</p>
            <button v-if="g.settings_uri" class="btn" @click="openSettingsPage(g.settings_uri!)">打开系统设置</button>
          </div>
          <p v-if="settingsError" class="ai-error small">{{ settingsError }}</p>
        </div>
        <p v-if="guidanceError" class="ai-error small">{{ guidanceError }}</p>
      </section>

      <section v-if="aiResult" class="explain ex-ai">
        <div class="ex-head">
          <span class="ex-badge chip-review">AI 推测 · 需要你来判断</span>
          <span class="muted small">可信度 {{ Math.round(aiResult.confidence * 100) }}%</span>
        </div>
        <dl class="ex-grid">
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
        <ul class="ex-notes small">
          <li v-for="n in aiResult.notes" :key="n">{{ n }}</li>
        </ul>
        <div class="ex-source small"><span class="muted">来源：</span>{{ sourceLabel(aiResult) }}</div>
      </section>

      <dl v-if="!node.pseudo" class="d-grid">
        <dt>路径</dt>
        <dd class="path">{{ node.path }}</dd>
        <dt>实际占用</dt>
        <dd><b>{{ humanBytes(node.allocated) }}</b> <span class="muted">（{{ fmtInt(node.allocated) }} 字节）</span></dd>
        <dt>逻辑大小</dt>
        <dd>{{ humanBytes(node.logical) }}</dd>
        <dt>修改时间</dt>
        <dd>{{ node.kind === "Rest" ? "（最新）" : "" }}{{ fmtTime(node.mtime) }}</dd>
        <template v-if="node.kind === 'Dir'">
          <dt>子项数</dt>
          <dd>{{ fmtInt(node.child_count) }} 项（内含 {{ fmtInt(node.files) }} 个文件、{{ fmtInt(node.dirs) }} 个文件夹）</dd>
        </template>
        <template v-if="node.kind !== 'Rest'">
          <dt>属性</dt>
          <dd>{{ attrText(node.attrs) }}</dd>
        </template>
      </dl>
      <ul v-if="status.length" class="d-status">
        <li v-for="s in status" :key="s">{{ s }}</li>
      </ul>
    </template>
    <template v-else>
      <p class="muted">点击左侧列表中的一项查看详情。</p>
    </template>
  </aside>
</template>
