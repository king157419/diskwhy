r"""Variant 3 · 叠层 Layered Plate — parametric generator for every SVG.

Run:  D:\conda\miniconda3\python.exe docs/logo/round2/variant-3/build.py
Two layers: a speech-bubble plate (R0 squircle, 45-degree tail grown out of its right
edge) and a treemap of three bricks inset by one rim width. One light (top-left):
the lime brick carries the only gradient, every brick drops one soft shadow onto
the plate, clipped to the plate.
"""
from pathlib import Path

from geom import poly, rect, f

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
U = 16  # 1 grid unit at 1024

INK, MOSS, LIME, PAPER, CLAY = "#141412", "#5c7a1a", "#d4f24a", "#f6f1e4", "#8a857a"
LIME_HI, LIME_LO = "#e0f766", "#c8e93a"

COLORWAYS = {
    # plate, big brick, top-right brick, bottom-right brick
    "moss": dict(plate=MOSS, lime=LIME, paper=PAPER, clay=CLAY),
    "ink": dict(plate=INK, lime=LIME, paper=PAPER, clay=CLAY),
    "moss-ink": dict(plate=MOSS, lime=LIME, paper=PAPER, clay=INK),
}


def plate_path(g):
    x0, y0, x1, y1 = g["body"]
    L = g["tail"]
    R0, s0 = g["R0"], g["s0"]
    tip, knee = g["tip_r"], g["knee_r"]
    c0 = ("sq", R0, s0) if s0 else ("arc", R0)
    if g.get("stair"):  # pixel sizes: 45-degree staircase, row k below the body is L-k px wide
        steps = []
        for k in range(L, 0, -1):
            steps += [(x1 - (L - k) - 1, y1 + k), (x1 - (L - k) - 1, y1 + k - 1)]
        pts = [(x0, y0), (x1, y0), (x1, y1 + L)] + steps[:-1] + [(x1 - L, y1), (x0, y1)]
        cs = [c0, c0] + [0] * (len(pts) - 3) + [c0]
        return poly(pts, cs)
    pts = [(x0, y0), (x1, y0), (x1, y1 + L), (x1 - L, y1), (x0, y1)]
    cs = [c0, c0, ("arc", tip) if tip else 0, ("arc", knee) if knee else 0, c0]
    return poly(pts, cs)


def bricks(g):
    """g["bricks"]: list of (role, x0, y0, x1, y1) in px. Corner radii are assigned by rule:
    a brick corner sitting in a corner of the inner box gets the concentric radius Ro
    (R0 - rim, smoothed like R0); every other corner gets Ri (R2). The inner box's
    bottom-right corner is the tail root, so it gets Rc instead."""
    x0, y0, x1, y1 = g["body"]
    t = g["rim"]
    ix0, iy0, ix1, iy1 = x0 + t, y0 + t, x1 - t, y1 - t
    Ro, so, Ri = g["Ro"], g["so"], g["Ri"]
    Rc = g.get("Rc", Ri)
    O = ("sq", Ro, so) if so else (("arc", Ro) if Ro else 0)
    I = ("arc", Ri) if Ri else 0
    C = ("arc", Rc) if Rc else 0
    out = []
    for role, bx0, by0, bx1, by1 in g["bricks"]:
        def k(cx, cy):
            if cx == ix1 and cy == iy1:
                return C
            if cx in (ix0, ix1) and cy in (iy0, iy1):
                return O
            return I
        out.append((role, (bx0, by0, bx1, by1),
                    rect(bx0, by0, bx1, by1, (k(bx0, by0), k(bx1, by0), k(bx1, by1), k(bx0, by1)))))
    return out


def mark(g, cw, uid="m", hard_shadow=None):
    """Inner SVG markup. cw: colour dict (plate + one entry per brick role)."""
    plate = plate_path(g)
    bs = bricks(g)
    out, defs = [], []
    fills = {}
    for i, (role, (bx0, by0, bx1, by1), d) in enumerate(bs):
        fill = cw[role]
        if g.get("grad") and role == "lime" and cw["lime"] == LIME:
            cx, cy = (bx0 + bx1) / 2, (by0 + by1) / 2
            dx, dy = 0.8660254, 0.5  # 120 degrees (CSS convention), light from top-left
            h = (bx1 - bx0) / 2 * dx + (by1 - by0) / 2 * dy
            defs.append(
                f'<linearGradient id="{uid}-g{i}" gradientUnits="userSpaceOnUse" '
                f'x1="{f(cx - dx * h)}" y1="{f(cy - dy * h)}" x2="{f(cx + dx * h)}" y2="{f(cy + dy * h)}">'
                f'<stop offset="0" stop-color="{LIME_HI}"/><stop offset="1" stop-color="{LIME_LO}"/></linearGradient>'
            )
            fill = f"url(#{uid}-g{i})"
        fills[i] = fill
    sh = g.get("shadow")
    if sh:
        dy_, blur, op = sh
        defs.append(f'<clipPath id="{uid}-clip"><path d="{plate}"/></clipPath>')
        defs.append(
            f'<filter id="{uid}-sh" x="-10%" y="-10%" width="120%" height="130%" color-interpolation-filters="sRGB">'
            f'<feGaussianBlur in="SourceAlpha" stdDeviation="{f(blur / 2)}"/>'
            f'<feOffset dy="{f(dy_)}" result="o"/>'
            f'<feFlood flood-color="{INK}" flood-opacity="{op}"/><feComposite in2="o" operator="in"/></filter>'
        )
    if defs:
        out.append("<defs>" + "".join(defs) + "</defs>")
    if hard_shadow:  # theme C: hard offset shadow of the whole silhouette
        hx, hy, hc = hard_shadow
        out.append(f'<path d="{plate}" fill="{hc}" transform="translate({hx},{hy})"/>')
    out.append(f'<path d="{plate}" fill="{cw["plate"]}"/>')
    hs = g.get("hard")  # small sizes: (dx, dy, colour) crisp 1px drop under each brick
    if sh:
        allb = " ".join(d for _, _, d in bs)
        out.append(f'<g clip-path="url(#{uid}-clip)"><path d="{allb}" fill="#000" filter="url(#{uid}-sh)"/></g>')
    if hs:
        allb = " ".join(d for _, _, d in bs)
        out.append(f'<path d="{allb}" fill="{hs[2]}" transform="translate({hs[0]},{hs[1]})"/>')
    for i, (role, _, d) in enumerate(bs):
        out.append(f'<path d="{d}" fill="{fills[i]}"/>')
    return "\n  ".join(out)


def svg_doc(g, cw, size=None, uid="m", hard_shadow=None):
    n = g["n"]
    size = size or n
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {n} {n}">\n'
        f"  {mark(g, cw, uid, hard_shadow)}\n</svg>\n"
    )


# ---------------------------------------------------------------- master
# Module m = rim = gutter = 2.5u = 40 px. Body and bricks are whole multiples of m;
# radii come from the brief's chain (8u / 4u / 2u / 1u, + the 5.5u concentric exception).
M = 2.5 * U

LAYOUTS = {
    # role, cells (c0, r0, c1, r1) in modules, relative to the inner box (18m x 16m)
    "window": [("lime", 0, 0, 11, 16), ("paper", 12, 0, 18, 10), ("clay", 12, 11, 18, 16)],
    "L": [("lime", 0, 0, 12, 10), ("clay", 0, 11, 12, 16), ("paper", 13, 0, 18, 16)],
    "top": [("lime", 0, 0, 18, 9), ("paper", 0, 10, 11, 16), ("clay", 12, 10, 18, 16)],
    "four": [("lime", 0, 0, 12, 10), ("paper", 13, 0, 18, 16), ("clay", 0, 11, 7, 16), ("lime", 8, 11, 12, 16)],
}


def G1024(layout="L", body_m=(20, 18), origin=(6.5 * U, 8 * U), tail_m=3, **kw):
    x0, y0 = origin
    x1, y1 = x0 + body_m[0] * M, y0 + body_m[1] * M
    ix0, iy0 = x0 + M, y0 + M
    g = dict(
        n=1024,
        body=(x0, y0, x1, y1),
        R0=8 * U, s0=0.6,
        tail=tail_m * M, tip_r=1 * U, knee_r=4 * U,  # knee = R1 (brief 3.2 chain), tip = R3
        rim=M, gutter=M,
        bricks=[(r, ix0 + a * M, iy0 + b * M, ix0 + c * M, iy0 + d * M) for r, a, b, c, d in LAYOUTS[layout]],
        Ro=8 * U - M, so=0.6, Ri=2 * U,
        shadow=(1 * U, 1 * U, 0.25),
        grad=True,
    )
    g.update(kw)
    return g


# ---------------------------------------------------------------- hand-tuned small sizes
# Every edge on a whole pixel. Plate corners are plain arcs (smoothing is invisible
# below ~2 % of the canvas); tail is a 45-degree staircase (row k is L-k px wide).
def Gpx(n, body, tail, R0, rim, bricks, Ro, Ri, **kw):
    g = dict(n=n, body=body, R0=R0, s0=0, tail=tail, tip_r=0, knee_r=0, rim=rim, gutter=rim, stair=True,
             bricks=bricks, Ro=Ro, so=0, Ri=Ri, shadow=None, grad=False)
    g.update(kw)
    return g


def G32(**kw):
    # body 30 x 26 at (1,2); rim = gutter = 2; inner 26 x 22
    return Gpx(32, (1, 2, 31, 28), 4, 4, 2,
               [("lime", 3, 4, 20, 18), ("paper", 22, 4, 29, 26),
                ("clay", 3, 20, 13, 26), ("lime", 15, 20, 20, 26)], 2, 1, **kw)


def G24(**kw):
    # body 22 x 19 at (1,2); rim = gutter = 1; inner 20 x 17
    return Gpx(24, (1, 2, 23, 21), 3, 3, 1,
               [("lime", 2, 3, 16, 14), ("paper", 17, 3, 22, 20),
                ("clay", 2, 15, 10, 20), ("lime", 11, 15, 16, 20)], 2, 0, **kw)


def G16(**kw):
    # body 16 x 13 at (0,1); rim = gutter = 1; inner 14 x 11; three bricks only
    return Gpx(16, (0, 1, 16, 14), 2, 2, 1,
               [("lime", 1, 2, 10, 9), ("paper", 11, 2, 15, 13), ("clay", 1, 10, 10, 13)], 1, 0, **kw)


def write(name, text):
    p = HERE / name
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")
    return p


# ---------------------------------------------------------------- colour sets
DARK = COLORWAYS["moss"]                       # master + dark taskbar: moss plate (3.30 : 1 on #202020)
LIGHT = dict(plate=INK, lime=LIME, paper=MOSS, clay=CLAY)  # light taskbar (lightunplated): plate and paper
#   each drop one value step (moss->ink, paper->moss); lime stays as the brand. Outline identical.
THEME = {
    "A": dict(plate="#1c1b18", lime="#2f7d5b", paper="#f6f1e4", clay="#f6f1e4"),
    "B": dict(plate="#f3ece2", lime="#e4bc70", paper="#1b1714", clay="#1b1714"),
    "C": dict(plate=INK, lime=LIME, paper=PAPER, clay=CLAY),
}
TEXT = {"light": INK, "dark": "#f3ece2", "A": "#1c1b18", "B": "#f3ece2", "C": INK}


def G16C(**kw):
    """Theme C title-bar mark: one px smaller so its 1 px hard offset shadow fits in 16."""
    return Gpx(16, (0, 0, 15, 13), 2, 2, 1,
               [("lime", 1, 1, 9, 8), ("paper", 10, 1, 14, 12), ("clay", 1, 9, 9, 12)], 1, 0, **kw)


# ---------------------------------------------------------------- wordmark (Geist, outlined)
def _word():
    import importlib.util
    spec = importlib.util.spec_from_file_location("c2build", HERE.parents[1] / "concept-2" / "build.py")
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m.word_paths(wght=600, tracking=-10)


def lockup(g, cw, text_fill, name, cap, gap, hard_shadow=None, pad=None):
    """mark + outlined 'DiskWhy'. cap = cap height in grid px, gap = mark->text gap in grid px.
    Caps are centred on the plate body (not the tail), so the tail hangs below like a speech mark."""
    d, adv = _word()
    n = g["n"]
    s = cap / 710.0  # Geist cap height = 710 units
    bx0, by0, bx1, by1 = g["body"]
    cy = (by0 + by1) / 2
    base = cy + cap / 2
    tx = bx1 + gap
    if pad is None:
        vx0, vy0, vh = 0, 0, n
    else:
        vx0, vy0, vh = bx0 - pad, by0 - pad, (by1 + g["tail"]) - by0 + 2 * pad
    w = tx + adv * s + (pad if pad is not None else 0) - vx0
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{f(vx0)} {f(vy0)} {f(w)} {f(vh)}" '
        f'width="{f(w)}" height="{f(vh)}">\n'
        f"  {mark(g, cw, 'L' + name.split('.')[0].replace('-', ''), hard_shadow)}\n"
        f'  <path transform="translate({f(tx)},{f(base)}) scale({s:.6f})" fill="{text_fill}" d="{d}"/>\n</svg>\n'
    )
    write(name, svg)


def build_final():
    write("icon.svg", svg_doc(G1024("four"), DARK, uid="m"))
    write("icon-light.svg", svg_doc(G1024("four"), LIGHT, uid="m"))
    for n, G in ((32, G32), (24, G24), (16, G16)):
        extra = dict(grad=True, hard=(0, 1, INK + "40")) if n == 32 else {}
        write(f"icon-{n}.svg", svg_doc(G(**extra), DARK, uid=f"s{n}"))
        write(f"icon-{n}-light.svg", svg_doc(G(**extra), LIGHT, uid=f"s{n}"))
    for t in ("A", "B"):
        write(f"icon-16-{t}.svg", svg_doc(G16(), THEME[t], uid=f"t{t}"))
    write("icon-16-C.svg", svg_doc(G16C(), THEME["C"], uid="tC", hard_shadow=(1, 1, INK)))
    big = G1024("four")
    lockup(big, DARK, TEXT["light"], "lockup.svg", cap=320, gap=112, pad=48)
    lockup(big, DARK, TEXT["dark"], "lockup-dark.svg", cap=320, gap=112, pad=48)
    lockup(G16(), THEME["A"], TEXT["A"], "lockup-A.svg", cap=9, gap=6)
    lockup(G16(), THEME["B"], TEXT["B"], "lockup-B.svg", cap=9, gap=6)
    lockup(G16C(), THEME["C"], TEXT["C"], "lockup-C.svg", cap=9, gap=7, hard_shadow=(1, 1, INK))


if __name__ == "__main__":
    import sys
    rnd = sys.argv[1] if len(sys.argv) > 1 else "final"
    if rnd == "r2":
        for lay in LAYOUTS:
            write(f"rounds/r2/master-{lay}.svg", svg_doc(G1024(lay), COLORWAYS["moss"], uid=lay))
    if rnd == "r3":
        write("rounds/r3/master.svg", svg_doc(G1024("four"), COLORWAYS["moss"]))
        for n, G in ((32, G32), (24, G24), (16, G16)):
            write(f"rounds/r3/s{n}-moss.svg", svg_doc(G(), COLORWAYS["moss"]))
            write(f"rounds/r3/s{n}-ink.svg", svg_doc(G(), COLORWAYS["ink"]))
            write(f"rounds/r3/s{n}-moss-hard.svg", svg_doc(G(hard=(0, 1, INK + "55")), COLORWAYS["moss"]))
    if rnd == "r4":
        inkclay = dict(plate=INK, lime=LIME, paper=CLAY, clay=CLAY)
        for n, G in ((32, G32), (24, G24), (16, G16)):
            extra = dict(grad=True, hard=(0, 1, INK + "40")) if n == 32 else {}
            write(f"rounds/r4/s{n}-moss.svg", svg_doc(G(**extra), COLORWAYS["moss"]))
            write(f"rounds/r4/s{n}-ink.svg", svg_doc(G(**extra), COLORWAYS["ink"]))
            write(f"rounds/r4/s{n}-inkclay.svg", svg_doc(G(**extra), inkclay))
    if rnd == "r5":
        cands = {"moss": DARK, "ink": LIGHT, "inkmoss": dict(plate=INK, lime=LIME, paper=MOSS, clay=CLAY)}
        for k, cw in cands.items():
            for n, G in ((32, G32), (24, G24), (16, G16)):
                extra = dict(grad=True, hard=(0, 1, INK + "40")) if n == 32 else {}
                write(f"rounds/r5/s{n}-{k}.svg", svg_doc(G(**extra), cw))
            write(f"rounds/r5/m-{k}.svg", svg_doc(G1024("four"), cw))
    if rnd == "final":
        build_final()
    print("ok", rnd)
