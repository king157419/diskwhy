r"""Variant 2 · Grown Tail (长出来的尾巴) — generator for every SVG in this folder.

One lime squircle *is* the speech bubble; its tail is the straight continuation of its own left
side; two treemap tiles are cut into its right side and separated by a lime bridge.

Run:  D:\conda\miniconda3\python.exe docs/logo/round2/variant-2/build.py
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from geom import path, rect  # noqa: E402

ROOT = HERE.parents[3]

# ---- palette (BRIEF §3.4) ---------------------------------------------------
INK, MOSS, LIME, PAPER, CLAY = "#141412", "#5c7a1a", "#d4f24a", "#f6f1e4", "#8a857a"
LIME_HI, LIME_LO = "#e0f766", "#c8e93a"  # the one allowed gradient (lime only, 120deg, mid = LIME)

# colour roles per version: body (the lime brick / bubble), big tile, small tile
PAL = {
    "dark": dict(body=LIME, t1=MOSS, t2=CLAY),   # master + dark taskbar
    "light": dict(body=INK, t1=LIME, t2=CLAY),   # light taskbar: value-inverted (body dark, big tile light)
}

# ---- 1024 master on a 64-unit grid (1 u = 16 px) ------------------------------
U = 16
P = dict(
    x0=6.5, x1=58.5,      # body 52 u wide
    y0=8.5, y1=52.5,      # body 44 u tall
    T=8,                  # tail: 8 u at 45 deg (<= width/4)
    split=38.5,           # left edge of the tile column: lime 32 u | tiles 20 u (32:20 = 1.6 ~ phi)
    cut=36.5,             # top of the small tile; big tile = 28 u, bridge 2 u, small = 14 u (2:1)
    gap=2,
    R0=8, S=0.6, R1=4, R2=2, R3=1,
)


def master_parts(p=P, grad=True, pal="dark"):
    u = lambda v: v * U
    R0 = ("smooth", u(p["R0"]), p["S"])
    R1, R2, R3 = ("arc", u(p["R1"])), ("arc", u(p["R2"])), ("arc", u(p["R3"]))
    x0, x1, y0, y1, T = (u(p[k]) for k in ("x0", "x1", "y0", "y1", "T"))
    sx, cy, g = u(p["split"]), u(p["cut"]), u(p["gap"])
    body = path([(x0, y0), (x1, y0), (x1, y1), (x0 + T, y1), (x0, y1 + T)], [R0, R0, R0, R1, R3])
    # tiles are the treemap clipped by the bubble: a tile corner lying on the silhouette takes the
    # silhouette's own geometry (R0 at a body corner, straight on a straight edge); only the two
    # corners fully inside the lime get R2.
    t1 = rect(sx, y0, x1, cy - g, 0, R0, 0, R2)
    t2 = rect(sx, cy, x1, y1, R2, 0, R0, 0)
    c = PAL[pal]
    defs = ""
    body_fill = c["body"]
    if grad and c["body"] == LIME:
        # 120deg (CSS convention: towards lower-right, light from top-left), one step
        defs = ('<defs><linearGradient id="lg" gradientUnits="userSpaceOnUse" '
                f'x1="{f2(x0)}" y1="{f2(y0)}" x2="{f2(x0 + (x1 - x0))}" y2="{f2(y0 + (x1 - x0) * 0.577)}">'
                f'<stop offset="0" stop-color="{LIME_HI}"/><stop offset="1" stop-color="{LIME_LO}"/>'
                '</linearGradient></defs>')
        body_fill = "url(#lg)"
    return defs + (f'<path d="{body}" fill="{body_fill}"/>'
                   f'<path d="{t1}" fill="{c["t1"]}"/>'
                   f'<path d="{t2}" fill="{c["t2"]}"/>')


def f2(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")


def svg_doc(inner, n, size=None, crisp=False):
    size = size or n
    sr = "crispEdges" if crisp else "geometricPrecision"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
            f'viewBox="0 0 {n} {n}" shape-rendering="{sr}">{inner}</svg>\n')


# ---- hand-drawn small sizes --------------------------------------------------
# Each size is laid out on its own whole-pixel grid (SMALL below), then the corner, knee and tail
# pixels are placed from hand-picked templates.  Characters, one per pixel:
#   .  empty          B body           b body at half alpha (silhouette edge pixel)
#   M  big tile       m big tile at half alpha over the body (inner R2 corner)  n big tile, half alpha, no body
#   C  small tile     c small tile at half alpha over the body                 d small tile, half alpha, no body
#   K  ink (cartoon theme outline / shadow)
HALF = 0.5

# top-left corner templates: '.' = cut, 'h' = half pixel, 'X' = keep.  Mirrored for TR / BR.
CORNER = {
    2: [".h", "hX"],
    3: ["..h", ".XX", "hXX"],
    4: ["..hX", ".hXX", "hXXX", "XXXX"],
}

# x0, x1, y0, y1: inclusive pixel indices of the body; split: first tile column;
# m_end: last row of the big tile; gap: bridge rows; k: tail steps (45 deg staircase);
# r: outer corner template; inner: draw the 1-px R2 tile corners; knee: half pixel at the knee
SMALL = {
    16: dict(x0=0, x1=15, y0=1, y1=13, split=10, m_end=8, gap=1, k=2, r=2, inner=0, knee=0),
    24: dict(x0=1, x1=22, y0=1, y1=19, split=15, m_end=12, gap=1, k=3, r=3, inner=0, knee=1),
    32: dict(x0=1, x1=30, y0=1, y1=26, split=19, m_end=16, gap=2, k=4, r=4, inner=1, knee=1),
}


def pixmap(n, g):
    G = [["." for _ in range(n)] for _ in range(n)]
    x0, x1, y0, y1, sx = g["x0"], g["x1"], g["y0"], g["y1"], g["split"]
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            G[y][x] = "B"
    for y in range(y0, g["m_end"] + 1):
        for x in range(sx, x1 + 1):
            G[y][x] = "M"
    for y in range(g["m_end"] + 1 + g["gap"], y1 + 1):
        for x in range(sx, x1 + 1):
            G[y][x] = "C"
    # tail: the left side continues straight down; the inner edge is a 45 deg staircase
    k = g["k"]
    for j in range(1, k + 1):
        for x in range(x0, x0 + k - j + 1):
            G[y1 + j][x] = "B"
    if g["knee"]:
        G[y1 + 1][x0 + k] = "b"
    # outer corners TL, TR, BR (BL is where the tail grows, so it stays square)
    T = CORNER[g["r"]]
    half_of = {"B": "b", "M": "n", "C": "d"}
    for (cx, cy, fx, fy) in ((x0, y0, 1, 1), (x1, y0, -1, 1), (x1, y1, -1, -1)):
        for ty, row in enumerate(T):
            for tx, t in enumerate(row):
                x, y = cx + fx * tx, cy + fy * ty
                if t == ".":
                    G[y][x] = "."
                elif t == "h":
                    G[y][x] = half_of[G[y][x]]
    if g["inner"]:  # 1-px R2 corners of the tiles where they meet the body
        me, cs = g["m_end"], g["m_end"] + 1 + g["gap"]
        for (x, y, ch) in ((sx, me, "m"), (sx, cs, "c")):
            G[y][x] = ch
    return ["".join(r) for r in G]


def pixel_svg(rows, pal, grad=False):
    c = pal if isinstance(pal, dict) else PAL[pal]
    n = len(rows)
    defs = ""
    body = c["body"]
    if grad and c["body"] == LIME:
        defs = (f'<defs><linearGradient id="pg" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="{n}" y2="{n * 0.577:.2f}">'
                f'<stop offset="0" stop-color="{LIME_HI}"/><stop offset="1" stop-color="{LIME_LO}"/></linearGradient></defs>')
        body = "url(#pg)"
    layer = {  # char -> stacked (fill, opacity) layers
        "B": [(body, 1)], "b": [(body, HALF)],
        "M": [(c["t1"], 1)], "m": [(body, 1), (c["t1"], HALF)], "n": [(c["t1"], HALF)],
        "C": [(c["t2"], 1)], "c": [(body, 1), (c["t2"], HALF)], "d": [(c["t2"], HALF)],
        "K": [(c.get("k", INK), 1)],
    }
    out = []
    for depth in (0, 1):
        for y, row in enumerate(rows):
            x = 0
            while x < n:
                ch = row[x]
                ls = layer.get(ch, [])
                if len(ls) <= depth:
                    x += 1
                    continue
                x2 = x
                while x2 + 1 < n and row[x2 + 1] == ch:
                    x2 += 1
                fill, op = ls[depth]
                if fill is not None:
                    o = "" if op == 1 else f' fill-opacity="{op}"'
                    out.append(f'<rect x="{x}" y="{y}" width="{x2 - x + 1}" height="1" fill="{fill}"{o}/>')
                x = x2 + 1
    return defs + "".join(out), n


# ---- theme title-bar marks (16 px) ---------------------------------------------
THEME = {
    "A": dict(body="#2f7d5b", t1="#1c1b18", t2="#1c1b18"),
    "B": dict(body="#f3ece2", t1="#e4bc70", t2="#e4bc70"),
    "C": dict(body=LIME, t1=PAPER, t2=LIME, k=INK),
}
# C speaks the cartoon theme's language: 1 px ink outline (K) and 1 px ink gap lines carry the structure;
# big tile paper, small tile lime. (Critic fix 2026-10-07: the earlier ink big tile merged with the outline
# into a 7 x 6 px black mass and the mark read as a flag / "F" rather than a bubble with tiles.)
C16 = """
................
.KKKKKKKKKKKKKK.
KBBBBBBBBKMMMMMK
KBBBBBBBBKMMMMMK
KBBBBBBBBKMMMMMK
KBBBBBBBBKMMMMMK
KBBBBBBBBKMMMMMK
KBBBBBBBBKMMMMMK
KBBBBBBBBKKKKKKK
KBBBBBBBBKCCCCCK
KBBBBBBBBKCCCCCK
KBBBBBBBBKCCCCCK
KBKKKKKKKKKKKKK.
KBK.............
KK..............
K...............
"""


def rows_of(txt):
    return [r for r in txt.strip("\n").split("\n")]


def write_small():
    (HERE / "wip").mkdir(exist_ok=True)
    for s, g in SMALL.items():
        rows = pixmap(s, g)
        (HERE / "wip" / f"pix-{s}.txt").write_text("\n".join(rows) + "\n", encoding="utf-8")
        for pal, suf in (("dark", ""), ("light", "-light")):
            inner, n = pixel_svg(rows, pal, grad=(s >= 32))
            (HERE / f"icon-{s}{suf}.svg").write_text(svg_doc(inner, n, crisp=True), encoding="utf-8")
    rows16 = pixmap(16, SMALL[16])
    for t in ("A", "B"):
        inner, n = pixel_svg(rows16, THEME[t])
        (HERE / f"mark-16-{t}.svg").write_text(svg_doc(inner, n, crisp=True), encoding="utf-8")
    inner, n = pixel_svg(rows_of(C16), THEME["C"])
    (HERE / "mark-16-C.svg").write_text(svg_doc(inner, n, crisp=True), encoding="utf-8")


# ---- wordmark + lockups --------------------------------------------------------
EXTRA_KERN = {("k", "W"): -38}  # Geist has no k/W pair


def word_paths(text="DiskWhy", wght=600, tracking=-10):
    """Outline `text` in Geist (OFL, bundled in src/assets/fonts); baseline y=0, font units."""
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    from fontTools.ttLib import TTFont
    from fontTools.varLib.instancer import instantiateVariableFont

    font = TTFont(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2")
    font = instantiateVariableFont(font, {"wght": wght})
    cmap = font.getBestCmap()
    gs = font.getGlyphSet()
    names = [cmap[ord(ch)] for ch in text]
    x, ds = 0, []
    for i, gn in enumerate(names):
        pen = SVGPathPen(gs)
        gs[gn].draw(TransformPen(pen, (1, 0, 0, -1, x, 0)))
        ds.append(pen.getCommands())
        x += gs[gn].width + tracking
        if i + 1 < len(names):
            x += EXTRA_KERN.get((gn, names[i + 1]), 0)
    return " ".join(ds), x - tracking


def lockup(mark_inner, n, text_fill, name, cap, gap, body, wght=600, crisp=False):
    """mark drawn on an n-grid; cap = cap height and gap = space after the body, both in grid units."""
    d, adv = word_paths(wght=wght)
    s = cap / 710.0  # Geist cap height = 710 units
    by0, by1, bx1 = body
    base = (by0 + by1) / 2 + cap / 2  # caps centred on the bubble body, not on the tail
    tx = bx1 + gap
    w = tx + adv * s + 0.02 * n
    sr = ' shape-rendering="crispEdges"' if crisp else ""
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.2f} {n}" height="{n}" width="{w:.2f}">'
           f'<g{sr}>{mark_inner}</g>'
           f'<path transform="translate({tx:.2f},{base:.2f}) scale({s:.5f})" fill="{text_fill}" d="{d}"/></svg>\n')
    (HERE / name).write_text(svg, encoding="utf-8")


def write_lockups():
    u = lambda v: v * U
    body = (u(P["y0"]), u(P["y1"]), u(P["x1"]))
    lockup(master_parts(), 1024, INK, "lockup.svg", cap=u(19), gap=u(9), body=body)
    lockup(master_parts(), 1024, "#f3ece2", "lockup-dark.svg", cap=u(19), gap=u(9), body=body)
    g = SMALL[16]
    b16 = (g["y0"], g["y1"] + 1, g["x1"] + 1)
    rows16 = pixmap(16, g)
    for t, fill in (("A", "#1c1b18"), ("B", "#f3ece2")):
        inner, _ = pixel_svg(rows16, THEME[t])
        lockup(inner, 16, fill, f"lockup-{t}.svg", cap=8.5, gap=5, body=b16, crisp=True)
    inner, _ = pixel_svg(rows_of(C16), THEME["C"])
    lockup(inner, 16, INK, "lockup-C.svg", cap=8.5, gap=5, body=(1, 13, 16), crisp=True)


def write_all():
    (HERE / "icon.svg").write_text(svg_doc(master_parts(), 1024), encoding="utf-8")
    (HERE / "icon-light.svg").write_text(svg_doc(master_parts(pal="light"), 1024), encoding="utf-8")
    (HERE / "icon-flat.svg").write_text(svg_doc(master_parts(grad=False), 1024), encoding="utf-8")
    write_small()
    write_lockups()


if __name__ == "__main__":
    write_all()
    print("ok")
