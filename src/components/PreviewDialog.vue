<script setup lang="ts">
import { computed, ref } from "vue";
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
const strategyLabel = computed(() =>
  props.report.strategy.kind === "recycle_bin" ? "回收站" : `隔离区（${props.report.strategy.dest_root}DiskWhy-Quarantine）`,
);
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
    <div class="modal card preview" role="dialog" aria-label="清理预览">
      <template v-if="phase !== 'done'">
        <h2>清理预览 · 真正会删掉的内容</h2>
        <div class="pv-summary">
          <div>
            将处理 <b>{{ fmtInt(report.total_entries) }}</b> 个文件 / 链接，合计 <b class="big">{{ humanBytes(report.total_bytes) }}</b>（{{ totalGb }} GB），删到：<b>{{ strategyLabel }}</b>
          </div>
          <div class="muted small">
            跳过 {{ fmtInt(report.total_skipped) }} 个（热文件 {{ report.hot_count }} · 云端 {{ report.cloud_count }} · 被占用 {{ report.locked_count }}）
            <span v-if="report.link_count"> · 其中 {{ report.link_count }} 个是链接，只删链接本身</span>
          </div>
        </div>

        <div v-for="w in report.warnings" :key="w.code + w.message" class="pv-warn" :class="{ blocking: w.blocking }">
          <span>⚠ {{ w.message }}</span>
          <template v-if="w.code === 'recycle_low_space' && report.quarantine_choices.length">
            <button
              v-for="d in report.quarantine_choices"
              :key="d.root"
              class="btn"
              :disabled="phase === 'running'"
              @click="emit('requote', { kind: 'quarantine', dest_root: d.root })"
            >
              改为隔离到 {{ d.root }}（剩余 {{ humanBytes(d.free_bytes) }}）
            </button>
          </template>
        </div>

        <div class="pv-items">
          <details v-for="it in report.items" :key="it.path" class="pv-item" :open="report.items.length <= 3">
            <summary>
              <span class="chip" :class="'chip-' + (it.blocked ? (it.unknown ? 'unknown' : it.tier) : it.tier)">{{ it.unknown ? "未知" : TIER_SHORT[it.tier] }}</span>
              <b :title="it.path">{{ baseName(it.path) }}</b>
              <span v-if="it.blocked" class="pv-blocked-tag">已拦下，不会删</span>
              <span v-else class="muted small">
                删 {{ fmtInt(it.will_delete_count) }} 个 · {{ humanBytes(it.will_delete_bytes) }}
                <template v-if="it.skipped_count"> · 跳过 {{ fmtInt(it.skipped_count) }}</template>
              </span>
            </summary>
            <div class="pv-path muted small">{{ it.path }}</div>
            <div class="muted small">{{ it.what }} · {{ it.source_label }}</div>
            <p v-if="it.blocked" class="pv-blocked">{{ it.blocked }}</p>
            <template v-else>
              <div class="pv-sub">会删掉的（{{ it.will_delete.length }} / {{ fmtInt(it.will_delete_count) }} 条{{ it.dirs_to_remove ? `，处理完后删掉变空的子文件夹 ${it.dirs_to_remove} 个；“${baseName(it.path)}”本身保留` : "" }}）</div>
              <ul class="pv-list">
                <li v-for="e in it.will_delete" :key="e.path">
                  <span class="pv-name" :title="e.path">{{ rel(it.path, e.path) }}</span>
                  <span class="pv-size">{{ e.kind === "link" ? "链接" : humanBytes(e.bytes) }}</span>
                  <span class="muted small">{{ fmtTime(e.mtime) }}</span>
                  <div v-if="e.note" class="pv-note small">{{ e.note }}</div>
                </li>
              </ul>
              <p v-if="it.will_delete_count > it.will_delete.length" class="muted small">
                …还有 {{ fmtInt(it.will_delete_count - it.will_delete.length) }} 条没有列出（只列前 200 条），合计已算在上面的总数里。
              </p>
              <template v-if="it.skipped_count">
                <div class="pv-sub">跳过的（不会动）：
                  <span v-for="(n, k) in it.skipped_by_reason" :key="k" class="tag">{{ REASON[k] ?? k }} {{ n }}</span>
                </div>
                <ul class="pv-list skipped">
                  <li v-for="s in it.skipped" :key="s.path">
                    <span class="pv-name" :title="s.path">{{ rel(it.path, s.path) }}</span>
                    <span class="pv-size">{{ humanBytes(s.bytes) }}</span>
                    <div class="pv-note small">{{ s.reason }}</div>
                  </li>
                </ul>
              </template>
              <ul v-if="it.enum_errors.length" class="small ai-error">
                <li v-for="x in it.enum_errors" :key="x">{{ x }}</li>
              </ul>
            </template>
          </details>
        </div>

        <div v-if="report.review_items.length" class="pv-review">
          <b>这些删了有代价：</b>
          <ul>
            <li v-for="r in report.review_items" :key="r.path">
              <b>{{ baseName(r.path) }}</b>（{{ humanBytes(r.bytes) }}）：{{ r.lose }}
            </li>
          </ul>
        </div>

        <div v-if="phase === 'running' && progress" class="pv-progress">
          <div class="bar"><div class="bar-fill" :style="{ width: (progress.total ? (progress.done * 100) / progress.total : 0) + '%' }"></div></div>
          <div class="small muted">
            正在处理 {{ fmtInt(progress.done) }} / {{ fmtInt(progress.total) }} · {{ humanBytes(progress.bytes_done) }}
            <span class="pv-current">{{ progress.current }}</span>
          </div>
        </div>

        <p v-if="error" class="ai-error">{{ error }}</p>

        <div v-if="phase === 'preview'" class="pv-confirm">
          <label for="confirm-gb">
            确认合计：请输入 <b>{{ totalGb }}</b>（GB，两位小数）来确认你看过上面的清单
          </label>
          <input id="confirm-gb" v-model="confirmText" placeholder="输入上面的数字" inputmode="decimal" autocomplete="off" spellcheck="false" />
        </div>

        <div class="modal-actions">
          <button class="btn" :disabled="phase === 'running'" @click="emit('close')">取消</button>
          <span class="spacer"></span>
          <button
            v-if="report.locked_count > 0"
            class="btn danger"
            :disabled="phase === 'running' || !confirmed || !report.can_execute || blockingWarning"
            @click="run(true)"
          >
            跳过被占用的 {{ report.locked_count }} 个，执行
          </button>
          <button
            v-else
            class="btn danger"
            :disabled="phase === 'running' || !confirmed || !report.can_execute || blockingWarning"
            @click="run(true)"
          >
            {{ phase === "running" ? "正在执行…" : `确认，删到${report.strategy.kind === "recycle_bin" ? "回收站" : "隔离区"}` }}
          </button>
        </div>
        <p v-if="!report.can_execute && phase === 'preview'" class="muted small">没有可以执行的内容（全部被拦下或跳过，或者有阻止执行的警告）。</p>
      </template>

      <template v-else-if="outcome">
        <h2>清理完成</h2>
        <div class="pv-summary">
          <div>
            成功 <b>{{ fmtInt(outcome.report.ok) }}</b> 个，腾出 <b class="big">{{ humanBytes(outcome.report.ok_bytes) }}</b>（{{ outcome.report.strategy_label }}）
          </div>
          <div class="small">
            跳过 {{ fmtInt(outcome.report.skipped) }} · <span :class="{ 'ai-error': outcome.report.failed }">失败 {{ fmtInt(outcome.report.failed) }}</span>（失败不会回滚已经成功的项）
          </div>
        </div>
        <ul class="small">
          <li v-for="n in outcome.report.notes" :key="n">{{ n }}</li>
          <li v-if="outcome.report.manifest_path">隔离区清单：<code>{{ outcome.report.manifest_path }}</code></li>
          <li>动作日志：<code>{{ outcome.report.log_path }}</code></li>
          <li v-if="outcome.refresh.rescanned.length">
            已重新扫描受影响的 {{ outcome.refresh.rescanned.length }} 个文件夹（{{ outcome.refresh.duration_ms }} ms），列表和方块图已更新（变化 {{ humanBytes(Math.abs(outcome.refresh.delta_bytes)) }}）。
          </li>
        </ul>
        <div class="pv-items">
          <details v-for="it in outcome.report.items" :key="it.path" class="pv-item" open>
            <summary>
              <b :title="it.path">{{ baseName(it.path) }}</b>
              <span v-if="it.blocked" class="pv-blocked-tag">已拦下</span>
              <span v-else class="muted small">
                成功 {{ it.ok }}（{{ humanBytes(it.ok_bytes) }}）· 跳过 {{ it.skipped }} · 失败 {{ it.failed }}
                <template v-if="it.links_removed"> · 删掉链接 {{ it.links_removed }}</template>
                <template v-if="it.dirs_removed"> · 删掉空文件夹 {{ it.dirs_removed }}</template>
              </span>
            </summary>
            <ul class="pv-list">
              <li v-for="d in it.details" :key="d.path + d.result" :class="'r-' + d.result">
                <span class="r-tag">{{ d.result === "ok" ? "✓" : d.result === "skipped" ? "跳过" : "失败" }}</span>
                <span class="pv-name" :title="d.path">{{ rel(it.path, d.path) }}</span>
                <span class="pv-size">{{ humanBytes(d.bytes) }}</span>
                <div v-if="d.reason" class="pv-note small">{{ d.reason }}</div>
              </li>
            </ul>
          </details>
        </div>
        <div class="modal-actions">
          <span class="spacer"></span>
          <button class="btn primary" @click="emit('close')">关闭</button>
        </div>
      </template>
    </div>
  </div>
</template>
