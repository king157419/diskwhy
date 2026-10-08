"""Small sizes (16 / 24 / 32) for variant 2 — hand-drawn pixel maps.

Every pixel below is placed by hand. Rule for what goes away as the icon shrinks:

  size  sticker  ink rim  gutter  inner-corner AA   hard shadow   tail stairs (2 px left per 1 px down)
  32    1 px     2 px     2 px    1 px blend         1 px (auto)   ink 6 -> 4 -> 2
  24    1 px     2 px     2 px    1 px blend         none          ink 4 -> 2
  16    1 px     1 px     1 px    outer blocks only  none          ink 3 -> 2(+tip)

The tail slope is 1:2 (26.6 deg) at every size, the same as the 1024 master (D : B = 8 : 16).
The 32 px shadow is not drawn by hand: it is the hand-drawn silhouette shifted (+1, +1),
exactly the rule the master uses (sticker shape shifted right/down), so it can never disagree.

Palette:
  .  clear          w  sticker white     W  white 50 % alpha     a  white 25 % alpha
  k  ink            x  ink/white 50:50   z  ink/white 75:25
  l  lime  p  paper  c  coral            L / P / C  block colour / ink 50:50 (inner corner blend)
"""

INK = (0x14, 0x14, 0x12)
LIME = (0xD4, 0xF2, 0x4A)
PAPER = (0xF6, 0xF1, 0xE4)
CORAL = (0xFF, 0x8A, 0x73)
WHITE = (0xFF, 0xFF, 0xFF)


def _mix(a, b, t):
    return tuple(round(a[i] * (1 - t) + b[i] * t) for i in range(3))


PAL = {
    ".": None,
    "w": WHITE + (255,),
    "W": WHITE + (128,),
    "a": WHITE + (64,),
    "k": INK + (255,),
    "x": _mix(WHITE, INK, 0.5) + (255,),
    "z": _mix(WHITE, INK, 0.75) + (255,),
    "l": LIME + (255,),
    "p": PAPER + (255,),
    "c": CORAL + (255,),
    "L": _mix(LIME, INK, 0.5) + (255,),
    "P": _mix(PAPER, INK, 0.5) + (255,),
    "C": _mix(CORAL, INK, 0.5) + (255,),
}

MAP16 = [
    ".aWwwwwwwwwwwWa.",
    "awxkkkkkkkkkkxwa",
    "WxklllllkppppkxW",
    "wkllllllkpppppkw",
    "wkllllllkpppppkw",
    "wkllllllkpppppkw",
    "wkllllllkkkkkkkw",
    "wkllllllkccccckw",
    "wkllllllkccccckw",
    "wkllllllkccccckw",
    "wkllllllkccccckw",
    "wkllllllkcccckxW",
    "wklllkkkkkkkkxwa",
    "wklkkwwwwwwwwWa.",
    "wkkwW...........",
    "WwW.............",
]

MAP24 = [
    "........................",
    "..aWwwwwwwwwwwwwwwwwWa..",
    ".awxkkkkkkkkkkkkkkkkxwa.",
    "awzkkkkkkkkkkkkkkkkkkzwa",
    "WxkklllllllllkkpppppkkxW",
    "wkkllllllllllkkppppppkkw",
    "wkkllllllllllkkppppppkkw",
    "wkkllllllllllkkppppppkkw",
    "wkkllllllllllkkppppppkkw",
    "wkkllllllllllkkppppppkkw",
    "wkkllllllllllkkkkkkkkkkw",
    "wkkllllllllllkkkkkkkkkkw",
    "wkkllllllllllkkcccccckkw",
    "wkkllllllllllkkcccccckkw",
    "wkkllllllllllkkcccccckkw",
    "wkkllllllllllkkcccccckkw",
    "wkkllllllllllkkcccccckkw",
    "wkkllllllllllkkcccccckkw",
    "wkkllllllllllkkccccckkxW",
    "wkklllkkkkkkkkkkkkkkkzwa",
    "wkklkkkkkkkkkkkkkkkkxwa.",
    "wkkkkwwwwwwwwwwwwwwwWa..",
    "wkkwwa..................",
    "WwwWa...................",
]

MAP32 = [
    "................................",
    "................................",
    "....WWwwwwwwwwwwwwwwwwwwwWW.....",
    "..WwxzkkkkkkkkkkkkkkkkkkkzxwW...",
    ".WwzkkkkkkkkkkkkkkkkkkkkkkkzwW..",
    ".wzkkklllllllllllkkppppppkkkzw..",
    "WxkklllllllllllllkkppppppppkkxW.",
    "WzkllllllllllllllkkpppppppppkzW.",
    "wkkllllllllllllllkkpppppppppkkw.",
    "wkkllllllllllllllkkpppppppppkkw.",
    "wkkllllllllllllllkkpppppppppkkw.",
    "wkkllllllllllllllkkpppppppppkkw.",
    "wkkllllllllllllllkkpppppppppkkw.",
    "wkkllllllllllllllkkkkkkkkkkkkkw.",
    "wkkllllllllllllllkkkkkkkkkkkkkw.",
    "wkkllllllllllllllkkccccccccckkw.",
    "wkkllllllllllllllkkccccccccckkw.",
    "wkkllllllllllllllkkccccccccckkw.",
    "wkkllllllllllllllkkccccccccckkw.",
    "wkkllllllllllllllkkccccccccckkw.",
    "wkkllllllllllllllkkccccccccckkw.",
    "wkkllllllllllllllkkccccccccckkw.",
    "wkkllllllllllllllkkccccccccckzW.",
    "wkkllllllllllllllkkcccccccckkxW.",
    "wkkllllllllllllllkkcccccckkkzw..",
    "wkkllllkkkkkkkkkkkkkkkkkkkkzwW..",
    "wkkllkkkkkkkkkkkkkkkkkkkkzxwW...",
    "wkkkkkkwwwwwwwwwwwwwwwwwwWW.....",
    "wkkkkww.........................",
    "wkkww...........................",
    "Wwwa............................",
    "................................",
]


def to_rgba(rows):
    n = len(rows)
    assert len(rows) == n
    for r in rows:
        assert len(r) == n, (n, r, len(r))
    return [[PAL[ch] for ch in row] for row in rows]


def add_shadow(img, dx=1, dy=1):
    """Hard shadow = the silhouette's alpha shifted (dx, dy), ink, composited underneath."""
    n = len(img)
    out = [row[:] for row in img]
    for y in range(n):
        for x in range(n):
            sx, sy = x - dx, y - dy
            if not (0 <= sx < n and 0 <= sy < n):
                continue
            src = img[sy][sx]
            if src is None:
                continue
            sa = src[3] / 255
            dst = out[y][x]
            if dst is None:
                out[y][x] = INK + (round(sa * 255),)
            elif dst[3] < 255:
                da = dst[3] / 255
                a = da + sa * (1 - da)
                rgb = tuple(round((dst[i] * da + INK[i] * sa * (1 - da)) / a) for i in range(3))
                out[y][x] = rgb + (round(a * 255),)
    return out


SMALL = {
    16: to_rgba(MAP16),
    24: to_rgba(MAP24),
    32: add_shadow(to_rgba(MAP32)),
}


def pixel_svg(img, inner_only=False):
    n = len(img)
    rects = []
    for y, row in enumerate(img):
        x = 0
        while x < n:
            c = row[x]
            if c is None:
                x += 1
                continue
            x2 = x
            while x2 + 1 < n and row[x2 + 1] == c:
                x2 += 1
            hexc = "#%02x%02x%02x" % c[:3]
            op = "" if c[3] == 255 else f' fill-opacity="{c[3] / 255:.3f}"'
            rects.append(f'<rect x="{x}" y="{y}" width="{x2 - x + 1}" height="1" fill="{hexc}"{op}/>')
            x = x2 + 1
    body = "".join(rects)
    if inner_only:
        return body
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{n}" height="{n}" viewBox="0 0 {n} {n}" '
        f'shape-rendering="crispEdges">{body}</svg>\n'
    )
