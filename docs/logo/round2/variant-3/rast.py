r"""Rasteriser + measurement helpers (Playwright + installed Edge, DPR 1).

Usage as a script (quick look during iteration):
  D:\conda\miniconda3\python.exe rast.py out.png a.svg b.svg ...
renders each svg at 256 on #f3f3f3 and #202020 plus 32/24/16 ×1 and ×4, in one strip.
"""
import base64
import sys
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent


def b64(p, mime):
    return f"data:{mime};base64," + base64.b64encode(Path(p).read_bytes()).decode()


class Raster:
    def __init__(self):
        from playwright.sync_api import sync_playwright
        self._pw = sync_playwright().start()
        self.b = self._pw.chromium.launch(channel="msedge")
        self.pg = self.b.new_page(device_scale_factor=1)

    def svg(self, src, px, out, bg=None):
        uri = b64(src, "image/svg+xml")
        self.pg.set_content(
            f"<html><body style='margin:0;background:{bg or 'transparent'}'>"
            f"<img id=i src='{uri}' width={px} height={px} style='display:block'></body></html>"
        )
        self.pg.wait_for_function("document.getElementById('i').complete")
        Path(out).parent.mkdir(parents=True, exist_ok=True)
        self.pg.locator("#i").screenshot(path=str(out), omit_background=bg is None)
        return Path(out)

    def html(self, html, out, width=1600):
        pg = self.b.new_page(viewport={"width": width, "height": 600}, device_scale_factor=1)
        pg.set_content(html)
        pg.wait_for_timeout(300)
        pg.screenshot(path=str(out), full_page=True)
        pg.close()

    def close(self):
        self.b.close()
        self._pw.stop()


def centroid(png):
    im = Image.open(png).convert("RGBA")
    W, H = im.size
    a = im.getchannel("A").load()
    sx = sy = n = 0
    minx, miny, maxx, maxy = W, H, 0, 0
    for y in range(H):
        for x in range(W):
            if a[x, y] >= 128:
                n += 1; sx += x; sy += y
                minx, maxx, miny, maxy = min(minx, x), max(maxx, x), min(miny, y), max(maxy, y)
    return dict(cx=sx / n, cy=sy / n, bbox=(minx, miny, maxx, maxy), fill=n / (W * H))


def areas(png, palette):
    """Share of opaque pixels closest to each palette colour (name -> hex)."""
    im = Image.open(png).convert("RGBA")
    cols = {k: tuple(int(v[i:i + 2], 16) for i in (1, 3, 5)) for k, v in palette.items()}
    cnt = {k: 0 for k in cols}
    n = 0
    for r, g, b, a in im.getdata():
        if a < 128:
            continue
        n += 1
        k = min(cols, key=lambda k: (cols[k][0] - r) ** 2 + (cols[k][1] - g) ** 2 + (cols[k][2] - b) ** 2)
        cnt[k] += 1
    return {k: v / n for k, v in cnt.items()}


def quick(out, svgs, sizes_src=None):
    """Strip for iteration: each row = one svg, 256 light/dark, then small sizes 1x and x4."""
    R = Raster()
    tmp = HERE / "_tmp"
    rows = []
    for s in svgs:
        s = Path(s)
        cells = []
        for bg in ("#f3f3f3", "#202020"):
            p = R.svg(s, 256, tmp / f"{s.stem}-256.png")
            cells.append(f"<div class=c style='background:{bg}'><img src='{b64(p,'image/png')}' width=256></div>")
        small = (sizes_src or {}).get(s.name, {})
        for bg in ("#f3f3f3", "#202020"):
            zs = []
            for px in (32, 24, 16):
                src = small.get(px, s)
                p = R.svg(src, px, tmp / f"{Path(src).stem}-{px}.png")
                u = b64(p, "image/png")
                zs.append(f"<div style='display:flex;flex-direction:column;align-items:center;gap:6px'><img class=px src='{u}' width={px*4}><img class=px src='{u}' width={px}></div>")
            cells.append(f"<div class=c style='background:{bg};gap:14px;padding:0 14px'>{''.join(zs)}</div>")
        rows.append(f"<div class=r><div class=lab>{s.stem}</div>{''.join(cells)}</div>")
    html = ("<html><body style='margin:0;background:#e8e4da;font:13px Segoe UI'><style>"
            ".r{display:flex;gap:10px;padding:10px;align-items:center}.lab{width:110px}"
            ".c{height:300px;min-width:300px;display:flex;align-items:center;justify-content:center;border-radius:10px}"
            ".px{image-rendering:pixelated}</style>" + "".join(rows) + "</body></html>")
    R.html(html, out, width=1700)
    R.close()


if __name__ == "__main__":
    quick(sys.argv[1], sys.argv[2:])


def zoomgrid(out, svgs, k=12, bgs=("#f3f3f3", "#202020")):
    """For each svg (rendered at its own viewBox size, 1x): 1x crops + xk nearest with pixel grid,
    on each background, laid out with PIL."""
    from PIL import ImageDraw
    R = Raster()
    tiles = []
    for s in svgs:
        s = Path(s)
        txt = s.read_text(encoding="utf-8")
        import re
        n = int(re.search(r'viewBox="0 0 (\d+)', txt).group(1))
        p = R.svg(s, n, HERE / "_tmp" / f"z-{s.stem}.png")
        im = Image.open(p).convert("RGBA")
        row = []
        for bg in bgs:
            base = Image.new("RGBA", (n, n), bg); base.alpha_composite(im)
            big = base.resize((n * k, n * k), Image.NEAREST)
            d = ImageDraw.Draw(big)
            gc = (128, 128, 128, 90)
            for i in range(n + 1):
                d.line([(i * k, 0), (i * k, n * k)], fill=gc); d.line([(0, i * k), (n * k, i * k)], fill=gc)
            row.append((base, big))
        tiles.append((s.stem, n, row))
    R.close()
    W = sum(max(n * k, 1) * len(bgs) + 40 * len(bgs) for _, n, _ in tiles[:1]) + 200
    Hh = sum(n * k + 60 for _, n, _ in tiles) + 20
    W = max(len(bgs) * (t[1] * k + 120) for t in tiles) + 40
    sheet = Image.new("RGB", (W, Hh), (232, 228, 218))
    d = ImageDraw.Draw(sheet)
    y = 10
    for name, n, row in tiles:
        x = 10
        d.text((x, y), name, fill=(40, 40, 40))
        for base, big in row:
            sheet.paste(big.convert("RGB"), (x, y + 16))
            # 1x on a padded swatch
            sw = Image.new("RGB", (n + 16, n + 16), base.getpixel((0, 0))[:3])
            sw.paste(base.convert("RGB"), (8, 8))
            sheet.paste(sw, (x + n * k + 10, y + 16))
            x += n * k + n + 50
        y += n * k + 40
    sheet.save(out)
