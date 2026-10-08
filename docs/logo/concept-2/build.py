"""Concept 2 "开口的磁盘" — generator for icon SVGs, lockups and the preview sheet.

Run:  D:\\conda\\miniconda3\\python.exe docs/logo/concept-2/build.py
Outputs land next to this file.
"""
from pathlib import Path

HERE = Path(__file__).resolve().parent
INK, LIME, PAPER, CORAL = "#141412", "#d4f24a", "#f6f1e4", "#ff8a73"


def rrect(x0, y0, x1, y1, r):
    """Rounded rect path; r = (tl, tr, br, bl)."""
    tl, tr, br, bl = r
    return (
        f"M{x0+tl:g},{y0:g}H{x1-tr:g}"
        + (f"A{tr:g},{tr:g} 0 0 1 {x1:g},{y0+tr:g}" if tr else "")
        + f"V{y1-br:g}"
        + (f"A{br:g},{br:g} 0 0 1 {x1-br:g},{y1:g}" if br else "")
        + f"H{x0+bl:g}"
        + (f"A{bl:g},{bl:g} 0 0 1 {x0:g},{y1-bl:g}" if bl else "")
        + f"V{y0+tl:g}"
        + (f"A{tl:g},{tl:g} 0 0 1 {x0+tl:g},{y0:g}" if tl else "")
        + "Z"
    )


def _offset_line(p, q, d):
    """Offset segment p->q by d to its left-hand normal (inward for our clockwise-in-screen path)."""
    (x1, y1), (x2, y2) = p, q
    dx, dy = x2 - x1, y2 - y1
    L = (dx * dx + dy * dy) ** 0.5
    nx, ny = -dy / L, dx / L
    return (x1 + nx * d, y1 + ny * d), (x2 + nx * d, y2 + ny * d)


def _intersect_h(p, q, y):
    (x1, y1), (x2, y2) = p, q
    return x1 + (x2 - x1) * (y - y1) / (y2 - y1), y


def _intersect_v(p, q, x):
    (x1, y1), (x2, y2) = p, q
    return x, y1 + (y2 - y1) * (x - x1) / (x2 - x1)


def _toward(p, q, d):
    """Point at distance d from p toward q."""
    dx, dy = q[0] - p[0], q[1] - p[1]
    L = (dx * dx + dy * dy) ** 0.5
    return p[0] + dx / L * d, p[1] + dy / L * d


def _poly_rounded(pts, rads):
    """Closed polygon (clockwise on screen) with a true tangent circular fillet of radius
    rads[i] at vertex i (0 = sharp). Works for convex and concave vertices."""
    import math
    f = lambda v: f"{v:.2f}".rstrip("0").rstrip(".")
    n = len(pts)
    corners = []
    for i in range(n):
        prv, v, nxt = pts[i - 1], pts[i], pts[(i + 1) % n]
        r = rads[i]
        if not r:
            corners.append((v, None, v, 0))
            continue
        u1 = (prv[0] - v[0], prv[1] - v[1])
        u2 = (nxt[0] - v[0], nxt[1] - v[1])
        l1, l2 = math.hypot(*u1), math.hypot(*u2)
        cosang = (u1[0] * u2[0] + u1[1] * u2[1]) / (l1 * l2)
        theta = math.acos(max(-1, min(1, cosang)))
        d = r / math.tan(theta / 2)
        pa = _toward(v, prv, d)
        pb = _toward(v, nxt, d)
        # incoming dir = v - prv, outgoing = nxt - v; cross > 0 -> convex for clockwise-on-screen
        cross = (v[0] - prv[0]) * (nxt[1] - v[1]) - (v[1] - prv[1]) * (nxt[0] - v[0])
        corners.append((pa, r, pb, 1 if cross > 0 else 0))
    d = f"M{f(corners[0][2][0])},{f(corners[0][2][1])}"
    for k in range(1, n + 1):
        pa, r, pb, sweep = corners[k % n]
        d += f"L{f(pa[0])},{f(pa[1])}"
        if r:
            d += f"A{f(r)},{f(r)} 0 0 {sweep} {f(pb[0])},{f(pb[1])}"
    return d + "Z"


def mark(g, uid="m"):
    """SVG inner markup for the mark on a g['n'] grid.

    A speech bubble (ink rim) whose body is a treemap: big lime block (its corner IS the
    bubble's tail, so the bubble still reads where the ink rim vanishes on dark taskbars),
    a paper block and a coral block.
    """
    x0, y0, x1, y1, R = g["body"]
    t, gut, ri = g["border"], g["gutter"], g["inner_r"]
    tx, ty = g["tail"]  # tail leaves the bottom edge at x=tx and its tip is (x0, ty)
    tip, knee = g.get("tip", 0), g.get("knee", 0)  # rounding of tail tip / tail-to-body joint

    sil = _poly_rounded(
        [(x0, y0), (x1, y0), (x1, y1), (tx, y1), (x0, ty)],
        [R, R, R, knee, tip],
    )
    ix0, iy0, ix1, iy1 = x0 + t, y0 + t, x1 - t, y1 - t
    Ri = max(R - t, ri)
    sx, sy = g["split_x"], g["split_y"]
    a, b = _offset_line((tx, y1), (x0, ty), t)
    pb = _intersect_h(a, b, iy1)
    pt = _intersect_v(a, b, ix0)
    lime = _poly_rounded(
        [(ix0, iy0), (sx, iy0), (sx, iy1), pb, pt],
        [Ri, ri, ri, knee + t if knee else 0, g.get("tip_in", 0)],
    )
    paper = rrect(sx + gut, iy0, ix1, sy, (ri, Ri, ri, ri))
    coral = rrect(sx + gut, sy + gut, ix1, iy1, (ri, ri, Ri, ri))
    out = []
    fills = {"ink": INK, "lime": LIME, "paper": PAPER, "coral": CORAL}
    if g.get("grad"):
        # one-step 120-degree ramps (Microsoft: subtle, light from top-left)
        ramps = {"ink": ("#2a2925", "#0d0d0b"), "lime": ("#e2f86e", "#c6ea2c"),
                 "paper": ("#fdfaf2", "#efe7d4"), "coral": ("#ff9e86", "#fb7a62")}
        defs = "".join(
            f'<linearGradient id="{uid}-{k}" x1="0.2" y1="0" x2="0.8" y2="1">'
            f'<stop offset="0" stop-color="{c0}"/><stop offset="1" stop-color="{c1}"/></linearGradient>'
            for k, (c0, c1) in ramps.items()
        )
        out.append(f"<defs>{defs}</defs>")
        fills = {k: f"url(#{uid}-{k})" for k in ramps}
    out += [
        f'<path d="{sil}" fill="{fills["ink"]}"/>',
        f'<path d="{lime}" fill="{fills["lime"]}"/>',
        f'<path d="{paper}" fill="{fills["paper"]}"/>',
        f'<path d="{coral}" fill="{fills["coral"]}"/>',
    ]
    return "\n  ".join(out)


# ---- size-specific geometry (each drawn on its own pixel grid) -------------
G1024 = dict(n=1024, body=(96, 96, 928, 800, 196), border=48, gutter=36, inner_r=28,
             split_x=548, split_y=400, tail=(360, 936), shadow=(0, 0), tip=44, knee=36, tip_in=14, grad=True)
G32 = dict(n=32, body=(2, 2, 30, 25, 6), border=2, gutter=2, inner_r=1,
           split_x=17, split_y=12, tail=(11, 30), shadow=(0, 0))
G24 = dict(n=24, body=(1, 2, 23, 19, 4.5), border=2, gutter=1, inner_r=0.5,
           split_x=13, split_y=9, tail=(8, 23), shadow=(0, 0))
G16 = dict(n=16, body=(0, 1, 16, 13, 3), border=1, gutter=1, inner_r=0,
           split_x=9, split_y=6, tail=(6, 16), shadow=(0, 0))


def svg_doc(g, size=None):
    n = g["n"]
    size = size or n
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
        f'viewBox="0 0 {n} {n}" shape-rendering="geometricPrecision">\n  {mark(g)}\n</svg>\n'
    )


def write_icons():
    (HERE / "icon.svg").write_text(svg_doc(G1024), encoding="utf-8")
    (HERE / "icon-32.svg").write_text(svg_doc(G32), encoding="utf-8")
    (HERE / "icon-24.svg").write_text(svg_doc(G24), encoding="utf-8")
    (HERE / "icon-16.svg").write_text(svg_doc(G16), encoding="utf-8")


EXTRA_KERN = {("k", "W"): -38}  # Geist has no k/W pair; optical fix


def word_paths(text="DiskWhy", wght=620, tracking=-12):
    """Outline `text` in Geist (OFL, bundled in src/assets/fonts) -> (path_d, advance) in font units,
    baseline at y=0, y pointing down. Applies GPOS pair kerning if present."""
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    from fontTools.ttLib import TTFont
    from fontTools.varLib.instancer import instantiateVariableFont

    font = TTFont(HERE.parents[2] / "src" / "assets" / "fonts" / "Geist-latin.woff2")
    font = instantiateVariableFont(font, {"wght": wght})
    cmap = font.getBestCmap()
    gs = font.getGlyphSet()
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
    """Very small GPOS PairPos (format 1 & 2) reader -> {(left, right): xAdvance}."""
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
                        v = pvr.Value1.XAdvance if pvr.Value1 and hasattr(pvr.Value1, "XAdvance") else 0
                        out.setdefault((first, pvr.SecondGlyph), v)
            elif st.Format == 2:
                c1 = st.ClassDef1.classDefs
                c2 = st.ClassDef2.classDefs
                for first in cov:
                    k1 = c1.get(first, 0)
                    for second, k2 in list(c2.items()):
                        rec = st.Class1Record[k1].Class2Record[k2]
                        v = rec.Value1.XAdvance if rec.Value1 and hasattr(rec.Value1, "XAdvance") else 0
                        if v:
                            out.setdefault((first, second), v)
    return out


def lockup(g, text_fill, name, cap_ratio, gap_ratio):
    """Mark (grid g) + outlined word. cap_ratio: cap height / grid size."""
    n = g["n"]
    d, adv = word_paths()
    s = cap_ratio * n / 710.0  # Geist cap height = 710 units
    bx0, by0, bx1, by1, _ = g["body"]
    cy = (by0 + by1) / 2  # optically centre caps on the bubble body, not on the tail
    base = cy + 710 * s / 2
    tx = bx1 + g["shadow"][0] + gap_ratio * n
    w = tx + adv * s + 0.02 * n
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.2f} {n}" height="{n}" width="{w:.2f}">\n'
        f"  {mark(g)}\n"
        f'  <path transform="translate({tx:.2f},{base:.2f}) scale({s:.5f})" fill="{text_fill}" d="{d}"/>\n</svg>\n'
    )
    (HERE / name).write_text(svg, encoding="utf-8")


if __name__ == "__main__":
    write_icons()
    lockup(G1024, INK, "lockup.svg", 0.40, 0.11)
    lockup(G1024, "#f3ece2", "lockup-dark.svg", 0.40, 0.11)
    lockup(G16, "#1c1b18", "lockup-A.svg", 0.58, 0.40)
    lockup(G16, "#f3ece2", "lockup-B.svg", 0.58, 0.40)
    lockup(G16, INK, "lockup-C.svg", 0.58, 0.40)
    print("ok")
