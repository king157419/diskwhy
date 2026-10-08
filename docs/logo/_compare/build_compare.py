r"""Build docs/logo/compare.png (three-concept comparison). Run with D:\conda\miniconda3\python.exe."""
import pathlib
from playwright.sync_api import sync_playwright
HERE = pathlib.Path(__file__).parent
LOGO = HERE.parent
FONTS = (LOGO.parent.parent / "src" / "assets" / "fonts").resolve().as_uri()

C = [
  dict(n="方案 1", name="好奇盒子", big="concept-1/png/mark-256.png", i32="concept-1/png/mark-32.png",
       i16="concept-1/png/mark-16-16.png", tC=("concept-1/mark-16.svg", 16), tA=("concept-1/wip/mono16-green.svg", 16)),
  dict(n="方案 2", name="会说话的磁盘", big="concept-2/png/icon@256.png", i32="concept-2/png/icon-32@32.png",
       i16="concept-2/png/icon-16@16.png", tC=("concept-2/icon-16.svg", 16), tA=("concept-2/icon-16.svg", 16)),
  dict(n="方案 3", name="Di 字标", big="concept-3/png/icon-auto-256.png", i32="concept-3/png/icon-32.png",
       i16="concept-3/png/icon-16.png", tC=("concept-3/mark-16-C.svg", 13), tA=("concept-3/mark-16-A.svg", 13)),
]
FACE = f"""@font-face{{font-family:Geist;src:url({FONTS}/Geist-latin.woff2)}}
@font-face{{font-family:NotoSC;src:url({FONTS}/NotoSansSC-DiskWhy.woff2)}}"""
THEME = {"C": ("#f4f0e6", "#141412", "2px solid #141412"), "A": ("#f3f1ec", "#1c1b18", "1px solid #e1ddd4")}

def titlebar_html(svg, w, theme):
    bg, ink, border = THEME[theme]
    src = (LOGO / svg).resolve().as_uri()
    return f"""<!doctype html><meta charset=utf-8><style>{FACE}
body{{margin:0}}.tb{{width:264px;height:44px;box-sizing:border-box;background:{bg};border-bottom:{border};
display:flex;align-items:center;gap:8px;padding:0 14px;color:{ink};font:600 13px Geist,NotoSC,sans-serif}}
.tb img{{width:{w}px;height:16px;display:block}}.c{{margin-left:auto;display:flex;gap:18px;opacity:.55}}
.c i{{display:block;width:10px;height:10px}}</style>
<div class=tb><img src="{src}">DiskWhy<span class=c>
<i style="border-bottom:1px solid {ink};height:5px"></i><i style="border:1px solid {ink};box-sizing:border-box"></i>
<i style="background:linear-gradient(45deg,transparent 45%,{ink} 45%,{ink} 55%,transparent 55%),linear-gradient(-45deg,transparent 45%,{ink} 45%,{ink} 55%,transparent 55%)"></i></span></div>"""

with sync_playwright() as p:
    b = p.chromium.launch(channel="msedge")
    pg = b.new_page(device_scale_factor=1, viewport={"width": 400, "height": 100})
    for k, c in enumerate(C, 1):
        for t in ("C", "A"):
            svg, w = c["t" + t]
            f = HERE / f"_tb.html"; f.write_text(titlebar_html(svg, w, t), encoding="utf8")
            pg.goto(f.as_uri()); pg.evaluate("document.fonts.ready"); pg.wait_for_timeout(150)
            pg.screenshot(path=str(HERE / f"tb-{k}-{t}.png"), clip={"x": 0, "y": 0, "width": 264, "height": 44})
    (HERE / "_tb.html").unlink()

    def u(rel): return (LOGO / rel).resolve().as_uri()
    def strip(c, bg, fg):
        return f"""<div class=strip style="background:{bg};color:{fg}">
<figure><img src="{u(c['i32'])}" width=32 height=32><figcaption>32</figcaption></figure>
<figure><img src="{u(c['i16'])}" width=16 height=16><figcaption>16</figcaption></figure>
<figure><img class=px src="{u(c['i16'])}" width=64 height=64><figcaption>16 ×4</figcaption></figure></div>"""
    cols = ""
    for k, c in enumerate(C, 1):
        tbs = ""
        for t, lab in (("C", "C 卡通"), ("A", "A 浅色")):
            tb = (HERE / f"tb-{k}-{t}.png").resolve().as_uri()
            tbs += f"""<div class=lab>标题栏 · {lab} · 1×</div><img src="{tb}" width=264 height=44 class=tb1>
<div class=lab>标题栏 · {lab} · ×2</div><img src="{tb}" width=528 height=88 class="px tb2">"""
        cols += f"""<section><h2><span>{c['n']}</span>{c['name']}</h2>
<div class=big><img src="{u(c['big'])}" width=256 height=256></div>
<div class=lab>任务栏 · 浅色 #f3f3f3 / 深色 #202020</div>
<div class=strips>{strip(c,'#f3f3f3','#7a7a7a')}{strip(c,'#202020','#9a9a9a')}</div>{tbs}</section>"""
    html = f"""<!doctype html><meta charset=utf-8><style>{FACE}
*{{box-sizing:border-box}}body{{margin:0;width:1800px;background:#f7f5ef;font-family:Geist,NotoSC,sans-serif;color:#141412}}
header{{padding:34px 48px 6px;font:700 30px Geist,NotoSC,sans-serif;letter-spacing:-.01em}}
header small{{font:500 15px NotoSC,sans-serif;color:#86817a;margin-left:14px}}
main{{display:grid;grid-template-columns:repeat(3,536px);gap:40px;padding:22px 48px 40px}}
h2{{margin:0 0 14px;font:700 26px NotoSC,Geist,sans-serif;display:flex;align-items:baseline;gap:12px}}
h2 span{{font:600 15px Geist,NotoSC;background:#141412;color:#d4f24c;padding:4px 10px;border-radius:999px}}
.big{{height:340px;background:#fff;border:1px solid #e4dfd3;border-radius:16px;display:grid;place-items:center}}
.lab{{font:500 13px NotoSC,Geist,sans-serif;color:#86817a;margin:16px 0 7px}}
.strips{{display:grid;grid-template-columns:1fr 1fr;gap:12px}}
.strip{{height:112px;border-radius:10px;display:flex;align-items:flex-end;justify-content:center;gap:26px;padding:0 0 12px}}
figure{{margin:0;display:flex;flex-direction:column;align-items:center;gap:8px}}
figcaption{{font:500 12px Geist,NotoSC}}
img{{display:block}}.px{{image-rendering:pixelated}}.tb1,.tb2{{border-radius:6px;box-shadow:0 0 0 1px #e4dfd3}}
</style><header>DiskWhy 图标方案对比<small>256 · 32 · 16 · 标题栏</small></header><main>{cols}</main>"""
    f = HERE / "compare.html"; f.write_text(html, encoding="utf8")
    pg.set_viewport_size({"width": 1800, "height": 1200})
    pg.goto(f.as_uri()); pg.evaluate("document.fonts.ready"); pg.wait_for_timeout(400)
    pg.screenshot(path=str(LOGO / "compare.png"), full_page=True)
    print(pg.evaluate("document.body.scrollHeight"))
    b.close()
