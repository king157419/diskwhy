"""Outline 'DiskWhy' from Geist (OFL, bundled in src/assets/fonts) to SVG paths.

Returns per-glyph contours so the dot of the 'i' can be coloured separately.
"""
import pathlib
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.boundsPen import BoundsPen

FONT = pathlib.Path(__file__).parent / "Geist-latin.woff2"


def outline(text="DiskWhy", wght=640, tracking=-20):
    f = TTFont(FONT)
    f = instantiateVariableFont(f, {"wght": wght})
    gs = f.getGlyphSet()
    cmap = f.getBestCmap()
    hmtx = f["hmtx"]
    upm = f["head"].unitsPerEm
    # pair kerning from GPOS is skipped; Geist is evenly spaced, we track slightly tighter
    x = 0
    glyphs = []  # (char, [contour path strings], x)
    for ch in text:
        name = cmap[ord(ch)]
        rec = DecomposingRecordingPen(gs)
        gs[name].draw(rec)
        # split recording into contours
        contours, cur = [], []
        for op, args in rec.value:
            cur.append((op, args))
            if op in ("closePath", "endPath"):
                contours.append(cur); cur = []
        paths = []
        for c in contours:
            pen = SVGPathPen(gs)
            for op, args in c:
                getattr(pen, op)(*args)
            bp = BoundsPen(gs)
            for op, args in c:
                getattr(bp, op)(*args)
            paths.append((pen.getCommands(), bp.bounds))
        glyphs.append((ch, paths, x))
        x += hmtx[name][0] + tracking
    asc = f["OS/2"].sCapHeight
    return glyphs, x - tracking, asc, upm


if __name__ == "__main__":
    g, w, cap, upm = outline()
    print(w, cap, upm)
    for ch, paths, x in g:
        print(ch, x, [b for _, b in paths])
