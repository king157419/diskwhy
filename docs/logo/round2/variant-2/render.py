r"""Variant 2 · Grown Tail — rasterise with Playwright + installed Edge, measure, compose sheet.png.

Run:  D:\conda\miniconda3\python.exe docs/logo/round2/variant-2/render.py
"""
import base64
import sys
from pathlib import Path

from PIL import Image, ImageDraw
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import build  # noqa: E402
from measure import measure  # noqa: E402

ROOT = HERE.parents[3]
OUT = HERE / "png"
OUT.mkdir(exist_ok=True)


def b64(p: Path, mime: str) -> str:
    return f"data:{mime};base64," + base64.b64encode(p.read_bytes()).decode()


def svg_uri(name):
    return b64(HERE / name, "image/svg+xml")


GEIST = b64(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2", "font/woff2")
NOTO = b64(ROOT / "src" / "assets" / "fonts" / "NotoSansSC-DiskWhy.woff2", "font/woff2")

# drawing used at each pixel size
SRC = {1024: "icon.svg", 256: "icon.svg", 64: "icon.svg", 48: "icon.svg", 32: "icon-32.svg", 24: "icon-24.svg", 16: "icon-16.svg"}


def raster(page, name, px, out_name):
    page.set_content(f"<html><body style='margin:0;background:transparent'>"
                     f"<img id=i src='{svg_uri(name)}' width={px} height={px} style='display:block'></body></html>")
    page.wait_for_function("document.getElementById('i').complete")
    out = OUT / out_name
    page.locator("#i").screenshot(path=str(out), omit_background=True)
    return out


def x4(png: Path, bg, k=4):
    """Nearest-neighbour enlargement on a background, for the sheet and for the record."""
    im = Image.open(png).convert("RGBA")
    s = im.size[0]
    base = Image.new("RGBA", (s, s), bg)
    base.alpha_composite(im)
    big = base.resize((s * k, s * k), Image.NEAREST)
    out = png.with_name(png.stem + f"-x{k}-{'dark' if bg[0] < 100 else 'light'}.png")
    big.convert("RGB").save(out)
    return out


def main():
    build.write_all()
    R = {}
    with sync_playwright() as p:
        b = p.chromium.launch(channel="msedge")
        pg = b.new_page(device_scale_factor=1)
        for px, src in SRC.items():
            R[px] = raster(pg, src, px, f"icon@{px}.png")
            lsrc = src.replace(".svg", "-light.svg")
            R[f"L{px}"] = raster(pg, lsrc, px, f"icon-light@{px}.png")
        R["n16"] = raster(pg, "icon.svg", 16, "naive@16.png")
        R["n24"] = raster(pg, "icon.svg", 24, "naive@24.png")
        R["n32"] = raster(pg, "icon.svg", 32, "naive@32.png")
        for k in ("16", "24"):
            x4(R[int(k)], (32, 32, 32, 255))
            x4(R[f"L{k}"], (243, 243, 243, 255))
        m = measure(R[1024], OUT / "centroid-overlay.png")
        ml = measure(R["L1024"])
        print("master", m)
        print("light ", ml)

        U = {k: b64(v, "image/png") for k, v in R.items()}
        OV = b64(OUT / "centroid-overlay.png", "image/png")
        TB = {t: svg_uri(f"lockup-{t}.svg") for t in ("A", "B", "C")}

        def bar(bg, fg, light):
            pre = "L" if light else ""
            cells = "".join(
                f"<div class=cell><img class=px src='{U[pre + str(s) if light else s]}' width={s} height={s}><span>{s}</span></div>"
                for s in (48, 32, 24, 16))
            return f"<div class=bar style='background:{bg};color:{fg}'>{cells}</div>"

        def zoom(key, s, bg, label):
            return (f"<div class=z><div class=zbox style='background:{bg};width:{s*4+24}px;height:{s*4+24}px'>"
                    f"<img class=px src='{U[key]}' width={s*4} height={s*4}></div><span>{label}</span></div>")

        def tbar(t, bg, fg, border=""):
            return (f"<div class=tb style='background:{bg};color:{fg};{border}'>"
                    f"<img class=px src='{TB[t]}' height=16><div style='flex:1'></div><i>—</i><i>▢</i><i>✕</i></div>"
                    f"<div class=tb2 style='background:{bg};color:{fg};{border}'>"
                    f"<img class=px src='{TB[t]}' height=32><div style='flex:1'></div><i>—</i><i>▢</i><i>✕</i></div>")

        def one_x(src, s, bg):
            return f"<span class=one style='background:{bg}'><img class=px src='{U[src]}' width={s} height={s}></span>"

        html = f"""<html><head><style>
        @font-face {{ font-family: Geist; src: url({GEIST}) format('woff2'); font-weight: 100 900; }}
        @font-face {{ font-family: NotoSC; src: url({NOTO}) format('woff2'); }}
        * {{ box-sizing:border-box }}
        body {{ margin:0; width:1600px; background:#ebe7dc; font-family:Geist, NotoSC, 'Segoe UI', sans-serif; color:#141412 }}
        .wrap {{ padding:44px 52px 52px }}
        h1 {{ font-size:30px; font-weight:650; letter-spacing:-.02em; margin:0 0 6px }}
        .sub {{ color:#6b665c; font-size:15px; margin-bottom:26px }}
        h2 {{ font-size:12.5px; text-transform:uppercase; letter-spacing:.12em; color:#6b665c; margin:30px 0 12px; font-weight:600 }}
        .row {{ display:flex; gap:18px; align-items:stretch }}
        .hero {{ flex:1; height:360px; border-radius:20px; display:flex; align-items:center; justify-content:center; gap:40px }}
        .bar {{ display:flex; gap:34px; align-items:flex-end; padding:20px 28px; border-radius:14px; flex:1 }}
        .cell {{ display:flex; flex-direction:column; align-items:center; gap:8px; font-size:12px }}
        .px {{ image-rendering: pixelated }}
        .z {{ display:flex; flex-direction:column; align-items:center; gap:7px; font-size:12px; color:#6b665c }}
        .zbox {{ display:flex; align-items:center; justify-content:center; border-radius:10px }}
        .tb, .tb2 {{ display:flex; align-items:center; padding:0 16px; border-radius:10px; font-size:13px; gap:22px }}
        .tb {{ height:34px }} .tb2 {{ height:60px; font-size:20px; gap:34px }}
        .tb i, .tb2 i {{ font-style:normal; opacity:.55 }}
        .lock {{ flex:1; height:190px; border-radius:20px; display:flex; align-items:center; justify-content:center }}
        .one {{ display:inline-flex; padding:8px; border-radius:8px; margin-right:6px }}
        .meta {{ font-size:13px; color:#6b665c; line-height:1.6 }}
        .meta b {{ color:#141412; font-weight:600 }}
        </style></head><body><div class=wrap>
        <h1>DiskWhy · round 2 · variant 2 — 长出来的尾巴 Grown Tail</h1>
        <div class=sub>一块青柠色的砖就是气泡本身：尾巴是它左边那条边直接长下来的，右侧切进两块小砖。lime 32 u | tiles 20 u（≈φ）· 大砖 : 小砖 = 2 : 1 · R 8/4/2/1 u · 45° 尾巴</div>

        <div class=row>
          <div class=hero style='background:#f3f3f3'><img src='{U[1024]}' width=256><img src='{U[64]}' width=64><img class=px src='{U[32]}' width=32></div>
          <div class=hero style='background:#202020'><img src='{U[1024]}' width=256><img src='{U[64]}' width=64><img class=px src='{U[32]}' width=32></div>
          <div class=hero style='background:#f3f3f3;flex:.62;flex-direction:column;gap:14px'><img src='{U["L1024"]}' width=200><span class=meta>浅色任务栏版 · lightunplated</span></div>
        </div>

        <h2>Taskbar · 1× pixels · main icon (48 32 24 16) | light-taskbar version on #f3f3f3</h2>
        <div class=row>{bar('#f3f3f3', '#555', False)}{bar('#202020', '#aaa', False)}{bar('#f3f3f3', '#555', True)}</div>

        <h2>Small sizes ×4, nearest neighbour — hand-drawn vs naive downscale of the 1024 master</h2>
        <div class=row style='gap:22px;flex-wrap:wrap;align-items:flex-end'>
          {zoom(16, 16, '#202020', '16 hand · dark')}
          {zoom('n16', 16, '#202020', '16 naive · dark')}
          {zoom('L16', 16, '#f3f3f3', '16 hand · light')}
          {zoom(16, 16, '#f3f3f3', '16 main · light')}
          {zoom(24, 24, '#202020', '24 hand · dark')}
          {zoom('n24', 24, '#202020', '24 naive · dark')}
          {zoom('L24', 24, '#f3f3f3', '24 hand · light')}
          {zoom(32, 32, '#202020', '32 hand · dark')}
          {zoom('n32', 32, '#202020', '32 naive')}
        </div>

        <h2>Title bars · 16 px mark (1×) and the same at 2×</h2>
        <div style='display:grid;gap:10px'>
          {tbar('A', '#f3f1ec', '#1c1b18', 'border:1px solid #e2ddd2')}
          {tbar('B', '#1b1714', '#f3ece2')}
          {tbar('C', '#f3efe3', '#141412', 'border:2px solid #141412;box-shadow:3px 3px 0 #141412')}
        </div>

        <h2>Horizontal lockup</h2>
        <div class=row>
          <div class=lock style='background:#f4f0e6'><img src='{svg_uri("lockup.svg")}' height=104></div>
          <div class=lock style='background:#14110e'><img src='{svg_uri("lockup-dark.svg")}' height=104></div>
        </div>

        <h2>Construction · 64-unit grid, safe area, alpha centroid (red) vs target y = 496 (pink)</h2>
        <div class=row>
          <div style='background:#fff;border-radius:16px;padding:16px'><img src='{OV}' width=420></div>
          <div class=meta style='padding:10px 18px;flex:1'>
            <b>Master</b> — centroid {m['centroid']} · bbox {m['bbox']} · ≥3:1 area on #f3f3f3 {m['pass_light']:.0%} · on #202020 {m['pass_dark']:.0%}<br>
            <b>Light-taskbar version</b> — ≥3:1 area on #f3f3f3 {ml['pass_light']:.0%}<br>
            Body 52 × 44 u, tail 8 u at 45°（≤ 宽的 1/4）· lime 32 u | tiles 20 u · big tile 28 u, bridge 2 u, small tile 14 u<br>
            Radii: R0 8 u (corner smoothing 60%) on the three outer corners · R1 4 u tail knee · R2 2 u tile corners against the lime · R3 1 u tail tip<br>
            One gradient: lime only, 120°, #e0f766 → #c8e93a (≥ 32 px); 16 / 24 flat #d4f24a
          </div>
        </div>
        </div></body></html>"""
        (HERE / "sheet.html").write_text(html, encoding="utf-8")
        pg2 = b.new_page(viewport={"width": 1600, "height": 900}, device_scale_factor=1)
        pg2.set_content(html)
        pg2.wait_for_timeout(500)
        pg2.screenshot(path=str(HERE / "sheet.png"), full_page=True)
        b.close()
    print("sheet ok")


if __name__ == "__main__":
    main()
