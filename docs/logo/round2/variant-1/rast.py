r"""Rasterise SVGs with Playwright + installed Edge.
Usage: python rast.py SIZE out_dir a.svg [b.svg ...]   (transparent PNG, DPR 1)"""
import base64, sys
from pathlib import Path
from playwright.sync_api import sync_playwright


def rasterise(page, svg_path, px, out, bg=None):
    uri = "data:image/svg+xml;base64," + base64.b64encode(Path(svg_path).read_bytes()).decode()
    page.set_content(f"<html><body style='margin:0;background:{bg or 'transparent'}'>"
                     f"<img id=i src='{uri}' width={px} height={px} style='display:block'></body></html>")
    page.wait_for_function("document.getElementById('i').complete")
    page.locator("#i").screenshot(path=str(out), omit_background=bg is None)
    return out


if __name__ == "__main__":
    px, od = int(sys.argv[1]), Path(sys.argv[2])
    od.mkdir(exist_ok=True, parents=True)
    with sync_playwright() as p:
        b = p.chromium.launch(channel="msedge")
        pg = b.new_page(device_scale_factor=1, viewport={"width": max(800, px), "height": max(600, px)})
        for s in sys.argv[3:]:
            o = od / f"{Path(s).stem}@{px}.png"
            rasterise(pg, s, px, o)
            print(o)
        b.close()
