<script setup lang="ts">
// 设置：主题（三个小色块 + 名字）、AI 识别（可选，服务商预设一点就填好）、其他（识别规则 / 权限 / 扫描结果 / 暂存区 / 版本）。
// 阶段 J §3.8：每一栏只有短标签，说明藏在「?」后面（点开才显示一句话）。
// 阶段 M：扫描完后台自动把认不出来的大文件夹 / 文件问 AI（后端按地址自动加关思考 / JSON 输出的参数），所以预设只留便宜、
// 并发要求低、国内连得上的；免费的智谱放第一个。OpenAI（国内连不上）不给预设；「本机」（Ollama）也去掉了（用户：没人会为这个
// 专门在本机部署模型）——手动填地址 / 模型照样能用（本机 / 局域网的地址照旧可以不填密钥）。
// 服务商预设的地址 / 模型按各家官方页面核对（2026-10-12）：
//   智谱       https://open.bigmodel.cn/api/paas/v4                glm-4.7-flash    永久免费（手机号注册，不用实名、不用绑卡）；
//              默认开思考（后端发 thinking: disabled）；免费账号并发低（1302 限流 / 1305 过载 → 后端一个一个问、退避重试）；
//              备用免费模型 glm-4-flash-250414。密钥页 https://bigmodel.cn/usercenter/proj-mgmt/apikeys（docs.bigmodel.cn）
//   通义千问   https://dashscope.aliyuncs.com/compatible-mode/v1    qwen-flash       ¥0.15 / ¥1.5 每百万 token，新用户 90 天 100 万免费额度；
//              qwen3.x 要 enable_thinking: false。密钥页 https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key（help.aliyun.com 百炼）
//   DeepSeek   https://api.deepseek.com                            deepseek-flash   默认开思考（thinking: disabled）；json_object 输出可用（api-docs.deepseek.com）
//   Kimi       https://api.kimi.com/coding/v1                      kimi-for-coding  Kimi Code（会员权益，没有免费档：kimi-for-coding 新套餐要 Plus 及以上，
//              档位不够返回 401）；海外是 api.kimi.ai/coding/v1。密钥在 Kimi Code 控制台 https://www.kimi.com/code/console 创建
//              （只在创建时显示一次，每个账号最多 5 个）。来源：www.kimi.com/code/docs/kimi-code/models.html、www.kimi.com/code/docs/
// 后端按 OpenAI 兼容方式 POST {地址}/chat/completions，密钥放 Authorization: Bearer（crates/diskwhy-core/src/explain/llm.rs）。
// 审查修正（2026-10-12）：
// - 开关拨到「关」马上存（只存 enabled: false，别的没保存的修改不跟着存）：后端随即停下正在问的、以后扫描完也不再问——
//   以前要再点「保存」，用户看到「关」就以为关了，其实还在往外发；
// - 换预设时密钥按服务商分开记（这次打开设置页里）：不会把上一家的密钥带去发给下一家；
// - 云服务没填密钥不算能用（同后端 LlmSettings::usable）：保存后提示「没填密钥」，后端也不开始自动识别。
import { computed, onMounted, ref, watch } from "vue";
import { getSettings, saveSettings, testLlm, type LlmSettings } from "../api";
import { hostOf, missingKey, splitErr } from "../ai";
import { THEMES, setTheme, theme } from "../theme";
import { actEnv, doOpenSnapshot, doRelaunch, doSaveSnapshot, env, onSettingsSaved, page, relaunching, snapBusy, summary } from "../store";

const s = ref<LlmSettings>({ enabled: false, base_url: "", model: "", api_key: "" });
const testing = ref(false);
const saving = ref(false);
const result = ref("");
const resultOk = ref(false);
/** 测试连接出错时的技术细节（悬停提示）；result 一变就清掉 */
const resultTip = ref("");
watch(result, () => (resultTip.value = ""), { flush: "sync" });

/** 后端存着的设置（读到的 / 上次保存的）：开关拨到「关」时存的是这份 + enabled: false */
const saved = ref<LlmSettings | null>(null);
/** 这次打开设置页里各家（按地址的主机名）填过的密钥：换预设时换成那一家的，没有就空着 */
const keys = new Map<string, string>();

onMounted(async () => {
  try {
    s.value = await getSettings();
    saved.value = { ...s.value };
    keys.set(hostOf(s.value.base_url), s.value.api_key);
  } catch (e) {
    result.value = `读取失败：${e}`;
  }
});

/** 开关拨到「关」：马上存，不用再点「保存」（拨到「开」还是要填好再保存） */
async function onToggle(on: boolean) {
  if (on || !saved.value?.enabled) return;
  const next: LlmSettings = { ...saved.value, enabled: false };
  try {
    await saveSettings(next);
    saved.value = next;
    onSettingsSaved(next);
    result.value = "";
  } catch (e) {
    result.value = `保存失败：${e}`;
    resultOk.value = false;
  }
}

interface Preset {
  id: string;
  name: string;
  url: string;
  model: string;
  /** 获取密钥的官方页面 */
  link: string;
  linkText: string;
  /** 按钮上跟着名字的小角标（≤ 3 个字）：永久免费的「免费」、要买会员的「会员」 */
  badge?: string;
  /** 要花钱的角标画成空心的（「免费」是实心的，两种一眼分得开） */
  paid?: boolean;
}
const PRESETS: Preset[] = [
  {
    id: "zhipu",
    name: "智谱",
    url: "https://open.bigmodel.cn/api/paas/v4",
    model: "glm-4.7-flash",
    link: "https://bigmodel.cn/usercenter/proj-mgmt/apikeys",
    linkText: "获取密钥",
    badge: "免费",
  },
  {
    id: "qwen",
    name: "通义千问",
    url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-flash",
    link: "https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key",
    linkText: "获取密钥",
  },
  { id: "deepseek", name: "DeepSeek", url: "https://api.deepseek.com", model: "deepseek-flash", link: "https://platform.deepseek.com/api_keys", linkText: "获取密钥" },
  { id: "kimi", name: "Kimi", url: "https://api.kimi.com/coding/v1", model: "kimi-for-coding", link: "https://www.kimi.com/code/console", linkText: "获取密钥", badge: "会员", paid: true },
];
const norm = (u: string) => u.trim().replace(/\/+$/, "").replace(/\/chat\/completions$/, "").toLowerCase();
const active = computed(() => PRESETS.find((p) => norm(p.url) === norm(s.value.base_url)) ?? null);
function usePreset(p: Preset) {
  if (active.value?.id === p.id) return;
  const from = hostOf(s.value.base_url);
  const to = hostOf(p.url);
  if (from !== to) {
    keys.set(from, s.value.api_key);
    s.value.api_key = keys.get(to) ?? "";
  }
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
  ai: "扫描完自动把认不出的大文件夹 / 大文件问 AI。只发名字、位置、里面最大几项的名字、大小和日期，不发内容；结果仅供参考。",
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
    // 后端给的是「短中文（技术细节）」：屏幕上只写短的（「密钥不对」），细节悬停才看（HTTP 状态码、服务器原话）
    const { short, detail } = splitErr(String(e));
    result.value = short;
    resultTip.value = detail;
    resultOk.value = false;
  } finally {
    testing.value = false;
  }
}

async function doSave() {
  saving.value = true;
  try {
    await saveSettings(s.value);
    saved.value = { ...s.value };
    onSettingsSaved({ ...s.value });
    // 开着、却是云服务没填密钥：存是存了，但不会自动问（后端同样判断）
    const noKey = s.value.enabled && missingKey(s.value);
    result.value = noKey ? "没填密钥" : "已保存";
    resultOk.value = !noKey;
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
            <input v-model="s.enabled" type="checkbox" aria-label="启用 AI 识别" @change="onToggle(($event.target as HTMLInputElement).checked)" />
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
            @click="usePreset(p)"
          >
            {{ p.name }}<span v-if="p.badge" class="free" :class="{ paid: p.paid }">{{ p.badge }}</span>
          </button>
        </div>
        <div class="form">
          <label for="llm-url">地址</label>
          <input id="llm-url" v-model.trim="s.base_url" class="input mono" placeholder="https://…/v1" spellcheck="false" />
          <label for="llm-model">模型</label>
          <input id="llm-model" v-model.trim="s.model" class="input mono" placeholder="glm-4.7-flash" spellcheck="false" />
          <label for="llm-key" title="存在本机 %LOCALAPPDATA%\DiskWhy\settings.json">密钥</label>
          <div class="with">
            <input id="llm-key" v-model.trim="s.api_key" class="input mono" type="password" placeholder="sk-…" autocomplete="off" />
            <button v-if="active" class="quiet ext" type="button" @click="openLink(active.link)">
              {{ active.linkText }}<svg class="i" viewBox="0 0 24 24"><path d="M7 17 17 7" /><path d="M8 7h9v9" /></svg>
            </button>
          </div>
        </div>
        <div class="acts">
          <button class="quiet" :disabled="testing || !s.base_url || !s.model" @click="doTest">{{ testing ? "测试中" : "测试连接" }}</button>
          <span v-if="result" class="res" :class="{ ok: resultOk }" :title="resultTip || undefined">{{ result }}</span>
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
/* 测试 / 保存的结果（连不上时是一句报错）：写全，长了折行（阶段 L：不用省略号截断） */
.res {
  min-width: 0;
  font-size: var(--fs-sub);
  line-height: 1.45;
  color: var(--danger);
  overflow-wrap: anywhere;
  user-select: text;
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
  /* 折行只折在标点 / 空格后面（整句「结果仅供参考。」一起换行，不会只掉下去「参考。」两个字）；极窄时照样能断 */
  word-break: keep-all;
  overflow-wrap: anywhere;
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
/* 「免费」角标（阶段 M）：跟着名字的一个小标签，中性色（不用三档颜色）；选中时跟着按钮反色 */
.pre {
  display: inline-flex;
  align-items: center;
  gap: 7px;
}
.free {
  display: inline-flex;
  align-items: center;
  height: 16px;
  padding: 0 5px;
  border-radius: 3px;
  font-size: 10.5px;
  font-weight: var(--w-semi);
  letter-spacing: 0.04em;
  line-height: 1;
  color: var(--ink2);
  background: var(--sunk);
}
.pre.on .free {
  color: inherit;
  background: rgba(255, 255, 255, 0.22);
}
[data-theme="B"] .free {
  background: rgba(255, 255, 255, 0.07);
}
[data-theme="B"] .pre.on .free {
  color: var(--accent-ink);
  background: rgba(36, 26, 13, 0.14);
}
[data-theme="C"] .free {
  height: 17px;
  padding: 0 6px;
  border-radius: 999px;
  color: #fff;
  background: #141412;
}
[data-theme="C"] .pre.on .free {
  color: #fff;
  background: #141412;
}
/* 「会员」（要花钱）：空心的（细描边、无底色），和实心的「免费」一眼分得开；选中时跟着按钮反色 */
.free.paid {
  background: transparent;
  box-shadow: inset 0 0 0 1px var(--line2);
}
.pre.on .free.paid {
  background: transparent;
  box-shadow: inset 0 0 0 1px currentColor;
}
[data-theme="B"] .free.paid {
  background: transparent;
}
[data-theme="B"] .pre.on .free.paid {
  background: transparent;
}
[data-theme="C"] .free.paid,
[data-theme="C"] .pre.on .free.paid {
  color: #141412;
  background: #fff;
  box-shadow: inset 0 0 0 1.5px #141412;
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
  /* 结果折行写全以后不会把按钮挤走：窄窗口也给够宽度，长报错不至于折成一条细长条 */
  .res {
    max-width: 320px;
  }
}
</style>
