"""Per-round side-by-side with the round-1 original (concept-2).

Run:  D:\\conda\\miniconda3\\python.exe docs/logo/round3/variant-3/compare.py <round-label>
Writes wip/<label>-compare.png and prints the alpha centroid of icon.svg.
"""
import sys

from playwright.sync_api import sync_playwright

from rast import HERE, ORIG, centroid, png_uri, raster, shot, svg_uri

label = sys.argv[1] if len(sys.argv) > 1 else "r"
W = HERE / "wip"
W.mkdir(exist_ok=True)

with sync_playwright() as p:
    b = p.chromium.launch(channel="msedge")
    pg = b.new_page(device_scale_factor=1)
    c = centroid(pg, HERE / "icon.svg")
    c0 = centroid(pg, ORIG / "icon.svg")
    print("new centroid", c, "\norig centroid", c0)
    mine = {}
    for px in (256, 64, 48):
        mine[px] = png_uri(raster(pg, HERE / "icon.svg", px, W / f"m{px}.png"))
    small = {}
    for px in (32, 24, 16):
        src = HERE / f"icon-{px}.svg"
        if src.exists():
            small[px] = png_uri(raster(pg, src, px, W / f"m{px}.png"))
        else:
            small[px] = mine[48]
    orig = {px: png_uri(ORIG / "png" / n) for px, n in
            ((256, "icon@256.png"), (64, "icon@64.png"), (48, "icon@48.png"),
             (32, "icon-32@32.png"), (24, "icon-24@24.png"), (16, "icon-16@16.png"))}
    BIG_O = svg_uri(ORIG / "icon.svg")
    BIG_N = svg_uri(HERE / "icon.svg")

    def col(title, big, sm, mid):
        def row(bg):
            cells = "".join(f"<img class=px src='{sm[s]}' width={s} height={s}>" for s in (32, 24, 16))
            z = "".join(f"<img class=px src='{sm[s]}' width={s*4} height={s*4}>" for s in (16, 24))
            return (f"<div class=bar style='background:{bg}'><img src='{mid[64]}' width=64>"
                    f"<img src='{mid[48]}' width=48>{cells}<span style='width:18px'></span>{z}</div>")
        return (f"<div class=col><h2>{title}</h2>"
                f"<div class=big style='background:#fff'><img src='{big}' width=480 height=480></div>"
                f"<div class=big2><div style='background:#202020'><img src='{big}' width=230></div>"
                f"<div style='background:#d4f24a'><img src='{big}' width=230></div></div>"
                f"{row('#f3f3f3')}{row('#202020')}</div>")

    html = f"""<html><head><style>
    body {{ margin:0; background:#ebe7dc; font-family:'Segoe UI',sans-serif; width:1600px }}
    .wrap {{ display:flex; gap:40px; padding:30px 40px }}
    .col {{ flex:1 }} h2 {{ font-size:18px; margin:0 0 10px; color:#333 }}
    .big {{ height:520px; display:flex; align-items:center; justify-content:center; border-radius:16px }}
    .big2 {{ display:flex; gap:12px; margin-top:12px }}
    .big2 div {{ flex:1; height:260px; display:flex; align-items:center; justify-content:center; border-radius:12px }}
    .bar {{ display:flex; gap:22px; align-items:center; padding:16px 22px; border-radius:12px; margin-top:12px }}
    .px {{ image-rendering:pixelated }}
    </style></head><body><div class=wrap>
    {col('第一版 · concept-2', BIG_O, orig, orig)}
    {col('变体 3 · 糖果光泽 · ' + label, BIG_N, small, mine)}
    </div></body></html>"""
    shot(b, html, W / f"{label}-compare.png")
    b.close()
print("wrote", W / f"{label}-compare.png")
