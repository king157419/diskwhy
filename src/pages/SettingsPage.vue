<script setup lang="ts">
import { onMounted, ref } from "vue";
import { Bot, BookOpen, CircleCheck, KeyRound, Monitor, Moon, Palette, PlugZap, Sun, TriangleAlert } from "lucide-vue-next";
import { getSettings, saveSettings, testLlm, type LlmSettings } from "../api";
import { fmtInt } from "../format";
import PageHeader from "../components/ui/PageHeader.vue";
import { setTheme, themePref, type ThemePref } from "../theme";
import { actEnv, env, onSettingsSaved } from "../store";

const s = ref<LlmSettings>({ enabled: false, base_url: "", model: "", api_key: "" });
const testing = ref(false);
const saving = ref(false);
const result = ref("");
const resultOk = ref(false);

onMounted(async () => {
  try {
    s.value = await getSettings();
  } catch (e) {
    result.value = `读取设置失败：${e}`;
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
    result.value = `连接失败：${e}`;
    resultOk.value = false;
  } finally {
    testing.value = false;
  }
}

async function doSave() {
  saving.value = true;
  try {
    const p = await saveSettings(s.value);
    onSettingsSaved({ ...s.value });
    result.value = `已保存到 ${p}`;
    resultOk.value = true;
  } catch (e) {
    result.value = `保存失败：${e}`;
    resultOk.value = false;
  } finally {
    saving.value = false;
  }
}

const themes: { key: ThemePref; label: string; sub: string; icon: unknown }[] = [
  { key: "dark", label: "深色", sub: "默认", icon: Moon },
  { key: "light", label: "浅色", sub: "白天更清楚", icon: Sun },
  { key: "system", label: "跟随系统", sub: "随 Windows 切换", icon: Monitor },
];
</script>

<template>
  <div class="page">
    <PageHeader title="设置" subtitle="外观、AI 推测（可选）和规则库信息" />

    <div class="page-body">
      <div class="col">
        <!-- 外观 -->
        <section class="sec card">
          <div class="sec-head">
            <span class="sec-ico"><Palette :size="17" /></span>
            <div>
              <div class="card-title">外观</div>
              <div class="muted small">选择只保存在这台电脑上</div>
            </div>
          </div>
          <div class="themes" role="radiogroup" aria-label="主题">
            <button
              v-for="t in themes"
              :key="t.key"
              class="theme"
              :class="{ on: themePref === t.key }"
              role="radio"
              :aria-checked="themePref === t.key"
              :aria-label="'主题：' + t.label"
              @click="setTheme(t.key)"
            >
              <span class="preview" :class="'pv-' + t.key">
                <span class="pv-side"></span>
                <span class="pv-main"><i></i><i></i><i class="short"></i></span>
              </span>
              <span class="theme-label"><component :is="t.icon" :size="14" /> {{ t.label }}</span>
              <span class="faint small">{{ t.sub }}</span>
            </button>
          </div>
        </section>

        <!-- AI -->
        <section class="sec card">
          <div class="sec-head">
            <span class="sec-ico"><Bot :size="17" /></span>
            <div class="grow">
              <div class="card-title">AI 推测（可选）</div>
              <div class="muted small">规则库和启发式都认不出来的文件夹，可以点“问 AI”让大模型猜一猜。默认关闭；支持任何 OpenAI 兼容接口，包括本机运行的 Ollama（不联网、不需要密钥）。</div>
            </div>
            <label class="switch">
              <input type="checkbox" v-model="s.enabled" aria-label="启用 AI 推测" />
              <span class="track"><span class="thumb"></span></span>
              <span class="sw-label">{{ s.enabled ? "已启用" : "已关闭" }}</span>
            </label>
          </div>

          <div class="form">
            <label for="llm-url">接口地址</label>
            <div class="with-btn">
              <input id="llm-url" v-model.trim="s.base_url" class="input" placeholder="http://localhost:11434/v1" spellcheck="false" />
              <button class="btn" type="button" @click="useOllama">填入本机 Ollama</button>
            </div>
            <label for="llm-model">模型名</label>
            <input id="llm-model" v-model.trim="s.model" class="input" placeholder="例如 qwen2.5:7b、gpt-4o-mini" spellcheck="false" />
            <label for="llm-key">API Key</label>
            <input id="llm-key" v-model.trim="s.api_key" class="input" type="password" placeholder="本机 Ollama 留空即可" autocomplete="off" />
          </div>

          <div class="privacy">
            <div><b>会发送什么：</b>文件夹名、父路径（C:\Users\你的用户名 会换成 ~）、最大的 10 个子项名和大小、文件类型分布、总大小、修改日期。</div>
            <div><b>绝不发送：</b>任何文件的内容。AI 的回答一律标“AI 推测 · 需要你来判断”，不会被当成“可以放心清理”。</div>
            <div class="faint">设置保存在 %LOCALAPPDATA%\DiskWhy\settings.json。</div>
          </div>

          <div class="callout warn" role="note">
            <KeyRound :size="16" />
            <span>
              API Key 目前<b>明文</b>保存在上面的 settings.json 里，没有加密：能登录这台电脑这个账户的人或程序都能读到。建议只用本机 Ollama（不需要 Key），或给 DiskWhy 单独建一个额度很低、随时可以作废的 Key。
            </span>
          </div>

          <div v-if="result" class="callout" :class="resultOk ? 'ok' : 'danger'">
            <CircleCheck v-if="resultOk" :size="16" />
            <TriangleAlert v-else :size="16" />
            <span>{{ result }}</span>
          </div>

          <div class="actions">
            <button class="btn" :disabled="testing || !s.base_url || !s.model" @click="doTest">
              <PlugZap :size="15" /> {{ testing ? "测试中…" : "测试连接" }}
            </button>
            <span class="spacer"></span>
            <button class="btn primary" :disabled="saving" @click="doSave">保存</button>
          </div>
        </section>

        <!-- 规则库 -->
        <section class="sec card">
          <div class="sec-head">
            <span class="sec-ico"><BookOpen :size="17" /></span>
            <div>
              <div class="card-title">规则库与数据</div>
              <div class="muted small">说明文字来自内置规则库（可在 %LOCALAPPDATA%\DiskWhy\rules 放自己的规则）</div>
            </div>
          </div>
          <dl class="kv">
            <dt>规则</dt>
            <dd class="num">{{ env ? fmtInt(env.rules_total) : "—" }} 条</dd>
            <dt>已登记的软件</dt>
            <dd class="num">{{ env ? fmtInt(env.registry_apps) : "—" }} 个（从注册表读取，用来推断文件夹属于谁）</dd>
            <dt>权限</dt>
            <dd>{{ env ? (env.elevated ? "管理员" : "普通用户") : "—" }}</dd>
            <dt>动作日志</dt>
            <dd class="mono path">{{ actEnv?.log_path ?? "—" }}</dd>
            <dt>版本</dt>
            <dd class="num">{{ env?.version ?? "—" }}</dd>
          </dl>
          <div v-if="env && env.rule_warnings.length" class="callout warn">
            <TriangleAlert :size="16" />
            <div>
              <b>规则库警告</b>
              <ul class="warns">
                <li v-for="w in env.rule_warnings" :key="w">{{ w }}</li>
              </ul>
            </div>
          </div>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.page-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 6px var(--sp-8) var(--sp-10);
}
.col {
  max-width: 820px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.sec {
  padding: 20px 22px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.sec-head {
  display: flex;
  gap: 14px;
  align-items: flex-start;
}
.sec-head .small {
  margin-top: 2px;
  line-height: 1.6;
}
.grow {
  flex: 1;
  min-width: 0;
}
.sec-ico {
  width: 34px;
  height: 34px;
  border-radius: 10px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--accent-soft);
  color: var(--accent);
}
.themes {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 12px;
}
.theme {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  padding: 10px 10px 12px;
  border-radius: var(--r-md);
  border: 1px solid var(--border-2);
  background: var(--surface-2);
  cursor: pointer;
  text-align: left;
  transition:
    border-color var(--dur-1) var(--ease),
    background var(--dur-1) var(--ease),
    transform var(--dur-1) var(--ease);
}
.theme:hover {
  border-color: var(--border-3);
}
.theme:active {
  transform: scale(0.99);
}
.theme.on {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.preview {
  width: 100%;
  height: 64px;
  border-radius: 8px;
  display: flex;
  overflow: hidden;
  margin-bottom: 6px;
  border: 1px solid var(--border-1);
}
.pv-side {
  width: 26%;
}
.pv-main {
  flex: 1;
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.pv-main i {
  display: block;
  height: 8px;
  border-radius: 3px;
}
.pv-main i.short {
  width: 55%;
}
.pv-dark .pv-side {
  background: #0e0e11;
}
.pv-dark .pv-main {
  background: #16161a;
}
.pv-dark .pv-main i {
  background: #2a2a31;
}
.pv-dark .pv-main i:first-child {
  background: #6c6df0;
  width: 40%;
}
.pv-light .pv-side {
  background: #efeff3;
}
.pv-light .pv-main {
  background: #ffffff;
}
.pv-light .pv-main i {
  background: #e6e6ec;
}
.pv-light .pv-main i:first-child {
  background: #5b5de6;
  width: 40%;
}
.pv-system {
  background: linear-gradient(135deg, #16161a 50%, #ffffff 50%);
}
.pv-system .pv-side {
  background: linear-gradient(135deg, #0e0e11 50%, #efeff3 50%);
}
.pv-system .pv-main i {
  background: rgba(128, 128, 140, 0.35);
}
.pv-system .pv-main i:first-child {
  background: #6c6df0;
  width: 40%;
}
.theme-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: 600;
  font-size: var(--fs-sm);
}
.switch {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  flex: none;
}
.switch input {
  position: absolute;
  opacity: 0;
  width: 1px;
  height: 1px;
}
.track {
  width: 38px;
  height: 22px;
  border-radius: 11px;
  background: var(--surface-3);
  border: 1px solid var(--border-2);
  position: relative;
  transition: background var(--dur-2) var(--ease);
}
.thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--text-2);
  transition:
    transform var(--dur-2) var(--ease),
    background var(--dur-2) var(--ease);
}
.switch input:checked + .track {
  background: var(--accent-fill);
  border-color: transparent;
}
.switch input:checked + .track .thumb {
  transform: translateX(16px);
  background: #fff;
}
.switch input:focus-visible + .track {
  outline: 2px solid var(--accent-ring);
  outline-offset: 2px;
}
.sw-label {
  font-size: var(--fs-xs);
  color: var(--text-2);
  width: 42px;
}
.form {
  display: grid;
  grid-template-columns: 90px 1fr;
  gap: 10px 14px;
  align-items: center;
  font-size: var(--fs-sm);
}
.form label {
  color: var(--text-2);
}
.with-btn {
  display: flex;
  gap: 8px;
}
.with-btn .input {
  flex: 1;
}
.privacy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px 14px;
  border-radius: var(--r-md);
  background: var(--surface-inset);
  border: 1px solid var(--border-1);
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.65;
}
.privacy b {
  color: var(--text-1);
}
.actions {
  display: flex;
  gap: 8px;
  align-items: center;
}
.kv {
  display: grid;
  grid-template-columns: 110px 1fr;
  gap: 8px 14px;
  margin: 0;
  font-size: var(--fs-sm);
}
.kv dt {
  color: var(--text-3);
}
.kv dd {
  margin: 0;
}
.kv .path {
  font-size: var(--fs-xs);
  word-break: break-all;
}
.warns {
  margin: 4px 0 0;
  padding-left: 18px;
}
</style>
