r"""Build docs/logo/round3/compare.png: round-1 concept-2 vs round-3 variants 1/2/3.
Run with D:\conda\miniconda3\python.exe (Playwright + installed Edge). Adapted from round2/_compare/build_compare.py.
Small sizes are first rasterised from each SVG at native size (1x), and the x4 / x2 views are
nearest-neighbour enlargements of those 1x pixels, so what you see is what the taskbar shows."""
import pathlib
from playwright.sync_api import sync_playwright
from PIL import Image
HERE = pathlib.Path(__file__).parent
R3 = HERE.parent
LOGO = R3.parent
FONTS = (LOGO.parent.parent / "src" / "assets" / "fonts").resolve().as_uri()
PX = HERE / "px"; PX.mkdir(exist_ok=True)

C = [
  dict(n="第一版", name="方案 2 原图", d="concept-2", big="png/icon@256.png", tC="icon-16.svg", tA="icon-16.svg", ref=True),
  dict(n="精修 1", name="原样精修", d="round3/variant-1", big="png/icon-256.png", tC="mark-16-C.svg", tA="mark-16-A.svg"),
  dict(n="精修 2", name="贴纸版", d="round3/variant-2", big="png/icon-256.png", tC="icon-16.svg", tA="icon-16.svg"),
  dict(n="精修 3", name="糖果光泽", d="round3/variant-3", big="png/icon@256.png", tC="icon-16.svg", tA="icon-16.svg"),
]
FACE = f"""@font-face{{font-family:Geist;src:url({FONTS}/Geist-latin.woff2)}}
@font-face{{font-family:NotoSC;src:url({FONTS}/NotoSansSC-DiskWhy.woff2)}}"""
THEME = {"C": ("#f4f0e6", "#141412", "2px solid #141412"), "A": ("#f3f1ec", "#1c1b18", "1px solid #e1ddd4")}
TBW = 188

def titlebar_html(svg, theme):
    bg, ink, border = THEME[theme]
    return f"""<!doctype html><meta charset=utf-8><style>{FACE}
body{{margin:0}}.tb{{width:{TBW}px;height:40px;box-sizing:border-box;background:{bg};border-bottom:{border};
display:flex;align-items:center;gap:8px;padding:0 12px;color:{ink};font:600 13px Geist,NotoSC,sans-serif}}
.tb img{{width:16px;height:16px;display:block}}.c{{margin-left:auto;display:flex;gap:14px;opacity:.55}}
.c i{{display:block;width:10px;height:10px}}</style>
<div class=tb><img src="{svg.resolve().as_uri()}">DiskWhy<span class=c>
<i style="border-bottom:1px solid {ink};height:5px"></i><i style="border:1px solid {ink};box-sizing:border-box"></i>
<i style="background:linear-gradient(45deg,transparent 45%,{ink} 45%,{ink} 55%,transparent 55%),linear-gradient(-45deg,transparent 45%,{ink} 45%,{ink} 55%,transparent 55%)"></i></span></div>"""

def go(pg, html, name):
    f = HERE / name; f.write_text(html, encoding="utf8"); pg.goto(f.as_uri())
    pg.evaluate("document.fonts.ready"); pg.wait_for_timeout(200); return f

with sync_playwright() as p:
    b = p.chromium.launch(channel="msedge")
    pg = b.new_page(device_scale_factor=1, viewport={"width": 400, "height": 100})
    for k, c in enumerate(C):
        d = LOGO / c["d"]
        for s in (32, 24, 16):
            go(pg, f'<!doctype html><style>html,body{{margin:0;background:transparent}}img{{display:block}}</style>'
                   f'<img src="{(d / f"icon-{s}.svg").resolve().as_uri()}" width={s} height={s}>', "_t.html")
            out = PX / f"{k}-{s}.png"
            pg.screenshot(path=str(out), clip={"x": 0, "y": 0, "width": s, "height": s}, omit_background=True)
        Image.open(PX / f"{k}-16.png").resize((64, 64), Image.NEAREST).save(PX / f"{k}-16x4.png")
        for t in ("C", "A"):
            go(pg, titlebar_html(d / c["t" + t], t), "_t.html")
            one = PX / f"tb-{k}-{t}.png"
            pg.screenshot(path=str(one), clip={"x": 0, "y": 0, "width": TBW, "height": 40})
            Image.open(one).resize((TBW * 2, 80), Image.NEAREST).save(PX / f"tb-{k}-{t}-x2.png")
    (HERE / "_t.html").unlink()

    def u(path): return pathlib.Path(path).resolve().as_uri()
    def strip(k, bg, fg):
        figs = "".join(f'<figure><img src="{u(PX / f"{k}-{s}.png")}" width={s} height={s}><figcaption>{s}</figcaption></figure>' for s in (32, 24, 16))
        return f"""<div class=strip style="background:{bg};color:{fg}">{figs}
<figure><img class=px src="{u(PX / f"{k}-16x4.png")}" width=64 height=64><figcaption>16 ×4</figcaption></figure></div>"""
    cols = ""
    for k, c in enumerate(C):
        tbs = "".join(f"""<div class=lab>标题栏 · {lab}</div><img src="{u(PX / f"tb-{k}-{t}.png")}" width={TBW} height=40 class=tb>
<img src="{u(PX / f"tb-{k}-{t}-x2.png")}" width={TBW * 2} height=80 class="px tb x2">""" for t, lab in (("C", "C 卡通 · 1× / ×2"), ("A", "A 浅色 · 1× / ×2")))
        cols += f"""<section class="{'ref' if c.get('ref') else ''}"><h2><span>{c['n']}</span>{c['name']}</h2>
<div class=big><img src="{u(LOGO / c['d'] / c['big'])}" width=256 height=256></div>
<div class=lab>任务栏 · 浅色</div>{strip(k, "#f3f3f3", "#7a7a7a")}
<div class=lab>任务栏 · 深色</div>{strip(k, "#202020", "#9a9a9a")}{tbs}</section>"""
    html = f"""<!doctype html><meta charset=utf-8><style>{FACE}
*{{box-sizing:border-box}}body{{margin:0;width:1800px;background:#f7f5ef;font-family:Geist,NotoSC,sans-serif;color:#141412}}
header{{padding:28px 40px 0;font:700 30px Geist,NotoSC,sans-serif;letter-spacing:-.01em}}
header small{{font:500 15px NotoSC,sans-serif;color:#86817a;margin-left:14px}}
main{{display:grid;grid-template-columns:repeat(4,400px);gap:40px;padding:18px 40px 32px;align-items:start}}
h2{{margin:0 0 12px;font:700 24px NotoSC,Geist,sans-serif;display:flex;align-items:baseline;gap:10px;white-space:nowrap}}
h2 span{{font:600 14px NotoSC,Geist;background:#141412;color:#d4f24c;padding:4px 10px;border-radius:999px}}
.ref h2 span{{background:#86817a;color:#fff}}
.big{{height:290px;background:#fff;border:1px solid #e4dfd3;border-radius:16px;display:grid;place-items:center}}
.ref .big{{background:#fff;border:2px dashed #c9c3b6}}
.lab{{font:500 13px NotoSC,Geist,sans-serif;color:#86817a;margin:14px 0 6px}}
.strip{{height:100px;border-radius:10px;display:flex;align-items:flex-end;justify-content:center;gap:26px;padding:0 0 10px}}
figure{{margin:0;display:flex;flex-direction:column;align-items:center;gap:7px}}
figcaption{{font:500 12px Geist,NotoSC;white-space:nowrap}}
img{{display:block}}.px{{image-rendering:pixelated}}.tb{{border-radius:6px;box-shadow:0 0 0 1px #e4dfd3}}.x2{{margin-top:8px}}
</style><header>DiskWhy 图标 · 第三轮精修对比<small>第一版 vs 精修 1 / 2 / 3</small></header><main>{cols}</main>"""
    f = HERE / "compare.html"; f.write_text(html, encoding="utf8")
    pg.set_viewport_size({"width": 1800, "height": 1100})
    pg.goto(f.as_uri()); pg.evaluate("document.fonts.ready"); pg.wait_for_timeout(500)
    pg.screenshot(path=str(R3 / "compare.png"), full_page=True)
    print(pg.evaluate("document.body.scrollHeight"))
    b.close()
