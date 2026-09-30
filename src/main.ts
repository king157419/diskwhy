import { createApp } from "vue";
import "./styles/fonts.css";
import "./styles/tokens.css";
import "./styles/base.css";
import App from "./App.vue";
import { applyTheme, loadTheme } from "./theme";

async function start() {
  // 只在 `npm run dev` 的浏览器预览里（不在 Tauri 窗口里）装假后端；正式构建里这段整个被去掉
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const mods = import.meta.glob<{ install: () => void }>("./dev/mock.ts");
    for (const load of Object.values(mods)) (await load()).install();
  }
  applyTheme();
  void loadTheme();
  createApp(App).mount("#app");
  if (import.meta.env.DEV) {
    const hooks = import.meta.glob<{ devNavigate: () => void }>("./dev/nav.ts");
    for (const load of Object.values(hooks)) (await load()).devNavigate();
  }
}
void start();
