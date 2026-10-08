"""Builds lockup SVGs: the mark + "DiskWhy" set in Geist (OFL, bundled in src/assets/fonts) outlined to paths."""
import re, pathlib
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
here = pathlib.Path(__file__).parent
font = TTFont(here / "../../../src/assets/fonts/Geist-latin.woff2")
font = instantiateVariableFont(font, {"wght": 700})
gs = font.getGlyphSet(); cmap = font.getBestCmap(); upm = font["head"].unitsPerEm
def text_path(text, size, tracking=-0.02, kern={}):
    pen = SVGPathPen(gs); x = 0.0; s = size / upm
    for i, ch in enumerate(text):
        g = cmap[ord(ch)]
        gs[g].draw(TransformPen(pen, (s, 0, 0, -s, x, 0)))
        x += gs[g].width * s + tracking * size + kern.get(text[i:i+2], 0) * size
    return pen.getCommands(), x - tracking * size
mark = (here / "mark.svg").read_text(encoding="utf-8")
inner = re.search(r"<svg[^>]*>(.*)</svg>", mark, re.S).group(1)
inner = re.sub(r"<!--.*?-->", "", inner, flags=re.S)
def lockup(name, text_fill, mark_inner=inner):
    # mark drawn in a 1024 box; visible art spans roughly y 106..926. Text cap height ~ 300 units, centred on the body.
    size = 420; cap = 0.71 * size
    d, w = text_path("DiskWhy", size, kern={"kW": -0.03})
    tx = 1000; base = 640 + cap / 2  # body centre ~ y 640 in mark space
    W = int(tx + w + 60)
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="40 90 {W-40} 850">
  <g>{mark_inner}</g>
  <path transform="translate({tx} {base:.1f})" fill="{text_fill}" d="{d}"/>
</svg>
'''
    (here / name).write_text(svg, encoding="utf-8")
lockup("lockup.svg", "#141412")
lockup("lockup-on-dark.svg", "#f3ece2")
print("ok")
