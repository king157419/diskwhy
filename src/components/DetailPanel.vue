<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { askAi, askable, explainPath, sourceKey, tierKey, type Explanation, type NodeView } from "../api";
import { HOW_LABEL, SOURCE_HINT, SOURCE_SHORT, TIER_BIG, TIER_HINT, attrText, fmtInt, fmtTime, humanBytes, sourceLabel } from "../format";
import TierChip from "./TierChip.vue";

const props = defineProps<{ node: NodeView | null; llmReady: boolean }>();
const emit = defineEmits<{ openSettings: []; openPath: [path: string] }>();

const aiResult = ref<Explanation | null>(null);
/** 列表 / 方块图没带回解释时（极少），用 explain 命令单独取一次 */
const fetched = ref<Explanation | null>(null);
const fetchError = ref("");
const aiLoading = ref(false);
const aiError = ref("");
const copied = ref("");
const linkError = ref("");

watch(
  () => (props.node ? props.node.path + "|" + props.node.kind : ""),
  async () => {
    aiResult.value = null;
    aiError.value = "";
    aiLoading.value = false;
    fetched.value = null;
    fetchError.value = "";
    const n = props.node;
    if (n && !n.explanation && !n.pseudo) {
      try {
        // “其余小文件”的 path 就是所在目录，后端按 kind 区分
        const e = await explainPath(n.path, n.kind);
        if (props.node === n) fetched.value = e;
      } catch (e) {
        if (props.node === n) fetchError.value = String(e);
      }
    }
  },
  { immediate: true },
);

const ex = computed(() => props.node?.explanation ?? fetched.value ?? null);
const sk = computed(() => sourceKey(ex.value));
const isUnknown = computed(() => sk.value === "unknown");
const tk = computed(() => tierKey(ex.value));
/** 只有“未认出”、读得到、有内容的项才能问 AI（第 1 轮安全评审：规则 / 推测认出来的不再发出去；后端也会再挡） */
const canAsk = computed(() => !!props.node && askable(props.node, ex.value));

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

/** 具体做法那一段的小标题：迁移类叫“迁移建议”，用工具的叫“怎么用”，其余叫“具体做法” */
function detailLabel(e: Explanation): string {
  if (e.how.includes("migrate") && !e.how.includes("builtin_tool")) return "迁移建议";
  if (e.how.includes("migrate")) return "迁移建议 / 怎么用";
  if (e.how.includes("builtin_tool")) return "怎么用";
  return "具体做法";
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
          <TierChip :tier="tk" big :text="TIER_BIG[tk]" />
          <span v-if="SOURCE_SHORT[sk]" class="src-tag" :class="'src-' + sk" :title="SOURCE_HINT[sk]">{{ SOURCE_SHORT[sk] }}</span>
        </div>
        <p class="ex-hint muted small">{{ TIER_HINT[tk] }}</p>
        <div v-if="isUnknown && ex.clues.length" class="ex-clues">
          <div class="ex-sub">我也不确定，这是我看到的线索：</div>
          <ul>
            <li v-for="c in ex.clues" :key="c">{{ c }}</li>
          </ul>
        </div>
        <dl class="ex-grid">
          <dt>这是什么</dt>
          <dd><b>{{ ex.what }}</b></dd>
          <dt>属于哪个软件</dt>
          <dd>{{ ex.owner ?? "不确定" }}</dd>
          <dt>为什么会有</dt>
          <dd>{{ ex.why }}</dd>
          <dt>删了会失去什么</dt>
          <dd>{{ ex.lose }}</dd>
          <dt>能腾多少</dt>
          <dd>{{ ex.gain_note }}</dd>
          <dt>怎么腾</dt>
          <dd>
            <div class="how-line">
              <span>{{ howText(ex) }}</span>
            </div>
            <div v-if="ex.tool" class="how-line how-tool-line">
              <span class="how-label">用这个工具：</span><span class="how-tool">{{ ex.tool }}</span>
            </div>
            <div v-if="ex.how_detail" class="how-detail">
              <span class="how-label">{{ detailLabel(ex) }}：</span>{{ ex.how_detail }}
            </div>
          </dd>
        </dl>
        <ul v-if="ex.notes.length" class="ex-notes small">
          <li v-for="n in ex.notes" :key="n">{{ n }}</li>
        </ul>
        <div v-if="!isUnknown && ex.clues.length" class="ex-clues">
          <div class="ex-sub">依据：</div>
          <ul>
            <li v-for="c in ex.clues" :key="c">{{ c }}</li>
          </ul>
        </div>
        <div class="ex-source small">
          <span class="muted">来源：</span>{{ sourceLabel(ex) }}
          <span v-if="ex.observed_only" class="observed-tag" title="这条规则没有找到官方出处，是在本机上观察到的">本机观察</span>
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
            只发送名字、最近两层位置、子文件夹的名字和各类文件的个数、总大小；用户名、邮箱、长数字会打码；不发送文件名和文件内容。
          </p>
          <p v-if="aiError" class="ai-error small">AI 这次没答上来：{{ aiError }}（不影响其他功能，可以稍后再试或检查设置）</p>
        </div>
      </section>

      <section v-if="aiResult" class="explain ex-ai">
        <div class="ex-head">
          <TierChip tier="review" big text="AI 推测 · 需要你来判断" />
          <span class="src-tag src-llm">AI</span>
          <span class="muted small">可信度 {{ Math.round(aiResult.confidence * 100) }}%</span>
        </div>
        <dl class="ex-grid">
          <dt>这是什么</dt>
          <dd><b>{{ aiResult.what }}</b></dd>
          <dt>属于哪个软件</dt>
          <dd>{{ aiResult.owner ?? "没查到（归属只看注册表，不让 AI 猜）" }}</dd>
          <dt>为什么会有</dt>
          <dd>{{ aiResult.why }}</dd>
          <dt>删了会失去什么</dt>
          <dd>{{ aiResult.lose }}</dd>
          <dt>能腾多少</dt>
          <dd>{{ aiResult.gain_note }}</dd>
          <dt>怎么腾</dt>
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
      <p v-if="fetchError" class="ai-error small">没取到说明：{{ fetchError }}</p>
      <p class="no-delete muted small">这里只解释、不删除：没有删除按钮。要腾空间，请按上面“怎么腾”自己动手，或用写明的工具。</p>
    </template>
    <template v-else>
      <p class="muted">点击左侧列表中的一项查看详情。</p>
    </template>
  </aside>
</template>
