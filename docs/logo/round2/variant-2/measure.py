r"""Measure a 1024 render: alpha centroid, bbox, colour area shares, share passing 3:1 on each taskbar.
Run: D:\conda\miniconda3\python.exe docs/logo/round2/variant-2/measure.py [png]"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent


def lin(c):
    c /= 255
    return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4


def lum(r, g, b):
    return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)


def cr(l1, l2):
    return (max(l1, l2) + 0.05) / (min(l1, l2) + 0.05)


def measure(png, overlay=None):
    im = Image.open(png).convert("RGBA")
    W, H = im.size
    px = im.load()
    sx = sy = n = 0
    minx, maxx, miny, maxy = W, 0, H, 0
    hist = {}
    pass_l = pass_d = 0
    Ll, Ld = lum(243, 243, 243), lum(32, 32, 32)
    cache = {}
    for y in range(H):
        for x in range(W):
            r, g, b, a = px[x, y]
            if a < 128:
                continue
            n += 1
            sx += x
            sy += y
            minx, maxx, miny, maxy = min(minx, x), max(maxx, x), min(miny, y), max(maxy, y)
            key = (r >> 3, g >> 3, b >> 3)
            hist[key] = hist.get(key, 0) + 1
            if (r, g, b) not in cache:
                L = lum(r, g, b)
                cache[(r, g, b)] = (cr(L, Ll) >= 3, cr(L, Ld) >= 3)
            pl, pd = cache[(r, g, b)]
            pass_l += pl
            pass_d += pd
    cx, cy = sx / n, sy / n
    res = dict(centroid=(round(cx, 1), round(cy, 1)), bbox=(minx, miny, maxx, maxy),
               fill=round(n / (W * H), 3), pass_light=round(pass_l / n, 3), pass_dark=round(pass_d / n, 3))
    if overlay:
        out = Image.new("RGBA", (W, H), (243, 243, 243, 255))
        out.alpha_composite(im)
        d = ImageDraw.Draw(out)
        for k in range(0, W + 1, 16):  # 64-unit grid
            d.line([(k, 0), (k, H)], fill=(0, 0, 0, 18))
            d.line([(0, k), (W, k)], fill=(0, 0, 0, 18))
        d.rectangle([64, 64, 960, 960], outline=(0, 120, 255, 150), width=2)
        d.line([(512, 0), (512, H)], fill=(0, 120, 255, 150), width=2)
        d.line([(0, 512), (W, 512)], fill=(0, 120, 255, 150), width=2)
        d.line([(0, 496), (W, 496)], fill=(255, 0, 160, 120), width=2)
        d.rectangle([minx, miny, maxx, maxy], outline=(255, 0, 0, 160), width=2)
        d.ellipse([cx - 12, cy - 12, cx + 12, cy + 12], fill=(255, 0, 0, 230))
        out.convert("RGB").save(overlay)
    return res


if __name__ == "__main__":
    p = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "png" / "icon@1024.png"
    print(measure(p))
