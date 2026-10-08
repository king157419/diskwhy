"""Shared path geometry for variant 3 (Layered Plate).

- `smooth_corner`: Figma-style corner smoothing (the figma-squircle construction:
  cubic -> circular arc -> cubic), used for the R0 plate corners and the concentric
  brick corners that sit inside them.
- `poly`: a closed polygon (clockwise on screen) whose vertices each carry a corner spec:
  0 (sharp), ("arc", r) (tangent circular fillet, any angle, convex or concave) or
  ("sq", r, smoothing) (90-degree convex corner with Figma smoothing).
"""
import math


def f(v):
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def _unit(p, q):
    dx, dy = q[0] - p[0], q[1] - p[1]
    L = math.hypot(dx, dy)
    return dx / L, dy / L


def sq_params(R, s):
    """figma-squircle getPathParamsForCorner (preserveSmoothing, unlimited budget)."""
    p = (1 + s) * R
    arc_measure = 90 * (1 - s)
    arc = math.sin(math.radians(arc_measure / 2)) * R * math.sqrt(2)
    alpha = (90 - arc_measure) / 2
    p3p4 = R * math.tan(math.radians(alpha / 2))
    beta = 45 * s
    c = p3p4 * math.cos(math.radians(beta))
    d = c * math.tan(math.radians(beta))
    b = (p - arc - c - d) / 3
    a = 2 * b
    return dict(p=p, a=a, b=b, c=c, d=d, arc=arc, R=R)


def poly(pts, corners):
    n = len(pts)
    segs = []  # per vertex: (entry_point, path_from_entry_to_exit, exit_point)
    for i in range(n):
        prv, v, nxt = pts[i - 1], pts[i], pts[(i + 1) % n]
        spec = corners[i]
        u = _unit(prv, v)   # incoming direction
        w = _unit(v, nxt)   # outgoing direction
        if not spec:
            segs.append((v, "", v))
            continue
        kind = spec[0]
        if kind == "sq":
            _, R, s = spec
            P = sq_params(R, s)
            S = (v[0] - u[0] * P["p"], v[1] - u[1] * P["p"])
            E = (v[0] + w[0] * P["p"], v[1] + w[1] * P["p"])
            a, b, c, d, arc = P["a"], P["b"], P["c"], P["d"], P["arc"]
            pt = lambda base, du, dw: (base[0] + u[0] * du + w[0] * dw, base[1] + u[1] * du + w[1] * dw)
            p1, p2, p3 = pt(S, a, 0), pt(S, a + b, 0), pt(S, a + b + c, d)
            p4 = pt(p3, arc, arc)
            q1, q2, q3 = pt(p4, d, c), pt(p4, d, b + c), pt(p4, d, a + b + c)
            cross = u[0] * w[1] - u[1] * w[0]
            sweep = 1 if cross > 0 else 0
            path = (f"C{f(p1[0])},{f(p1[1])} {f(p2[0])},{f(p2[1])} {f(p3[0])},{f(p3[1])}"
                    + (f"A{f(R)},{f(R)} 0 0 {sweep} {f(p4[0])},{f(p4[1])}" if arc > 1e-6 else "")
                    + f"C{f(q1[0])},{f(q1[1])} {f(q2[0])},{f(q2[1])} {f(q3[0])},{f(q3[1])}")
            segs.append((S, path, E))
        else:  # arc fillet
            r = spec[1]
            ux, uy = -u[0], -u[1]  # toward previous
            cosang = ux * w[0] + uy * w[1]
            theta = math.acos(max(-1, min(1, cosang)))
            dd = r / math.tan(theta / 2)
            S = (v[0] + ux * dd, v[1] + uy * dd)
            E = (v[0] + w[0] * dd, v[1] + w[1] * dd)
            cross = u[0] * w[1] - u[1] * w[0]
            sweep = 1 if cross > 0 else 0
            segs.append((S, f"A{f(r)},{f(r)} 0 0 {sweep} {f(E[0])},{f(E[1])}", E))
    d = f"M{f(segs[0][2][0])},{f(segs[0][2][1])}"
    for k in range(1, n + 1):
        S, path, E = segs[k % n]
        d += f"L{f(S[0])},{f(S[1])}" + path
    return d + "Z"


def rect(x0, y0, x1, y1, corners):
    """corners: (tl, tr, br, bl) specs as in poly()."""
    return poly([(x0, y0), (x1, y0), (x1, y1), (x0, y1)], list(corners))
