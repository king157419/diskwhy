# Round 2 · Variant 1 · 留白气泡 Quiet Grid

一句话：三块大小不等的砖（青柠、苔绿、陶灰）拼成一个对话气泡，砖缝是透明的，尾巴从苔绿砖的左下角弯出来——先看到"磁盘被分成几块"，再看到"它在说话"。

## Files
- `icon.svg` 1024 master (dark-taskbar / neutral colours) · `icon-light.svg` same outline, light-taskbar colours
- `icon-48.svg` `icon-32.svg` `icon-24.svg` `icon-16.svg` hand-set on their own pixel grids (dark taskbar, "altform-unplated")
- `icon-48-light.svg` … `icon-16-light.svg` identical outlines, light taskbar ("altform-lightunplated")
- `mark-16-A.svg` `mark-16-B.svg` `mark-16-C.svg` title-bar marks per app theme
- `lockup.svg` (light bg) · `lockup-dark.svg` · `lockup-A.svg` `lockup-B.svg` `lockup-C.svg` (16 px title-bar lockups); wordmark = Geist 600 (A/B/C 620), outlined to paths, k–W −38 optical kern
- `png/` 1024 / 256 / 64 (master), 48 / 32 / 24 / 16 (hinted), light versions, `naive@*` downscales for comparison, `*-x4.png` nearest-neighbour enlargements of 16 / 24, `centroid-overlay.png`
- `sheet.png` (1600 px wide) · `build.py` (all SVGs) · `render.py` (PNGs + sheet) · `geom.py` (Figma corner smoothing + tangent fillets) · `measure.py` (centroid, area share) · `rast.py` (Playwright + Edge rasteriser) · `wip/` (round 1–3 contact sheets: `r1.png`, `r2.png`, `r3.png`, `r3b.png`)

## System (BRIEF §3)
| | value |
|---|---|
| grid | 64 u, 1 u = 16 px; every coordinate an integer unit → the 64 px render is pixel-exact |
| body | 53 × 38 u (83 % of the width), x 6→59 u, y 12→50 u; tail to 58 u (inside the 4 u safe area) |
| treemap | slice-and-dice: lime row 53 × 20 u · 2 u gap · bottom row 16 u = moss 33 u + 2 u + clay 18 u |
| areas | lime : moss : clay = 1060 : 528 : 288 ≈ 3.7 : 1.8 : 1 — the same ×2 chain as the radii |
| radii | R0 8 u, Figma smoothing 60 % (the three outer corners) · R1 4 u (tail bend) · R2 2 u (every corner at a gap + tail knee) · R3 1 u (tail tip). Nothing outside the chain. |
| line | no stroke; one gap width 2 u, transparent (taskbar shows through) |
| tail | grows out of the moss brick: its left edge bends 45° (R1) into a straight 45° edge (tangent continuation of that arc) to the tip (R3); inner edge vertical, concave R2 knee. Depth 8 u = 15 % of body width (limit 25 %). |
| colour | dark/master: lime `#d4f24a` · moss `#5c7a1a` · clay `#8a857a`; light set: every role one step darker → moss · ink `#141412` · clay. Coral gone, paper not used (it vanishes on light backgrounds without a stroke). |
| gradient | one: lime brick only, CSS 120°, `#e0f766 → #c8e93a` (mid = `#d4f24a`), ≥ 32 px; 16 / 24 flat `#d4f24a` |
| themes | A: big brick `#2f7d5b`, small bricks `#1c1b18` · B: big brick `#e4bc70`, small bricks `#f3ece2` · C: lime / lime / paper with 1 px ink outline (= the 1 px gap) + 1 px hard offset shadow, bricks shrunk 1 px to fit |

Small-size table (px): 
| | 48 | 32 | 24 | 16 |
|---|---|---|---|---|
| body | 40 × 28 | 30 × 22 | 22 × 17 | 16 × 12 |
| lime row / bottom row | 15 / 11 | 11 / 9 | 9 / 7 | 6 / 5 |
| moss / clay width | 25 / 13 | 19 / 9 | 14 / 7 | 10 / 5 |
| gap | 2 | 2 | 1 | 1 |
| R0 | 6 (smoothed) | 4 | 3 | 2 |
| tail (bend / depth) | 3 / 6 | 2 / 4 (4 steps) | 1 / 3 (3 steps) | 1 / 2 (2 steps) |
| gradient | yes | yes | no | no |

## Measurements (`measure.py` on `png/icon@1024.png`)
- alpha centroid **(511.3, 496.9)** — target (512 ± 8, 496 ± 8) ✓; alpha bbox (96, 192)–(943, 904)
- silhouette covers 45 % of the canvas
- area share, master: lime 56 % · moss 30 % · clay 15 %. On #202020: lime 12.9 : 1, clay 4.4, moss 3.3 → 100 % of the area ≥ 3 : 1; lime + clay = 71 % (brief: lime + paper/clay ≥ 50 %) ✓
- light set: moss 56 % (4.45 : 1) · ink 30 % (16.6) · clay 15 % (3.31) → 100 % ≥ 3 : 1 on #f3f3f3 ✓
- master colours on #f3f3f3: only moss + clay (45 %) pass 3 : 1; the lime row reads by hue only → that is why the light set exists
- 16 px dark: 3 bricks countable (lime / moss / clay, 1 px gaps); 16 px light: outline complete

## Iterations
1. **Round 1 — build the system, try two tails** (`wip/r1.png`). Wrote `geom.py` (Figma corner smoothing per figma-squircle + tangent fillets) and the brief's φ layout (lime column 32 u | right column 18 u split 24/12). Tails: T1 = vertical outer edge + 45° inner edge (the classic chat tail) vs T2 = the left edge bends 45° with R1 and runs on to the tip. Saw: T1 reads "chat app" immediately; T2 is the brief's "grown from the corner" and less generic. Paper vanished completely on #f3f3f3 (1.02 : 1) — with no stroke the bottom-right brick simply disappeared → master switched to clay. Centroid (495, 468): 17 px left and 28 px high.
2. **Round 2 — centre and tune the tail** (`wip/r2.png`). Moved the body +1 u / +2 u → centroid (511, 500). Swept the T2 tail (bend 2/4/6/8 u, depth 6/8/10 u); 4 u bend + 8 u depth was the only one where the bend still reads as the brick's corner and the tail does not dominate. Chose lime / moss / clay as the master.
3. **Round 3 — the composition was still "chat first"** (`wip/r3.png`, `r3b.png`). Looking at the round-2 render at 256 px: the lime column plus its tail is a complete chat bubble on its own and the right column reads like a sidebar beside it — exactly the §1.8 problem. Tried four layouts: lime column (L1), lime row on top with moss + clay below (L2, two row splits), lime + moss column beside a clay column (L4, too much grey), moss + clay above a lime row with the tail (L6, the lime row again becomes a bubble by itself). L2 is the only one where no single brick is a bubble — the bubble only exists as the sum of the three, and the top row doubles as the app's capacity bar. Tuned it to 33 | 17 and 20 | 16 so the areas follow the radius chain's ×2 steps (≈ 4 : 2 : 1).
4. **Round 4 — rebuild for real + small sizes** (`sheet.png` v1–v2). Rewrote `build.py` / `render.py`; drew 16 / 24 / 32 on their own grids (integer edges, 45° stairs). Centroid was now (519, 497) — inside tolerance but at the edge; rather than use a half unit (which would blur the 64 px render) I widened the body to 53 u at x0 = 6 u → (511.3, 496.9) with every edge still on a whole unit. Title-bar C: first try (lime / ink / paper) merged the ink brick with the outline and shadow into one black blob at 16 px → changed to lime / lime / paper so the ink lines are the structure.
5. **Round 5 — 48 px and the sheet.** The 48 px render of the master put the 2 u gap on 1.5 px → a soft grey seam and soft edges (seen ×8 in `wip/z48-64.png`); added a hand-set `icon-48.svg` (2 px gaps, integer edges, smoothed 6 px corners) and show it beside the naive downscale. Rewrote the sheet note after re-reading it: my earlier caption overstated how well lime holds on a light taskbar.

## Self-critique
- **The tail is on the dimmest brick on dark taskbars.** Moss is 3.30 : 1 on #202020; at 16 px dark the speech cue (2 px moss stairs) is the weakest thing on the icon. Putting the tail on lime brought back the "lime alone is a bubble" problem (L1/L6), so I accepted it.
- **Wide, flat proportion.** 53 × 46 u including the tail; in a square taskbar slot it looks a bit smaller than square icons, and at 16 px it is 16 × 14.
- **May read as a "layout / dashboard / stacked bar" icon** before it reads as a speech bubble. That is the deliberate trade (bricks first), but the bubble reading leans entirely on one small tail.
- **No lime on light taskbars.** The light set drops the brand colour. Also: a Win32 `.ico` cannot switch by taskbar theme (only MSIX `altform-*` assets can), so a Tauri build has to pick one set; the master colours on a light taskbar rely on hue only.
- **Rule conflict:** the moss brick carries R2 corners plus the tail's R1 bend and R3 tip — three radii on one brick, which the brief's own tail spec forces but its "≤ 2 radii per brick" rule forbids.
- **C title-bar mark is the busiest:** outline + shadow eat most of 16 px; the tail is 1–2 lime pixels inside ink.
- **Not verified:** the MangoDisk side-by-side and the WeChat / Messenger / LINE row (no reference images used here).
