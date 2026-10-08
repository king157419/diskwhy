r"""Rasterise variant-1 with Playwright + installed Edge, measure, and compose sheet.png.

Run:  D:\conda\miniconda3\python.exe docs/logo/round2/variant-1/render.py
"""
import base64
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

from measure import measure
from rast import rasterise

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
FONTS = ROOT / "src" / "assets" / "fonts"
OUT = HERE / "png"
OUT.mkdir(exist_ok=True)


def b64(p: Path, mime="image/png") -> str:
    return f"data:{mime};base64," + base64.b64encode(Path(p).read_bytes()).decode()


def svg(name):
    return b64(HERE / name, "image/svg+xml")


def x4(src: Path, k=4):
    im = Image.open(src)
    out = src.with_name(src.stem + f"-x{k}.png")
    im.resize((im.width * k, im.height * k), Image.NEAREST).save(out)
    return out


def main():
    R = {}
    with sync_playwright() as p:
        b = p.chromium.launch(channel="msedge")
        pg = b.new_page(device_scale_factor=1, viewport={"width": 1100, "height": 1100})
        for px in (1024, 256, 64):
            R[f"d{px}"] = rasterise(pg, HERE / "icon.svg", px, OUT / f"icon@{px}.png")
            R[f"l{px}"] = rasterise(pg, HERE / "icon-light.svg", px, OUT / f"icon-light@{px}.png")
        for px in (48, 32, 24, 16):
            R[f"d{px}"] = rasterise(pg, HERE / f"icon-{px}.svg", px, OUT / f"icon-{px}@{px}.png")
            R[f"l{px}"] = rasterise(pg, HERE / f"icon-{px}-light.svg", px, OUT / f"icon-{px}-light@{px}.png")
            R[f"n{px}"] = rasterise(pg, HERE / "icon.svg", px, OUT / f"naive@{px}.png")
            R[f"nl{px}"] = rasterise(pg, HERE / "icon-light.svg", px, OUT / f"naive-light@{px}.png")
        for t in "ABC":
            R[f"t{t}"] = rasterise(pg, HERE / f"mark-16-{t}.svg", 16, OUT / f"mark-16-{t}@16.png")
        for k in ("d16", "l16", "n16", "d24", "l24", "n24", "tC"):
            x4(R[k])
        m = measure(OUT / "icon@1024.png", HERE / "png" / "centroid-overlay.png", quiet=True)
        ml = measure(OUT / "icon-light@1024.png", quiet=True)
        print("master", m)
        print("light ", ml)

        I = {k: b64(v) for k, v in R.items()}
        OV = b64(OUT / "centroid-overlay.png")

        def taskbar(bg, fg, pre):
            cells = "".join(
                f"<div class=cell><img class=px src='{I[pre + str(s)]}' width={s} height={s}><span>{s}</span></div>"
                for s in (64, 48, 32, 24, 16))
            return f"<div class=bar style='background:{bg};color:{fg}'>{cells}</div>"

        def zoom(key, s, bg, label, k=4):
            return (f"<div class=z><div class=zbox style='background:{bg};width:{s*k+24}px;height:{s*k+24}px'>"
                    f"<img class=px src='{I[key]}' width={s*k} height={s*k}></div><span>{label}</span></div>")

        def titlebar(t, bg, fg, border):
            return (f"<div class=tb style='background:{bg};color:{fg};border-bottom:{border}'>"
                    f"<img src='{svg(f'lockup-{t}.svg')}' height=16><div class=tbsp></div>"
                    f"<i>—</i><i>▢</i><i>✕</i><b>{t}</b><span class=tbz><img class=px src='{I['t' + t]}' width=64 height=64></span></div>")

        share = " · ".join(f"{k} {v:.0%}" for k, v in m["share"].items())
        html = f"""<html><head><style>
        @font-face {{ font-family: Geist; src: url({b64(FONTS / 'Geist-latin.woff2', 'font/woff2')}) format('woff2'); font-weight: 100 900; }}
        @font-face {{ font-family: NotoSC; src: url({b64(FONTS / 'NotoSansSC-DiskWhy.woff2', 'font/woff2')}) format('woff2'); }}
        * {{ box-sizing:border-box }}
        body {{ margin:0; width:1600px; background:#ebe7dc; font-family:Geist, NotoSC, 'Segoe UI', sans-serif; color:#141412; }}
        .wrap {{ padding:44px 56px 52px }}
        h1 {{ font-size:30px; font-weight:650; letter-spacing:-.02em; margin:0 0 4px }}
        .sub {{ color:#6b665c; font-size:15px; margin-bottom:24px }}
        h2 {{ font-size:13px; text-transform:uppercase; letter-spacing:.12em; color:#6b665c; margin:30px 0 12px; font-weight:600 }}
        .row {{ display:flex; gap:20px; align-items:stretch }}
        .hero {{ flex:1; height:340px; border-radius:20px; display:flex; align-items:center; justify-content:center; gap:40px }}
        .bar {{ display:flex; gap:34px; align-items:flex-end; padding:20px 28px; border-radius:14px; flex:1 }}
        .cell {{ display:flex; flex-direction:column; align-items:center; gap:8px; font-size:12px }}
        .px {{ image-rendering: pixelated; }}
        .z {{ display:flex; flex-direction:column; align-items:center; gap:8px; font-size:12px; color:#6b665c }}
        .zbox {{ display:flex; align-items:center; justify-content:center; border-radius:10px }}
        .tb {{ height:44px; display:flex; align-items:center; padding:0 18px; border-radius:10px; font-size:14px; gap:22px; position:relative }}
        .tb i {{ font-style:normal; opacity:.55; font-size:13px }}
        .tb b {{ font-size:12px; opacity:.5; font-weight:600; width:20px }}
        .tbsp {{ flex:1 }}
        .tbz {{ display:none }}
        .lock {{ flex:1; height:200px; border-radius:20px; display:flex; align-items:center; justify-content:center }}
        .meta {{ font-size:13px; color:#6b665c; line-height:1.7 }}
        </style></head><body><div class=wrap>
        <h1>DiskWhy — round 2 · variant 1 · 留白气泡 Quiet Grid</h1>
        <div class=sub>三块大小不等的砖拼成一个对话气泡，砖缝是透明的；尾巴从苔绿砖的左下角弯出来。 lime · moss · clay · 无描边 · Geist 600</div>
        <div class=row>
          <div class=hero style='background:#f3f3f3'><img src='{I['d1024']}' width=256 height=256><img src='{I['d64']}' width=64><img class=px src='{I['d32']}' width=32></div>
          <div class=hero style='background:#202020'><img src='{I['d1024']}' width=256 height=256><img src='{I['d64']}' width=64><img class=px src='{I['d32']}' width=32></div>
          <div class=hero style='background:#f3f3f3;flex:.62;flex-direction:column;gap:10px'><img src='{I['l1024']}' width=200 height=200><span class=meta>light-taskbar set (lightunplated)</span></div>
        </div>

        <h2>Taskbar · 1× pixels · dark set on #202020, light set on #f3f3f3 (64 48 32 24 16)</h2>
        <div class=row>{taskbar('#f3f3f3', '#555', 'l')}{taskbar('#202020', '#aaa', 'd')}</div>
        <div class=row style='margin-top:12px'>{taskbar('#f3f3f3', '#555', 'd')}<div class=meta style='flex:1;padding:14px 8px'>← for comparison: the master colours on a light taskbar. The lime row has only 1.14 : 1 luminance contrast there (it reads by hue alone), so the light set above is what should ship for light taskbars.</div></div>

        <h2>Small sizes ×4 (nearest neighbour) · hand-tuned vs. naive downscale of the 1024 master</h2>
        <div class=row style='gap:16px;flex-wrap:wrap;align-items:flex-start'>
          {zoom('d16', 16, '#202020', '16 hinted · dark')}
          {zoom('n16', 16, '#202020', '16 naive · dark')}
          {zoom('l16', 16, '#f3f3f3', '16 hinted · light')}
          {zoom('nl16', 16, '#f3f3f3', '16 naive · light')}
          {zoom('d24', 24, '#202020', '24 hinted · dark')}
          {zoom('n24', 24, '#202020', '24 naive · dark')}
          {zoom('l24', 24, '#f3f3f3', '24 hinted · light')}
          {zoom('nl24', 24, '#f3f3f3', '24 naive · light')}
          {zoom('d32', 32, '#202020', '32 hinted · dark', 2)}
          {zoom('l32', 32, '#f3f3f3', '32 hinted · light', 2)}
          {zoom('d48', 48, '#202020', '48 hinted · dark', 2)}
          {zoom('n48', 48, '#202020', '48 naive · dark', 2)}
        </div>

        <h2>Title bar · 16 px mark · themes A 浅色 / B 深色 / C 卡通</h2>
        <div class=row style='gap:28px'>
          <div style='display:grid;gap:12px;flex:1'>
            {titlebar('A', '#f3f1ec', '#1c1b18', '1px solid #e1ddd4')}
            {titlebar('B', '#1b1714', '#f3ece2', '1px solid #2b2520')}
            {titlebar('C', '#f3efe3', '#141412', '2px solid #141412')}
          </div>
          {zoom('tA', 16, '#f3f1ec', 'A ×4')}{zoom('tB', 16, '#1b1714', 'B ×4')}{zoom('tC', 16, '#f3efe3', 'C ×4')}
        </div>

        <h2>Horizontal lockup</h2>
        <div class=row>
          <div class=lock style='background:#f4f0e6'><img src='{svg("lockup.svg")}' height=96></div>
          <div class=lock style='background:#14110e'><img src='{svg("lockup-dark.svg")}' height=96></div>
        </div>

        <h2>Construction · 64-unit grid, safe area, optical centre</h2>
        <div class=row style='align-items:flex-start;gap:28px'>
          <img src='{OV}' width=360 height=360 style='border-radius:12px'>
          <div class=meta>
            red dot = alpha centroid {m['centroid']} · target (512 ± 8, 496 ± 8) · blue cross = (512, 496) · blue box = 4 u safe area<br>
            alpha bbox {m['bbox']} · silhouette covers {m['fill']:.0%} of the canvas · body 53 u = 83 % of the width<br>
            area share (master): {share}<br>
            light set: {" · ".join(f"{k} {v:.0%}" for k, v in ml['share'].items())} — all of it ≥ 3 : 1 on #f3f3f3<br>
            grid: lime row 53 × 20 u · 2 u gap · bottom row 16 u = moss 33 u | 2 u | clay 18 u · areas lime : moss : clay ≈ 3.7 : 1.8 : 1 (the radius chain's ×2) · tail depth 8 u, 45°<br>
            radii: R0 8 u (smoothing 60 %) · R1 4 u (tail bend) · R2 2 u (every gap corner + tail knee) · R3 1 u (tail tip)<br>
            colour: lime #d4f24a (one 120° ramp #e0f766→#c8e93a, ≥ 32 px) · moss #5c7a1a · clay #8a857a · light set swaps lime→moss, moss→ink #141412
          </div>
        </div>
        </div></body></html>"""
        (HERE / "wip" / "sheet.html").write_text(html, encoding="utf-8")
        pg2 = b.new_page(viewport={"width": 1600, "height": 900}, device_scale_factor=1)
        pg2.set_content(html)
        pg2.evaluate("document.fonts.ready")
        pg2.wait_for_timeout(500)
        pg2.screenshot(path=str(HERE / "sheet.png"), full_page=True)
        b.close()
    print("sheet ok")


if __name__ == "__main__":
    main()
