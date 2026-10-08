r"""Measure a 1024 PNG: alpha bbox, alpha centroid, area share by nearest palette colour.
Usage: D:\conda\miniconda3\python.exe measure.py png/icon@1024.png [overlay.png]"""
import sys
from PIL import Image, ImageDraw

PAL = {"ink": (0x14, 0x14, 0x12), "lime": (0xd4, 0xf2, 0x4a), "paper": (0xf6, 0xf1, 0xe4),
       "moss": (0x5c, 0x7a, 0x1a), "clay": (0x8a, 0x85, 0x7a)}


def measure(path, overlay=None, quiet=False):
    im = Image.open(path).convert("RGBA")
    W, H = im.size
    px = im.load()
    sx = sy = n = 0
    minx, maxx, miny, maxy = W, 0, H, 0
    cnt = {k: 0 for k in PAL}
    for y in range(H):
        for x in range(W):
            r, g, b, a = px[x, y]
            if a < 128:
                continue
            n += 1; sx += x; sy += y
            minx, maxx, miny, maxy = min(minx, x), max(maxx, x), min(miny, y), max(maxy, y)
            k = min(PAL, key=lambda k: sum((c - d) ** 2 for c, d in zip(PAL[k], (r, g, b))))
            cnt[k] += 1
    cx, cy = sx / n, sy / n
    res = dict(bbox=(minx, miny, maxx, maxy), centroid=(round(cx, 1), round(cy, 1)),
               fill=round(n / (W * H), 3), share={k: round(v / n, 3) for k, v in cnt.items() if v})
    if not quiet:
        print(res)
    if overlay:
        out = Image.new("RGBA", (W, H), (243, 243, 243, 255)); out.alpha_composite(im)
        d = ImageDraw.Draw(out)
        s = W / 1024
        for i in range(0, 65):  # 64-unit grid
            c = (0, 0, 0, 40) if i % 4 else (0, 0, 0, 90)
            d.line([(i * 16 * s, 0), (i * 16 * s, H)], fill=c); d.line([(0, i * 16 * s), (W, i * 16 * s)], fill=c)
        d.rectangle([64 * s, 64 * s, 960 * s, 960 * s], outline=(0, 120, 255, 200), width=3)
        d.line([(512 * s, 0), (512 * s, H)], fill=(0, 120, 255, 160), width=2)
        d.line([(0, 496 * s), (W, 496 * s)], fill=(0, 120, 255, 160), width=2)
        d.rectangle([minx, miny, maxx, maxy], outline=(255, 0, 0, 180), width=2)
        d.ellipse([cx - 12, cy - 12, cx + 12, cy + 12], fill=(255, 0, 0, 230))
        out.convert("RGB").save(overlay)
    return res


if __name__ == "__main__":
    measure(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
