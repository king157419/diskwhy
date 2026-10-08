r"""Variant 3 · Layered Plate — rasterise (Playwright + installed Edge) and compose sheet.png.

Run:  D:\conda\miniconda3\python.exe docs/logo/round2/variant-3/render.py
Writes png/*.png, png/centroid.png, metrics.json and sheet.png next to this file.
"""
import json
from pathlib import Path

from PIL import Image, ImageDraw

import rast
from rast import b64

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
OUT = HERE / "png"
OUT.mkdir(exist_ok=True)
GEIST = b64(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2", "font/woff2")

PAL = dict(moss="#5c7a1a", lime="#d4f24a", paper="#f6f1e4", clay="#8a857a", ink="#141412")


def x4(png, k=4):
    im = Image.open(png)
    big = im.resize((im.width * k, im.height * k), Image.NEAREST)
    out = png.with_name(png.stem + f"x{k}.png")
    big.save(out)
    return out


def centroid_overlay(png, out):
    c = rast.centroid(png)
    im = Image.open(png).convert("RGBA")
    W, H = im.size
    bg = Image.new("RGBA", (W, H), (243, 243, 243, 255))
    bg.alpha_composite(im)
    d = ImageDraw.Draw(bg)
    for i in range(0, W + 1, 16):  # 64-unit grid
        d.line([(i, 0), (i, H)], fill=(0, 0, 0, 18) if i % 64 else (0, 0, 0, 40))
        d.line([(0, i), (W, i)], fill=(0, 0, 0, 18) if i % 64 else (0, 0, 0, 40))
    d.rectangle([64, 64, 960, 960], outline=(150, 150, 150, 255), width=3)  # 4u safe margin
    d.line([(512, 0), (512, H)], fill=(0, 120, 255, 200), width=3)
    d.line([(0, 512), (W, 512)], fill=(0, 120, 255, 200), width=3)
    d.rectangle([504, 488, 520, 504], outline=(0, 160, 0, 255), width=3)  # target (512±8, 496±8)
    cx, cy = c["cx"], c["cy"]
    d.ellipse([cx - 14, cy - 14, cx + 14, cy + 14], fill=(230, 30, 30, 230))
    bg.convert("RGB").save(out)
    return c


def main():
    R = rast.Raster()
    P = {}
    # masters
    for name in ("icon", "icon-light"):
        for px in (1024, 256, 64, 48):
            P[(name, px)] = R.svg(HERE / f"{name}.svg", px, OUT / f"{name}@{px}.png")
    # hand-tuned small sizes
    for n in (32, 24, 16):
        for suf in ("", "-light"):
            p = R.svg(HERE / f"icon-{n}{suf}.svg", n, OUT / f"icon-{n}{suf}@{n}.png")
            P[(f"icon{suf}", n)] = p
            if n in (16, 24):
                x4(p)
    for t in "ABC":
        P[("theme", t)] = R.svg(HERE / f"icon-16-{t}.svg", 16, OUT / f"icon-16-{t}@16.png")
    naive = {n: R.svg(HERE / "icon.svg", n, OUT / f"naive@{n}.png") for n in (32, 24, 16)}

    # metrics
    c = centroid_overlay(P[("icon", 1024)], OUT / "centroid.png")
    areas = rast.areas(P[("icon", 1024)], PAL)
    small = {n: rast.areas(P[("icon", n)], PAL) for n in (32, 24, 16)}
    metrics = dict(centroid=dict(x=round(c["cx"], 1), y=round(c["cy"], 1)), bbox=c["bbox"],
                   silhouette_fill=round(c["fill"], 3),
                   area_1024={k: round(v, 3) for k, v in areas.items()},
                   area_small={n: {k: round(v, 3) for k, v in a.items()} for n, a in small.items()})
    (HERE / "metrics.json").write_text(json.dumps(metrics, indent=1), encoding="utf-8")

    U = {k: b64(v, "image/png") for k, v in P.items()}
    N = {n: b64(v, "image/png") for n, v in naive.items()}

    def bar(bg, fg, theme):
        key = "icon-light" if theme == "light" else "icon"
        cells = "".join(
            f"<div class=cell><img class=px src='{U[(key, s)]}' width={s} height={s}><span>{s}</span></div>"
            for s in (64, 48, 32, 24, 16))
        return f"<div class=bar style='background:{bg};color:{fg}'>{cells}<div class=bl>{'浅色任务栏 · ink plate + moss column (lightunplated)' if theme=='light' else '深色任务栏 · moss plate'}</div></div>"

    def zoom(src, n, bg, label, k=4):
        return (f"<div class=z><div class=zbox style='background:{bg};width:{n*k+24}px;height:{n*k+24}px'>"
                f"<img class=px src='{src}' width={n*k} height={n*k}></div><span>{label}</span></div>")

    def titlebar(t, bg, fg):
        return (f"<div class=tb style='background:{bg};color:{fg}'><img src='{b64(HERE / f'lockup-{t}.svg', 'image/svg+xml')}' height=16>"
                f"<span class=tl>{t} · {'浅色' if t=='A' else '深色' if t=='B' else '卡通'}</span><div class=sp></div><i>—</i><i>▢</i><i>✕</i></div>")

    html = f"""<html><head><style>
    @font-face {{ font-family: Geist; src: url({GEIST}) format('woff2'); font-weight: 100 900; }}
    * {{ box-sizing:border-box }}
    body {{ margin:0; width:1600px; background:#ece8de; font-family:Geist,'Segoe UI','Microsoft YaHei',sans-serif; color:#141412 }}
    .wrap {{ padding:44px 52px 52px }}
    h1 {{ font-size:30px; font-weight:650; letter-spacing:-.02em; margin:0 0 6px }}
    .sub {{ color:#6b665c; font-size:15px; margin-bottom:24px }}
    h2 {{ font-size:12px; text-transform:uppercase; letter-spacing:.12em; color:#6b665c; margin:30px 0 12px; font-weight:600 }}
    .row {{ display:flex; gap:18px; align-items:stretch }}
    .hero {{ flex:1; height:340px; border-radius:20px; display:flex; align-items:center; justify-content:center; gap:40px }}
    .bar {{ display:flex; gap:34px; align-items:flex-end; padding:20px 28px; border-radius:14px; flex:1; position:relative }}
    .bl {{ position:absolute; right:22px; top:16px; font-size:12px; opacity:.6 }}
    .cell {{ display:flex; flex-direction:column; align-items:center; gap:8px; font-size:12px }}
    .px {{ image-rendering:pixelated }}
    .z {{ display:flex; flex-direction:column; align-items:center; gap:8px; font-size:12px; color:#6b665c }}
    .zbox {{ display:flex; align-items:center; justify-content:center; border-radius:10px }}
    .tb {{ height:40px; display:flex; align-items:center; padding:0 16px; border-radius:10px; font-size:12px; gap:20px }}
    .tb i {{ font-style:normal; opacity:.5; font-size:12px }} .tl {{ opacity:.45 }} .sp {{ flex:1 }}
    .lock {{ flex:1; height:190px; border-radius:20px; display:flex; align-items:center; justify-content:center }}
    .meta {{ font-size:13px; color:#4a463e; line-height:1.65 }}
    .meta b {{ color:#141412 }}
    </style></head><body><div class=wrap>
    <h1>DiskWhy · round 2 · variant 3 · 叠层 Layered Plate</h1>
    <div class=sub>一块苔绿色的对话气泡板，上面平放着几块大小不一的“磁盘方块”，光从左上来——磁盘把自己的每一块摆出来给你看。 moss · lime · paper · clay · Geist 600</div>
    <div class=row>
      <div class=hero style='background:#f3f3f3'><img src='{U[("icon",1024)]}' width=256><img src='{U[("icon",64)]}' width=64><img class=px src='{U[("icon",32)]}' width=32></div>
      <div class=hero style='background:#202020'><img src='{U[("icon",1024)]}' width=256><img src='{U[("icon",64)]}' width=64><img class=px src='{U[("icon",32)]}' width=32></div>
      <div class=hero style='background:#ffffff;flex:.62;flex-direction:column;gap:10px'><img src='{b64(OUT / "centroid.png", "image/png")}' width=250><div class=meta style='font-size:12px'>质心 ({metrics['centroid']['x']}, {metrics['centroid']['y']}) · 目标 (512±8, 496±8)</div></div>
    </div>

    <h2>Taskbar · 1× pixels · 64 48 (master) · 32 24 16 (hand-tuned)</h2>
    <div class=row>{bar('#f3f3f3', '#555', 'light')}{bar('#202020', '#aaa', 'dark')}</div>

    <h2>Small sizes ×4 nearest-neighbour · hand-tuned vs naive downscale of the 1024 master</h2>
    <div class=row style='gap:22px;flex-wrap:wrap;align-items:flex-end'>
      {zoom(U[("icon-light",16)], 16, '#f3f3f3', '16 · light')}
      {zoom(U[("icon",16)], 16, '#202020', '16 · dark')}
      {zoom(N[16], 16, '#f3f3f3', '16 naive')}
      {zoom(U[("icon-light",24)], 24, '#f3f3f3', '24 · light')}
      {zoom(U[("icon",24)], 24, '#202020', '24 · dark')}
      {zoom(N[24], 24, '#202020', '24 naive')}
      {zoom(U[("icon-light",32)], 32, '#f3f3f3', '32 · light')}
      {zoom(U[("icon",32)], 32, '#202020', '32 · dark')}
      {zoom(N[32], 32, '#202020', '32 naive')}
    </div>

    <h2>Title bar · 16 px mark · themes A / B / C</h2>
    <div style='display:grid;gap:10px'>
      {titlebar('A', '#f3f1ec', '#1c1b18')}
      {titlebar('B', '#1b1714', '#f3ece2')}
      {titlebar('C', '#f3efe3', '#141412')}
    </div>
    <div class=row style='gap:22px;margin-top:14px'>
      {zoom(U[("theme","A")], 16, '#f3f1ec', 'A ×4')}{zoom(U[("theme","B")], 16, '#1b1714', 'B ×4')}{zoom(U[("theme","C")], 16, '#f3efe3', 'C ×4')}
    </div>

    <h2>Horizontal lockup</h2>
    <div class=row>
      <div class=lock style='background:#f4f0e6'><img src='{b64(HERE / "lockup.svg", "image/svg+xml")}' height=110></div>
      <div class=lock style='background:#14110e'><img src='{b64(HERE / "lockup-dark.svg", "image/svg+xml")}' height=110></div>
    </div>
    </div></body></html>"""
    (HERE / "_tmp").mkdir(exist_ok=True)
    (HERE / "_tmp" / "sheet.html").write_text(html, encoding="utf-8")
    R.html(html, HERE / "sheet.png", width=1600)
    R.close()
    print(json.dumps(metrics))


if __name__ == "__main__":
    main()
