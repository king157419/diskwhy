<script setup lang="ts">
import { onMounted, ref } from "vue";
import { getSettings, saveSettings, testLlm, type LlmSettings } from "../api";

const emit = defineEmits<{ close: []; saved: [s: LlmSettings] }>();

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
    emit("saved", { ...s.value });
    result.value = `已保存到 ${p}`;
    resultOk.value = true;
    emit("close");
  } catch (e) {
    result.value = `保存失败：${e}`;
    resultOk.value = false;
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal card" role="dialog" aria-label="设置">
      <h2>设置 · AI 推测（可选）</h2>
      <p class="muted small">
        规则库和启发式都认不出来的文件夹，可以点“问 AI”让大模型猜一猜。默认关闭；支持任何 OpenAI 兼容接口，
        包括在本机运行的 Ollama（不联网、不需要密钥）。
      </p>

      <label class="switch-row">
        <input type="checkbox" v-model="s.enabled" />
        <span>启用 AI 推测</span>
      </label>

      <div class="form-grid">
        <label for="llm-url">接口地址</label>
        <div class="with-btn">
          <input id="llm-url" v-model.trim="s.base_url" placeholder="http://localhost:11434/v1" spellcheck="false" />
          <button class="btn" type="button" @click="useOllama">填入本机 Ollama</button>
        </div>
        <label for="llm-model">模型名</label>
        <input id="llm-model" v-model.trim="s.model" placeholder="例如 qwen2.5:7b、gpt-4o-mini" spellcheck="false" />
        <label for="llm-key">API Key</label>
        <input id="llm-key" v-model.trim="s.api_key" type="password" placeholder="本机 Ollama 留空即可" autocomplete="off" />
      </div>

      <div class="privacy small">
        <b>会发送什么：</b>文件夹名、父路径（C:\Users\你的用户名 会换成 ~）、最大的 10 个子项名和大小、文件类型分布、总大小、修改日期。<br />
        <b>绝不发送：</b>任何文件的内容。AI 的回答一律标“AI 推测 · 需要你来判断”，不会被当成“可以放心清理”。<br />
        设置保存在 %LOCALAPPDATA%\DiskWhy\settings.json（API Key 以明文保存在本机）。
      </div>

      <p v-if="result" class="small" :class="resultOk ? 'ok-line' : 'ai-error'">{{ result }}</p>

      <div class="modal-actions">
        <button class="btn" :disabled="testing || !s.base_url || !s.model" @click="doTest">
          {{ testing ? "测试中…" : "测试连接" }}
        </button>
        <span class="spacer"></span>
        <button class="btn" @click="emit('close')">取消</button>
        <button class="btn primary" :disabled="saving" @click="doSave">保存</button>
      </div>
    </div>
  </div>
</template>
