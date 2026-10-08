# Round 2 · Variant 2 · 长出来的尾巴 Grown Tail

一句话：一块青柠色的砖本身就是对话气泡，尾巴是它左边那条边直接长下来的，右侧切进一深一浅两块小砖：磁盘开口，告诉你每一块是什么。

## Construction (64-unit grid, 1 u = 16 px)

| item | value |
|---|---|
| body | 52 × 44 u (x 6.5–58.5, y 8.5–52.5); half-unit offset chosen so the alpha centroid lands on target |
| tail | 8 u at 45° (≤ width/4 = 13 u). Outer edge = the body's left side carried straight down (tangent continuation of the top-left R0 curve); inner edge 45°; knee R1, tip R3 |
| split | lime 32 u : tiles 20 u = 1.6 (≈ φ) |
| tiles | big tile 28 u, lime bridge 2 u, small tile 14 u (2 : 1) |
| radii | R0 8 u with 60 % Figma corner smoothing (TL, TR, BR) · R1 4 u tail knee · R2 2 u the two tile corners that sit inside the lime · R3 1 u tail tip. Nothing outside the chain. |
| line widths | none. The only "gap" is the 2 u lime bridge (§3.3 no-stroke rule). |
| gradient | one, lime only, 120°, `#e0f766 → #c8e93a` (mid = `#d4f24a`), used at ≥ 32 px; 16/24 flat `#d4f24a` |

Corner smoothing is implemented in `geom.py` (port of the figma-squircle corner parameters: two cubics + a shortened arc per corner).

## Colour

| version | body | big tile | small tile |
|---|---|---|---|
| main (master, dark taskbar, ≥ 48 everywhere) | lime `#d4f24a` | moss `#5c7a1a` | clay `#8a857a` |
| `*-light` (light taskbar / lightunplated) | ink `#141412` | lime | clay |
| title bar A | `#2f7d5b` | `#1c1b18` | `#1c1b18` |
| title bar B | `#f3ece2` | `#e4bc70` | `#e4bc70` |
| title bar C | lime + 1 px ink outline | ink | paper |

- Adjacencies in the main icon: lime/moss 3.90, lime/clay 2.90; moss/clay never touch (lime bridge). Paper never touches lime anywhere (§3.4). Coral is gone.
- Why the small tile is clay and not paper: in this variant every tile is bordered by lime, so paper would sit directly against lime (1.12 : 1, forbidden).
- Why the light version is ink, not moss: tiles reach the silhouette edge, so whatever sits there has to pass on `#f3f3f3`. With a moss body the only colour that contrasts with moss *and* passes on light is ink; moss/clay is 1.34. Inverting the values (body dark, big tile light) keeps the lime brand colour, keeps the silhouette identical, and passes 78 %.

## Measurements (`measure.py` on `png/icon@1024.png`)

- alpha centroid (512.1, 494.9): target (512 ± 8, 496 ± 8) ✓
- bbox 104–935 × 136–944, inside the 64–960 safe area (tail tip 944 < 960)
- area shares: lime 64.5 %, moss 22.2 %, clay 13.4 %
- area ≥ 3 : 1 — main icon: 100 % on `#202020`, **36 % on `#f3f3f3`** (fails the 50 % rule on its own; see critique) · light version: 78 % on `#f3f3f3`

## Small sizes (hand-placed pixels; maps in `wip/pix-16|24|32.txt`)

| | 16 | 24 | 32 |
|---|---|---|---|
| body | 16 × 13 | 22 × 19 | 30 × 26 |
| lime / tiles | 10 / 6 | 14 / 8 | 18 / 12 |
| big / bridge / small | 8 / 1 / 4 | 12 / 1 / 6 | 16 / 2 / 8 |
| outer corner | 2 px (half pixels on the diagonal) | 3 px | 4 px |
| tail | 2-step 45° staircase | 3-step + half-pixel knee | 4-step + half-pixel knee |
| tile R2 corners | square | square | 1 px half-blend |
| gradient | no | no | yes (lime) |

Every axis-aligned edge is on a whole pixel; the only partial pixels are on corners and the tail knee. Sheet column "naive" shows the 1024 master squashed to the same sizes for comparison.

## Iterations

1. **Tail construction × tile embedding** (`wip/explore1.png`). Tried the brief's literal tail (outer edge = 45° tangent out of an R1 corner, inner edge vertical) and a classic tail (left side straight down, 45° inner edge), each with tiles inset in the lime vs tiles cut to the silhouette edge. Saw: the literal tail makes a peg/stair under the body; inset tiles read as a remote control / device panel; the 52 × 40 body read as a TV. Kept: classic tail (it *is* the brick's own side growing down), edge-cut tiles.
2. **Proportions** (`wip/explore2.png`). Squarer body 52 × 44, five layouts: right column, inverted column, top band (looked like an app window — dropped), three cut tiles (busy at 16), left-pointing tail (looked like a foot). Kept the right column, big tile on top so the heavy tile balances the tail diagonally. Also saw the moss-body/lime+paper light version lose its bottom-right corner on `#f3f3f3` (paper 1.02 : 1).
3. **Split ratio + light palette** (`wip/explore3.png`, `wip/r3a-look.png`). 26 | 26 made the right half dull and static; 32 | 20 (φ) lets the lime dominate. Light version: ink/moss/clay was murky, ink/lime/clay keeps the brand and passes. Centroid already on target, so no move needed.
4. **First full sheet** (`wip/r3-zoom-*.png`). Small sizes crisp vs. naive blur. Title bar B with knocked-out tiles read as a gold flag/"F" — changed to cream body + gold tiles. Title bar C with outline + 1 px hard shadow turned into an ink blob and a broken tail — redrew it as outline only, ink big tile, paper small tile, tail with a 1 px lime channel.
5. **Master detail at 1024** (`wip/r4-master-detail.png`). The R2 corner where the moss tile met the top edge left a lime sliver and a dent in the silhouette, and the bridge flared at the right edge. Rule changed to "tiles are the treemap clipped by the bubble": a tile corner on the silhouette takes the silhouette's geometry; R2 only on the two corners that sit inside the lime. Top and right edges are now unbroken. Pixel versions follow the same rule.
6. **1× taskbar check** (`wip/r5-tb-*-x3.png`). 16 on dark: three blocks countable, tail visible as a 2 px nub. 16 light version: complete outline. No further change.

## Files

`icon.svg` (1024 master) · `icon-light.svg` · `icon-flat.svg` (no gradient) · `icon-16/24/32.svg` + `-light` · `mark-16-A/B/C.svg` · `lockup.svg`, `lockup-dark.svg`, `lockup-A/B/C.svg` (Geist 600, outlined, k–W −38) · `png/` (16–1024 main + light, ×4 for 16/24, naive downscales, centroid overlay) · `sheet.png` · `build.py`, `geom.py`, `render.py`, `measure.py`.

## Critic fix (2026-10-07, round-2 review)

- `mark-16-C.svg` / `lockup-C.svg`: the ink big tile merged with the 1 px outline into a 7 × 6 px black mass and the theme-C mark read as a gold-less "flag / F", not a bubble with tiles. Recoloured in `build.py` (`THEME["C"]`, `C16`): big tile paper, small tile lime, 1 px ink gap lines carry the structure (same approach as variant 1's C mark). Outline and tail unchanged. Previous file kept at `wip/mark-16-C.before-critic.svg`.
