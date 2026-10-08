"""Round 3 · variant 3「糖果光泽」— generator for the 1024 master, lockups and title-bar marks.

Run:  D:\\conda\\miniconda3\\python.exe docs/logo/round3/variant-3/build.py
Small sizes (32 / 24 / 16) are hand-placed on their own pixel grids (SMALL below).
"""
import math
from pathlib import Path

from geom import contour, f, offset_pts, smooth_corner_pts

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]

# ---- palette (round-1 hues; coral nudged a touch toward warm peach) ---------
INK = "#141412"
LIME = "#d4f24a"
PAPER = "#f6f1e4"
CORAL = "#ff8f72"
RING = PAPER  # dark-taskbar hairline outside the ink rim

# candy rule: one light (top-left), same on every block
SHADE = {"lime": "#93b512", "paper": "#cbbf9f", "coral": "#e8693f"}
# the light itself is tinted per block (white would turn coral milky); same offset / blur / opacity
LIGHT = {"lime": "#f7ffd0", "paper": "#ffffff", "coral": "#ffe2d6"}

# ---- 1024 master geometry: 64-unit grid, u = 16 px ---------------------------
U = 16
M = dict(
    t=3 * U,            # ink rim = gutter = 48 (one line weight everywhere)
    w=52 * U,           # body 832 x 720
    h=45 * U,
    left=26 * U,        # lime column 416 : right column 272 = 1.53 (~3 : 2)
    top=16 * U,         # paper 256 : coral 320 = 4 : 5 (= 1 : 1.25)
    R=11 * U,           # outer corner radius 176, Figma smoothing 50 %
    xi=0.5,
    ri=36,              # inner gutter corners = 0.75 t
    D=7 * U,            # tail drop below the body: 112
    W=13 * U,           # tail root from the left edge: 208
    rk=256,             # knee fillet where tail grows out of the bottom edge (concave)
    rt=48,              # tail tip radius = gutter
    ring=12,            # dark-taskbar hairline = t / 4
    ox=106, oy=127,       # body origin (set from the alpha centroid, see NOTES)
)


def inner_corner(prv, v, nxt, R, xi, t, n=32):
    """Exact inward offset (by t) of the outer corner at v -> concentric block corner."""
    if not xi:
        return ("arc", R - t)
    return ("pts", offset_pts(smooth_corner_pts(prv, v, nxt, R, xi, n=n), t))


def shapes(m):
    t, R, xi, ri = m["t"], m["R"], m["xi"], m["ri"]
    x0, y0 = m["ox"], m["oy"]
    x1, y1 = x0 + m["w"], y0 + m["h"]
    D, W = m["D"], m["W"]
    sil = contour([
        ((x0, y0), ("smooth", R, xi)),
        ((x1, y0), ("smooth", R, xi)),
        ((x1, y1), ("smooth", R, xi)),
        ((x0 + W, y1), ("arc", m["rk"])),
        ((x0, y1 + D), ("arc", m["rt"])),
    ])
    # interior = silhouette offset inward by t; concentric corners
    Ri = R - t
    p = (1 + xi) * R
    xii = (p - t) / Ri - 1          # keeps the inner transition starting where the outer one does
    xi0, yi0, xi1, yi1 = x0 + t, y0 + t, x1 - t, y1 - t
    xs = xi0 + m["left"]
    ys = yi0 + m["top"]
    L = math.hypot(W, D)
    nx, ny = -D / L, -W / L         # inward normal of the tail's upper edge
    Tx, Ty = x0 + t * nx, y1 + D + t * ny
    s = (Ty - yi1) * L / D
    knee_i = (Tx + s * W / L, yi1)
    s2 = (xi0 - Tx) * L / W
    tip_i = (xi0, Ty - s2 * D / L)
    cTL = inner_corner((x0, y1), (x0, y0), (x1, y0), R, xi, t)
    cTR = inner_corner((x0, y0), (x1, y0), (x1, y1), R, xi, t)
    cBR = inner_corner((x1, y0), (x1, y1), (x0, y1), R, xi, t)
    lime = contour([
        ((xi0, yi0), cTL),
        ((xs, yi0), ("arc", ri)),
        ((xs, yi1), ("arc", ri)),
        (knee_i, ("arc", m["rk"] + t)),
        (tip_i, ("arc", ri)),
    ])
    paper = contour([
        ((xs + t, yi0), ("arc", ri)),
        ((xi1, yi0), cTR),
        ((xi1, ys), ("arc", ri)),
        ((xs + t, ys), ("arc", ri)),
    ])
    coral = contour([
        ((xs + t, ys + t), ("arc", ri)),
        ((xi1, ys + t), ("arc", ri)),
        ((xi1, yi1), cBR),
        ((xs + t, yi1), ("arc", ri)),
    ])
    return dict(sil=sil, lime=lime, paper=paper, coral=coral)


def candy_defs(uid, k=1.0):
    """k scales the light rule with the drawing size (1.0 at 1024)."""
    hl_off, hl_blur = 18 * k, 14 * k
    sh_off, sh_blur = 18 * k, 18 * k
    out = []
    for name, c in LIGHT.items():
        out.append(
            f'<filter id="{uid}-hl-{name}" x="-5%" y="-5%" width="110%" height="110%" color-interpolation-filters="sRGB">'
            f'<feOffset in="SourceAlpha" dx="{f(hl_off)}" dy="{f(hl_off)}"/>'
            f'<feGaussianBlur stdDeviation="{f(hl_blur)}" result="b"/>'
            f'<feComposite in="SourceAlpha" in2="b" operator="out" result="rim"/>'
            f'<feFlood flood-color="{c}" flood-opacity="0.6"/>'
            f'<feComposite in2="rim" operator="in"/></filter>'
        )
    for name, c in SHADE.items():
        out.append(
            f'<filter id="{uid}-sh-{name}" x="-5%" y="-5%" width="110%" height="110%" color-interpolation-filters="sRGB">'
            f'<feOffset in="SourceAlpha" dx="{f(-sh_off)}" dy="{f(-sh_off)}"/>'
            f'<feGaussianBlur stdDeviation="{f(sh_blur)}" result="b"/>'
            f'<feComposite in="SourceAlpha" in2="b" operator="out" result="rim"/>'
            f'<feFlood flood-color="{c}" flood-opacity="0.35"/>'
            f'<feComposite in2="rim" operator="in"/></filter>'
        )
    # broad soft sheen from the top-left, same on every block
    out.append(
        f'<linearGradient id="{uid}-sheen" x1="0" y1="0" x2="0.75" y2="0.9">'
        f'<stop offset="0" stop-color="#ffffff" stop-opacity="0.28"/>'
        f'<stop offset="0.55" stop-color="#ffffff" stop-opacity="0"/></linearGradient>'
    )
    return "<defs>" + "".join(out) + "</defs>"


def mark(m=M, uid="v3", candy=True, ring=True):
    s = shapes(m)
    k = 1.0
    out = []
    if candy:
        out.append(candy_defs(uid, k))
    if ring:
        out.append(
            f'<path d="{s["sil"]}" fill="none" stroke="{RING}" stroke-opacity="{RING_A}" '
            f'stroke-width="{2 * m["ring"]}" stroke-linejoin="round"/>'
        )
    out.append(f'<path d="{s["sil"]}" fill="{INK}"/>')
    for name, col in (("lime", LIME), ("paper", PAPER), ("coral", CORAL)):
        out.append(f'<path d="{s[name]}" fill="{col}"/>')
        if candy:
            out.append(f'<path d="{s[name]}" fill="url(#{uid}-sheen)"/>')
            out.append(f'<path d="{s[name]}" fill="#000" filter="url(#{uid}-sh-{name})"/>')
            out.append(f'<path d="{s[name]}" fill="#000" filter="url(#{uid}-hl-{name})"/>')
    return "\n  ".join(out)


# ---- hand-placed small sizes ----------------------------------------------------
# Every straight edge sits on a pixel boundary; the tail is a 1 : 2 staircase (2 px across,
# 1 px down) at all three sizes, so its angle matches the master's 112 : 208 slope.
# rim = gutter (t px), ring = 1 px paper @ RING_A outside the silhouette.
RING_A = 0.45
HI = {"lime": "#e3f87a", "paper": "#fffdf7", "coral": "#ffaa8f"}  # 1 px top light edge (critic: softened so the 16 px coral no longer reads as two colours)
SMALL = {
    32: dict(x0=1, y0=1, w=30, h=26, t=2, left=14, top=9, R=6, xi=0.5, ri=1, steps=4, knee_aa=True,
             smooth_tail=dict(D=4, W=8, rk=8, rt=1.5)),
    24: dict(x0=1, y0=1, w=22, h=19, t=2, left=10, top=6, R=4, xi=0.5, ri=1, steps=3, knee_aa=True),
    16: dict(x0=1, y0=1, w=14, h=12, t=1, left=7, top=4, R=3, xi=0.0, ri=0, steps=2, knee_aa=True),
}


def _corner(R, xi):
    return ("smooth", R, xi) if xi else ("arc", R)


def small_shapes(s):
    x0, y0, t, n = s["x0"], s["y0"], s["t"], s["steps"]
    x1, y1 = x0 + s["w"], y0 + s["h"]
    R, xi, ri = s["R"], s["xi"], s["ri"]
    # outer staircase from the knee (x0 + 2n, y1) down to the tip (x0, y1 + n)
    stairs = []
    for k in range(n):
        stairs += [(x0 + 2 * (n - k), y1 + k), (x0 + 2 * (n - k), y1 + k + 1)]
    stairs.append((x0, y1 + n))
    sil = contour([((x0, y0), _corner(R, xi)), ((x1, y0), _corner(R, xi)), ((x1, y1), _corner(R, xi))]
                  + [(p, None) for p in stairs])
    Ri = R - t
    xii = 0.0 if not xi else min(1.0, ((1 + xi) * R - t) / Ri - 1)
    xi0, yi0, xi1, yi1 = x0 + t, y0 + t, x1 - t, y1 - t
    xs, ys = xi0 + s["left"], yi0 + s["top"]
    rc = ("arc", ri) if ri else None
    # lime tongue = the outer staircase moved in by t (same 2-across / 1-down steps)
    rows = 1 if t == 1 else n - 1
    tongue = []
    for j in range(rows):
        tongue += [(x0 + 2 * (n - j), yi1 + j), (x0 + 2 * (n - j), yi1 + j + 1)]
    tongue.append((xi0, yi1 + rows))
    cTL = inner_corner((x0, y1), (x0, y0), (x1, y0), R, xi, t, n=4)
    cTR = inner_corner((x0, y0), (x1, y0), (x1, y1), R, xi, t, n=4)
    cBR = inner_corner((x1, y0), (x1, y1), (x0, y1), R, xi, t, n=4)
    lime = contour([((xi0, yi0), cTL), ((xs, yi0), rc), ((xs, yi1), rc)]
                   + [(p, None) for p in tongue])
    paper = contour([((xs + t, yi0), rc), ((xi1, yi0), cTR), ((xi1, ys), rc), ((xs + t, ys), rc)])
    coral = contour([((xs + t, ys + t), rc), ((xi1, ys + t), rc), ((xi1, yi1), cBR), ((xs + t, yi1), rc)])
    boxes = {"lime": (xi0, yi0, xs), "paper": (xs + t, yi0, xi1), "coral": (xs + t, ys + t, xi1)}
    return dict(sil=sil, lime=lime, paper=paper, coral=coral, boxes=boxes,
                knee=(x0 + 2 * n, y1), tip_aa=(x0 + 2, y1 + n - 1), tongue_knee=(x0 + 2 * n, yi1))


def small_svg(n, smooth=False):
    s = SMALL[n]
    g = small_shapes(s)
    if smooth and s.get("smooth_tail"):
        m = dict(s, **s["smooth_tail"], ox=s["x0"], oy=s["y0"])
        g.update(shapes(m))
    out = [f'<path d="{g["sil"]}" fill="none" stroke="{RING}" stroke-opacity="{RING_A}" stroke-width="2" stroke-linejoin="round"/>',
           f'<path d="{g["sil"]}" fill="{INK}"/>']
    n_ = s["steps"]
    if s["knee_aa"]:  # one 45 % ink pixel at the outer end of every step softens the 1 : 2 staircase
        for k in range(n_):
            out.append(f'<rect x="{s["x0"] + 2 * (n_ - k)}" y="{s["y0"] + s["h"] + k}" width="1" height="1" fill="{INK}" fill-opacity="0.45"/>')
    defs = []
    for name, col in (("lime", LIME), ("paper", PAPER), ("coral", CORAL)):
        out.append(f'<path d="{g[name]}" fill="{col}"/>')
        bx0, by0, bx1 = g["boxes"][name]
        defs.append(f'<clipPath id="c{n}{name}"><path d="{g[name]}"/></clipPath>')
        out.append(f'<rect x="{bx0}" y="{by0}" width="{bx1 - bx0}" height="1" fill="{HI[name]}" clip-path="url(#c{n}{name})"/>')
    if s["knee_aa"]:  # and one 50 % lime pixel at the end of every tongue step
        rows = 1 if s["t"] == 1 else n_ - 1
        yi1 = s["y0"] + s["h"] - s["t"]
        for j in range(rows):
            out.append(f'<rect x="{s["x0"] + 2 * (n_ - j)}" y="{yi1 + j}" width="1" height="1" fill="{LIME}" fill-opacity="0.5"/>')
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{n}" height="{n}" viewBox="0 0 {n} {n}">\n'
            f'  <defs>{"".join(defs)}</defs>\n  ' + "\n  ".join(out) + "\n</svg>\n")



# ---- wordmark & lockups ---------------------------------------------------------
EXTRA_KERN = {("k", "W"): -38}  # Geist has no k/W pair; optical fix (same as round 1)


def word_paths(text="DiskWhy", wght=700, tracking=-12):
    """Outline `text` in Geist (OFL, src/assets/fonts) -> (path_d, advance) in font units,
    baseline at y = 0, y down. Applies GPOS pair kerning."""
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    from fontTools.ttLib import TTFont
    from fontTools.varLib.instancer import instantiateVariableFont

    font = instantiateVariableFont(TTFont(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2"), {"wght": wght})
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
                        v = pvr.Value1.XAdvance if pvr.Value1 and hasattr(pvr.Value1, "XAdvance") else 0
                        out.setdefault((first, pvr.SecondGlyph), v)
            elif st.Format == 2:
                c1, c2 = st.ClassDef1.classDefs, st.ClassDef2.classDefs
                for first in cov:
                    k1 = c1.get(first, 0)
                    for second, k2 in list(c2.items()):
                        rec = st.Class1Record[k1].Class2Record[k2]
                        v = rec.Value1.XAdvance if rec.Value1 and hasattr(rec.Value1, "XAdvance") else 0
                        if v:
                            out.setdefault((first, second), v)
    return out


CAP = 710  # Geist cap height (units)


def lockup_master(text_fill, name, cap=368, gap=128, uid="lk"):
    """1024 mark + DiskWhy (Geist Bold). Caps are centred on the bubble body, not on the tail."""
    m = M
    d, adv = word_paths()
    sc = cap / CAP
    pad = m["ring"]
    bx0, by0 = m["ox"] - pad, m["oy"] - pad
    bx1, by1 = m["ox"] + m["w"] + pad, m["oy"] + m["h"] + m["D"] + pad
    base = m["oy"] + m["h"] / 2 + cap / 2
    tx = bx1 + gap
    w = tx + adv * sc - bx0 + 8
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{f(bx0)} {f(by0)} {f(w)} {f(by1 - by0)}" '
           f'width="{f(w / 4)}" height="{f((by1 - by0) / 4)}">\n  {mark(uid=uid)}\n'
           f'  <path transform="translate({f(tx)},{f(base)}) scale({sc:.5f})" fill="{text_fill}" d="{d}"/>\n</svg>\n')
    write(name, svg)


def titlebar_mark(theme, text_fill):
    """16 px tall: the hand-placed 16 px icon + DiskWhy at title-bar size (Geist 700, 15 px)."""
    icon = small_svg(16)
    inner = icon.split("\n", 1)[1].rsplit("</svg>", 1)[0].replace('id="c16', f'id="t{theme}').replace("url(#c16", f"url(#t{theme}")
    d, adv = word_paths(wght=700)
    sc = 15 / 1000
    tx = 16 + 6
    base = 7 + 10.65 / 2  # caps centred on the bubble body (rows 1..13)
    w = tx + adv * sc + 1
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {f(w)} 16" width="{f(w)}" height="16">\n'
           f'{inner}  <path transform="translate({tx},{f(base)}) scale({sc})" fill="{text_fill}" d="{d}"/>\n</svg>\n')
    write(f"titlebar-{theme}.svg", svg)


def svg_doc(inner, n=1024, size=None):
    size = size or n
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
            f'viewBox="0 0 {n} {n}">\n  {inner}\n</svg>\n')


def write(name, text):
    (HERE / name).write_text(text, encoding="utf-8")


if __name__ == "__main__":
    import sys
    write("icon.svg", svg_doc(mark()))
    write("wip/icon-flat.svg", svg_doc(mark(candy=False)))
    for n in SMALL:
        write(f"icon-{n}.svg", small_svg(n))
    write("wip/icon-32-smooth.svg", small_svg(32, smooth=True))
    lockup_master(INK, "lockup-light.svg")
    lockup_master("#f6f1e4", "lockup-dark.svg")
    titlebar_mark("A", "#1c1b18")
    titlebar_mark("C", "#141412")
    print("ok")
