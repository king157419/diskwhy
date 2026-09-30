// 主题（阶段 F）：A 浅色·纸本 / B 深色·暖炭 / C 趣味·汽水。
// - 真正的存档在 %LOCALAPPDATA%\DiskWhy\ui.json（后端 get_ui_prefs / set_ui_prefs）；
//   localStorage 只是镜像，让 index.html 在样式加载前就能设好主题，避免启动时闪一下别的颜色。
// - 第一次启动（还没存过）按系统深浅色给默认值：深色 → B，浅色 → A，然后存下来。
// - 同时让 Windows 窗口主题跟着变（Tauri window.setTheme：A / C 浅色，B 深色）。
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export type ThemeId = "A" | "B" | "C";
export const THEMES: { id: ThemeId; name: string }[] = [
  { id: "A", name: "浅色" },
  { id: "B", name: "深色" },
  { id: "C", name: "趣味" },
];
const KEY = "diskwhy.theme3";

export function isThemeId(v: unknown): v is ThemeId {
  return v === "A" || v === "B" || v === "C";
}

/** 第一次启动的默认主题：系统深色 → B，否则 A */
export function defaultTheme(prefersDark: boolean): ThemeId {
  return prefersDark ? "B" : "A";
}

function systemDark(): boolean {
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    return false;
  }
}

function readMirror(): ThemeId | null {
  try {
    const v = localStorage.getItem(KEY);
    return isThemeId(v) ? v : null;
  } catch {
    return null;
  }
}

function writeMirror(t: ThemeId) {
  try {
    localStorage.setItem(KEY, t);
  } catch {
    /* 存储不可用：只是少了防闪烁 */
  }
}

export const theme = ref<ThemeId>(readMirror() ?? defaultTheme(systemDark()));

async function applyWindowTheme(t: ThemeId) {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTheme(t === "B" ? "dark" : "light");
  } catch {
    /* 不在 Tauri 里（vitest / 浏览器预览）或没有权限：只换页面颜色 */
  }
}

export function applyTheme(t: ThemeId = theme.value) {
  document.documentElement.setAttribute("data-theme", t);
  void applyWindowTheme(t);
}

/** 启动时：读后端存档；没有就按系统深浅色定一个并存下来 */
export async function loadTheme() {
  let saved: ThemeId | null = null;
  let backend = true;
  try {
    const p = await invoke<{ theme: string | null }>("get_ui_prefs");
    saved = isThemeId(p.theme) ? p.theme : null;
  } catch {
    backend = false;
  }
  const t = saved ?? readMirror() ?? defaultTheme(systemDark());
  theme.value = t;
  writeMirror(t);
  applyTheme(t);
  if (backend && !saved) {
    try {
      await invoke("set_ui_prefs", { prefs: { theme: t } });
    } catch {
      /* 写不进去也不影响使用 */
    }
  }
}

export async function setTheme(t: ThemeId) {
  theme.value = t;
  writeMirror(t);
  applyTheme(t);
  try {
    await invoke("set_ui_prefs", { prefs: { theme: t } });
  } catch {
    /* 不在 Tauri 里：只存 localStorage */
  }
}
