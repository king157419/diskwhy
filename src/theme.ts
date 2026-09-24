// 主题：深色（默认）/ 浅色 / 跟随系统。选择存在 localStorage（读不到就用默认），
// 同时让 Windows 标题栏跟着变（Tauri 的 window.setTheme；不在 Tauri 里运行时忽略）。
import { ref } from "vue";

export type ThemePref = "dark" | "light" | "system";
const KEY = "diskwhy.theme";

function load(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "dark" || v === "light" || v === "system") return v;
  } catch {
    /* 存储不可用 */
  }
  return "dark";
}

export const themePref = ref<ThemePref>(load());

export async function applyTheme(p: ThemePref = themePref.value) {
  const root = document.documentElement;
  if (p === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", p);
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTheme(p === "system" ? null : p);
  } catch {
    /* 不在 Tauri 里（vitest / 浏览器预览）或没有权限：只换页面颜色 */
  }
}

export function setTheme(p: ThemePref) {
  themePref.value = p;
  try {
    localStorage.setItem(KEY, p);
  } catch {
    /* 忽略 */
  }
  void applyTheme(p);
}

/** 当前实际生效的是不是深色（方块图等 Canvas 绘制用） */
export function isDark(): boolean {
  const t = document.documentElement.getAttribute("data-theme");
  if (t) return t === "dark";
  return !window.matchMedia("(prefers-color-scheme: light)").matches;
}
