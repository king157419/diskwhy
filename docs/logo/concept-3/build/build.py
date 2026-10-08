"""Build concept-3 ("Di" lettermark): SVG sources, PNG previews, and the preview sheet.

Run:  D:\\conda\\miniconda3\\python.exe build.py
"""
import pathlib, sys
HERE = pathlib.Path(__file__).parent
sys.path.insert(0, str(HERE))
from geom import rrect
from render import Renderer
from wordmark import outline

ROOT = HERE.parent
PNG = ROOT / "png"
PNG.mkdir(exist_ok=True)

INK, LIME, PAPER = "#141412", "#d4f24c", "#f4f0e6"
A_INK, A_GREEN, A_PAPER = "#1c1b18", "#2a7a5a", "#fbfaf7"
B_BG, B_GOLD, B_TILE = "#14110e", "#ebc47a", "#221d18"


def svg(body, w=1024, h=None, vb=None, extra=""):
    h = h or w
    vb = vb or f"0 0 {w} {h}"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{vb}"{extra}>\n'
            f'{body}\n</svg>\n')


def d_outer(x, y, w, h, rl):
    """Letter D: flat stem on the left (soft corners), true semicircle bowl on the right."""
    r = h / 2
    return (f"M{x + rl:g} {y:g}H{x + w - r:g}A{r:g} {r:g} 0 0 1 {x + w - r:g} {y + h:g}"
            f"H{x + rl:g}A{rl:g} {rl:g} 0 0 1 {x:g} {y + h - rl:g}V{y + rl:g}"
            f"A{rl:g} {rl:g} 0 0 1 {x + rl:g} {y:g}Z")


# ---------- master geometry (1024 artboard) -------------------------------------------
# D 576 x 704. Counter = an "i": a flat slice on top (the dot) + a tall slot (the stem).
# Walls: left 192, right (at the waist) 224, top 152, bottom 152 — the bowl is a touch
# heavier than the stem, as round strokes are in type.
D = d_outer(224, 160, 576, 704, 44)
DOT = rrect(416, 312, 160, 80, 24, 24, 24, 24)
STEM = rrect(416, 456, 160, 256, 20, 36, 36, 20)
# optical centring of the letter on the tile (tile centre is 496,496; the D's mass sits left)
MARK_T = 'transform="translate(-8 -12) translate(512 512) scale(0.9) translate(-512 -512)"'
TILE = rrect(72, 72, 848, 848, 188, 188, 188, 188)


def letter_cut(fill):
    """D with the i punched out (single colour, true holes)."""
    return f'<path {MARK_T} fill="{fill}" fill-rule="evenodd" d="{D}{DOT}{STEM}"/>'


def icon_sticker():
    """Primary app icon — C theme sticker: lime plate, ink outline + hard offset shadow, ink D."""
    return (f'<path fill="{INK}" transform="translate(32 32)" d="{TILE}"/>'
            f'<path fill="{LIME}" stroke="{INK}" stroke-width="36" d="{TILE}"/>'
            + letter_cut(INK))


def icon_theme(tile, letter, dot=None, line=None):
    """Flat colourway for themes A/B: plate + letter, dot optionally in the accent colour."""
    s = f'<path fill="{tile}" d="{rrect(64, 64, 896, 896, 200, 200, 200, 200)}"'
    s += f' stroke="{line}" stroke-width="12"/>' if line else "/>"
    s += letter_cut(letter)
    if dot:
        s += f'<path {MARK_T} fill="{dot}" d="{DOT}"/>'
    return s


# ---------- hand-tuned small sizes (pixel grid) ----------------------------------------
def icon_24():
    # tile 0..22 (+1px shadow to 23); inner lime 1..21; D 12x14 at (5,4); walls 4/3/5, 3/2/1/5/3
    return (f'<rect x="1" y="1" width="22" height="22" rx="5" fill="{INK}"/>'
            f'<rect x="0" y="0" width="22" height="22" rx="5" fill="{INK}"/>'
            f'<rect x="1" y="1" width="20" height="20" rx="4" fill="{LIME}"/>'
            f'<path fill="{INK}" fill-rule="evenodd" d="{d_outer(5, 4, 12, 14, 1)}'
            f'M9 7h3v2h-3zM9 10h3v5h-3z"/>')


def icon_32():
    # critic pass: hand-tuned 32 (the plain downscale of icon.svg went soft on the D bowl).
    # tile 0..30 (+1px shadow to 31), 1px ink rim, D 16x20 at (7,5), walls 6/6/4/4, i = 4x2 dot + 4x8 stem
    return (f'<rect x="1" y="1" width="30" height="30" rx="7" fill="{INK}"/>'
            f'<rect x="0" y="0" width="30" height="30" rx="7" fill="{INK}"/>'
            f'<rect x="1" y="1" width="28" height="28" rx="6" fill="{LIME}"/>'
            f'<path fill="{INK}" fill-rule="evenodd" d="{d_outer(7, 5, 16, 20, 1.5)}'
            f'M13 9h4v2h-4zM13 13h4v8h-4z"/>')


def icon_16():
    # no shadow at 16 (Microsoft: simplify small sizes); tile 0..16, 1px outline, inner 1..15
    # D 8x10 at (4,3); walls 3/2/3 ; 2/1/1/4/2
    return (f'<rect x="0" y="0" width="16" height="16" rx="3.5" fill="{INK}"/>'
            f'<rect x="1" y="1" width="14" height="14" rx="2.5" fill="{LIME}"/>'
            f'<path fill="{INK}" fill-rule="evenodd" d="{d_outer(4, 3, 8, 10, 0.5)}'
            f'M7 5h2v1h-2zM7 7h2v4h-2z"/>')


def mark_16(letter, counter=None, dot=None):
    """Bare D for the title bar, 13 x 16 px, pixel-snapped. counter/dot fill the i if given."""
    body = (f'<path fill="{letter}" fill-rule="evenodd" d="{d_outer(0, 0, 13, 16, 1)}'
            f'M4 3h4v2h-4zM4 6h4v7h-4z"/>')
    if counter:
        body += f'<path fill="{counter}" d="M4 6h4v7h-4z"/>'
    if dot:
        body += f'<path fill="{dot}" d="M4 3h4v2h-4z"/>'
    return body


# ---------- wordmark -------------------------------------------------------------------
def wordmark(ink, dot, wght=700, dot_stroke=None):
    """'DiskWhy' outlined from Geist; the i's round dot is swapped for a flat slice (echo of
    the mark's dot) in the accent colour. Returns (svg_body, width, cap) in font units."""
    glyphs, width, cap, upm = outline("DiskWhy", wght=wght, tracking=-18)
    body = []
    for ch, paths, x in glyphs:
        # keep all contours of a glyph in ONE path so counters (D, h...) stay open
        keep = "".join(d for i, (d, b) in enumerate(paths) if not (ch == "i" and i == 1))
        body.append(f'<path transform="translate({x} 0) scale(1 -1)" d="{keep}" fill="{ink}"/>')
        for i, (d, b) in enumerate(paths):
            if ch == "i" and i == 1:
                x0, y0, x1, y1 = b
                # flat slice: same width as the i stem, a bit shorter than the round dot
                sx0, sx1 = 68.5, 205.3
                body.append(f'<rect x="{x + sx0:.1f}" y="{-(604 + 104):.1f}" width="{sx1 - sx0:.1f}" '
                            f'height="104" rx="16" fill="{dot}"'
                            + (f' stroke="{dot_stroke}" stroke-width="34" paint-order="stroke"' if dot_stroke else "")
                            + "/>")
    return "".join(body), width, cap


def lockup(tile_body, ink, dot, bg=None, dot_stroke=None):
    """Horizontal lockup: 1024-unit icon scaled to the lockup height + wordmark."""
    wm, ww, cap = wordmark(ink, dot, dot_stroke=dot_stroke)
    H = 400                       # lockup height (icon height)
    cap_h = 168                   # wordmark cap height ~ 0.42 x icon (reads as a pair)
    s = cap_h / cap
    gap = 92
    icon_s = H / 1024
    text_x = H + gap
    baseline = H / 2 + cap_h / 2 - 6
    W = int(text_x + ww * s + 24)
    out = ""
    if bg:
        out += f'<rect width="{W}" height="{H}" fill="{bg}"/>'
    out += f'<g transform="scale({icon_s})">{tile_body}</g>'
    out += f'<g transform="translate({text_x} {baseline}) scale({s})">{wm}</g>'
    return svg(out, W, H)


# ---------- write SVGs -----------------------------------------------------------------
files = {
    "icon.svg": svg(icon_sticker()),
    "icon-32.svg": svg(icon_32(), 32),
    "icon-24.svg": svg(icon_24(), 24),
    "icon-16.svg": svg(icon_16(), 16),
    "icon-mono.svg": svg(letter_cut(INK)),
    "icon-theme-A.svg": svg(icon_theme(A_PAPER, A_INK, A_GREEN, "#e1ddd4")),
    "icon-theme-B.svg": svg(icon_theme(B_TILE, B_GOLD)),
    "icon-theme-ink.svg": svg(icon_theme(INK, LIME)),
    "mark-16-A.svg": svg(mark_16(A_INK, dot=A_GREEN), 13, 16),
    "mark-16-B.svg": svg(mark_16(B_GOLD), 13, 16),
    "mark-16-C.svg": svg(mark_16(INK, counter=LIME, dot=LIME), 13, 16),
    "lockup-light.svg": lockup(icon_sticker(), INK, LIME, dot_stroke=INK),
    "lockup-dark.svg": lockup(icon_sticker(), PAPER, LIME, bg=None),
}
for name, text in files.items():
    (ROOT / name).write_text(text, encoding="utf8")

# ---------- render ---------------------------------------------------------------------
SHEET = ROOT / "build" / "sheet.html"
TBAR = ROOT / "build" / "titlebars.html"

TITLEBARS = f"""<!doctype html><meta charset=utf-8><style>
@font-face{{font-family:Geist;src:url(Geist-latin.woff2)}}
@font-face{{font-family:DWKai;src:url(../../../../src/assets/fonts/DiskWhyKai-Bold.woff2)}}
body{{margin:0;background:transparent;width:420px}}
.tb{{height:40px;display:flex;align-items:center;gap:9px;padding:0 14px;font:500 13px Geist,sans-serif;letter-spacing:.01em}}
.tb img{{width:13px;height:16px;display:block}}
.ctl{{margin-left:auto;display:flex;gap:28px;font:12px sans-serif;opacity:.6}}
</style>
<div class=tb style="background:#f3f1ec;color:#5f5b53"><img src="../mark-16-A.svg">DiskWhy<span class=ctl><span>&#8212;</span><span>&#9744;</span><span>&#10005;</span></span></div>
<div class=tb style="background:#1b1713;color:#b6ab9c"><img src="../mark-16-B.svg">DiskWhy<span class=ctl><span>&#8212;</span><span>&#9744;</span><span>&#10005;</span></span></div>
<div class=tb style="background:#f4f0e6;color:#141412;font:700 15px DWKai,Geist,sans-serif"><img src="../mark-16-C.svg">DiskWhy<span class=ctl><span>&#8212;</span><span>&#9744;</span><span>&#10005;</span></span></div>
"""
TBAR.write_text(TITLEBARS, encoding="utf8")

with Renderer() as r:
    for s in (256, 128, 64, 48, 32, 24, 16):
        r.svg(ROOT / "icon.svg", s, PNG / f"icon-auto-{s}.png")
    r.svg(ROOT / "icon-32.svg", 32, PNG / "icon-32.png")
    r.svg(ROOT / "icon-24.svg", 24, PNG / "icon-24.png")
    r.svg(ROOT / "icon-16.svg", 16, PNG / "icon-16.png")
    for n in ("icon-mono", "icon-theme-A", "icon-theme-B", "icon-theme-ink"):
        r.svg(ROOT / f"{n}.svg", 160, PNG / f"{n}-160.png")
        r.svg(ROOT / f"{n}.svg", 24, PNG / f"{n}-24.png")
    r.svg(ROOT / "icon.svg", 160, PNG / "icon-160.png")
    r.page.set_viewport_size({"width": 420, "height": 120})
    r.page.goto(TBAR.resolve().as_uri())
    r.page.evaluate("document.fonts.ready")
    r.page.wait_for_timeout(300)
    r.page.screenshot(path=str(PNG / "titlebars.png"), clip={"x": 0, "y": 0, "width": 420, "height": 120},
                      omit_background=True)
    r.html(SHEET, ROOT / "sheet.png", 1600)
print("built")
