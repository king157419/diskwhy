<script setup lang="ts">
// 设置：主题（三个小色块 + 名字）、AI 识别（可选，服务商预设一点就填好）、其他（识别规则 / 权限 / 扫描结果 / 暂存区 / 版本）。
// 阶段 J §3.8：每一栏只有短标签，说明藏在「?」后面（点开才显示一句话）。
// 服务商预设的地址 / 模型按各家官方文档核对（2026-10-06）：
//   DeepSeek   https://api.deepseek.com                            deepseek-flash   （api-docs.deepseek.com）
//   通义千问   https://dashscope.aliyuncs.com/compatible-mode/v1    qwen-plus        （help.aliyun.com 百炼 OpenAI 兼容：原域名仍可正常使用）
//   Kimi       https://api.moonshot.cn/v1                          kimi-k2.6        （platform.kimi.com 快速开始 / 价格）
//   OpenAI     https://api.openai.com/v1                           gpt-5.4-mini     （developers.openai.com 模型页）
//   本机       http://localhost:11434/v1                           qwen2.5:7b       （Ollama 的 OpenAI 兼容接口）
// 后端按 OpenAI 兼容方式 POST {地址}/chat/completions，密钥放 Authorization: Bearer（crates/diskwhy-core/src/explain/llm.rs）。
import { computed, onMounted, ref } from "vue";
import { getSettings, saveSettings, testLlm, type LlmSettings } from "../api";
import { THEMES, setTheme, theme } from "../theme";
import { actEnv, doOpenSnapshot, doRelaunch, doSaveSnapshot, env, onSettingsSaved, page, relaunching, snapBusy, summary } from "../store";

const s = ref<LlmSettings>({ enabled: false, base_url: "", model: "", api_key: "" });
const testing = ref(false);
const saving = ref(false);
const result = ref("");
const resultOk = ref(false);

onMounted(async () => {
  try {
    s.value = await getSettings();
  } catch (e) {
    result.value = `读取失败：${e}`;
  }
});

interface Preset {
  id: string;
  name: string;
  url: string;
  model: string;
  /** 获取密钥的官方页面（本机是下载页） */
  link: string;
  linkText: string;
  tip?: string;
}
const PRESETS: Preset[] = [
  { id: "deepseek", name: "DeepSeek", url: "https://api.deepseek.com", model: "deepseek-flash", link: "https://platform.deepseek.com/api_keys", linkText: "获取密钥" },
  {
    id: "qwen",
    name: "通义千问",
    url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-plus",
    link: "https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key",
    linkText: "获取密钥",
  },
  { id: "kimi", name: "Kimi", url: "https://api.moonshot.cn/v1", model: "kimi-k2.6", link: "https://platform.moonshot.cn/console/api-keys", linkText: "获取密钥" },
  { id: "openai", name: "OpenAI", url: "https://api.openai.com/v1", model: "gpt-5.4-mini", link: "https://platform.openai.com/api-keys", linkText: "获取密钥" },
  { id: "local", name: "本机", url: "http://localhost:11434/v1", model: "qwen2.5:7b", link: "https://ollama.com/download", linkText: "下载", tip: "本机小模型猜得不准" },
];
const norm = (u: string) => u.trim().replace(/\/+$/, "").replace(/\/chat\/completions$/, "").toLowerCase();
const active = computed(() => PRESETS.find((p) => norm(p.url) === norm(s.value.base_url)) ?? null);
function usePreset(p: Preset) {
  if (active.value?.id === p.id) return;
  s.value.base_url = p.url;
  s.value.model = p.model;
  result.value = "";
}
async function openLink(url: string) {
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener");
  }
}

// 「?」：同一时间只展开一条说明
const help = ref<string | null>(null);
const toggleHelp = (k: string) => (help.value = help.value === k ? null : k);
const HELP: Record<string, string> = {
  ai: "认不出的文件夹问 AI 猜一猜。只发名字和大小，不发内容；结果仅供参考。",
  snap: "存成文件，下次不用重扫。",
  stash: "删掉的先放这里 7 天，可以放回原处。",
};
const stashHelp = computed(() => HELP.stash.replace("7", String(actEnv.value?.keep_days ?? 7)));

async function doTest() {
  testing.value = true;
  result.value = "";
  try {
    result.value = await testLlm(s.value);
    resultOk.value = true;
  } catch (e) {
    result.value = `连不上：${e}`;
    resultOk.value = false;
  } finally {
    testing.value = false;
  }
}

async function doSave() {
  saving.value = true;
  try {
    await saveSettings(s.value);
    onSettingsSaved({ ...s.value });
    result.value = "已保存";
    resultOk.value = true;
  } catch (e) {
    result.value = `保存失败：${e}`;
    resultOk.value = false;
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="settings">
    <div class="wrap">
      <section class="sec">
        <h3>主题</h3>
        <div class="themes" role="radiogroup" aria-label="主题">
          <button
            v-for="t in THEMES"
            :key="t.id"
            class="th"
            :class="{ on: theme === t.id }"
            role="radio"
            :aria-checked="theme === t.id"
            :aria-label="'主题：' + t.name"
            @click="setTheme(t.id)"
          >
            <span class="pv" :class="'pv-' + t.id"><i class="n">32</i><i class="bar"></i></span>
            <span class="nm">{{ t.name }}</span>
          </button>
        </div>
      </section>

      <section class="sec">
        <h3>
          AI 识别
          <button class="qm" :class="{ on: help === 'ai' }" :aria-expanded="help === 'ai'" aria-label="AI 识别是什么" @click="toggleHelp('ai')">?</button>
          <label class="switch">
            <input v-model="s.enabled" type="checkbox" aria-label="启用 AI 识别" />
            <span class="track"><span class="thumb"></span></span>
            <span class="sw">{{ s.enabled ? "开" : "关" }}</span>
          </label>
        </h3>
        <p v-if="help === 'ai'" class="help">{{ HELP.ai }}</p>
        <div class="presets" role="radiogroup" aria-label="服务商">
          <button
            v-for="p in PRESETS"
            :key="p.id"
            class="pre"
            :class="{ on: active?.id === p.id }"
            role="radio"
            :aria-checked="active?.id === p.id"
            :title="p.tip"
            @click="usePreset(p)"
          >
            {{ p.name }}
          </button>
        </div>
        <div class="form">
          <label for="llm-url">地址</label>
          <input id="llm-url" v-model.trim="s.base_url" class="input mono" placeholder="https://…/v1" spellcheck="false" />
          <label for="llm-model">模型</label>
          <input id="llm-model" v-model.trim="s.model" class="input mono" placeholder="qwen-plus" spellcheck="false" />
          <label for="llm-key" title="存在本机 %LOCALAPPDATA%\DiskWhy\settings.json">密钥</label>
          <div class="with">
            <input id="llm-key" v-model.trim="s.api_key" class="input mono" type="password" :placeholder="active?.id === 'local' ? '本机不用填' : 'sk-…'" autocomplete="off" />
            <button v-if="active" class="quiet ext" type="button" @click="openLink(active.link)">
              {{ active.linkText }}<svg class="i" viewBox="0 0 24 24"><path d="M7 17 17 7" /><path d="M8 7h9v9" /></svg>
            </button>
          </div>
        </div>
        <div class="acts">
          <button class="quiet" :disabled="testing || !s.base_url || !s.model" @click="doTest">{{ testing ? "测试中" : "测试连接" }}</button>
          <span v-if="result" class="res" :class="{ ok: resultOk }" :title="result">{{ result }}</span>
          <button class="btn" :disabled="saving" @click="doSave">保存</button>
        </div>
      </section>

      <section class="sec">
        <h3>其他</h3>
        <dl class="kv">
          <dt>识别规则</dt>
          <dd class="num">{{ env?.rules_total ?? "—" }} 条</dd>
          <dt>权限</dt>
          <dd>
            {{ env ? (env.elevated ? "管理员" : "普通") : "—" }}
            <button v-if="env && !env.elevated" class="quiet" :disabled="relaunching" @click="doRelaunch">以管理员重启</button>
          </dd>
          <dt>扫描结果</dt>
          <dd>
            <button class="quiet" :disabled="snapBusy || !summary" @click="doSaveSnapshot">保存</button>
            <button class="quiet" :disabled="snapBusy" @click="doOpenSnapshot()">打开</button>
            <button class="qm" :class="{ on: help === 'snap' }" :aria-expanded="help === 'snap'" aria-label="扫描结果是什么" @click="toggleHelp('snap')">?</button>
          </dd>
          <dd v-if="help === 'snap'" class="help kv-help">{{ HELP.snap }}</dd>
          <dt>暂存区</dt>
          <dd>
            <span class="num">{{ actEnv?.batches.length ?? 0 }}</span> 批
            <button class="quiet" @click="page = 'quarantine'">查看</button>
            <button class="qm" :class="{ on: help === 'stash' }" :aria-expanded="help === 'stash'" aria-label="暂存区是什么" @click="toggleHelp('stash')">?</button>
          </dd>
          <dd v-if="help === 'stash'" class="help kv-help">{{ stashHelp }}</dd>
          <dt>版本</dt>
          <dd class="num">{{ env?.version ?? "—" }}</dd>
        </dl>
        <ul v-if="env && env.rule_warnings.length" class="warns">
          <li v-for="w in env.rule_warnings" :key="w">{{ w }}</li>
        </ul>
      </section>
    </div>
  </div>
</template>

<style scoped>
.settings {
  overflow-y: auto;
}
.wrap {
  width: min(720px, calc(100% - 2 * var(--gutter)));
  margin: 88px 0 48px var(--gutter);
}
.sec {
  padding: 26px 0 30px;
  border-top: var(--bw) solid var(--line2);
}
.sec:first-child {
  border-top: 0;
  padding-top: 0;
}
h3 {
  display: flex;
  align-items: center;
  gap: 16px;
  font-family: var(--heading);
  font-size: var(--fs-group);
  font-weight: var(--heading-w);
  margin-bottom: 18px;
}
/* C 主题的标题字体（文楷子集）缺“识”“暂”等字：这一页的分区标题统一用界面字体，不在一个词里混两种字体 */
[data-theme="C"] h3 {
  font-family: var(--ui);
  font-weight: var(--w-bold);
}
.themes {
  display: flex;
  gap: 18px;
}
.th {
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: flex-start;
}
.pv {
  width: 132px;
  height: 84px;
  border-radius: 8px;
  position: relative;
  overflow: hidden;
  border: 1px solid var(--line2);
  transition: transform var(--dur-1) var(--ease);
}
.th:hover .pv {
  transform: translateY(-2px);
}
.th.on .pv {
  outline: 2px solid var(--ink);
  outline-offset: 3px;
}
.pv .n {
  position: absolute;
  left: 12px;
  top: 10px;
  font-style: normal;
  font-size: 44px;
  line-height: 1;
}
.pv .bar {
  position: absolute;
  right: 14px;
  top: 12px;
  bottom: 12px;
  width: 16px;
  border-radius: 2px;
}
.pv-A {
  background: #f3f1ec;
}
.pv-A .n {
  font-family: "Instrument Serif", serif;
  color: #1c1b18;
}
.pv-A .bar {
  background: linear-gradient(#e6e2da 20%, #2a7a5a 20% 45%, #bc7a22 45% 65%, #cbc4b7 65%);
}
.pv-B {
  background: #14110e;
}
.pv-B .n {
  font-family: "Geist", sans-serif;
  font-weight: 200;
  color: #ebc47a;
}
.pv-B .bar {
  background: linear-gradient(#2a241e 20%, #ebc47a 20% 45%, #e3876b 45% 65%, #4a4138 65%);
}
.pv-C {
  background: #d4f24c;
  border: 2px solid #141412;
}
.pv-C .n {
  font-family: "Bricolage Grotesque", sans-serif;
  font-weight: 800;
  font-variation-settings: "wdth" 75, "opsz" 96;
  color: #141412;
}
.pv-C .bar {
  border: 2px solid #141412;
  background: linear-gradient(#fff 20%, #d4f24c 20% 45%, #ff8a76 45% 65%, #d6d0c1 65%);
}
.nm {
  font-size: var(--fs-sub);
  color: var(--ink2);
  font-weight: var(--w-med);
}
.th.on .nm {
  color: var(--ink);
  font-weight: var(--w-semi);
}
[data-theme="C"] .nm {
  font-family: var(--heading);
  font-weight: 700;
}

.switch {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  font-family: var(--ui);
  font-size: var(--fs-sub);
  font-weight: var(--w-reg);
  color: var(--ink2);
}
.switch input {
  position: absolute;
  opacity: 0;
  width: 1px;
  height: 1px;
}
.track {
  width: 34px;
  height: 20px;
  border-radius: 10px;
  background: var(--line2);
  position: relative;
  transition: background-color var(--dur-2) var(--ease);
}
.thumb {
  position: absolute;
  left: 2px;
  top: 2px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--surface);
  transition: transform var(--dur-2) var(--ease);
}
.switch input:checked + .track {
  background: var(--accent);
}
.switch input:checked + .track .thumb {
  transform: translateX(14px);
}
.switch input:focus-visible + .track {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.form {
  display: grid;
  grid-template-columns: 64px 1fr;
  gap: 10px 16px;
  align-items: center;
}
.form label {
  font-size: var(--fs-sub);
  color: var(--ink3);
}
.with {
  display: flex;
  gap: 12px;
  align-items: center;
}
.with .quiet {
  white-space: nowrap;
}
.input {
  width: 100%;
  height: 38px;
  padding: 0 12px;
  border-radius: var(--r);
  border: var(--bw) solid var(--line2);
  background: var(--surface);
  font-size: 13px;
  color: var(--ink);
  outline: none;
}
.input:focus {
  border-color: var(--accent);
}
[data-theme="A"] .input:focus {
  border-color: var(--ink);
}
.acts {
  margin-top: 20px;
  display: flex;
  align-items: center;
  gap: 18px;
}
.acts .btn {
  margin-left: auto;
  height: 44px;
  min-width: 120px;
}
.res {
  font-size: var(--fs-sub);
  color: var(--danger);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 360px;
}
.res.ok {
  color: var(--safe-label);
}
.kv {
  display: grid;
  grid-template-columns: 64px 1fr;
  gap: 12px 16px;
  font-size: var(--fs-sub);
}
.kv dt {
  color: var(--ink3);
}
.kv dd {
  display: flex;
  align-items: center;
  gap: 14px;
  color: var(--ink);
}
.warns {
  margin-top: 14px;
  padding-left: 18px;
  font-size: var(--fs-meta);
  color: var(--review-label);
  user-select: text;
}

/* ── 「?」与一句话说明（阶段 J §3.8） ── */
.qm {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  display: inline-grid;
  place-items: center;
  font-family: var(--ui);
  font-size: 12px;
  font-weight: var(--w-semi);
  line-height: 1;
  color: var(--ink3);
  border: 1px solid var(--line2);
  flex: none;
  transition:
    background-color var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease);
}
.qm:hover,
.qm.on {
  color: var(--ink);
  background: var(--hover);
}
[data-theme="C"] .qm {
  border: 1.5px solid #141412;
  color: #141412;
  background: #fff;
}
[data-theme="C"] .qm:hover,
[data-theme="C"] .qm.on {
  background: var(--accent);
}
.help {
  margin: -6px 0 16px;
  font-size: var(--fs-sub);
  color: var(--ink2);
  line-height: 1.6;
  padding-left: 12px;
  border-left: 2px solid var(--line2);
  animation: help-in var(--dur-2) var(--ease);
}
@keyframes help-in {
  from {
    opacity: 0;
    transform: translateY(-3px);
  }
}
[data-theme="C"] .help {
  border-left: 3px solid #141412;
}
.kv-help {
  grid-column: 2;
  margin: -4px 0 0;
}

/* ── 服务商预设：分段 ── */
.presets {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 18px;
}
.pre {
  height: 34px;
  padding: 0 14px;
  font-size: var(--fs-sub);
  font-weight: var(--w-med);
  color: var(--ink2);
  border: var(--bw) solid var(--line2);
  border-radius: var(--r-pill);
  white-space: nowrap;
  transition:
    background-color var(--dur-1) var(--ease),
    color var(--dur-1) var(--ease);
}
.pre:hover:not(.on) {
  background: var(--hover);
  color: var(--ink);
}
.pre.on {
  background: var(--ink);
  border-color: var(--ink);
  color: var(--bg);
  font-weight: var(--w-semi);
}
[data-theme="A"] .pre.on {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}
[data-theme="B"] .pre.on {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--accent-ink);
}
/* 服务商名是品牌名，用界面字体（手写体子集不含这些字） */
[data-theme="C"] .pre {
  border-color: #141412;
  background: #fff;
  color: #141412;
  font-weight: var(--w-semi);
}
[data-theme="C"] .pre.on {
  background: var(--accent);
  box-shadow: 3px 3px 0 #141412;
}
.ext {
  white-space: nowrap;
  gap: 2px;
}
.ext .i {
  width: 14px;
  height: 14px;
}
[data-theme="C"] .ext {
  font-weight: var(--w-semi);
  color: #141412;
  text-decoration: underline;
  text-decoration-thickness: 2px;
  text-underline-offset: 4px;
}

/* ───────── 窄窗口 / 竖屏（阶段 J §3.7）：主题预览缩到 100px ───────── */
@media (max-width: 1099px), (max-aspect-ratio: 1/1) {
  .wrap {
    width: auto;
    max-width: 720px;
    margin: 72px var(--gutter) 40px;
  }
  .themes {
    gap: 14px;
  }
  .pv {
    width: 100px;
    height: 64px;
  }
  .pv .n {
    font-size: 34px;
  }
  .pv .bar {
    width: 12px;
    top: 9px;
    bottom: 9px;
    right: 11px;
  }
  .res {
    max-width: 200px;
  }
}
</style>
