"""Crop regions of the 1024 master at 1:1 for close inspection: zoom.py out.png x y w h [svg]"""
import sys
from playwright.sync_api import sync_playwright
from rast import HERE, svg_uri
out, x, y, w, h = sys.argv[1], *map(int, sys.argv[2:6])
src = sys.argv[6] if len(sys.argv) > 6 else "icon.svg"
bg = sys.argv[7] if len(sys.argv) > 7 else "#ffffff"
with sync_playwright() as p:
    b = p.chromium.launch(channel="msedge")
    pg = b.new_page(viewport={"width": w, "height": h}, device_scale_factor=1)
    pg.set_content(f"<html><body style='margin:0;background:{bg};overflow:hidden'><img src='{svg_uri(HERE/src)}' width=1024 height=1024 style='position:absolute;left:{-x}px;top:{-y}px'></body></html>")
    pg.wait_for_timeout(300)
    pg.screenshot(path=str(HERE / "wip" / out))
    b.close()
