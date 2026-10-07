import { createApp } from "vue";
import "./styles/fonts.css";
import "./styles/tokens.css";
import "./styles/base.css";
import App from "./App.vue";
import { logFrontendError } from "./api";
import { applyTheme, loadTheme } from "./theme";

/** 前端出错记进 %LOCALAPPDATA%\DiskWhy\crash.log（阶段 J 取证）；记日志本身失败就算了，不能再抛 */
function report(message: string, stack?: string, source?: string) {
  try {
    void logFrontendError(message.slice(0, 2000), stack?.slice(0, 4000), source).catch(() => {});
  } catch {
    /* 没有后端（单元测试 / 纯浏览器）时忽略 */
  }
}

function hookErrors(app: ReturnType<typeof createApp>) {
  window.addEventListener("error", (e) => {
    const err = e.error as Error | undefined;
    report(String(e.message || err?.message || "error"), err?.stack, e.filename ? `${e.filename}:${e.lineno}:${e.colno}` : "window.error");
  });
  window.addEventListener("unhandledrejection", (e) => {
    const r = e.reason as unknown;
    const err = r instanceof Error ? r : null;
    report(err ? err.message : String(r), err?.stack, "unhandledrejection");
  });
  // 组件里抛的错 Vue 会自己接住，不会冒到 window：单独接一下，照常打到控制台
  app.config.errorHandler = (err, _inst, info) => {
    const e = err instanceof Error ? err : null;
    report(e ? e.message : String(err), e?.stack, `vue: ${info}`);
    console.error(err);
  };
}

async function start() {
  // 只在 `npm run dev` 的浏览器预览里（不在 Tauri 窗口里）装假后端；正式构建里这段整个被去掉
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const mods = import.meta.glob<{ install: () => void }>("./dev/mock.ts");
    for (const load of Object.values(mods)) (await load()).install();
  }
  applyTheme();
  void loadTheme();
  const app = createApp(App);
  hookErrors(app);
  app.mount("#app");
  if (import.meta.env.DEV) {
    const hooks = import.meta.glob<{ devNavigate: () => void }>("./dev/nav.ts");
    for (const load of Object.values(hooks)) (await load()).devNavigate();
  }
}
void start();
