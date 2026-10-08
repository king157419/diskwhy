"""Rasterise variant 3 with Playwright + installed Edge; write png/ and sheet.png.

Run:  D:\\conda\\miniconda3\\python.exe docs/logo/round3/variant-3/render.py
"""
from playwright.sync_api import sync_playwright

from rast import HERE, ORIG, ROOT, b64, png_uri, raster, shot, svg_uri

OUT = HERE / "png"
OUT.mkdir(exist_ok=True)
SRC = {1024: "icon.svg", 256: "icon.svg", 64: "icon.svg", 48: "icon.svg",
       32: "icon-32.svg", 24: "icon-24.svg", 16: "icon-16.svg"}


def nearest(page, png, s, k, out):
    """Integer nearest-neighbour enlargement of a 1x raster (transparent kept)."""
    page.set_content(f"<html><body style='margin:0;background:transparent'><img id=i src='{png_uri(png)}' "
                     f"width={s * k} height={s * k} style='display:block;image-rendering:pixelated'></body></html>")
    page.wait_for_function("document.getElementById('i').complete")
    page.locator("#i").screenshot(path=str(out), omit_background=True)
    return out


def strip(page, svg, h, scale, out):
    """Render a title-bar strip SVG at its natural height x scale (transparent)."""
    page.set_content(f"<html><body style='margin:0;background:transparent'><img id=i src='{svg_uri(svg)}' "
                     f"height={h * scale} style='display:block;image-rendering:{'pixelated' if scale > 1 else 'auto'}'></body></html>")
    page.wait_for_function("document.getElementById('i').complete")
    page.locator("#i").screenshot(path=str(out), omit_background=True)
    return out


def main():
    with sync_playwright() as p:
        b = p.chromium.launch(channel="msedge")
        pg = b.new_page(device_scale_factor=1)
        R = {}
        for px, src in SRC.items():
            R[px] = raster(pg, HERE / src, px, OUT / f"icon@{px}.png")
        for s in (16, 24):
            nearest(pg, R[s], s, 4, OUT / f"icon@{s}-x4.png")
        tb = {}
        for th in ("A", "C"):
            tb[th] = strip(pg, HERE / f"titlebar-{th}.svg", 16, 1, OUT / f"titlebar-{th}@1x.png")
            # 2x of the 1x pixels (what a 200 % screen shows if it reuses the 16 px art)
            strip(pg, HERE / f"titlebar-{th}.svg", 16, 2, OUT / f"titlebar-{th}@2x-preview.png")

        U = {k: png_uri(v) for k, v in R.items()}
        O = {s: png_uri(ORIG / "png" / f"icon-{s}@{s}.png") for s in (32, 24, 16)}
        O_BIG = svg_uri(ORIG / "icon.svg")
        TB = {k: png_uri(v) for k, v in tb.items()}

        def taskbar(bg, fg, src, label):
            cells = "".join(f"<div class=cell><img class=px src='{src[s]}' width={s} height={s}><span>{s}</span></div>"
                            for s in (32, 24, 16))
            zoom = "".join(f"<div class=cell><img class=px src='{src[s]}' width={s*4} height={s*4}><span>{s} ×4</span></div>"
                           for s in (32, 24, 16))
            return (f"<div class=bar style='background:{bg};color:{fg}'><b>{label}</b>{cells}"
                    f"<i class=sep></i>{zoom}</div>")

        def titlebar(th, bg, fg, label):
            return (f"<div class=tbwrap><span class=lab>{label}</span>"
                    f"<div class=tb style='background:{bg};color:{fg}'><img class=px src='{TB[th]}' height=16>"
                    f"<div class=sp></div><i>—</i><i>▢</i><i>✕</i></div>"
                    f"<div class=tb2 style='background:{bg};color:{fg}'><img class=px src='{TB[th]}' height=32>"
                    f"<div class=sp></div><i>—</i><i>▢</i><i>✕</i></div></div>")

        GEIST = b64(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2", "font/woff2")
        html = f"""<html><head><style>
        @font-face {{ font-family: Geist; src: url({GEIST}) format('woff2'); font-weight: 100 900; }}
        * {{ box-sizing:border-box }}
        body {{ margin:0; width:1600px; background:#ebe7dc; font-family:Geist,'Microsoft YaHei UI','Segoe UI',sans-serif; color:#141412 }}
        .wrap {{ padding:40px 48px 48px }}
        h1 {{ font-size:28px; font-weight:700; letter-spacing:-.02em; margin:0 0 4px }}
        .sub {{ color:#6b665c; font-size:14px; margin-bottom:24px }}
        h2 {{ font-size:12px; letter-spacing:.12em; color:#6b665c; margin:28px 0 10px; font-weight:600 }}
        .row {{ display:flex; gap:16px; align-items:stretch }}
        .card {{ flex:1; height:380px; border-radius:18px; display:flex; flex-direction:column; align-items:center; justify-content:center; gap:10px; position:relative }}
        .card .t {{ position:absolute; top:14px; left:18px; font-size:13px; font-weight:600; opacity:.7 }}
        .orig {{ outline:2px dashed #b9b2a3; outline-offset:-8px }}
        .bar {{ display:flex; gap:26px; align-items:flex-end; padding:18px 24px; border-radius:14px; margin-bottom:12px }}
        .bar b {{ font-size:12px; font-weight:600; width:92px; align-self:center; opacity:.8 }}
        .cell {{ display:flex; flex-direction:column; align-items:center; gap:6px; font-size:11px; opacity:.95 }}
        .sep {{ width:1px; align-self:stretch; background:currentColor; opacity:.2 }}
        .px {{ image-rendering:pixelated }}
        .tbwrap {{ display:grid; grid-template-columns:120px 1fr 1fr; gap:12px; align-items:center; margin-bottom:10px }}
        .lab {{ font-size:12px; color:#6b665c }}
        .tb {{ height:44px; display:flex; align-items:center; padding:0 16px; border-radius:10px; gap:20px }}
        .tb2 {{ height:72px; display:flex; align-items:center; padding:0 20px; border-radius:12px; gap:30px }}
        .tb i, .tb2 i {{ font-style:normal; opacity:.5; font-size:13px }}
        .sp {{ flex:1 }}
        .lock {{ flex:1; height:190px; border-radius:18px; display:flex; align-items:center; justify-content:center }}
        </style></head><body><div class=wrap>
        <h1>DiskWhy 图标 · 第三轮 · 变体 3「糖果光泽」</h1>
        <div class=sub>第一版的气泡 + 三块方块，原配色、原黑边；每块加同一方向（左上）的柔和高光和缝边轻内阴影，像三颗软糖。≤ 32 px 只留 1 px 顶部亮边。</div>
        <div class=row>
          <div class='card orig' style='background:#fff;flex:.9'><span class=t>第一版 · 对照</span><img src='{O_BIG}' width=280 height=280></div>
          <div class=card style='background:#fff;flex:.9'><span class=t>变体 3</span><img src='{U[1024]}' width=280 height=280></div>
          <div class=card style='background:#202020;color:#ddd;flex:.9'><span class=t>深色 #202020</span><img src='{U[1024]}' width=280 height=280></div>
          <div class=card style='background:#f6f1e4;flex:.6'><span class=t>256 / 64 / 48</span><img src='{U[256]}' width=128><div style='display:flex;gap:16px;align-items:flex-end'><img src='{U[64]}' width=64><img src='{U[48]}' width=48></div></div>
        </div>

        <h2>任务栏 · 1× 像素 · 浅色 #f3f3f3 / 深色 #202020（上：第一版，下：变体 3）</h2>
        {taskbar('#f3f3f3', '#555', O, '第一版 浅色')}
        {taskbar('#f3f3f3', '#555', U, '变体 3 浅色')}
        {taskbar('#202020', '#bbb', O, '第一版 深色')}
        {taskbar('#202020', '#bbb', U, '变体 3 深色')}

        <h2>标题栏 · 16 px 小标（左 1×，右 2× 像素放大）</h2>
        {titlebar('C', '#f4f0e6', '#141412', 'C 主题 #f4f0e6')}
        {titlebar('A', '#f3f1ec', '#1c1b18', 'A 主题 #f3f1ec')}

        <h2>横排组合</h2>
        <div class=row>
          <div class=lock style='background:#f4f0e6'><img src='{svg_uri(HERE / "lockup-light.svg")}' height=104></div>
          <div class=lock style='background:#14110e'><img src='{svg_uri(HERE / "lockup-dark.svg")}' height=104></div>
        </div>
        </div></body></html>"""
        (HERE / "wip" / "sheet.html").write_text(html, encoding="utf-8")
        shot(b, html, HERE / "sheet.png")
        b.close()
    print("sheet ok")


if __name__ == "__main__":
    main()
