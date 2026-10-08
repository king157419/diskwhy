# concept-1 · “嗯？盒” (Huh? Box)

一句话：一个柠檬绿小盒子歪着头、掀开盖子，好奇地“嗯？”一声，意思是“打开你的磁盘，看看里面到底是什么”。

## Rules applied (and where they come from)

1. **Use one metaphor, two at most, with no decorative extras.** Here the box with its lid popped open stands for "look inside your disk", and the tilted head plus the "o" mouth stand for "why?". There are no sparkles, motion lines or text.
   Source: Microsoft, *Design guidelines for Windows app icons*, section Metaphor / Representation. https://learn.microsoft.com/en-us/windows/apps/design/style/iconography/app-icon-design
2. **Keep text out of the icon.** The name sits next to the icon in Windows, so it belongs only in the lockup. Microsoft says "Icons should not include typography", and Apple also advises against text. Sources: same Microsoft page, and Apple HIG App icons https://developer.apple.com/design/human-interface-guidelines/app-icons
3. **Use few shapes, few corners and a recognisable outline.** The mark has two rounded rectangles (body and lid), two eyes and a mouth. Its outline (a box with a slanted bar above it) still reads when the whole icon is filled in one colour. Source: Microsoft, section Shape / Silhouette. Also David Airey, *Logo Design Love*, "Aim for distinction… so recognizable that just its shape or outline gives it away". https://www.logodesignlove.com/images/books/Logo_Design_Love_free_chapter.pdf
4. **Use rounded corners for an approachable feel.** Microsoft gives a 2/48 outer radius. This mark uses a much softer radius on purpose to suit theme C, and every corner stays round.
   Source: Microsoft, section The grid and rounded corners.
5. **Make sure it reads on both light and dark taskbars.** Microsoft asks that at least half the icon pass 3:1 contrast on both themes and that colours cover dark, medium and light values. The lime fill passes on #202020, and the ink outline and shadow pass on #f3f3f3. The background is transparent, which Microsoft recommends because it handles theming for free.
   Sources: Microsoft, section Color contrast; and *Construct your Windows app's icon*, section Transparent backgrounds. https://learn.microsoft.com/en-us/windows/apps/design/style/iconography/app-icon-construction
6. **Draw separate artwork for the small sizes.** The title bar uses 16 px and the taskbar uses 24 px at 100% scaling. Windows looks for an exact size before it scales a larger one down, so 16 and 24 are hand-pixelled grids (`mark-16.txt`, `mark-24.txt`) rather than shrunk vectors. At those sizes the body is upright, the lid is stepped and the mouth is dropped at 16 px. The 32 px and larger sizes come from the vector. Ship at least 16, 24, 32, 48 and 256.
   Source: Microsoft, app-icon-construction, sections Icon sizes and Icon scaling.
7. **Use few colours and no gradients.** The icon uses lime #d4f24a, ink #141412, and white only for the two eye highlights. Microsoft says gradients and tints "should be kept to a minimum". Paul Rand's test is that a logo must be "simple, scalable and work in black and white", which is what `mark-mono.svg` is for.
   Sources: Microsoft, section Color and gradients; Paul Rand principles summary https://99designs.com/blog/famous-design/4-principles-by-paul-rand-that-may-surprise-you/
8. **Keep it simple, memorable and relevant, and give it one feature.** The one feature is the popped lid. Source: Airey, *Logo Design Love*, the seven ingredients.
9. **Look at it straight on and keep it flat.** The only depth cue is theme C's hard offset shadow, a flat ink shape behind the body. There is no 3/4 view and no highlights.
   Source: Microsoft, sections Perspective / Layering and shadow.

## Files

| File | What |
|---|---|
| `mark.svg` | Main app icon, 1024 viewBox, transparent background, theme C hard shadow |
| `mark-24.svg` / `mark-24.txt` | Hand-pixelled 24 px icon for the taskbar at 100% (`.txt` is the editable grid) |
| `mark-16.svg` / `mark-16.txt` | Hand-pixelled 16 px icon for the title bar and tray |
| `mark-mono.svg` | One-colour version (`currentColor`): no shadow, face knocked out, lid separated by a gap |
| `mark-16-mono.svg` / `.txt` | One-colour 16 px version, e.g. green in the A title bar, gold in B |
| `lockup.svg`, `lockup-on-dark.svg` | Mark plus "DiskWhy" in Geist Bold 700 (OFL, already bundled), outlined to paths |
| `png/` | Rasters: 1024, 256, 64, 48, 32, plus vector-scaled 24/16 (`mark-24.png`, `mark-16.png`) and hand-pixelled 24/16 (`mark-24-24.png`, `mark-16-16.png`) |
| `sheet.html` → `sheet.png` | Preview sheet: light and dark taskbars, size ladder with ×4 nearest-neighbour zoom, A/B/C title bars, lockups |
| `render.py`, `make_pixels.py`, `pixel2svg.py`, `build_lockup.py` | Rebuild scripts. Run with `D:\conda\miniconda3\python.exe render.py mark.svg:1024,256,64,48,32,24,16 mark-24.svg:24 mark-16.svg:16 sheet.html` (uses Playwright with installed Edge) |
| `wip/` | Earlier rounds (v2: level box, v3: first head tilt), plus single-colour copies that the sheet uses |

## Iteration log

- **r1:** A lime rounded square with a black bottom band (a drive LED), two eyes and a raised eyebrow. It looked like a generic robot or battery, the black band made it bottom-heavy, and the eyebrow disappeared below 32 px. Rejected.
- **r2:** A box with a slightly lifted lid, tried in three colourways. The lid looked like it was resting on the box, and the face had no expression.
- **r3:** Added the head tilt (−7°) and the "o" mouth. This gave the character its "huh?" personality. The lid shadow filled the gap, so the lid looked stuck to the body.
- **r4–r6:** Removed the lid shadow. Fixed the hinge direction so it is hinged on the low side and opens on the high side. Centred the mark from its measured bounding box and scaled it to 0.93. Hand-pixelled the 16 and 24 sizes and mirrored their lids to match.
- **r7:** Lowered the lid onto its hinge so it reads as opening, not floating. Added the mono version with a carved gap around the lid.
