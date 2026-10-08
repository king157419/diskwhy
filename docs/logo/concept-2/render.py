"""Rasterise concept-2 with Playwright + installed Edge and compose sheet.png.

Run:  D:\\conda\\miniconda3\\python.exe docs/logo/concept-2/render.py
"""
import base64
from pathlib import Path

from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
FONTS = ROOT / "src" / "assets" / "fonts"
OUT = HERE / "png"
OUT.mkdir(exist_ok=True)


def b64(p: Path, mime: str) -> str:
    return f"data:{mime};base64," + base64.b64encode(p.read_bytes()).decode()


def svg_uri(name):
    return b64(HERE / name, "image/svg+xml")


GEIST = b64(FONTS / "Geist-latin.woff2", "font/woff2")
FONT_CSS = f"""
@font-face {{ font-family: Geist; src: url({GEIST}) format('woff2'); font-weight: 100 900; }}
"""

# which drawing is used at which pixel size
SIZE_SRC = {256: "icon.svg", 64: "icon.svg", 48: "icon.svg", 32: "icon-32.svg", 24: "icon-24.svg", 16: "icon-16.svg"}


def raster(page, name, px, bg=None):
    """Render svg `name` at exactly px x px (1x, DPR 1) -> PNG path."""
    bgcss = bg or "transparent"
    page.set_content(
        f"<html><body style='margin:0;background:{bgcss}'>"
        f"<img id=i src='{svg_uri(name)}' width={px} height={px} style='display:block'></body></html>"
    )
    page.wait_for_function("document.getElementById('i').complete")
    out = OUT / f"{Path(name).stem}@{px}{'' if not bg else '-' + bg.strip('#')}.png"
    page.locator("#i").screenshot(path=str(out), omit_background=bg is None)
    return out


def main():
    with sync_playwright() as p:
        b = p.chromium.launch(channel="msedge")
        pg = b.new_page(device_scale_factor=1)
        # 1x rasters of every size, transparent
        rasters = {}
        for px, src in SIZE_SRC.items():
            rasters[px] = raster(pg, src, px)
        # for comparison: the full 1024 drawing squashed to 16 / 24
        naive16 = raster(pg, "icon.svg", 16)
        naive24 = raster(pg, "icon.svg", 24)
        rasters[1024] = raster(pg, "icon.svg", 1024)
        old16 = None
        oldsrc = ROOT / "src-tauri" / "icons" / "icon-source.svg"

        R = {k: b64(v, "image/png") for k, v in rasters.items()}
        N16, N24 = b64(naive16, "image/png"), b64(naive24, "image/png")
        OLD = b64(oldsrc, "image/svg+xml")
        LOCK = {t: svg_uri(f"lockup-{t}.svg") for t in ("A", "B", "C")}
        LOCKBIG = svg_uri("lockup.svg")

        def taskbar(bg, fg_label):
            cells = "".join(
                f"<div class=cell><img class=px src='{R[s]}' width={s} height={s}><span>{s}</span></div>"
                for s in (64, 48, 32, 24, 16)
            )
            return f"<div class=bar style='background:{bg};color:{fg_label}'>{cells}</div>"

        def zoom(src, s, bg, label):
            return (
                f"<div class=z><div class=zbox style='background:{bg};width:{s*5}px;height:{s*5}px'>"
                f"<img class=pix src='{src}' width={s*4} height={s*4}></div><span>{label}</span></div>"
            )

        def titlebar(theme, bg, fg):
            return (
                f"<div class=tb style='background:{bg};color:{fg}'>"
                f"<img src='{LOCK[theme]}' height=16><div class=tbsp></div>"
                f"<i>—</i><i>▢</i><i>✕</i></div>"
            )

        html = f"""<html><head><style>{FONT_CSS}
        * {{ box-sizing:border-box }}
        body {{ margin:0; width:1600px; background:#ebe7dc; font-family:Geist, 'Segoe UI', sans-serif; color:#141412; }}
        .wrap {{ padding:48px 56px 56px }}
        h1 {{ font-size:30px; font-weight:650; letter-spacing:-.02em; margin:0 0 4px }}
        .sub {{ color:#6b665c; font-size:15px; margin-bottom:28px }}
        h2 {{ font-size:13px; text-transform:uppercase; letter-spacing:.12em; color:#6b665c; margin:34px 0 12px; font-weight:600 }}
        .row {{ display:flex; gap:20px; align-items:stretch }}
        .hero {{ flex:1; height:360px; border-radius:20px; display:flex; align-items:center; justify-content:center; gap:44px }}
        .bar {{ display:flex; gap:36px; align-items:flex-end; padding:22px 30px; border-radius:14px; flex:1 }}
        .cell {{ display:flex; flex-direction:column; align-items:center; gap:8px; font-size:12px; opacity:.95 }}
        .px, .pix {{ image-rendering: pixelated; }}
        .z {{ display:flex; flex-direction:column; align-items:center; gap:8px; font-size:12px; color:#6b665c }}
        .zbox {{ display:flex; align-items:center; justify-content:center; border-radius:10px }}
        .tb {{ height:44px; display:flex; align-items:center; padding:0 18px; border-radius:10px; font-size:14px; gap:22px }}
        .tb i {{ font-style:normal; opacity:.55; font-size:13px }}
        .tbsp {{ flex:1 }}
        .lock {{ flex:1; height:200px; border-radius:20px; display:flex; align-items:center; justify-content:center }}
        .note {{ font-size:13px; color:#6b665c; margin-top:8px }}
        </style></head><body><div class=wrap>
        <h1>DiskWhy — concept 2 · 开口的磁盘</h1>
        <div class=sub>一个会说话的磁盘：对话气泡里装着一张“方块地图”，每一块都有人给你讲清楚。 Ink · lime · paper · coral · Geist 620</div>

        <div class=row>
          <div class=hero style='background:#f3f3f3'><img src='{R[1024]}' width=256 height=256><img src='{R[64]}' width=64><img class=px src='{R[32]}' width=32></div>
          <div class=hero style='background:#202020'><img src='{R[1024]}' width=256 height=256><img src='{R[64]}' width=64><img class=px src='{R[32]}' width=32></div>
          <div class=hero style='background:#d4f24a;flex:.55'><img src='{R[1024]}' width=200 height=200></div>
        </div>

        <h2>Taskbar · 1× pixels (64 48 32 24 16)</h2>
        <div class=row>{taskbar('#f3f3f3', '#555')}{taskbar('#202020', '#aaa')}</div>

        <h2>Small sizes enlarged ×4 (nearest neighbour)</h2>
        <div class=row style='gap:26px;flex-wrap:wrap'>
          {zoom(R[16], 16, '#f3f3f3', '16 hinted · light')}
          {zoom(R[16], 16, '#202020', '16 hinted · dark')}
          {zoom(N16, 16, '#f3f3f3', '16 naive downscale')}
          {zoom(R[24], 24, '#f3f3f3', '24 hinted · light')}
          {zoom(R[24], 24, '#202020', '24 hinted · dark')}
          {zoom(N24, 24, '#f3f3f3', '24 naive downscale')}
          {zoom(R[32], 32, '#202020', '32 hinted · dark')}
          <div class=z><div class=zbox style='background:#202020;width:80px;height:80px'><img src='{OLD}' width=64></div><span>current (for reference)</span></div>
        </div>

        <h2>Title bar · 16 px mark in each theme</h2>
        <div style='display:grid;gap:12px'>
          {titlebar('A', '#f3f1ec', '#1c1b18')}
          {titlebar('B', '#1b1714', '#f3ece2')}
          {titlebar('C', '#f3efe3', '#141412')}
        </div>

        <h2>Horizontal lockup</h2>
        <div class=row>
          <div class=lock style='background:#f4f0e6'><img src='{LOCKBIG}' height=96></div>
          <div class=lock style='background:#14110e'><img src='{svg_uri("lockup-dark.svg")}' height=96></div>
        </div>
        </div></body></html>"""
        (HERE / "sheet.html").write_text(html, encoding="utf-8")
        pg2 = b.new_page(viewport={"width": 1600, "height": 900}, device_scale_factor=1)
        pg2.set_content(html)
        pg2.wait_for_timeout(400)
        pg2.screenshot(path=str(HERE / "sheet.png"), full_page=True)
        b.close()
    print("sheet ok")


if __name__ == "__main__":
    main()
