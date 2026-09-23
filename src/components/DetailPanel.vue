<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { askAi, tierKey, type Explanation, type NodeView } from "../api";
import { HOW_LABEL, TIER_BIG, TIER_HINT, attrText, fmtInt, fmtTime, humanBytes, sourceLabel } from "../format";

const props = defineProps<{ node: NodeView | null; llmReady: boolean }>();
const emit = defineEmits<{ openSettings: [] }>();

const aiResult = ref<Explanation | null>(null);
const aiLoading = ref(false);
const aiError = ref("");
const copied = ref("");

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
  if (!e || !props.node || props.node.kind === "Rest") return false;
  return e.source.kind === "unknown" || e.source.kind === "heuristic";
});

const status = computed(() => {
  const n = props.node;
  if (!n) return [] as string[];
  const s: string[] = [];
  if (n.inaccessible) s.push(`无法读取（Win32 错误 ${n.error_code}${n.error_code === 5 ? "：拒绝访问" : ""}）`);
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
            <span class="ref-url">{{ r }}</span>
            <button class="mini" @click="copy(r)">{{ copied === r ? "已复制" : "复制链接" }}</button>
          </div>
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

      <dl class="d-grid">
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
      <p class="muted small">阶段 B 只做解释，不提供删除按钮。</p>
    </template>
    <template v-else>
      <p class="muted">点击左侧列表中的一项查看详情。</p>
    </template>
  </aside>
</template>
