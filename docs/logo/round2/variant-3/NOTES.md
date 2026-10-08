# Round 2 · variant 3 · 叠层 Layered Plate

一句话：一块苔绿色的对话气泡“板”，上面平放着四块大小不一的磁盘方块，光从左上照下来——磁盘把自己的每一块摆出来给你看。

Brief: `../BRIEF.md` §4 变体 C. Two layers, one light, one gradient, one shadow.

## Construction (1024 master, 64-unit grid, 1 u = 16 px)

- **Module m = rim = gutter = 2.5 u = 40 px** (brief §3.3: if there is a "line", line = gutter = 2.5 u; there is exactly one line width in the icon).
- **Plate (bottom layer)**: body 20 m × 18 m = 800 × 720 at (104, 128); tail 3 m = 120 px (≤ ¼ body width), 45°, its outer edge is the plate's right edge carried straight down (tangent extension), knee R1 = 64, tip R3 = 16. Colour moss `#5c7a1a`.
- **Bricks (top layer)**, inset one m, on an 18 m × 16 m inner grid, every edge a whole module:
  lime 12×10 m (可放心清理) · paper 5×16 m (剩余空间) · clay 7×5 m (需你判断) · lime 4×5 m.
  Areas 120 : 80 : 35 : 20 → clearly unequal = treemap, not a window grid.
- **Radius chain**: plate R0 = 8 u with Figma corner smoothing 60 % (figma-squircle construction, `geom.py`); brick corners that sit in a plate corner = 8 − 2.5 = **5.5 u, the one exception the brief allows for this variant**, smoothed the same way so the rim stays parallel; all brick-to-brick corners R2 = 2 u; tail knee R1 = 4 u; tail tip R3 = 1 u. No other radii. A brick carries at most two radii (5.5 u outer + 2 u inner).
- **Light**: top-left. Lime bricks carry the only gradient, 120°, `#e0f766 → #c8e93a` (midpoint = `#d4f24a`). Every brick drops one soft shadow onto the plate: offset (0, 1 u), blur 1 u, ink at 25 %, clipped to the plate (never spills onto the background).
- **Optical centre** (measured, `metrics.json`, `png/centroid.png`): alpha centroid **(510.2, 494.4)**, target (512 ± 8, 496 ± 8) ✔. Bbox 104–903 × 128–944: everything, tail tip included, is inside the 64–960 safe area.

## Colour

| set | plate | lime bricks | paper role | clay role | used for |
|---|---|---|---|---|---|
| dark (`icon.svg`, `icon-NN.svg`) | moss | lime | paper | clay | master ≥ 48, dark taskbar |
| light (`icon-light.svg`, `icon-NN-light.svg`) | ink | lime | **moss** | clay | light taskbar (`altform-lightunplated`) |

Light set rule: outline identical; plate and the paper column each drop one value step down the lime → moss → ink chain; lime stays as the brand. Coral is gone.

Contrast (`_diag/contrast.py`): moss vs #f3f3f3 4.45, vs #202020 3.30 · lime/moss 3.90 · paper/moss 4.38 · moss/ink 3.74 · clay/ink 5.02 · clay/moss **1.34** (see critique).

Area of icon that clears 3 : 1 against its taskbar (measured on the 1× renders):

| | 16 | 24 | 32 | 1024 |
|---|---|---|---|---|
| dark set on #202020 | 100 % | 100 % | 100 % | 100 % |
| light set on #f3f3f3 | 69.7 % | 57.2 % | 65.8 % | 61.4 % |
| (dark set on #f3f3f3, for reference) | 48.6 % | 37.0 % | 46.0 % | 39.4 % |

Master area share (1024, dark set): moss 30.0 % · lime 38.6 % · paper 22.0 % · clay 9.4 %.

## Small sizes (hand-drawn, whole pixels)

| | 32 | 24 | 16 |
|---|---|---|---|
| body | 30 × 26 | 22 × 19 | 16 × 13 |
| plate corner | 4 px | 3 px | 2 px |
| rim = gutter | 2 px | 1 px | 1 px |
| bricks | 4 (min 5 × 6) | 4 (min 5 × 5) | 3 (min 4 × 11, 9 × 3) |
| brick corners | 2 outer / 1 inner | 2 / 0 | 1 / 0 |
| tail | 4-step 45° stair | 3-step | 2-step |
| light | gradient + 1 px 25 % ink drop | flat | flat |

Tails are true pixel staircases (row k under the body is L − k px wide) — no grey pixels on the diagonal. The only antialiasing is in corner pixels.

Theme title-bar marks (16 px): A = `#1c1b18` plate, `#2f7d5b` lime-role bricks, paper for the rest; B = `#f3ece2` plate, `#e4bc70` lime-role, B background `#1b1714` for the rest (reads as cut-outs); C = ink plate + lime/paper/clay and a 1 px hard (1, 1) ink offset (C's own language, separate 15 × 15 drawing `G16C` so the shadow fits in 16).

Lockups: Geist (OFL, `src/assets/fonts/Geist-latin.woff2`) at wght 600, tracking −10, outlined to paths (reuses `concept-2/build.py::word_paths`, incl. its k–W kern fix); caps (320 / 1024) centred on the plate body, not the tail; gap 7 u. Title-bar lockups use the 16 px marks with 9 px caps.

## Iteration log (draw → render → look → fix)

1. **r1 – colour of the plate** (`rounds/r1/strip.png`). Body 52.5 × 43.5 u, "window" layout, three colourways + a flat one. Saw: ink plate is just concept-2 again (heavy black rim, dead on #202020); an ink brick on moss reads as a hole; moss plate + lime is the most harmonious and keeps an outline on both taskbars. Shape too wide — reads like a monitor/app window (big pane + sidebar). → keep moss, rework proportions.
2. **r2 – one module, four layouts** (`rounds/r2/strip.png`). Rebuilt everything on m = 2.5 u, body 20 × 18 m. Compared window / L / top / four. "top" looks like a chat card, "window" like an app; **"four"** (lime 12×10, paper column, clay + small lime below) is the only one that reads as a treemap first. → chosen.
3. **r3 – hand-drawn 32 / 24 / 16** (`rounds/r3/zoom*.png`). Saw: the 45° tail drawn as a diagonal through pixel corners renders as a 1 px grey sliver at 16/24 — tail effectively disappears; a 1 px hard drop shadow at 16/24 is invisible at 1×, just muddies the bottom gutter. → stair tails, no shadow below 32.
4. **r4 – stairs + first full sheet** (`rounds/r4/zoom*.png`, `rounds/r4-sheet.png`). Stair tails now read at 1×. 32 got the gradient + 1 px 25 % drop (≈ the master's soft shadow at that size). Measured: dark set fails the light-taskbar 50 % rule (37–49 %), ink-plate light set looked like concept-2 and still failed (same areas). Centroid x = 518 (6 px right). Lockup text too big (cap 400).
5. **r5 – light set + centring** (`rounds/r5/light-candidates.png`, `rounds/r5-sheet.png`, `rounds/r5-tailcrop.png`). Compared moss / ink / ink+moss for the light taskbar → **ink plate + moss column** is the only one that is solid at 1× and passes (57–70 %). Shifted body 0.5 u left → centroid 510.2. Cap 400 → 336. 1024 crop showed the R2 knee as an abrupt notch where the tail leaves the bottom edge.
6. **r6 – knee + lockup** (`rounds/r6-sheet.png`, `rounds/r6-tailcrop.png`, `rounds/r6-titlebars-x3.png`). Knee R2 → R1 (64): the tail now leaves the plate in one soft curve. Lockup cap 320, gap 7 u. Checked the three title bars at ×3: all three marks hold their 3 bricks at 16 px.

## Self-critique

- The 3 : 1 rule against the light taskbar is met only via the separate light set; the master `icon.svg` on a light background clears 3 : 1 on just ~39 % of its area (the moss plate itself is 4.45, so the silhouette is intact).
- **clay on moss is 1.34 : 1.** The grey brick is separated from the plate only by hue and by the soft shadow; at 24/32 on the dark set it reads as a dull grey patch rather than a crisp brick. It is the weakest colour relation in the icon.
- Visible moss (rim + gutters + tail) is 30 % of the silhouette — the brief's 18 % cap is written for strokes; the plate is a layer, but by my estimate (perimeter × 40 px) the rim alone is still about 20 %. The icon is a bit "framed".
- The tail is the classic bottom-right speech-bubble wedge with a straight outer edge. It is cleaner than concept-2's (45°, R1 knee, stair at small sizes) but it is still the familiar chat-bubble silhouette; reading-order is helped by the treemap, not by the outline.
- At 256 the composition (big block + tall column) still has a faint "dashboard / app layout" read.
- The 5.5 u smoothed brick corners are concentric with R0 only approximately (smoothing does not offset exactly); the rim thickens by a few px through the corner — invisible below 512 px.

## Files

`icon.svg` (1024 master) · `icon-light.svg` · `icon-32.svg` `icon-24.svg` `icon-16.svg` (dark set) · `icon-32-light.svg` `icon-24-light.svg` `icon-16-light.svg` · `icon-16-A.svg` `icon-16-B.svg` `icon-16-C.svg` · `lockup.svg` `lockup-dark.svg` `lockup-A.svg` `lockup-B.svg` `lockup-C.svg` · `png/` (1024/256/64/48 masters, 32/24/16 hand-tuned, 16/24 ×4, naive downscales, centroid overlay) · `sheet.png` · `metrics.json` · `build.py` (+ `geom.py`) · `render.py` (+ `rast.py`) · `rounds/` (every iteration's renders).

Rebuild: `D:\conda\miniconda3\python.exe build.py` then `render.py` (run from this folder).
