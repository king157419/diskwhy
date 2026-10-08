"""Small sizes at x8 (nearest neighbour), original vs mine, light and dark: pix.py <label>"""
import sys
from playwright.sync_api import sync_playwright
from rast import HERE, ORIG, png_uri, raster, shot
label = sys.argv[1]
W = HERE / "wip"
with sync_playwright() as p:
    b = p.chromium.launch(channel="msedge")
    pg = b.new_page(device_scale_factor=1)
    mine = {s: png_uri(raster(pg, HERE / f"icon-{s}.svg", s, W / f"m{s}.png")) for s in (32, 24, 16)}
    orig = {s: png_uri(ORIG / "png" / f"icon-{s}@{s}.png") for s in (32, 24, 16)}
    def cell(src, s, bg, z=8):
        return f"<div style='background:{bg};padding:{z*2}px;border-radius:8px'><img src='{src}' width={s*z} height={s*z} style='image-rendering:pixelated;display:block'></div>"
    rows = ""
    for name, R in (("第一版", orig), ("变体3 " + label, mine)):
        rows += f"<h3>{name}</h3><div class=r>" + "".join(cell(R[s], s, bg, 8 if s < 32 else 6) for s in (32, 24, 16) for bg in ("#f3f3f3", "#202020")) + "</div>"
    html = f"<html><body style='margin:0;background:#ebe7dc;font-family:Segoe UI;padding:20px;width:1600px'><style>.r{{display:flex;gap:14px;align-items:flex-end}} h3{{margin:10px 0}}</style>{rows}</body></html>"
    shot(b, html, W / f"{label}-pix.png", 1640)
    b.close()
