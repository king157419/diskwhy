"""Round 2: true semicircular bowl, taller i, balanced walls; straight vs lifted dot; tile vs sticker."""
import pathlib, sys
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from geom import rrect, svg
from render import Renderer

OUT = pathlib.Path(__file__).parent.parent / "iter" / "r2"
OUT.mkdir(parents=True, exist_ok=True)
INK, LIME, CREAM = "#141412", "#d4f24a", "#f3efe3"


def d_path(x, y, w, h, rl=44):
    r = h / 2
    return (f"M{x + rl} {y}H{x + w - r}A{r} {r} 0 0 1 {x + w - r} {y + h}"
            f"H{x + rl}A{rl} {rl} 0 0 1 {x} {y + h - rl}V{y + rl}A{rl} {rl} 0 0 1 {x + rl} {y}Z")


def mark(dx=0, scale=1.0, lift=False):
    # D 576 x 704, centred optically a bit left of the artboard centre (bowl carries more weight)
    D = d_path(224, 160, 576, 704)
    stem = rrect(408, 456, 160, 256, 20, 20, 20, 20)
    dot = rrect(408, 312, 160, 80, 20, 20, 20, 20)
    dot_t = ' transform="rotate(-10 488 352) translate(18 -14)"' if lift else ""
    return D, stem, dot, dot_t


TILE = rrect(80, 80, 864, 864, 192, 192, 192, 192)
variants = {}
for lift in (False, True):
    D, stem, dot, dt = mark(lift=lift)
    tag = "lift" if lift else "flat"
    # tile, ink plate, lime letter
    variants[f"a_{tag}"] = (f'<path fill="{INK}" d="{TILE}"/>'
                            f'<mask id="m"><path fill="#fff" d="{D}"/><path d="{stem}"/><path{dt} d="{dot}"/></mask>'
                            f'<rect width="1024" height="1024" fill="{LIME}" mask="url(#m)"/>')
    # sticker tile (C theme): lime plate, ink outline, hard shadow, ink letter
    T2 = rrect(72, 72, 848, 848, 188, 188, 188, 188)
    variants[f"b_{tag}"] = (f'<path fill="{INK}" transform="translate(32 32)" d="{T2}"/>'
                            f'<path fill="{LIME}" stroke="{INK}" stroke-width="36" d="{T2}"/>'
                            f'<mask id="m"><path fill="#fff" d="{D}"/><path d="{stem}"/><path{dt} d="{dot}"/></mask>'
                            f'<rect width="1024" height="1024" fill="{INK}" mask="url(#m)"/>')
    # no tile: lime D with ink outline + hard shadow, i filled ink
    variants[f"e_{tag}"] = (f'<path fill="{INK}" transform="translate(40 40)" d="{D}"/>'
                            f'<path fill="{LIME}" stroke="{INK}" stroke-width="44" stroke-linejoin="round" d="{D}"/>'
                            f'<path fill="{INK}" d="{stem}"/><path fill="{INK}"{dt} d="{dot}"/>')

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
.row{display:flex;align-items:center;gap:20px;margin-bottom:12px}.row b{width:80px}
.tb{display:flex;gap:16px;align-items:center;padding:12px}.px{image-rendering:pixelated}</style>""" + "".join(rows))
    r.html(html, OUT / "sheet.png", 1600)
print("ok")
