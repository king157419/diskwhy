import sys, pathlib
from playwright.sync_api import sync_playwright
here = pathlib.Path(__file__).parent
out = here / "png"; out.mkdir(exist_ok=True)
# (svg file, sizes)
jobs = [(a.split(":")[0], [int(s) for s in a.split(":")[1].split(",")]) for a in sys.argv[1:-1]]
sheet = sys.argv[-1]
with sync_playwright() as p:
    b = p.chromium.launch(channel="msedge")
    for svg, sizes in jobs:
        src = (here / svg).read_text(encoding="utf-8")
        for s in sizes:
            pg = b.new_page(viewport={"width": s, "height": s}, device_scale_factor=1)
            pg.set_content(f'<html><body style="margin:0;background:transparent"><img src="data:image/svg+xml;base64,' +
                           __import__("base64").b64encode(src.encode()).decode() +
                           f'" width="{s}" height="{s}" style="display:block"></body></html>')
            pg.wait_for_timeout(50)
            pg.screenshot(path=str(out / f"{pathlib.Path(svg).stem}-{s}.png"), omit_background=True)
            pg.close()
    if sheet != "-":
        pg = b.new_page(viewport={"width": 1600, "height": 800}, device_scale_factor=1)
        pg.goto((here / sheet).resolve().as_uri())
        pg.wait_for_timeout(400)
        pg.screenshot(path=str(here / "sheet.png"), full_page=True)
    b.close()
