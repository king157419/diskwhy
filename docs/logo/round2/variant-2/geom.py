"""Shared geometry for variant-2 (Grown Tail).

Paths are built from a clockwise-on-screen polygon where every vertex carries a corner spec:
  0                 sharp
  ("arc", r)        tangent circular fillet of radius r (any angle, convex or concave)
  ("smooth", r, s)  Figma-style corner smoothing (s = 0..1, 0.6 ~ iOS) for 90-degree convex corners
All numbers are in canvas pixels.
"""
import math


def f(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def _unit(p, q):
    dx, dy = q[0] - p[0], q[1] - p[1]
    L = math.hypot(dx, dy)
    return dx / L, dy / L


def _smooth_params(R, s):
    """Figma corner-smoothing parameters (figma-squircle getPathParamsForCorner, preserve budget)."""
    p = (1 + s) * R
    arc_measure = 90 * (1 - s)
    arc_len = math.sin(math.radians(arc_measure / 2)) * R * math.sqrt(2)
    alpha = (90 - arc_measure) / 2
    p3p4 = R * math.tan(math.radians(alpha / 2))
    beta = 45 * s
    c = p3p4 * math.cos(math.radians(beta))
    d = c * math.tan(math.radians(beta))
    b = (p - arc_len - c - d) / 3
    a = 2 * b
    return p, a, b, c, d, arc_len


def path(pts, corners):
    n = len(pts)
    segs = []  # per vertex: (entry_point, svg_cmds_from_entry_to_exit, exit_point)
    for i in range(n):
        prv, v, nxt = pts[i - 1], pts[i], pts[(i + 1) % n]
        spec = corners[i]
        if not spec:
            segs.append((v, "", v))
            continue
        u = _unit(prv, v)  # incoming direction
        w = _unit(v, nxt)  # outgoing direction
        cross = u[0] * w[1] - u[1] * w[0]
        if spec[0] == "smooth":
            R, s = spec[1], spec[2]
            p, a, b, c, d, L = _smooth_params(R, s)
            S = (v[0] - u[0] * p, v[1] - u[1] * p)
            mp = lambda x, y: (S[0] + u[0] * x + w[0] * y, S[1] + u[1] * x + w[1] * y)
            sweep = 1 if cross > 0 else 0
            c1, c2, e1 = mp(a, 0), mp(a + b, 0), mp(a + b + c, d)
            e2 = mp(a + b + c + L, d + L)
            c3, c4, e3 = mp(a + b + c + L + d, d + L + c), mp(a + b + c + L + d, d + L + b + c), mp(p, p)
            cmd = (f"C{f(c1[0])},{f(c1[1])} {f(c2[0])},{f(c2[1])} {f(e1[0])},{f(e1[1])}"
                   f"A{f(R)},{f(R)} 0 0 {sweep} {f(e2[0])},{f(e2[1])}"
                   f"C{f(c3[0])},{f(c3[1])} {f(c4[0])},{f(c4[1])} {f(e3[0])},{f(e3[1])}")
            segs.append((S, cmd, e3))
        else:
            r = spec[1]
            u1 = (-u[0], -u[1])
            cosang = u1[0] * w[0] + u1[1] * w[1]
            theta = math.acos(max(-1, min(1, cosang)))
            dd = r / math.tan(theta / 2)
            pa = (v[0] - u[0] * dd, v[1] - u[1] * dd)
            pb = (v[0] + w[0] * dd, v[1] + w[1] * dd)
            sweep = 1 if cross > 0 else 0
            segs.append((pa, f"A{f(r)},{f(r)} 0 0 {sweep} {f(pb[0])},{f(pb[1])}", pb))
    d = f"M{f(segs[0][2][0])},{f(segs[0][2][1])}"
    for k in range(1, n + 1):
        ent, cmd, ex = segs[k % n]
        d += f"L{f(ent[0])},{f(ent[1])}" + cmd
    return d + "Z"


def rect(x0, y0, x1, y1, tl, tr, br, bl):
    return path([(x0, y0), (x1, y0), (x1, y1), (x0, y1)], [tl, tr, br, bl])
