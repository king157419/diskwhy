"""Geometry helpers for the D-i mark (all in a 1024 artboard)."""


def rrect(x, y, w, h, tl=0, tr=0, br=0, bl=0):
    """Rounded rectangle path with individual corner radii (clockwise)."""
    def a(r, ex, ey):
        return f"A{r} {r} 0 0 1 {ex:g} {ey:g}" if r else f"L{ex:g} {ey:g}"
    return (f"M{x + tl:g} {y:g}H{x + w - tr:g}" + a(tr, x + w, y + tr) +
            f"V{y + h - br:g}" + a(br, x + w - br, y + h) +
            f"H{x + bl:g}" + a(bl, x, y + h - bl) +
            f"V{y + tl:g}" + a(tl, x + tl, y) + "Z")


def svg(body, size=1024, vb=1024):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
            f'viewBox="0 0 {vb} {vb}">\n{body}\n</svg>\n')
