r"""Build docs/logo/round2/compare.png: round-1 concept-2 (reference) vs round-2 variants 1/2/3.
Run with D:\conda\miniconda3\python.exe  (Playwright + installed Edge). Adapted from docs/logo/_compare/build_compare.py."""
import pathlib
from playwright.sync_api import sync_playwright
HERE = pathlib.Path(__file__).parent
R2 = HERE.parent
LOGO = R2.parent
FONTS = (LOGO.parent.parent / "src" / "assets" / "fonts").resolve().as_uri()

C = [
  dict(n="上一版", name="会说话的磁盘", d="concept-2", big="png/icon@256.png", i32="icon-32.svg", i16="icon-16.svg",
       l32="icon-32.svg", l16="icon-16.svg", tC="icon-16.svg", tA="icon-16.svg", narrow=True),
  dict(n="变体 1", name="留白气泡", d="round2/variant-1", big="png/icon@256.png", i32="icon-32.svg", i16="icon-16.svg",
       l32="icon-32-light.svg", l16="icon-16-light.svg", tC="mark-16-C.svg", tA="mark-16-A.svg"),
  dict(n="变体 2", name="长出来的尾巴", d="round2/variant-2", big="png/icon@256.png", i32="icon-32.svg", i16="icon-16.svg",
       l32="icon-32-light.svg", l16="icon-16-light.svg", tC="mark-16-C.svg", tA="mark-16-A.svg"),
  dict(n="变体 3", name="叠层气泡", d="round2/variant-3", big="png/icon@256.png", i32="icon-32.svg", i16="icon-16.svg",
       l32="icon-32-light.svg", l16="icon-16-light.svg", tC="icon-16-C.svg", tA="icon-16-A.svg"),
]
FACE = f"""@font-face{{font-family:Geist;src:url({FONTS}/Geist-latin.woff2)}}
@font-face{{font-family:NotoSC;src:url({FONTS}/NotoSansSC-DiskWhy.woff2)}}"""
THEME = {"C": ("#f4f0e6", "#141412", "2px solid #141412"), "A": ("#f3f1ec", "#1c1b18", "1px solid #e1ddd4")}
TBW = 220

def titlebar_html(svg, theme):
    bg, ink, border = THEME[theme]
    src = svg.resolve().as_uri()
    return f"""<!doctype html><meta charset=utf-8><style>{FACE}
body{{margin:0}}.tb{{width:{TBW}px;height:44px;box-sizing:border-box;background:{bg};border-bottom:{border};
display:flex;align-items:center;gap:8px;padding:0 14px;color:{ink};font:600 13px Geist,NotoSC,sans-serif}}
.tb img{{width:16px;height:16px;display:block}}.c{{margin-left:auto;display:flex;gap:16px;opacity:.55}}
.c i{{display:block;width:10px;height:10px}}</style>
<div class=tb><img src="{src}">DiskWhy<span class=c>
<i style="border-bottom:1px solid {ink};height:5px"></i><i style="border:1px solid {ink};box-sizing:border-box"></i>
<i style="background:linear-gradient(45deg,transparent 45%,{ink} 45%,{ink} 55%,transparent 55%),linear-gradient(-45deg,transparent 45%,{ink} 45%,{ink} 55%,transparent 55%)"></i></span></div>"""

with sync_playwright() as p:
    b = p.chromium.launch(channel="msedge")
    pg = b.new_page(device_scale_factor=1, viewport={"width": 400, "height": 100})
    for k, c in enumerate(C):
        for t in ("C", "A"):
            f = HERE / "_tb.html"; f.write_text(titlebar_html(LOGO / c["d"] / c["t" + t], t), encoding="utf8")
            pg.goto(f.as_uri()); pg.evaluate("document.fonts.ready"); pg.wait_for_timeout(150)
            pg.screenshot(path=str(HERE / f"tb-{k}-{t}.png"), clip={"x": 0, "y": 0, "width": TBW, "height": 44})
    (HERE / "_tb.html").unlink()

    def u(c, rel): return (LOGO / c["d"] / rel).resolve().as_uri()
    def strip(c, bg, fg, k32, k16, extra=""):
        return f"""<div class=strip style="background:{bg};color:{fg}">
<figure><img src="{u(c,c[k32])}" width=32 height=32><figcaption>32</figcaption></figure>
<figure><img src="{u(c,c[k16])}" width=16 height=16><figcaption>16</figcaption></figure>
<figure><img class=px src="{u(c,c[k16])}" width=64 height=64><figcaption>16 ×4</figcaption></figure>{extra}</div>"""
    cols = ""
    for k, c in enumerate(C):
        narrow = c.get("narrow", False)
        tbs = ""
        for t, lab in (("C", "C 卡通"), ("A", "A 浅色")):
            tb = (HERE / f"tb-{k}-{t}.png").resolve().as_uri()
            tbs += f"""<div class=lab>标题栏 · {lab} · 1× / ×2</div><img src="{tb}" width={TBW} height=44 class=tb1>
<div class=clip><img src="{tb}" width={TBW*2} height=88 class="px tb2"></div>"""
        # light taskbar: the master set (what a single .ico shows) + the light-set 16 ×4 for variants that have one
        extra = "" if narrow else f"""<figure><img class=px src="{u(c,c['l16'])}" width=64 height=64><figcaption>浅色套 ×4</figcaption></figure>"""
        light = strip(c, "#f3f3f3", "#7a7a7a", "i32", "i16", extra)
        dark = strip(c, "#202020", "#9a9a9a", "i32", "i16")
        cols += f"""<section class="{'narrow' if narrow else ''}"><h2><span>{c['n']}</span>{c['name']}</h2>
<div class=big><img src="{u(c,c['big'])}" width={200 if narrow else 256} height={200 if narrow else 256}></div>
<div class=lab>任务栏 · 浅色 #f3f3f3</div>{light}
<div class=lab>任务栏 · 深色 #202020</div>{dark}{tbs}</section>"""
    html = f"""<!doctype html><meta charset=utf-8><style>{FACE}
*{{box-sizing:border-box}}body{{margin:0;width:1800px;background:#f7f5ef;font-family:Geist,NotoSC,sans-serif;color:#141412}}
header{{padding:32px 40px 4px;font:700 30px Geist,NotoSC,sans-serif;letter-spacing:-.01em}}
header small{{font:500 15px NotoSC,sans-serif;color:#86817a;margin-left:14px}}
main{{display:grid;grid-template-columns:300px repeat(3,440px);gap:32px;padding:18px 40px 36px;align-items:start}}
h2{{margin:0 0 12px;font:700 24px NotoSC,Geist,sans-serif;display:flex;align-items:baseline;gap:10px;white-space:nowrap}}
h2 span{{font:600 14px Geist,NotoSC;background:#141412;color:#d4f24c;padding:4px 10px;border-radius:999px}}
.narrow h2 span{{background:#86817a;color:#fff}}
.big{{height:280px;background:#fff;border:1px solid #e4dfd3;border-radius:16px;display:grid;place-items:center}}
.narrow .big{{background:#f0ede6}}
.lab{{font:500 13px NotoSC,Geist,sans-serif;color:#86817a;margin:14px 0 6px}}
.strip{{height:104px;border-radius:10px;display:flex;align-items:flex-end;justify-content:center;gap:22px;padding:0 0 10px}}
figure{{margin:0;display:flex;flex-direction:column;align-items:center;gap:7px}}
figcaption{{font:500 12px Geist,NotoSC;white-space:nowrap}}
img{{display:block}}.px{{image-rendering:pixelated}}.tb1,.tb2{{border-radius:6px;box-shadow:0 0 0 1px #e4dfd3}}
.clip{{margin-top:8px;overflow:hidden;border-radius:6px}}.narrow .clip{{width:300px}}
</style><header>DiskWhy 图标 · 第二轮对比<small>256 · 32 · 16 · 标题栏 C / A</small></header><main>{cols}</main>"""
    f = HERE / "compare.html"; f.write_text(html, encoding="utf8")
    pg.set_viewport_size({"width": 1800, "height": 1200})
    pg.goto(f.as_uri()); pg.evaluate("document.fonts.ready"); pg.wait_for_timeout(400)
    pg.screenshot(path=str(R2 / "compare.png"), full_page=True)
    print(pg.evaluate("document.body.scrollHeight"))
    b.close()
