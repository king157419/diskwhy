r"""Variant 1 · Quiet Grid (留白气泡) — generator for every SVG in this folder.

Run:  D:\conda\miniconda3\python.exe docs/logo/round2/variant-1/build.py

System (BRIEF §3), master on a 64-unit grid, 1 u = 16 px:
  body 52 u wide x 38 u tall, centred by its alpha centroid (x0 = 7 u, y0 = 12 u)
  treemap, slice-and-dice: lime row 20 u on top | 2 u gap | bottom row 16 u
                           bottom row = moss 33 u | 2 u gap | clay 17 u
  areas lime : moss : clay = 1040 : 528 : 272 = 3.8 : 1.9 : 1 -> the same x2 chain as the radii
  radius chain R0 8 u (Figma smoothing 60 %) · R1 4 u · R2 2 u · R3 1 u
  no stroke; the 2 u gaps are transparent (the taskbar shows through)
  tail grows out of the moss brick's bottom-left: its left edge bends 45 deg with R1 into a
  straight 45 deg edge that runs on to the tip (R3); the inner edge is vertical and meets the
  body at a concave R2 knee. Tail depth 8 u (= 15 % of body width, brief: <= 25 %).
Small sizes are re-drawn on their own pixel grids (integer edges), see SMALL below.
"""
from pathlib import Path

from geom import contour, rect

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]

INK, LIME, PAPER, MOSS, CLAY = "#141412", "#d4f24a", "#f6f1e4", "#5c7a1a", "#8a857a"
LIME_HI, LIME_LO = "#e0f766", "#c8e93a"  # the one allowed gradient, mid-point = LIME

# colour roles per brick: (top row, bottom-left + tail, bottom-right)
DARK = (LIME, MOSS, CLAY)      # master and dark taskbar ("altform-unplated")
LIGHT = (MOSS, INK, CLAY)      # light taskbar ("altform-lightunplated"): every role one step darker

# ---- geometry, one dict per drawing grid ------------------------------------------------
# x0, y0: top-left of the body; W: body width; wm: moss width; gap; ht / hb: top / bottom row
# heights; R0 smooth outer corner (xi = smoothing), R2 gap corners + knee, R1 tail bend, R3 tail
# tip; cv: the bend starts cv above the bottom edge; h: tail depth below the body. px of that grid.
U = 16
MASTER = dict(n=1024, x0=6 * U, W=53 * U, wm=33 * U, gap=2 * U, y0=12 * U, ht=20 * U, hb=16 * U,
              R0=8 * U, xi=0.6, R1=4 * U, R2=2 * U, R3=1 * U, cv=4 * U, h=8 * U, grad=True)
SMALL = {
    48: dict(n=48, x0=4, W=40, wm=25, gap=2, y0=9, ht=15, hb=11, R0=6, xi=0.6, R1=3, R2=1.5, R3=1, cv=3, h=6, grad=True),
    32: dict(n=32, x0=1, W=30, wm=19, gap=2, y0=3, ht=11, hb=9, R0=4, xi=0, R1=2, R2=1, R3=0, cv=2, h=4, grad=True),
    24: dict(n=24, x0=1, W=22, wm=14, gap=1, y0=2, ht=9, hb=7, R0=3, xi=0, R1=0, R2=0, R3=0, cv=1, h=3, grad=False),
    16: dict(n=16, x0=0, W=16, wm=10, gap=1, y0=1, ht=6, hb=5, R0=2, xi=0, R1=0, R2=0, R3=0, cv=1, h=2, grad=False),
}


def _c(r, kind="arc", xi=0):
    if not r:
        return None
    return ("smooth", r, xi) if kind == "smooth" and xi else ("arc", r)


def body_box(g):
    return g["x0"], g["y0"], g["x0"] + g["W"], g["y0"] + g["ht"] + g["gap"] + g["hb"]


def shapes(g):
    """-> (top_path, tail_brick_path, right_brick_path)"""
    x0, y0, x1, y1 = body_box(g)
    gap, wm = g["gap"], g["wm"]
    ym, yb = y0 + g["ht"], y0 + g["ht"] + gap
    xm, xc = x0 + wm, x0 + wm + gap
    R0 = _c(g["R0"], "smooth", g["xi"])
    R1, R2, R3 = _c(g["R1"]), _c(g["R2"]), _c(g["R3"])
    cv, h = g["cv"], g["h"]
    tx = x0 + cv + h  # the 45-degree edge from (x0, y1-cv) reaches depth h at x = x0+cv+h
    top = rect(x0, y0, x1, ym, R0, R0, R2, R2)
    tail = contour([((x0, yb), R2), ((xm, yb), R2), ((xm, y1), R2),
                    ((tx, y1), R2), ((tx, y1 + h), R3), ((x0, y1 - cv), R1)])
    right = rect(xc, yb, x1, y1, R2, R2, R0, R2)
    return top, tail, right


def mark(g, cols=DARK, uid="m", grad=None):
    top, tail, right = shapes(g)
    grad = g["grad"] if grad is None else grad
    c0, c1, c2 = cols
    defs, f0 = "", c0
    if grad and c0 == LIME:
        # one 120-degree, one-step ramp over the lime brick, light from the top-left
        # (CSS 120deg: direction (sin 120, -cos 120) = (0.866, 0.5))
        x0, y0 = g["x0"], g["y0"]
        L = g["W"] * 0.866 + g["ht"] * 0.5
        defs = (f'<defs><linearGradient id="{uid}g" gradientUnits="userSpaceOnUse" x1="{x0}" y1="{y0}" '
                f'x2="{x0 + L * 0.866:.2f}" y2="{y0 + L * 0.5:.2f}"><stop offset="0" stop-color="{LIME_HI}"/>'
                f'<stop offset="1" stop-color="{LIME_LO}"/></linearGradient></defs>')
        f0 = f"url(#{uid}g)"
    return (f'{defs}<path d="{top}" fill="{f0}"/><path d="{tail}" fill="{c1}"/>'
            f'<path d="{right}" fill="{c2}"/>')


def svg_doc(g, cols=DARK, size=None, crisp=False):
    n = g["n"]
    sr = "crispEdges" if crisp else "geometricPrecision"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size or n}" height="{size or n}" '
            f'viewBox="0 0 {n} {n}" shape-rendering="{sr}">{mark(g, cols)}</svg>\n')


# ---- wordmark ----------------------------------------------------------------------------
EXTRA_KERN = {("k", "W"): -38}


def word_paths(text="DiskWhy", wght=600, tracking=-10):
    """Outline text in Geist (OFL, bundled in src/assets/fonts) -> (d, advance) in font units."""
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    from fontTools.ttLib import TTFont
    from fontTools.varLib.instancer import instantiateVariableFont

    font = TTFont(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2")
    font = instantiateVariableFont(font, {"wght": wght})
    cmap, gs = font.getBestCmap(), font.getGlyphSet()
    names = [cmap[ord(c)] for c in text]
    kern = _pair_kern(font)
    x, ds = 0, []
    for i, gn in enumerate(names):
        pen = SVGPathPen(gs)
        gs[gn].draw(TransformPen(pen, (1, 0, 0, -1, x, 0)))
        ds.append(pen.getCommands())
        x += gs[gn].width + tracking
        if i + 1 < len(names):
            x += kern.get((gn, names[i + 1]), 0) + EXTRA_KERN.get((gn, names[i + 1]), 0)
    return " ".join(ds), x - tracking


def _pair_kern(font):
    out = {}
    if "GPOS" not in font:
        return out
    for lk in font["GPOS"].table.LookupList.Lookup:
        for st in lk.SubTable:
            if lk.LookupType == 9:
                st = st.ExtSubTable
            if getattr(st, "LookupType", lk.LookupType) != 2:
                continue
            cov = st.Coverage.glyphs
            if st.Format == 1:
                for i, first in enumerate(cov):
                    for pvr in st.PairSet[i].PairValueRecord:
                        v = getattr(pvr.Value1, "XAdvance", 0) if pvr.Value1 else 0
                        out.setdefault((first, pvr.SecondGlyph), v)
            elif st.Format == 2:
                c1, c2 = st.ClassDef1.classDefs, st.ClassDef2.classDefs
                for first in cov:
                    k1 = c1.get(first, 0)
                    for second, k2 in list(c2.items()):
                        rec = st.Class1Record[k1].Class2Record[k2]
                        v = getattr(rec.Value1, "XAdvance", 0) if rec.Value1 else 0
                        if v:
                            out.setdefault((first, second), v)
    return out


def lockup(g, cols, text_fill, name, cap_ratio, gap_ratio, wght=600, inner=None):
    """Mark + outlined word. Caps are centred on the body (not on the tail)."""
    n = g["n"]
    d, adv = word_paths(wght=wght)
    s = cap_ratio * n / 710.0  # Geist cap height = 710 units
    _, y0, xr, y1 = body_box(g)
    base = (y0 + y1) / 2 + 710 * s / 2
    tx = xr + gap_ratio * n
    w = tx + adv * s + g["x0"]
    body = inner if inner is not None else mark(g, cols, uid=name.split(".")[0])
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.2f} {n}" height="{n}" width="{w:.2f}">'
           f'{body}<path transform="translate({tx:.2f},{base:.2f}) scale({s:.5f})" fill="{text_fill}" d="{d}"/></svg>\n')
    (HERE / name).write_text(svg, encoding="utf-8")


# ---- title-bar marks (16 px), one silhouette, three colourings -------------------------
THEMES = {
    # A 浅色: light layer = A's green (big brick), dark layer = ink-A (small bricks)
    "A": ("#2f7d5b", "#1c1b18", "#1c1b18"),
    # B 深色: accent gold (big brick), paper-B (small bricks)
    "B": ("#e4bc70", "#f3ece2", "#f3ece2"),
}
TEXT = {"A": "#1c1b18", "B": "#f3ece2", "C": INK}
# C 卡通: the C language = ink 1 px outline + 1 px hard offset shadow; bricks shrink by 1 px so
# outline + shadow still fit the 16 grid. Gap 1 px becomes the shared outline.
C16 = dict(n=16, x0=1, W=13, wm=8, gap=1, y0=1, ht=5, hb=4, R0=2, xi=0, R1=0, R2=0, R3=0, cv=1, h=2, grad=False)


def mark_C(g=C16, cols=(LIME, LIME, PAPER)):
    lime, moss, clay = shapes(g)
    st = 'stroke="#141412" stroke-width="2" stroke-linejoin="round"'
    shadow = (f'<g transform="translate(1,1)" fill="{INK}" {st}><path d="{lime}"/><path d="{moss}"/>'
              f'<path d="{clay}"/></g>')
    outline = f'<g fill="{INK}" {st}><path d="{lime}"/><path d="{moss}"/><path d="{clay}"/></g>'
    fills = f'<path d="{lime}" fill="{cols[0]}"/><path d="{moss}" fill="{cols[1]}"/><path d="{clay}" fill="{cols[2]}"/>'
    return shadow + outline + fills


def theme_mark(t):
    return mark_C() if t == "C" else mark(SMALL[16], THEMES[t])


def write_all():
    (HERE / "icon.svg").write_text(svg_doc(MASTER, DARK), encoding="utf-8")
    (HERE / "icon-light.svg").write_text(svg_doc(MASTER, LIGHT), encoding="utf-8")
    for s, g in SMALL.items():
        (HERE / f"icon-{s}.svg").write_text(svg_doc(g, DARK), encoding="utf-8")
        (HERE / f"icon-{s}-light.svg").write_text(svg_doc(g, LIGHT), encoding="utf-8")
    for t in "ABC":
        (HERE / f"mark-16-{t}.svg").write_text(
            f'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">{theme_mark(t)}</svg>\n',
            encoding="utf-8")
        lockup(SMALL[16] if t != "C" else C16, None, TEXT[t], f"lockup-{t}.svg", 0.58, 0.42, wght=620,
               inner=theme_mark(t))
    lockup(MASTER, DARK, INK, "lockup.svg", 0.40, 0.10)
    lockup(MASTER, DARK, "#f3ece2", "lockup-dark.svg", 0.40, 0.10)


if __name__ == "__main__":
    write_all()
    print("ok")
