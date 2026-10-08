# Concept 2 · 开口的磁盘 (The Talking Disk)

一句话：一个对话气泡，里面装着一张“方块地图”——磁盘自己开口，把每一块空间是什么讲给你听。

- Bubble = explanation ("why"); tiles = the treemap / capacity bar the app already uses.
- Lime block = 可放心清理 (largest, brand colour, and its corner *is* the bubble's tail).
  Paper block = 剩余空间, coral block = 需你判断. Same three meanings as the home screen bar.
- Palette: ink `#141412`, lime `#d4f24a`, paper `#f6f1e4`, coral `#ff8a73` (+ one-step 120° ramps on the 1024 master only).
- Wordmark: Geist (OFL, already bundled in `src/assets/fonts`), weight 620, outlined to paths, one optical kern fix (k–W −38 units).

## Rules applied

1. **One metaphor, at most two.** "Illustrate the concept of your app in a singular element using simple forms … use no more than two metaphors." Two here: speech bubble + treemap, fused into one shape rather than placed side by side.
   — https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-design
2. **Few shapes, few corners, distinct silhouette that survives small sizes.** "Use as few shapes with as few corners as possible." Four filled shapes total; silhouette is a bubble with a corner tail.
   — same Microsoft page (Silhouette); Apple HIG: "express it in a simple, unique way with a minimal number of shapes." https://developer.apple.com/design/human-interface-guidelines/app-icons
3. **No text in the icon.** Both Microsoft and Apple say letters should be avoided unless essential; the name sits next to the icon in Windows anyway. The "D/W" letters live only in the lockup.
4. **Contrast on light *and* dark taskbars; use dark, medium and light values.** "Make sure at least half of your icon passes a 3.0:1 contrast ratio on light and dark theme." Ink rim carries the shape on #f3f3f3; lime/paper/coral carry it on #202020 — which is why the lime block owns the tail (otherwise the bubble disappears on dark).
   — Microsoft app-icon-design (Color contrast)
5. **Gradients minimal: one step, 120°, light from top-left.** Only on the large master; small sizes are flat.
   — Microsoft app-icon-design (Color and gradients)
6. **Provide hand-tuned small sizes; the minimum ICO set is 16/24/32/48/256.** Title bar / tray = 16 px at 100 %, taskbar = 24 px. `icon-16.svg`, `icon-24.svg`, `icon-32.svg` are separately drawn on whole-pixel grids (integer rim, gutter and tile edges) instead of shrinking the 1024 drawing — compare "hinted" vs "naive downscale" on the sheet.
   — https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction
7. **Avoid hairlines and sharp corners.** Apple: "Avoid extremely thin line weights and sharp corners, because they tend to lose detail." The tail tip and knee have real tangent fillets; rim is ≥ 2 px at 48 px.
8. **Rounded = approachable.** Microsoft: soft corners communicate approachability; outer radius generous, inner radii small and consistent.
9. **Simple, memorable, appropriate, scalable** (general logo canon): one idea, limited colours, readable from favicon to billboard, fits a calm "explain + safe" product rather than a scary "booster".
   — https://www.logodesign.net/blog/principles-of-good-logo-design/ ·  https://cieden.com/book/atoms/logo/principles-of-logo-design
10. **Recognition at first sight.** NN/g: if an icon's meaning isn't immediately clear it becomes noise — so the mark reuses the exact colour-meaning the user already sees on the home screen.
   — https://www.nngroup.com/articles/icon-usability/

## Files

- `icon.svg` — 1024 master (sizes ≥ 48)
- `icon-32.svg`, `icon-24.svg`, `icon-16.svg` — pixel-grid small sizes
- `lockup.svg`, `lockup-dark.svg` — horizontal lockup, large
- `lockup-A.svg`, `lockup-B.svg`, `lockup-C.svg` — title-bar lockups (16 px mark) per theme
- `png/` — 1× rasters (16, 24, 32, 48, 64, 256, 1024)
- `sheet.png` — preview sheet (rendered by `render.py` with Playwright + Edge)
- `build.py` — generates every SVG from parameters; `render.py` — rasterises and builds the sheet
