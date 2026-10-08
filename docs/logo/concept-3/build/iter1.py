"""Round 1: rough variants of the D-with-i-counter mark, rendered side by side."""
import pathlib, sys
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from geom import rrect, svg
from render import Renderer

OUT = pathlib.Path(__file__).parent.parent / "iter" / "r1"
OUT.mkdir(parents=True, exist_ok=True)
INK, LIME, CREAM, GREEN = "#141412", "#d4f24a", "#f3efe3", "#2f7d5b"

# D outer + counter pieces
D = rrect(224, 192, 576, 640, 48, 280, 280, 48)
STEM = rrect(384, 432, 176, 240, 16, 40, 40, 16)
DOT = rrect(384, 320, 176, 72, 16, 24, 24, 16)
TILE = rrect(64, 64, 896, 896, 200, 200, 200, 200)
CUT = f'<path fill-rule="evenodd" d="{D}{STEM}{DOT}"/>'

variants = {
    "a_ink_tile_lime": f'<path fill="{INK}" d="{TILE}"/><path fill="{LIME}" fill-rule="evenodd" d="{D}{STEM}{DOT}"/>',
    "a2_ink_tile_cream_limedot": f'<path fill="{INK}" d="{TILE}"/><path fill="{CREAM}" fill-rule="evenodd" d="{D}{STEM}{DOT}"/><path fill="{LIME}" d="{DOT}"/>',
    "b_lime_tile_cartoon": (f'<path fill="{INK}" transform="translate(36 36)" d="{rrect(64,64,860,860,190,190,190,190)}"/>'
                             f'<path fill="{LIME}" stroke="{INK}" stroke-width="40" d="{rrect(64,64,860,860,190,190,190,190)}"/>'
                             f'<path fill="{INK}" fill-rule="evenodd" d="{D}{STEM}{DOT}"/>'),
    "c_cream_tile_green": f'<path fill="{CREAM}" d="{TILE}"/><path fill="{INK}" fill-rule="evenodd" d="{D}{STEM}{DOT}"/><path fill="{GREEN}" d="{DOT}"/>',
    "d_onecolor": f'<path fill="{INK}" fill-rule="evenodd" d="{D}{STEM}{DOT}"/>',
}

with Renderer() as r:
    rows = []
    for name, body in variants.items():
        p = OUT / f"{name}.svg"
        p.write_text(svg(body), encoding="utf8")
        for s in (256, 32, 24, 16):
            r.svg(p, s, OUT / f"{name}-{s}.png")
        cells = f"<img src='{name}-256.png' width=200>"
        for bg in ("#f3f3f3", "#202020"):
            cells += f"<div class=tb style='background:{bg}'>" + "".join(
                f"<img class=px src='{name}-{s}.png' width={s*4}>" for s in (32, 24, 16)) + "</div>"
        rows.append(f"<div class=row><b>{name}</b>{cells}</div>")
    html = OUT / "sheet.html"
    html.write_text("""<style>body{margin:20px;font:14px sans-serif;background:#ddd}
.row{display:flex;align-items:center;gap:20px;margin-bottom:16px}.row b{width:180px}
.tb{display:flex;gap:16px;align-items:center;padding:16px}.px{image-rendering:pixelated}</style>""" + "".join(rows))
    r.html(html, OUT / "sheet.png", 1600)
print("ok")
