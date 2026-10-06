// 自适应断点（阶段 J §3.7）：和各页 CSS 里的 @media 写法一致，JS 只在需要换组件形态时用（首页容量柱横放）。
// narrow = 宽 < 1100 或竖屏（宽 ≤ 高）；short = 高 < 760。
import { onBeforeUnmount, ref, type Ref } from "vue";

export const NARROW_QUERY = "(max-width: 1099px), (max-aspect-ratio: 1/1)";
export const SHORT_QUERY = "(max-height: 759px)";

/** 跟着媒体查询变化的布尔值（组件卸载时自动解绑）；没有 matchMedia（vitest）时为 false */
export function useMedia(query: string): Ref<boolean> {
  const mq = typeof window !== "undefined" && "matchMedia" in window ? window.matchMedia(query) : null;
  const on = ref(mq?.matches ?? false);
  if (mq) {
    const upd = (e: MediaQueryListEvent) => (on.value = e.matches);
    mq.addEventListener("change", upd);
    onBeforeUnmount(() => mq.removeEventListener("change", upd));
  }
  return on;
}
