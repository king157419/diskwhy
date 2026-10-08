"""Rasterise variant 2 with Playwright + installed Edge; centroid-centre the master; build sheet.

Run:  D:\\conda\\miniconda3\\python.exe docs/logo/round3/variant-2/render.py [compare TAG] [sheet]
  compare TAG  -> wip/TAG.png  side-by-side with the round-1 original (concept-2)
  sheet        -> png/*, sheet.png
"""
import base64
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import build  # noqa: E402

ROOT = HERE.parents[3]
C2 = HERE.parents[1] / "concept-2"
OUT = HERE / "png"
WIP = HERE / "wip"
OUT.mkdir(exist_ok=True)
WIP.mkdir(exist_ok=True)


def b64(p: Path, mime: str) -> str:
    return f"data:{mime};base64," + base64.b64encode(p.read_bytes()).decode()


def svg_uri(p: Path):
    return b64(p, "image/svg+xml")


GEIST = b64(ROOT / "src" / "assets" / "fonts" / "Geist-latin.woff2", "font/woff2")
FONT_CSS = f"@font-face {{ font-family: Geist; src: url({GEIST}) format('woff2'); font-weight: 100 900; }}"


def raster(page, svg: Path, px: int, out: Path):
    page.set_content(
        "<html><body style='margin:0;background:transparent'>"
        f"<img id=i src='{svg_uri(svg)}' width={px} height={px} style='display:block'></body></html>"
    )
    page.wait_for_function("document.getElementById('i').complete")
    page.locator("#i").screenshot(path=str(out), omit_background=True)
    return out


def centroid(png: Path):
    im = Image.open(png).convert("RGBA")
    w, h = im.size
    a = im.getchannel("A").load()
    sx = sy = sm = 0
    for y in range(h):
        for x in range(w):
            v = a[x, y]
            if v:
                sx += x * v
                sy += y * v
                sm += v
    return sx / sm + 0.5, sy / sm + 0.5


def bbox(png: Path):
    im = Image.open(png).convert("RGBA")
    return im.getchannel("A").getbbox()


def centre_master(page):
    """Place the master so its alpha centroid sits at (512, 512 - 1.5%)."""
    target = (512.0, 512.0 - 0.015 * 1024)
    for _ in range(3):
        m = build.write_all()
        png = raster(page, HERE / "icon.svg", 1024, WIP / "_centre.png")
        cx, cy = centroid(png)
        ox, oy = build.load_offset(m)
        dx, dy = target[0] - cx, target[1] - cy
        if abs(dx) < 0.5 and abs(dy) < 0.5:
            break
        build.OFFSET_FILE.write_text(json.dumps({"x": round(ox + dx, 2), "y": round(oy + dy, 2)}))
    m = build.write_all()
    png = raster(page, HERE / "icon.svg", 1024, WIP / "_centre.png")
    print("centroid", centroid(png), "bbox", bbox(png))


SIZES = {1024: "icon.svg", 256: "icon.svg", 64: "icon.svg", 48: "icon.svg",
         32: "icon-32.svg", 24: "icon-24.svg", 16: "icon-16.svg"}


def all_pngs(page):
    R = {}
    for px, src in SIZES.items():
        R[px] = raster(page, HERE / src, px, OUT / f"icon-{px}.png")
    for px in (16, 24):
        im = Image.open(R[px])
        im.resize((px * 4, px * 4), Image.NEAREST).save(OUT / f"icon-{px}@x4.png")
    return R


def c2_pngs(page):
    """Round-1 original at the same sizes (its own hand-drawn smalls)."""
    src = {256: "icon.svg", 64: "icon.svg", 48: "icon.svg", 32: "icon-32.svg", 24: "icon-24.svg", 16: "icon-16.svg"}
    return {px: raster(page, C2 / s, px, WIP / f"_c2-{px}.png") for px, s in src.items()}


CSS = f"""{FONT_CSS}
* {{ box-sizing:border-box }}
body {{ margin:0; background:#ebe7dc; font-family:Geist, 'Microsoft YaHei UI', sans-serif; color:#141412 }}
.px {{ image-rendering:pixelated }}
.lab {{ font-size:12px; color:#6b665c; letter-spacing:.04em }}
"""


def compare(page, browser, tag, R, C):
    def u(p):
        return b64(p, "image/png")

    def col(title, imgs):
        cells = ""
        for bg, fg in (("#f3f3f3", "#555"), ("#202020", "#aaa")):
            small = "".join(
                f"<div style='display:flex;flex-direction:column;align-items:center;gap:6px'>"
                f"<img class=px src='{u(imgs[s])}' width={s} height={s}><span style='font-size:11px;color:{fg}'>{s}</span></div>"
                for s in (48, 32, 24, 16)
            )
            zoom = "".join(
                f"<img class=px src='{u(imgs[s])}' width={s*5} height={s*5}>" for s in (32, 24, 16)
            )
            cells += (
                f"<div style='background:{bg};border-radius:16px;padding:22px;display:flex;flex-direction:column;gap:18px;align-items:center'>"
                f"<img src='{u(imgs[256])}' width=256 height=256>"
                f"<div style='display:flex;gap:26px;align-items:flex-end'>{small}</div>"
                f"<div style='display:flex;gap:18px;align-items:center'>{zoom}</div></div>"
            )
        return f"<div style='display:flex;flex-direction:column;gap:12px'><div style='font-weight:700;font-size:20px'>{title}</div><div style='display:flex;gap:14px'>{cells}</div></div>"

    html = (
        f"<html><head><style>{CSS}</style></head><body><div style='padding:28px;display:flex;gap:40px'>"
        f"{col('第一版 · concept-2', C)}{col('变体 2 · 贴纸版 · ' + tag, R)}</div></body></html>"
    )
    pg = browser.new_page(viewport={"width": 1900, "height": 800}, device_scale_factor=1)
    pg.set_content(html)
    pg.wait_for_timeout(300)
    pg.screenshot(path=str(WIP / f"{tag}.png"), full_page=True)
    pg.close()


def sheet(browser, R, C):
    def u(p):
        return b64(p, "image/png")

    def px(imgs, s, k=1):
        return f"<img class=px src='{u(imgs[s])}' width={s*k} height={s*k}>"

    def tbar(bg, fg):
        def grp(imgs, name):
            cells = "".join(
                f"<div class=cell><div style='height:34px;display:flex;align-items:flex-end'>{px(imgs, s)}</div>"
                f"<span style='color:{fg}'>{s}</span></div>" for s in (32, 24, 16))
            return f"<div class=grp><div class=gname style='color:{fg}'>{name}</div><div style='display:flex;gap:22px;align-items:flex-end'>{cells}</div></div>"
        return (f"<div class=bar style='background:{bg}'>{grp(C, '第一版')}"
                f"<div class=sep style='background:{fg}'></div>{grp(R, '变体 2')}</div>")

    def zrow(bg, fg):
        old = "".join(f"<div class=z>{px(C, s, 4)}</div>" for s in (32, 24, 16))
        new = "".join(f"<div class=z>{px(R, s, 4)}</div>" for s in (32, 24, 16))
        return (f"<div class=bar style='background:{bg};align-items:center'><div class=gname style='color:{fg}'>第一版</div>{old}"
                f"<div class=sep style='background:{fg}'></div><div class=gname style='color:{fg}'>变体 2</div>{new}</div>")

    def tb(name, k):
        return f"<img src='{svg_uri(name)}' height={32*k} style='image-rendering:pixelated;display:block'>"

    C2 = HERE.parents[1] / "concept-2"
    html = f"""<html><head><style>{CSS}
      .sec {{ font-size:12px; letter-spacing:.12em; color:#6b665c; font-weight:600; margin:6px 0 -6px }}
      .hero {{ border-radius:20px; height:320px; display:flex; align-items:center; justify-content:center; position:relative }}
      .hero .t {{ position:absolute; left:18px; top:14px; font-size:13px; font-weight:600 }}
      .bar {{ display:flex; gap:26px; align-items:flex-end; padding:18px 24px; border-radius:14px }}
      .cell {{ display:flex; flex-direction:column; align-items:center; gap:6px; font-size:11px }}
      .grp {{ display:flex; flex-direction:column; gap:10px }}
      .gname {{ font-size:12px; font-weight:600; opacity:.8 }}
      .sep {{ width:1px; align-self:stretch; opacity:.25 }}
    </style></head><body><div style='padding:40px 48px;width:1600px;display:flex;flex-direction:column;gap:22px'>
      <div><div style='font-size:28px;font-weight:700;letter-spacing:-.02em'>DiskWhy · 第三轮 · 变体 2「贴纸版」</div>
      <div class=lab style='margin-top:6px'>同一个图标：气泡 + 三块方块 + 墨黑粗框，外面多一圈白色模切边和右下方的硬投影。每一行最左边都是第一版（concept-2）原图。</div></div>
      <div style='display:grid;grid-template-columns:1fr 1fr 1fr;gap:16px'>
        <div class=hero style='background:#f3f3f3'><span class=t style='color:#6b665c'>第一版 · 原图</span><img src='{u(C[256])}' width=256 height=256></div>
        <div class=hero style='background:#f3f3f3'><span class=t style='color:#6b665c'>变体 2 · 浅色</span><img src='{u(R[1024])}' width=256 height=256></div>
        <div class=hero style='background:#202020'><span class=t style='color:#aaa'>变体 2 · 深色</span><img src='{u(R[1024])}' width=256 height=256></div>
      </div>
      <div class=sec>任务栏 · 1× 像素 · 浅色 #f3f3f3 / 深色 #202020</div>
      <div style='display:flex;gap:16px'>{tbar('#f3f3f3', '#555')}{tbar('#202020', '#bbb')}</div>
      <div class=sec>×4 最近邻放大 · 32 / 24 / 16</div>
      <div style='display:flex;flex-direction:column;gap:12px'>{zrow('#f3f3f3', '#555')}{zrow('#202020', '#bbb')}</div>
      <div class=sec>标题栏 · 16 px 小标 · 1× 与 ×2</div>
      <div style='display:grid;grid-template-columns:auto auto 1fr;gap:14px 28px;align-items:center'>
        <span class=lab>第一版 · C</span>{tb(C2 / 'lockup-C.svg', 1).replace("height=32", "height=16")}<span></span>
        <span class=lab>变体 2 · C 主题</span>{tb(HERE / 'titlebar-C.svg', 1)}{tb(HERE / 'titlebar-C.svg', 2)}
        <span class=lab>变体 2 · A 主题</span>{tb(HERE / 'titlebar-A.svg', 1)}{tb(HERE / 'titlebar-A.svg', 2)}
      </div>
      <div class=sec>横排组合 · Geist Bold 转路径</div>
      <div style='display:grid;grid-template-columns:1fr 1fr;gap:16px'>
        <div style='background:#f4f0e6;border-radius:18px;height:180px;display:flex;align-items:center;justify-content:center'><img src='{svg_uri(HERE / "lockup-light.svg")}' height=96></div>
        <div style='background:#14110e;border-radius:18px;height:180px;display:flex;align-items:center;justify-content:center'><img src='{svg_uri(HERE / "lockup-dark.svg")}' height=96></div>
      </div>
    </div></body></html>"""
    (HERE / "_sheet.html").write_text(html, encoding="utf-8")
    pg = browser.new_page(viewport={"width": 1696, "height": 900}, device_scale_factor=1)
    pg.set_content(html)
    pg.wait_for_timeout(400)
    pg.screenshot(path=str(HERE / "sheet.png"), full_page=True)
    pg.close()


def main():
    args = sys.argv[1:]
    with sync_playwright() as p:
        b = p.chromium.launch(channel="msedge")
        pg = b.new_page(device_scale_factor=1)
        centre_master(pg)
        R = all_pngs(pg)
        C = c2_pngs(pg)
        if "compare" in args:
            compare(pg, b, args[args.index("compare") + 1], R, C)
        if "sheet" in args:
            sheet(b, R, C)
        b.close()
    print("ok")


if __name__ == "__main__":
    main()
