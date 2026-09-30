<script setup lang="ts">
// 设置：主题（三个小色块 + 名字）、AI 猜文件夹（可选）、其他（识别规则 / 权限 / 扫描存档 / 隔离区 / 版本）。
import { onMounted, ref } from "vue";
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

function useOllama() {
  s.value.base_url = "http://localhost:11434/v1";
  if (!s.value.model) s.value.model = "qwen2.5:7b";
}

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
          AI 猜文件夹
          <label class="switch">
            <input v-model="s.enabled" type="checkbox" aria-label="启用 AI 猜文件夹" />
            <span class="track"><span class="thumb"></span></span>
            <span class="sw">{{ s.enabled ? "开" : "关" }}</span>
          </label>
        </h3>
        <div class="form">
          <label for="llm-url">地址</label>
          <div class="with">
            <input id="llm-url" v-model.trim="s.base_url" class="input mono" placeholder="http://localhost:11434/v1" spellcheck="false" />
            <button class="quiet" type="button" @click="useOllama">本机 Ollama</button>
          </div>
          <label for="llm-model">模型</label>
          <input id="llm-model" v-model.trim="s.model" class="input mono" placeholder="qwen2.5:7b" spellcheck="false" />
          <label for="llm-key">密钥</label>
          <input id="llm-key" v-model.trim="s.api_key" class="input mono" type="password" placeholder="本机 Ollama 留空" autocomplete="off" />
        </div>
        <p class="fine">只发文件夹名和大小，不发文件内容 · 密钥明文存在 %LOCALAPPDATA%\DiskWhy\settings.json</p>
        <div class="acts">
          <button class="quiet" :disabled="testing || !s.base_url || !s.model" @click="doTest">{{ testing ? "测试中" : "测试连接" }}</button>
          <span v-if="result" class="res" :class="{ ok: resultOk }">{{ result }}</span>
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
          <dt>扫描存档</dt>
          <dd>
            <button class="quiet" :disabled="snapBusy" @click="doOpenSnapshot()">打开</button>
            <button class="quiet" :disabled="snapBusy || !summary" @click="doSaveSnapshot">保存</button>
          </dd>
          <dt>隔离区</dt>
          <dd>
            <span class="num">{{ actEnv?.batches.length ?? 0 }}</span> 批
            <button class="quiet" @click="page = 'quarantine'">查看</button>
          </dd>
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
.fine {
  margin-top: 14px;
  font-size: var(--fs-meta);
  color: var(--ink3);
}
.acts {
  margin-top: 18px;
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
</style>
