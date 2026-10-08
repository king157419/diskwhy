"""Shared Playwright/Edge helpers: rasterise an SVG, measure the alpha centroid, screenshot HTML."""
import base64
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
ORIG = ROOT / "docs" / "logo" / "concept-2"


def b64(p: Path, mime: str) -> str:
    return f"data:{mime};base64," + base64.b64encode(Path(p).read_bytes()).decode()


def svg_uri(p):
    return b64(p, "image/svg+xml")


def png_uri(p):
    return b64(p, "image/png")


def raster(page, svg_path, px, out):
    page.set_content(
        "<html><body style='margin:0;background:transparent'>"
        f"<img id=i src='{svg_uri(svg_path)}' width={px} height={px} style='display:block'></body></html>"
    )
    page.wait_for_function("document.getElementById('i').complete")
    page.wait_for_timeout(50)
    page.locator("#i").screenshot(path=str(out), omit_background=True)
    return out


CENTROID_JS = """async (src) => {
  const img = new Image(); img.src = src; await img.decode();
  const c = document.createElement('canvas'); c.width = 1024; c.height = 1024;
  const g = c.getContext('2d'); g.drawImage(img, 0, 0, 1024, 1024);
  const d = g.getImageData(0, 0, 1024, 1024).data;
  let sx = 0, sy = 0, s = 0, minx = 1e9, miny = 1e9, maxx = -1, maxy = -1;
  for (let y = 0; y < 1024; y++) for (let x = 0; x < 1024; x++) {
    const a = d[(y * 1024 + x) * 4 + 3] / 255;
    if (a > 0) { sx += a * x; sy += a * y; s += a;
      if (a > 0.5) { minx = Math.min(minx, x); maxx = Math.max(maxx, x); miny = Math.min(miny, y); maxy = Math.max(maxy, y); } }
  }
  return {cx: sx / s + 0.5, cy: sy / s + 0.5, bbox: [minx, miny, maxx + 1, maxy + 1]};
}"""


def centroid(page, svg_path):
    page.set_content("<html><body></body></html>")
    return page.evaluate(CENTROID_JS, svg_uri(svg_path))


def shot(browser, html, out, width=1600):
    pg = browser.new_page(viewport={"width": width, "height": 900}, device_scale_factor=1)
    pg.set_content(html)
    pg.wait_for_timeout(400)
    pg.screenshot(path=str(out), full_page=True)
    pg.close()
    return out
