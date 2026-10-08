"""Round 3 · variant 1「原样精修」 — builds every SVG, rasterises with Playwright + Edge, composes sheet.png.

Run:  D:\\conda\\miniconda3\\python.exe docs/logo/round3/variant-1/make.py [tag]
`tag` (optional) also copies sheet.png to wip/<tag>.png so every round is kept.
"""
import base64
import json
import math
import shutil
import sys
from pathlib import Path

from PIL import Image

from geom import contour, rect, f

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]  # .../diskwhy
C2 = HERE.parents[1] / "concept-2"
PNG = HERE / "png"
PNG.mkdir(exist_ok=True)
(HERE / "wip").mkdir(exist_ok=True)

# ---------------------------------------------------------------- palette
INK = "#141412"
LIME = "#d4f24a"
PAPER = "#f6f1e4"
CORAL = "#ff8f73"
RING = PAPER  # outer hairline, only visible on dark grounds

# ---------------------------------------------------------------- master (1024)
U = 16  # 64-unit grid on 1024


def _offset_line(p, q, d):
    (x1, y1), (x2, y2) = p, q
    dx, dy = x2 - x1, y2 - y1
    L = math.hypot(dx, dy)
    nx, ny = -dy / L, dx / L
    return (x1 + nx * d, y1 + ny * d), (x2 + nx * d, y2 + ny * d)


def _ix_h(p, q, y):
    (x1, y1), (x2, y2) = p, q
    return (x1 + (x2 - x1) * (y - y1) / (y2 - y1), y)


def _ix_v(p, q, x):
    (x1, y1), (x2, y2) = p, q
    return (x, y1 + (y2 - y1) * (x - x1) / (x2 - x1))


M = dict(
    W=812, H=708,                # bubble body = 2t + g + blocks  (812 x 708)
    t=44, g=44,                  # rim = gutter = 44 = 2.75 u (on the 4 px sub-grid -> crisp at 256)
    lime_w=408,                  # left column 408 : right column 272  (3 : 2, = 25.5 u : 17 u)
    paper_h=16 * U,              # right column 256 : 320  (1 : 1.25)
    R=216, xi=0.5,               # outer corner 216 = 13.5 u (block corners 216 - 44 = 172), smoothing 50 %
    ri=36,                       # inner corners where blocks meet = 0.75 t
    D=120, run=240,              # tail: drop 120, run 240  (slope 1 : 2 = 26.57 deg)
    rk=160,                      # concave knee where the tail leaves the bottom edge = 10 u
    rt=64,                       # tail tip = 4 u
    rt_in=20,                    # lime tip inside the tail = rt - t (concentric)
    ring=11, ring_op=0.5,        # paper hairline outside the rim = t / 4
)


def master_paths(m, dx=0.0, dy=0.0):
    W, H, t, g = m["W"], m["H"], m["t"], m["g"]
    R, xi, ri = m["R"], m["xi"], m["ri"]
    x0, y0, x1, y1 = dx, dy, dx + W, dy + H
    kx, ty = x0 + m["run"], y1 + m["D"]
    sil = contour([
        ((x0, y0), ("smooth", R, xi)),
        ((x1, y0), ("smooth", R, xi)),
        ((x1, y1), ("smooth", R, xi)),
        ((kx, y1), ("arc", m["rk"])),
        ((x0, ty), ("arc", m["rt"])),
    ])
    ix0, iy0, ix1, iy1 = x0 + t, y0 + t, x1 - t, y1 - t
    Ri = R - t
    sx = ix0 + m["lime_w"]
    sy = iy0 + m["paper_h"]
    a, b = _offset_line((kx, y1), (x0, ty), t)
    pk = _ix_h(a, b, iy1)
    pt = _ix_v(a, b, ix0)
    lime = contour([
        ((ix0, iy0), ("smooth", Ri, xi)),
        ((sx, iy0), ("arc", ri)),
        ((sx, iy1), ("arc", ri)),
        (pk, ("arc", m["rk"] + t)),
        (pt, ("arc", m["rt_in"])),
    ])
    paper = contour([
        ((sx + g, iy0), ("arc", ri)),
        ((ix1, iy0), ("smooth", Ri, xi)),
        ((ix1, sy), ("arc", ri)),
        ((sx + g, sy), ("arc", ri)),
    ])
    coral = contour([
        ((sx + g, sy + g), ("arc", ri)),
        ((ix1, sy + g), ("arc", ri)),
        ((ix1, iy1), ("smooth", Ri, xi)),
        ((sx + g, iy1), ("arc", ri)),
    ])
    return sil, lime, paper, coral


def mark_svg(paths, n, ring_w, ring_op=1.0, join="round", size=None):
    sil, lime, paper, coral = paths
    size = size or n
    ring = ""
    if ring_w:
        ring = (f'<path d="{sil}" fill="none" stroke="{RING}" stroke-opacity="{ring_op:g}" '
                f'stroke-width="{2 * ring_w:g}" stroke-linejoin="{join}"/>\n  ')
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {n} {n}">\n  '
        f'{ring}<path d="{sil}" fill="{INK}"/>\n  <path d="{lime}" fill="{LIME}"/>\n  '
        f'<path d="{paper}" fill="{PAPER}"/>\n  <path d="{coral}" fill="{CORAL}"/>\n</svg>\n'
    )


# ---------------------------------------------------------------- small sizes, drawn on the pixel grid
def small_paths(s):
    """s: integer geometry. tail = list of outer stair vertices after the bottom-right corner
    (ending at the tip on the left edge); tail_in = same for the lime block."""
    x0, y0, x1, y1 = s["body"]
    t, g, R, Ri, ri = s["t"], s["g"], s["R"], s["Ri"], s["ri"]
    A = lambda r: ("arc", r) if r else None
    sil = contour(
        [((x0, y0), A(R)), ((x1, y0), A(R)), ((x1, y1), A(R))]
        + [(p, None) for p in s["tail"][:-1]] + [(s["tail"][-1], A(s.get("tip", 0)))]
    )
    ix0, iy0, ix1, iy1 = x0 + t, y0 + t, x1 - t, y1 - t
    sx, sy = s["sx"], s["sy"]
    lime = contour(
        [((ix0, iy0), A(Ri)), ((sx, iy0), A(ri)), ((sx, iy1), A(ri))]
        + [(p, None) for p in s["tail_in"]]
    )
    paper = rect(sx + g, iy0, ix1, sy, A(ri), A(Ri), A(ri), A(ri))
    coral = rect(sx + g, sy + g, ix1, iy1, A(ri), A(ri), A(Ri), A(ri))
    return sil, lime, paper, coral


def stairs(xk, y, steps, x0):
    """outer stair from knee (xk, y) going down-left: each step = 1 px down then `w` px left."""
    pts, x = [(xk, y)], xk
    for w in steps:
        y += 1
        pts.append((x, y))
        x = max(x0, x - w)
        pts.append((x, y))
    return pts


# 32 px · ring 1 · rim 2 · gutter 2 · body 30 x 24 · lime 14 : right 10 · paper 8 : coral 10
S32 = dict(n=32, body=(1, 2, 31, 26), t=2, g=2, R=6, Ri=4, ri=1, sx=17, sy=12, tip=1)
S32["tail"] = stairs(9, 26, [2, 2, 4], 1)
S32["tail_in"] = stairs(9, 24, [2, 2, 2], 3)
S32["tip"] = 2
# 24 px · ring 1 · rim 1 · gutter 1 · body 22 x 19 · lime 11 : right 8 · paper 7 : coral 9
S24 = dict(n=24, body=(1, 1, 23, 20), t=1, g=1, R=4, Ri=3, ri=0, sx=13, sy=9, tip=1)
S24["tail"] = stairs(8, 20, [2, 2, 3], 1)
S24["tail_in"] = stairs(8, 19, [2, 2, 2], 2)
# 24 px rejected alternative (round 2/3): rim 2 · gutter 2 — too much black at taskbar size
S24B = dict(n=24, body=(1, 1, 23, 20), t=2, g=2, R=4, Ri=2, ri=1, sx=13, sy=9, tip=0)
S24B["tail"] = stairs(7, 20, [2, 2, 2], 1)
S24B["tail_in"] = stairs(7, 18, [2, 2], 3)
# 16 px · ring 1 · rim 1 · gutter 1 · body 14 x 12
S16 = dict(n=16, body=(1, 1, 15, 13), t=1, g=1, R=2, Ri=1, ri=0, sx=9, sy=6, tip=0)
S16["tail"] = stairs(6, 13, [2, 3], 1)
S16["tail_in"] = stairs(6, 12, [2, 2], 2)
# 16 px title-bar mark (known light ground → no ring, body uses the full 16 px)
T16 = dict(n=16, body=(0, 1, 16, 13), t=1, g=1, R=2, Ri=1, ri=0, sx=9, sy=6, tip=0)
T16["tail"] = stairs(5, 13, [2, 3], 0)
T16["tail_in"] = stairs(5, 12, [2, 2], 1)

RING_SMALL_OP = 0.42


def small_svg(s, ring=True):
    return mark_svg(small_paths(s), s["n"], 1 if ring else 0, RING_SMALL_OP, join="miter")


# ---------------------------------------------------------------- word mark
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
    kern = _pair_kern(font)
    extra = {("k", "W"): -64}
    x, ds = 0, []
    for i, gn in enumerate(names):
        pen = SVGPathPen(gs)
        gs[gn].draw(TransformPen(pen, (1, 0, 0, -1, x, 0)))
        ds.append(pen.getCommands())
        x += gs[gn].width + tracking
        if i + 1 < len(names):
            x += kern.get((gn, names[i + 1]), 0) + extra.get((gn, names[i + 1]), 0)
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


def lockup_svg(paths, text_fill, ring_w, cap, gap, body_box, n, ring_op=1.0, join="round"):
    """icon (n x n viewBox) + outlined word; cap = cap height in icon units; gap after body."""
    d, adv = word_paths()
    s = cap / 710.0
    bx0, by0, bx1, by1 = body_box
    base = (by0 + by1) / 2 + cap / 2
    tx = bx1 + gap
    w = tx + adv * s + 0.02 * n
    inner = mark_svg(paths, n, ring_w, ring_op, join).split("\n", 1)[1].rsplit("</svg>", 1)[0]
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.2f} {n}" width="{w:.2f}" height="{n}">\n'
        f"{inner}"
        f'  <path transform="translate({tx:.2f},{base:.2f}) scale({s:.5f})" fill="{text_fill}" d="{d}"/>\n</svg>\n'
    )


# ---------------------------------------------------------------- rendering
def uri(p: Path, mime=None):
    mime = mime or {"svg": "image/svg+xml", "png": "image/png", "woff2": "font/woff2"}[p.suffix[1:]]
    return f"data:{mime};base64," + base64.b64encode(p.read_bytes()).decode()


def raster(page, src: Path, px, out: Path):
    page.set_content(
        f"<html><body style='margin:0;background:transparent'>"
        f"<img id=i src='{uri(src)}' width={px} height={px} style='display:block'></body></html>")
    page.wait_for_function("document.getElementById('i').complete")
    page.locator("#i").screenshot(path=str(out), omit_background=True)
    return out


def centroid(png: Path):
    im = Image.open(png).convert("RGBA")
    a = im.getchannel("A")
    w, h = a.size
    data = a.load()
    sx = sy = tot = 0
    for y in range(h):
        for x in range(w):
            v = data[x, y]
            if v:
                sx += v * x
                sy += v * y
                tot += v
    return (sx / tot + 0.5, sy / tot + 0.5)


def upscale(src: Path, k: int, out: Path):
    im = Image.open(src)
    im.resize((im.width * k, im.height * k), Image.NEAREST).save(out)
    return out


def main(tag=None):
    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        b = p.chromium.launch(channel="msedge")
        pg = b.new_page(device_scale_factor=1)

        # pass 1: draw at origin 0,0 and measure the alpha centroid at 1024 → place it
        probe = HERE / "wip" / "_probe.svg"
        probe.write_text(mark_svg(master_paths(M, 80, 80), 1024, M["ring"], M["ring_op"]), encoding="utf-8")
        cx, cy = centroid(raster(pg, probe, 1024, HERE / "wip" / "_probe.png"))
        target = (512, 512 - 0.015 * 1024)
        dx, dy = 80 + round(target[0] - cx), 80 + round(target[1] - cy)
        paths = master_paths(M, dx, dy)
        (HERE / "icon.svg").write_text(mark_svg(paths, 1024, M["ring"], M["ring_op"]), encoding="utf-8")
        (HERE / "icon-32.svg").write_text(small_svg(S32), encoding="utf-8")
        (HERE / "icon-24.svg").write_text(small_svg(S24), encoding="utf-8")
        (HERE / "icon-16.svg").write_text(small_svg(S16), encoding="utf-8")
        (HERE / "mark-16-A.svg").write_text(small_svg(T16, ring=False), encoding="utf-8")
        (HERE / "mark-16-C.svg").write_text(small_svg(T16, ring=False), encoding="utf-8")
        body = (dx, dy, dx + M["W"], dy + M["H"])
        (HERE / "lockup-light.svg").write_text(
            lockup_svg(paths, INK, M["ring"], 0.50 * M["H"], 0.115 * 1024, body, 1024, M["ring_op"]), encoding="utf-8")
        (HERE / "lockup-dark.svg").write_text(
            lockup_svg(paths, PAPER, M["ring"], 0.50 * M["H"], 0.115 * 1024, body, 1024, M["ring_op"]), encoding="utf-8")
        tp = small_paths(T16)
        for th, col in (("A", "#1c1b18"), ("C", INK)):
            (HERE / f"titlebar-{th}.svg").write_text(
                lockup_svg(tp, col, 0, 8.4, 5, T16["body"], 16), encoding="utf-8")

        # rasters
        R = {}
        for px in (1024, 256, 64, 48):
            R[px] = raster(pg, HERE / "icon.svg", px, PNG / f"icon-{px}.png")
        R[32] = raster(pg, HERE / "icon-32.svg", 32, PNG / "icon-32.png")
        R[24] = raster(pg, HERE / "icon-24.svg", 24, PNG / "icon-24.png")
        R[16] = raster(pg, HERE / "icon-16.svg", 16, PNG / "icon-16.png")
        (HERE / "wip" / "icon-24-alt.svg").write_text(small_svg(S24B), encoding="utf-8")
        R["24b"] = raster(pg, HERE / "wip" / "icon-24-alt.svg", 24, HERE / "wip" / "icon-24-alt.png")
        upscale(R[16], 4, PNG / "icon-16@x4.png")
        upscale(R[24], 4, PNG / "icon-24@x4.png")
        T = raster(pg, HERE / "mark-16-C.svg", 16, HERE / "wip" / "_mark16.png")
        cxf, cyf = centroid(R[1024])

        O = {k: C2 / "png" / v for k, v in {1024: "icon@1024.png", 32: "icon-32@32.png",
                                             24: "icon-24@24.png", 16: "icon-16@16.png"}.items()}
        sheet(b, R, O, T, (cxf, cyf), tag)
        b.close()
    print("centroid", round(cxf, 1), round(cyf, 1), "offset", dx, dy)


def sheet(b, R, O, T, cen, tag):
    u = {k: uri(v) for k, v in R.items()}
    o = {k: uri(v) for k, v in O.items()}
    geist = uri(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2")
    LL, LD = uri(HERE / "lockup-light.svg"), uri(HERE / "lockup-dark.svg")
    TA, TC = uri(HERE / "titlebar-A.svg"), uri(HERE / "titlebar-C.svg")
    OLDLA = uri(C2 / "lockup-C.svg")

    def strip(src, bg, fg):
        cells = "".join(
            f"<div class=cell><img class=px src='{src[s]}' width={s} height={s}><span>{s}</span></div>"
            for s in (32, 24, 16))
        return f"<div class=strip style='background:{bg};color:{fg}'>{cells}</div>"

    def z(src, s, k, bg, label):
        return (f"<div class=z><div class=zb style='background:{bg};padding:{k * 2}px'>"
                f"<img class=px src='{src}' width={s * k} height={s * k}></div><span>{label}</span></div>")

    def tbar(src, h, bg, fg, border, w=None):
        return (f"<div class=tb style='background:{bg};color:{fg};border-bottom:{border}'>"
                f"<img src='{src}' height={h}><div style='flex:1'></div><i>—</i><i>▢</i><i>✕</i></div>")

    html = f"""<html><head><style>
    @font-face {{ font-family: Geist; src: url({geist}) format('woff2'); font-weight: 100 900; }}
    * {{ box-sizing:border-box }}
    body {{ margin:0; width:1600px; background:#efebe1; font-family:Geist,'Microsoft YaHei UI',sans-serif; color:#141412 }}
    .wrap {{ padding:40px 48px 48px }}
    h1 {{ font-size:28px; margin:0 0 4px; letter-spacing:-.02em }}
    .sub {{ color:#6b665c; font-size:14px; margin-bottom:22px }}
    h2 {{ font-size:12px; letter-spacing:.12em; color:#6b665c; margin:26px 0 10px; font-weight:600; text-transform:uppercase }}
    .row {{ display:flex; gap:16px; align-items:stretch }}
    .card {{ border-radius:18px; display:flex; flex-direction:column; align-items:center; justify-content:center; gap:10px; padding:18px }}
    .card span, .cell span, .z span {{ font-size:12px; color:#8a8579 }}
    .px {{ image-rendering:pixelated }}
    .strip {{ display:flex; gap:26px; align-items:flex-end; padding:16px 22px; border-radius:12px }}
    .cell {{ display:flex; flex-direction:column; align-items:center; gap:6px }}
    .z {{ display:flex; flex-direction:column; align-items:center; gap:6px }}
    .zb {{ border-radius:8px; display:flex }}
    .tb {{ height:40px; display:flex; align-items:center; padding:0 14px; border-radius:8px; gap:20px; font-size:13px; flex:1 }}
    .tb i {{ font-style:normal; opacity:.5 }}
    .tag {{ font-size:12px; font-weight:700; padding:3px 8px; border-radius:6px; background:#141412; color:#f6f1e4 }}
    .lab {{ display:flex; gap:8px; align-items:center; font-size:15px; font-weight:650; margin-bottom:8px }}
    </style></head><body><div class=wrap>
    <h1>DiskWhy · 第三轮 · 变体 1「原样精修」</h1>
    <div class=sub>左：第一版方案 2 原图 · 右：同一个图标，按 64 格（1 u = 16 px，4 px 细格）重画：外框 = 缝 = 44，左右 3 : 2，右列上下 1 : 1.25，外圆角 216 + 50% 平滑、色块圆角 172 同心、相邻角 36，尾巴 1 : 2 斜度从框里长出来，平涂，深色底一圈 11 px 米白细边。重心 ({cen[0]:.0f}, {cen[1]:.0f})</div>
    <div class=row>
      <div style='flex:1'><div class=lab><span class=tag style='background:#8a8579'>第一版</span>方案 2 原图</div>
        <div class=row>
          <div class=card style='background:#f3f3f3;flex:1;height:340px'><img src='{o[1024]}' width=300></div>
          <div class=card style='background:#202020;flex:1;height:340px'><img src='{o[1024]}' width=300></div>
        </div></div>
      <div style='flex:1'><div class=lab><span class=tag>变体 1</span>原样精修</div>
        <div class=row>
          <div class=card style='background:#f3f3f3;flex:1;height:340px'><img src='{u[1024]}' width=300></div>
          <div class=card style='background:#202020;flex:1;height:340px'><img src='{u[1024]}' width=300></div>
        </div></div>
    </div>

    <h2>任务栏 1× · 浅色 #f3f3f3 / 深色 #202020 · 左：第一版　右：变体 1</h2>
    <div class=row>
      {strip(o, '#f3f3f3', '#777')}{strip(u, '#f3f3f3', '#777')}
      {strip(o, '#202020', '#999')}{strip(u, '#202020', '#999')}
      <div class=strip style='background:#f3f3f3'><div class=cell><img src='{u[64]}' width=64><span>64</span></div><div class=cell><img src='{u[48]}' width=48><span>48</span></div></div>
      <div class=strip style='background:#202020;color:#999'><div class=cell><img src='{u[64]}' width=64><span>64</span></div><div class=cell><img src='{u[48]}' width=48><span>48</span></div></div>
    </div>

    <h2>小尺寸 ×6 最近邻放大</h2>
    <div class=row style='flex-wrap:wrap;gap:14px'>
      {z(o[16], 16, 6, '#f3f3f3', '第一版 16')}{z(u[16], 16, 6, '#f3f3f3', '16 浅')}{z(u[16], 16, 6, '#202020', '16 深')}
      {z(o[24], 24, 6, '#f3f3f3', '第一版 24')}{z(u[24], 24, 6, '#f3f3f3', '24 浅')}{z(u[24], 24, 6, '#202020', '24 深')}
    </div>
    <div class=row style='flex-wrap:wrap;gap:14px;margin-top:14px'>
      {z(o[32], 32, 5, '#f3f3f3', '第一版 32')}{z(u[32], 32, 5, '#f3f3f3', '32 浅')}{z(u[32], 32, 5, '#202020', '32 深')}
      {z(uri(T), 16, 6, '#f4f0e6', '标题栏 16（无白边）')}
      {z(u["24b"], 24, 5, '#f3f3f3', '24 旧 2/2（弃）')}{z(u["24b"], 24, 5, '#202020', '24 旧 2/2 深（弃）')}
    </div>

    <h2>标题栏 · C 卡通 / A 浅色 · 1× 与 2×</h2>
    <div class=row>{tbar(TC, 16, '#f4f0e6', '#141412', '2px solid #141412')}{tbar(TA, 16, '#f3f1ec', '#1c1b18', '1px solid #e1ddd4')}{tbar(OLDLA, 16, '#f4f0e6', '#141412', '2px solid #141412')}</div>
    <div class=row style='margin-top:10px'>{tbar(TC, 32, '#f4f0e6', '#141412', '2px solid #141412')}{tbar(TA, 32, '#f3f1ec', '#1c1b18', '1px solid #e1ddd4')}</div>

    <h2>横排组合</h2>
    <div class=row>
      <div class=card style='background:#f4f0e6;flex:1;height:170px'><img src='{LL}' height=104></div>
      <div class=card style='background:#14110e;flex:1;height:170px'><img src='{LD}' height=104></div>
    </div>
    </div></body></html>"""
    (HERE / "wip" / "sheet.html").write_text(html, encoding="utf-8")
    pg = b.new_page(viewport={"width": 1600, "height": 900}, device_scale_factor=1)
    pg.set_content(html)
    pg.wait_for_timeout(500)
    pg.screenshot(path=str(HERE / "sheet.png"), full_page=True)
    if tag:
        shutil.copy(HERE / "sheet.png", HERE / "wip" / f"{tag}.png")
    # side-by-side big compare for judging
    pg.set_content(f"<html><body style='margin:0;background:#f3f3f3;display:flex;gap:0'>"
                   f"<img src='{o[1024]}' width=640><img src='{u[1024]}' width=640></body></html>")
    pg.set_viewport_size({"width": 1280, "height": 640})
    pg.wait_for_timeout(200)
    pg.screenshot(path=str(HERE / "wip" / f"{tag or 'cur'}-big.png"))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else None)
