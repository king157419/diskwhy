"""Quick side-by-side explorations of master parameters (writes wip/explore-<name>.png)."""
import sys, json
from pathlib import Path
from make import M, master_paths, mark_svg, uri, C2, HERE
from playwright.sync_api import sync_playwright

def run(name, variants, bg="#f3f3f3", px=300):
    px = int(__import__("os").environ.get("PX", px))
    cells = [f"<div><img src='{uri(C2/'png'/'icon@1024.png')}' width={px}><p>original</p></div>"]
    for label, over in variants:
        m = dict(M); m.update(over)
        svg = mark_svg(master_paths(m, 80, 100), 1024, m["ring"], m.get("ring_op", 1))
        p = HERE / "wip" / f"_x_{len(cells)}.svg"; p.write_text(svg, encoding="utf-8")
        cells.append(f"<div><img src='{uri(p)}' width={px}><p>{label}</p></div>")
    html = (f"<html><body style='margin:0;background:{bg};display:flex;flex-wrap:wrap;width:{(px+10)*4}px;font:13px sans-serif;color:#888'>"
            + "".join(cells) + "</body></html>")
    with sync_playwright() as p:
        b = p.chromium.launch(channel="msedge"); pg = b.new_page(viewport={"width": (px+10)*4, "height": 400})
        pg.set_content(html); pg.wait_for_timeout(300)
        pg.screenshot(path=str(HERE / "wip" / f"explore-{name}.png"), full_page=True); b.close()

if __name__ == "__main__":
    name = sys.argv[1]; variants = json.loads(sys.argv[2]); bg = sys.argv[3] if len(sys.argv) > 3 else "#f3f3f3"
    run(name, variants, bg)
