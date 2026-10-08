"""Round 3 · variant 2 「贴纸版」 — generator for every SVG in this folder.

Run:  D:\\conda\\miniconda3\\python.exe docs/logo/round3/variant-2/build.py
(render.py imports this; it also measures the alpha centroid and feeds it back as OFFSET.)
"""
import json
from pathlib import Path

import geom
from pixels import SMALL, pixel_svg

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]

INK = "#141412"
LIME = "#d4f24a"
PAPER = "#f6f1e4"
CORAL = "#ff8a73"
STICKER = "#ffffff"

U = 16  # 64-unit grid on the 1024 canvas
# ---- master geometry, all in grid units (1 u = 16 px) ----------------------------------------
P = dict(
    t=3,          # ink rim = ink gutter (one weight everywhere)
    lime=24,      # left column width   } 3 : 2
    right=16,     # right column width  }
    paper=15,       # right column top    } 1 : 1.25
    coral=18.75, # right column bottom }
    R=10.75,        # outer squircle radius (middle arc)
    xi=0.45,      # corner smoothing
    ri=2.25,      # inner corners where blocks meet a gutter (0.75 t)
    s=1.75,       # sticker (die-cut) margin outside the ink
    sh=2,         # hard shadow offset (right & down)
    B=14,         # tail: joint on the bottom edge, measured from the left edge
    D=7,          # tail: tip drop below the bottom edge
    k=10,         # tail: concave sweep radius where the bottom edge turns into the tail
    tr=3,         # tail: tip radius
    tip_in=1,     # minimum inner (lime) tip radius
)

OFFSET_FILE = HERE / "_offset.json"


def master_shapes(p=P):
    t, g = p["t"] * U, p["t"] * U
    W = (2 * p["t"] + p["lime"] + p["t"] + p["right"]) * U
    H = (2 * p["t"] + p["paper"] + p["t"] + p["coral"]) * U
    R = p["R"] * U
    reach = R * (1 + p["xi"])
    sq = ("sq", R, reach)
    B, D = p["B"] * U, p["D"] * U
    sil_v = [(0, 0), (W, 0), (W, H), (B, H), (0, H + D)]
    sil_s = [sq, sq, sq, ("arc", p["k"] * U), ("arc", p["tr"] * U)]
    inn_v, inn_s = geom.offset(sil_v, sil_s, t, min_r=p["tip_in"] * U)
    ri = p["ri"] * U
    sx = t + p["lime"] * U
    sy = t + p["paper"] * U
    lime_v = [inn_v[0], (sx, t), (sx, H - t), inn_v[3], inn_v[4]]
    lime_s = [inn_s[0], ("arc", ri), ("arc", ri), inn_s[3], inn_s[4]]
    # sanity: the tail sweep must end before the lime block's own bottom-right corner
    assert inn_v[3][0] + geom.reach(inn_v, inn_s, 3) < sx - ri, "tail sweep too wide"
    paper_v = [(sx + g, t), (W - t, t), (W - t, sy), (sx + g, sy)]
    paper_s = [("arc", ri), inn_s[1], ("arc", ri), ("arc", ri)]
    coral_v = [(sx + g, sy + g), (W - t, sy + g), (W - t, H - t), (sx + g, H - t)]
    coral_s = [("arc", ri), ("arc", ri), inn_s[2], ("arc", ri)]
    for vs, ss, name in ((paper_v, paper_s, "paper"), (coral_v, coral_s, "coral")):
        h = vs[2][1] - vs[1][1]
        w = vs[1][0] - vs[0][0]
        need = inn_s[1][2] + ri
        assert h >= need - 1e-6 and w >= need - 1e-6, (name, w, h, need)
    st_v, st_s = geom.offset(sil_v, sil_s, -p["s"] * U)
    return dict(
        W=W, H=H,
        sticker=geom.path(st_v, st_s),
        sil=geom.path(sil_v, sil_s),
        lime=geom.path(lime_v, lime_s),
        paper=geom.path(paper_v, paper_s),
        coral=geom.path(coral_v, coral_s),
        sh=p["sh"] * U,
    )


def mark_markup(m, shadow=True, sticker=True):
    out = []
    if shadow and sticker:
        out.append(f'<path d="{m["sticker"]}" fill="{INK}" transform="translate({m["sh"]},{m["sh"]})"/>')
    if sticker:
        out.append(f'<path d="{m["sticker"]}" fill="{STICKER}"/>')
    out += [
        f'<path d="{m["sil"]}" fill="{INK}"/>',
        f'<path d="{m["lime"]}" fill="{LIME}"/>',
        f'<path d="{m["paper"]}" fill="{PAPER}"/>',
        f'<path d="{m["coral"]}" fill="{CORAL}"/>',
    ]
    return "\n    ".join(out)


def load_offset(m):
    if OFFSET_FILE.exists():
        o = json.loads(OFFSET_FILE.read_text())
        return o["x"], o["y"]
    # first guess: centre the footprint box
    s, sh = P["s"] * U, P["sh"] * U
    fw = m["W"] + 2 * s + sh
    fh = m["H"] + P["D"] * U + 2 * s + sh
    return (1024 - fw) / 2 + s, (1024 - fh) / 2 + s


def icon_svg(m, ox, oy, shadow=True, sticker=True):
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">\n'
        f'  <g transform="translate({geom.fmt(ox)},{geom.fmt(oy)})">\n    {mark_markup(m, shadow, sticker)}\n  </g>\n</svg>\n'
    )


# ---- wordmark ---------------------------------------------------------------------------------
EXTRA_KERN = {("k", "W"): -38}


def word_paths(text="DiskWhy", wght=700, tracking=-14):
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    from fontTools.ttLib import TTFont
    from fontTools.varLib.instancer import instantiateVariableFont

    font = TTFont(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2")
    font = instantiateVariableFont(font, {"wght": wght})
    cmap = font.getBestCmap()
    gs = font.getGlyphSet()
    names = [cmap[ord(c)] for c in text]
    x, ds = 0, []
    for i, gn in enumerate(names):
        pen = SVGPathPen(gs)
        gs[gn].draw(TransformPen(pen, (1, 0, 0, -1, x, 0)))
        ds.append(pen.getCommands())
        x += gs[gn].width + tracking
        if i + 1 < len(names):
            x += EXTRA_KERN.get((gn, names[i + 1]), 0)
    return " ".join(ds), x - tracking


def lockup_svg(m, text_fill, cap_ratio=0.36, gap_u=5):
    """Large lockup on a 1024-high artboard: mark (with sticker + shadow) + Geist Bold word."""
    d, adv = word_paths()
    s = cap_ratio * 1024 / 710.0  # Geist cap height = 710 units
    st, sh = P["s"] * U, P["sh"] * U
    ox, oy = st, st  # mark's ink box starts after the sticker margin
    top = 0
    body_cy = oy + m["H"] / 2 + sh / 2
    base = body_cy + 710 * s / 2
    tx = ox + m["W"] + st + sh + gap_u * U
    w = tx + adv * s + 8
    h = oy + m["H"] + P["D"] * U + st + sh
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 {top} {geom.fmt(w)} {geom.fmt(h)}" '
        f'width="{geom.fmt(w)}" height="{geom.fmt(h)}">\n'
        f'  <g transform="translate({ox},{oy})">\n    {mark_markup(m)}\n  </g>\n'
        f'  <path transform="translate({geom.fmt(tx)},{geom.fmt(base)}) scale({s:.5f})" fill="{text_fill}" d="{d}"/>\n</svg>\n'
    )


def titlebar_svg(text_fill, bg, size=16, cap_px=8.5, gap=6):
    """A 16-px title-bar lockup: the hand-drawn 16 mark + the word at UI size, on a theme bg."""
    d, adv = word_paths(wght=650, tracking=-6)
    s = cap_px / 710.0
    px_svg = pixel_svg(SMALL[16], inner_only=True)
    tx = 8 + size + gap
    base = 8 + 7.5 + cap_px / 2  # caps centred on the bubble body (rows ~3..12)
    w = tx + adv * s + 8
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{geom.fmt(w)}" height="32" viewBox="0 0 {geom.fmt(w)} 32">\n'
        f'  <rect width="100%" height="100%" fill="{bg}"/>\n'
        f'  <g transform="translate(8,8)" shape-rendering="crispEdges">{px_svg}</g>\n'
        f'  <path transform="translate({geom.fmt(tx)},{geom.fmt(base)}) scale({s:.6f})" fill="{text_fill}" d="{d}"/>\n</svg>\n'
    )


def write_all():
    m = master_shapes()
    ox, oy = load_offset(m)
    (HERE / "icon.svg").write_text(icon_svg(m, ox, oy), encoding="utf-8")
    for n in (32, 24, 16):
        (HERE / f"icon-{n}.svg").write_text(pixel_svg(SMALL[n]), encoding="utf-8")
    (HERE / "lockup-light.svg").write_text(lockup_svg(m, INK), encoding="utf-8")
    (HERE / "lockup-dark.svg").write_text(lockup_svg(m, PAPER), encoding="utf-8")
    (HERE / "titlebar-A.svg").write_text(titlebar_svg("#1c1b18", "#f3f1ec"), encoding="utf-8")
    (HERE / "titlebar-C.svg").write_text(titlebar_svg("#141412", "#f4f0e6"), encoding="utf-8")
    return m


if __name__ == "__main__":
    write_all()
    print("ok")
