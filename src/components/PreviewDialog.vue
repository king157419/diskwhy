<script setup lang="ts">
import { computed, ref } from "vue";
import { Ban, ChevronRight, CircleCheck, CircleX, FileText, Link2, ScanEye, ShieldAlert, SkipForward, TriangleAlert, X } from "lucide-vue-next";
import { executePlan, type ExecOutcome, type ExecProgress, type PreflightReport, type Strategy } from "../api";
import { TIER_SHORT, fmtInt, fmtTime, humanBytes } from "../format";
import { confirmMatches, gbText } from "../action";

const props = defineProps<{ report: PreflightReport }>();
const emit = defineEmits<{ close: []; requote: [strategy: Strategy]; executed: [outcome: ExecOutcome] }>();

type Phase = "preview" | "running" | "done";
const phase = ref<Phase>("preview");
const confirmText = ref("");
const progress = ref<ExecProgress | null>(null);
const outcome = ref<ExecOutcome | null>(null);
const error = ref("");

const totalGb = computed(() => gbText(props.report.total_bytes));
/** 防手滑：必须输入合计 GB 数（两位小数） */
const confirmed = computed(() => confirmMatches(confirmText.value, props.report.total_bytes));
const blockingWarning = computed(() => props.report.warnings.some((w) => w.blocking));
const strategyLabel = computed(() => {
  const s = props.report.strategy;
  if (s.kind === "recycle_bin") return "回收站";
  // dest_root 可能是 “D:\” 也可能是 “D:\某个文件夹”（没有结尾的反斜杠），拼路径时补上分隔符
  const root = s.dest_root.endsWith("\\") ? s.dest_root : s.dest_root + "\\";
  return `隔离区（${root}DiskWhy-Quarantine）`;
});
const REASON: Record<string, string> = {
  hot: "30 分钟内改过",
  cloud: "云端 / 占位文件",
  locked: "被占用",
  no_permission: "没有权限",
  protected: "受保护",
  changed: "被改过",
  missing: "已经不在",
  unreadable: "读不到",
};

/** 很长的警告（例如列出几十个占用文件的进程）默认只显示三行，可以展开；内容本身一个字不少 */
const expanded = ref<Set<string>>(new Set());
function toggleExpand(k: string) {
  const s = new Set(expanded.value);
  if (s.has(k)) s.delete(k);
  else s.add(k);
  expanded.value = s;
}
const totalText = computed(() => humanBytes(props.report.total_bytes));

function rel(item: string, p: string): string {
  return p.toLowerCase().startsWith(item.toLowerCase() + "\\") ? p.slice(item.length + 1) : p;
}
function baseName(p: string): string {
  const i = p.lastIndexOf("\\");
  return i >= 0 ? p.slice(i + 1) : p;
}

async function run(skipLocked: boolean) {
  phase.value = "running";
  error.value = "";
  progress.value = { done: 0, total: props.report.total_entries, bytes_done: 0, current: "" };
  try {
    const o = await executePlan(props.report.plan_id, skipLocked, (p) => (progress.value = p));
    outcome.value = o;
    phase.value = "done";
    emit("executed", o);
  } catch (e) {
    error.value = String(e);
    phase.value = "preview";
  }
}
</script>

<template>
  <div class="modal-mask">
    <div class="modal preview" role="dialog" aria-label="清理预览">
      <template v-if="phase !== 'done'">
        <header class="pv-head">
          <span class="pv-hico"><ScanEye :size="19" /></span>
          <div class="grow">
            <h2>清理预览 · 真正会删掉的内容</h2>
            <p class="faint small">后端按磁盘上的真实内容重新枚举过；下面每一条都会被处理，跳过的也写明了原因</p>
          </div>
          <button class="btn icon ghost" aria-label="取消" title="取消" :disabled="phase === 'running'" @click="emit('close')"><X :size="17" /></button>
        </header>

        <div class="pv-stats">
          <div class="st">
            <div class="st-k">将处理</div>
            <div class="st-v num">{{ fmtInt(report.total_entries) }} <span class="st-u">个文件 / 链接</span></div>
          </div>
          <div class="st">
            <div class="st-k">合计</div>
            <div class="st-v num">{{ totalText }} <span v-if="!totalText.endsWith(' GB')" class="st-u">（{{ totalGb }} GB）</span></div>
          </div>
          <div class="st wide">
            <div class="st-k">删到</div>
            <div class="st-dest">{{ strategyLabel }}</div>
          </div>
        </div>
        <div class="pv-skipline small">
          <SkipForward :size="14" />
          跳过 <b class="num">{{ fmtInt(report.total_skipped) }}</b> 个（热文件 {{ report.hot_count }} · 云端 {{ report.cloud_count }} · 被占用 {{ report.locked_count }}）
          <span v-if="report.link_count"> · 其中 {{ report.link_count }} 个是链接，只删链接本身</span>
        </div>

        <div class="pv-body">
          <div v-for="w in report.warnings" :key="w.code + w.message" class="callout" :class="w.blocking ? 'danger' : 'warn'">
            <Ban v-if="w.blocking" :size="16" />
            <TriangleAlert v-else :size="16" />
            <div class="w-text">
              <span :class="{ clamp: w.message.length > 180 && !expanded.has(w.code + w.message) }">{{ w.message }}</span>
              <button v-if="w.message.length > 180" class="w-more" @click="toggleExpand(w.code + w.message)">
                {{ expanded.has(w.code + w.message) ? "收起" : "展开全部" }}
              </button>
              <div v-if="w.code === 'recycle_low_space' && report.quarantine_choices.length" class="w-actions">
                <button
                  v-for="d in report.quarantine_choices"
                  :key="d.root"
                  class="btn sm"
                  :disabled="phase === 'running'"
                  @click="emit('requote', { kind: 'quarantine', dest_root: d.root })"
                >
                  改为隔离到 {{ d.root }}（剩余 {{ humanBytes(d.free_bytes) }}）
                </button>
              </div>
            </div>
          </div>

          <details v-for="it in report.items" :key="it.path" class="pv-item" :open="report.items.length <= 3">
            <summary>
              <ChevronRight :size="15" class="sum-chev" />
              <span class="chip" :class="'chip-' + (it.blocked ? (it.unknown ? 'unknown' : it.tier) : it.tier)">{{ it.unknown ? "未知" : TIER_SHORT[it.tier] }}</span>
              <b class="it-name" :title="it.path">{{ baseName(it.path) }}</b>
              <span v-if="it.blocked" class="tag danger">已拦下，不会删</span>
              <span v-else class="faint small">
                删 <span class="num">{{ fmtInt(it.will_delete_count) }}</span> 个 · <span class="num">{{ humanBytes(it.will_delete_bytes) }}</span>
                <template v-if="it.skipped_count"> · 跳过 <span class="num">{{ fmtInt(it.skipped_count) }}</span></template>
              </span>
            </summary>
            <div class="it-body">
              <div class="it-path mono">{{ it.path }}</div>
              <div class="faint small">{{ it.what }} · {{ it.source_label }}</div>
              <div v-if="it.blocked" class="callout danger"><ShieldAlert :size="16" /><span>{{ it.blocked }}</span></div>
              <template v-else>
                <div class="sub">
                  会删掉的（{{ it.will_delete.length }} / {{ fmtInt(it.will_delete_count) }} 条{{
                    it.dirs_to_remove ? `，处理完后删掉变空的子文件夹 ${it.dirs_to_remove} 个；“${baseName(it.path)}”本身保留` : ""
                  }}）
                </div>
                <ul class="flist">
                  <li v-for="e in it.will_delete" :key="e.path">
                    <component :is="e.kind === 'link' ? Link2 : FileText" :size="13" class="f-ico" />
                    <span class="f-name mono" :title="e.path">{{ rel(it.path, e.path) }}</span>
                    <span class="f-time faint num">{{ fmtTime(e.mtime) }}</span>
                    <span class="f-size num">{{ e.kind === "link" ? "链接" : humanBytes(e.bytes) }}</span>
                    <div v-if="e.note" class="f-note">{{ e.note }}</div>
                  </li>
                </ul>
                <p v-if="it.will_delete_count > it.will_delete.length" class="faint small">
                  …还有 {{ fmtInt(it.will_delete_count - it.will_delete.length) }} 条没有列出（只列前 200 条），合计已算在上面的总数里。
                </p>
                <template v-if="it.skipped_count">
                  <div class="sub">
                    跳过的（不会动）：
                    <span v-for="(n, k) in it.skipped_by_reason" :key="k" class="tag warn">{{ REASON[k] ?? k }} {{ n }}</span>
                  </div>
                  <ul class="flist skipped">
                    <li v-for="s in it.skipped" :key="s.path">
                      <SkipForward :size="13" class="f-ico" />
                      <span class="f-name mono" :title="s.path">{{ rel(it.path, s.path) }}</span>
                      <span class="f-time"></span>
                      <span class="f-size num">{{ humanBytes(s.bytes) }}</span>
                      <div
                        class="f-note warnish"
                        :class="{ clamp2: s.reason.length > 120 && !expanded.has('n:' + s.path), toggle: s.reason.length > 120 }"
                        :title="s.reason.length > 120 ? (expanded.has('n:' + s.path) ? '点击收起' : '点击展开全部') : undefined"
                        @click="s.reason.length > 120 && toggleExpand('n:' + s.path)"
                      >
                        {{ s.reason }}
                      </div>
                    </li>
                  </ul>
                </template>
                <ul v-if="it.enum_errors.length" class="small err-text errs">
                  <li v-for="x in it.enum_errors" :key="x">{{ x }}</li>
                </ul>
              </template>
            </div>
          </details>

          <div v-if="report.review_items.length" class="cost">
            <div class="cost-title"><TriangleAlert :size="16" /> 这些删了有代价：</div>
            <ul>
              <li v-for="r in report.review_items" :key="r.path">
                <b>{{ baseName(r.path) }}</b>（<span class="num">{{ humanBytes(r.bytes) }}</span>）：{{ r.lose }}
              </li>
            </ul>
          </div>
        </div>

        <footer class="pv-foot">
          <div v-if="phase === 'running' && progress" class="pv-progress">
            <div class="meter"><i :style="{ width: (progress.total ? (progress.done * 100) / progress.total : 0) + '%' }"></i></div>
            <div class="small faint">
              正在处理 <span class="num">{{ fmtInt(progress.done) }} / {{ fmtInt(progress.total) }}</span> · <span class="num">{{ humanBytes(progress.bytes_done) }}</span>
              <span class="pv-current mono">{{ progress.current }}</span>
            </div>
          </div>
          <p v-if="error" class="err-text small">{{ error }}</p>

          <div v-if="phase === 'preview'" class="confirm" :class="{ ok: confirmed }">
            <label for="confirm-gb" class="confirm-label">
              确认合计：请输入 <b class="num">{{ totalGb }}</b>（GB，两位小数）来确认你看过上面的清单
            </label>
            <div class="confirm-input">
              <input id="confirm-gb" v-model="confirmText" class="input num" placeholder="输入上面的数字" inputmode="decimal" autocomplete="off" spellcheck="false" />
              <CircleCheck v-if="confirmed" :size="18" class="confirm-ok" />
            </div>
          </div>

          <div class="foot-actions">
            <button class="btn" :disabled="phase === 'running'" @click="emit('close')">取消</button>
            <span class="spacer"></span>
            <span v-if="!report.can_execute && phase === 'preview'" class="faint small">没有可以执行的内容（全部被拦下或跳过，或者有阻止执行的警告）。</span>
            <button
              v-if="report.locked_count > 0"
              class="btn danger lg"
              :disabled="phase === 'running' || !confirmed || !report.can_execute || blockingWarning"
              @click="run(true)"
            >
              跳过被占用的 {{ report.locked_count }} 个，执行
            </button>
            <button v-else class="btn danger lg" :disabled="phase === 'running' || !confirmed || !report.can_execute || blockingWarning" @click="run(true)">
              {{ phase === "running" ? "正在执行…" : `确认，删到${report.strategy.kind === "recycle_bin" ? "回收站" : "隔离区"}` }}
            </button>
          </div>
        </footer>
      </template>

      <template v-else-if="outcome">
        <header class="pv-head">
          <span class="pv-hico ok"><CircleCheck :size="19" /></span>
          <div class="grow">
            <h2>清理完成</h2>
            <p class="faint small">{{ outcome.report.strategy_label }}</p>
          </div>
        </header>
        <div class="pv-stats">
          <div class="st">
            <div class="st-k">成功</div>
            <div class="st-v num">{{ fmtInt(outcome.report.ok) }} <span class="st-u">个</span></div>
          </div>
          <div class="st">
            <div class="st-k">腾出</div>
            <div class="st-v num ok-text">{{ humanBytes(outcome.report.ok_bytes) }}</div>
          </div>
          <div class="st wide">
            <div class="st-k">跳过 / 失败</div>
            <div class="st-dest">
              <span class="num">{{ fmtInt(outcome.report.skipped) }}</span> ·
              <span :class="{ 'err-text': outcome.report.failed }" class="num">失败 {{ fmtInt(outcome.report.failed) }}</span>
              <span class="faint small">（失败不会回滚已经成功的项）</span>
            </div>
          </div>
        </div>
        <div class="pv-body">
          <ul class="notes small">
            <li v-for="n in outcome.report.notes" :key="n">{{ n }}</li>
            <li v-if="outcome.report.manifest_path">隔离区清单：<code>{{ outcome.report.manifest_path }}</code></li>
            <li>动作日志：<code>{{ outcome.report.log_path }}</code></li>
            <li v-if="outcome.refresh.rescanned.length">
              已重新扫描受影响的 {{ outcome.refresh.rescanned.length }} 个文件夹（{{ outcome.refresh.duration_ms }} ms），列表和方块图已更新（变化
              {{ humanBytes(Math.abs(outcome.refresh.delta_bytes)) }}）。
            </li>
          </ul>
          <details v-for="it in outcome.report.items" :key="it.path" class="pv-item" open>
            <summary>
              <ChevronRight :size="15" class="sum-chev" />
              <b class="it-name" :title="it.path">{{ baseName(it.path) }}</b>
              <span v-if="it.blocked" class="tag danger">已拦下</span>
              <span v-else class="faint small">
                成功 {{ it.ok }}（{{ humanBytes(it.ok_bytes) }}）· 跳过 {{ it.skipped }} · 失败 {{ it.failed }}
                <template v-if="it.links_removed"> · 删掉链接 {{ it.links_removed }}</template>
                <template v-if="it.dirs_removed"> · 删掉空文件夹 {{ it.dirs_removed }}</template>
              </span>
            </summary>
            <div class="it-body">
              <ul class="flist">
                <li v-for="d in it.details" :key="d.path + d.result" :class="'r-' + d.result">
                  <CircleCheck v-if="d.result === 'ok'" :size="13" class="f-ico ok-text" />
                  <SkipForward v-else-if="d.result === 'skipped'" :size="13" class="f-ico" />
                  <CircleX v-else :size="13" class="f-ico err-text" />
                  <span class="f-name mono" :title="d.path">{{ rel(it.path, d.path) }}</span>
                  <span class="f-time faint small">{{ d.result === "ok" ? "✓" : d.result === "skipped" ? "跳过" : "失败" }}</span>
                  <span class="f-size num">{{ humanBytes(d.bytes) }}</span>
                  <div v-if="d.reason" class="f-note">{{ d.reason }}</div>
                </li>
              </ul>
            </div>
          </details>
        </div>
        <footer class="pv-foot">
          <div class="foot-actions">
            <span class="spacer"></span>
            <button class="btn primary" @click="emit('close')">关闭</button>
          </div>
        </footer>
      </template>
    </div>
  </div>
</template>

<style scoped>
.preview {
  width: min(920px, 100%);
}
.grow {
  flex: 1;
  min-width: 0;
}
.pv-head {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 20px 22px 14px;
}
.pv-head h2 {
  font-size: var(--fs-lg);
  font-weight: 650;
  letter-spacing: -0.01em;
}
.pv-hico {
  width: 38px;
  height: 38px;
  border-radius: 11px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--accent-soft);
  color: var(--accent);
}
.pv-hico.ok {
  background: var(--ok-bg);
  color: var(--ok);
}
.pv-stats {
  display: grid;
  grid-template-columns: 1fr 1.2fr 1.6fr;
  gap: 10px;
  padding: 0 22px;
}
.st {
  padding: 12px 14px;
  border-radius: var(--r-md);
  background: var(--surface-2);
  border: 1px solid var(--border-1);
  min-width: 0;
}
.st-k {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.st-v {
  font-size: var(--fs-xl);
  font-weight: 600;
  letter-spacing: -0.02em;
  line-height: 1.3;
}
.st-u {
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--text-3);
  letter-spacing: 0;
}
.st-dest {
  font-size: var(--fs-sm);
  font-weight: 600;
  margin-top: 3px;
  line-height: 1.4;
  word-break: break-all;
}
.pv-skipline {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 22px 12px;
  color: var(--text-2);
  border-bottom: 1px solid var(--border-1);
}
.pv-body {
  flex: 1;
  min-height: 80px;
  overflow-y: auto;
  padding: 14px 22px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.w-text {
  flex: 1;
  min-width: 0;
}
.clamp {
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.w-more {
  margin-top: 4px;
  padding: 0;
  border: 0;
  background: none;
  color: var(--accent);
  font-size: var(--fs-xs);
  cursor: pointer;
}
.w-more:hover {
  text-decoration: underline;
}
.w-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}
.pv-item {
  border-radius: var(--r-md);
  border: 1px solid var(--border-1);
  background: var(--surface-inset);
  flex: none;
}
.pv-item summary {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 11px 14px;
  cursor: pointer;
  list-style: none;
  border-radius: var(--r-md);
  transition: background var(--dur-1) var(--ease);
}
.pv-item summary::-webkit-details-marker {
  display: none;
}
.pv-item summary:hover {
  background: var(--surface-2);
}
.sum-chev {
  color: var(--text-3);
  transition: transform var(--dur-2) var(--ease);
  flex: none;
}
.pv-item[open] .sum-chev {
  transform: rotate(90deg);
}
.it-name {
  font-weight: 600;
}
.it-body {
  padding: 2px 14px 14px 38px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.it-path {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  word-break: break-all;
}
.sub {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  font-weight: 600;
  margin-top: 4px;
}
.flist {
  list-style: none;
  margin: 0;
  padding: 4px;
  border-radius: var(--r-sm);
  background: var(--surface-1);
  border: 1px solid var(--border-1);
  max-height: 220px;
  overflow-y: auto;
}
.flist li {
  display: grid;
  grid-template-columns: 16px minmax(0, 1fr) auto 76px;
  align-items: center;
  gap: 8px;
  padding: 5px 8px;
  border-radius: 5px;
  font-size: var(--fs-xs);
}
.flist li:hover {
  background: var(--surface-2);
}
.f-ico {
  color: var(--text-3);
}
.f-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-2xs);
}
.f-time {
  font-size: var(--fs-2xs);
}
.f-size {
  text-align: right;
  font-size: var(--fs-2xs);
}
.f-note {
  grid-column: 2 / -1;
  font-size: var(--fs-2xs);
  color: var(--text-2);
  margin-top: -2px;
}
.f-note.warnish {
  color: var(--warn);
}
.f-note.toggle {
  cursor: pointer;
}
.f-note.clamp2 {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.errs {
  margin: 0;
  padding-left: 18px;
}
.cost {
  padding: 13px 16px;
  border-radius: var(--r-md);
  background: var(--danger-bg);
  border: 1px solid var(--danger-border);
  font-size: var(--fs-sm);
  flex: none;
}
.cost-title {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--danger);
  font-weight: 650;
  margin-bottom: 4px;
}
.cost ul {
  margin: 0;
  padding-left: 22px;
  color: var(--danger);
  line-height: 1.65;
}
.cost b {
  color: var(--danger);
}
.pv-foot {
  padding: 14px 22px 18px;
  border-top: 1px solid var(--border-1);
  background: var(--surface-1);
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.pv-progress {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.pv-current {
  margin-left: 8px;
}
.confirm {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 12px 14px;
  border-radius: var(--r-md);
  background: var(--surface-2);
  border: 1px solid var(--border-2);
  transition:
    border-color var(--dur-2) var(--ease),
    background var(--dur-2) var(--ease);
}
.confirm.ok {
  border-color: var(--ok-border);
  background: var(--ok-bg);
}
.confirm-label {
  flex: 1;
  font-size: var(--fs-sm);
}
.confirm-label b {
  font-size: var(--fs-md);
}
.confirm-input {
  position: relative;
}
.confirm-input .input {
  width: 170px;
  height: 38px;
  font-size: var(--fs-lg);
  font-weight: 600;
  padding-right: 34px;
}
.confirm-ok {
  position: absolute;
  right: 10px;
  top: 10px;
  color: var(--ok);
}
.foot-actions {
  display: flex;
  align-items: center;
  gap: 10px;
}
.notes {
  margin: 0;
  padding-left: 18px;
  color: var(--text-2);
  line-height: 1.7;
}
</style>
