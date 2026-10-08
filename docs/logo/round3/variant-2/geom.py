"""Vector geometry for round-3 variant 2 (贴纸版).

One clockwise polygon (screen coords, y down) with a corner spec per vertex.
Corner specs:
    ("sq", r, p)  continuous-curvature (Figma corner-smoothing) corner, 90deg only;
                  r = radius of the middle arc, p = how far the curve reaches along each edge.
    ("arc", r)    plain tangent circular fillet (any angle, convex or concave)
    None          sharp
offset(poly, d) moves every edge d inward (d<0 = outward) and adjusts the corner specs so the
result is the exact parallel curve for arcs and the matching squircle for "sq" corners
(r-d, p-d), i.e. every nested shape is concentric with the one around it.
"""
import math


def fmt(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def _sub(a, b):
    return (a[0] - b[0], a[1] - b[1])


def _unit(v):
    L = math.hypot(*v)
    return (v[0] / L, v[1] / L)


def _toward(p, q, d):
    u = _unit(_sub(q, p))
    return (p[0] + u[0] * d, p[1] + u[1] * d)


def _convex(prv, v, nxt):
    return (v[0] - prv[0]) * (nxt[1] - v[1]) - (v[1] - prv[1]) * (nxt[0] - v[0]) > 0


def _angle(prv, v, nxt):
    u1, u2 = _unit(_sub(prv, v)), _unit(_sub(nxt, v))
    return math.acos(max(-1.0, min(1.0, u1[0] * u2[0] + u1[1] * u2[1])))


def squircle_params(r, p):
    """Figma corner smoothing (figma-squircle) for a 90deg corner, given r and reach p (>= r)."""
    xi = p / r - 1.0
    assert -1e-9 <= xi <= 1.0 + 1e-9, (r, p, xi)
    arc_measure = 90 * (1 - xi)
    arc_len = math.sin(math.radians(arc_measure / 2)) * r * math.sqrt(2)
    alpha = (90 - arc_measure) / 2
    p3p4 = r * math.tan(math.radians(alpha / 2))
    beta = 45 * xi
    c = p3p4 * math.cos(math.radians(beta))
    d = c * math.tan(math.radians(beta))
    b = (p - arc_len - c - d) / 3
    a = 2 * b
    return a, b, c, d, arc_len


def path(verts, specs):
    """SVG path data for the closed polygon with rounded corners."""
    n = len(verts)
    pts = []  # per vertex: (entry_point, curve_cmd)
    for i in range(n):
        prv, v, nxt = verts[i - 1], verts[i], verts[(i + 1) % n]
        sp = specs[i]
        if sp is None:
            pts.append((v, ""))
            continue
        if sp[0] == "sq":
            _, r, p = sp
            e1 = _unit(_sub(v, prv))  # incoming direction
            e2 = _unit(_sub(nxt, v))  # outgoing direction
            S = (v[0] - e1[0] * p, v[1] - e1[1] * p)
            a, b, c, d, L = squircle_params(r, p)
            loc = lambda u, w, o=S: (o[0] + e1[0] * u + e2[0] * w, o[1] + e1[1] * u + e2[1] * w)
            c1, c2, P1 = loc(a, 0), loc(a + b, 0), loc(a + b + c, d)
            P2 = loc(a + b + c + L, d + L)
            c3 = (P2[0] + e1[0] * d + e2[0] * c, P2[1] + e1[1] * d + e2[1] * c)
            c4 = (P2[0] + e1[0] * d + e2[0] * (b + c), P2[1] + e1[1] * d + e2[1] * (b + c))
            E = (P2[0] + e1[0] * d + e2[0] * (a + b + c), P2[1] + e1[1] * d + e2[1] * (a + b + c))
            sweep = 1 if _convex(prv, v, nxt) else 0
            cmd = (
                f"C{fmt(c1[0])},{fmt(c1[1])} {fmt(c2[0])},{fmt(c2[1])} {fmt(P1[0])},{fmt(P1[1])}"
                f"A{fmt(r)},{fmt(r)} 0 0 {sweep} {fmt(P2[0])},{fmt(P2[1])}"
                f"C{fmt(c3[0])},{fmt(c3[1])} {fmt(c4[0])},{fmt(c4[1])} {fmt(E[0])},{fmt(E[1])}"
            )
            pts.append((S, cmd))
        else:
            r = sp[1]
            th = _angle(prv, v, nxt)
            dd = r / math.tan(th / 2)
            pa, pb = _toward(v, prv, dd), _toward(v, nxt, dd)
            sweep = 1 if _convex(prv, v, nxt) else 0
            pts.append((pa, f"A{fmt(r)},{fmt(r)} 0 0 {sweep} {fmt(pb[0])},{fmt(pb[1])}"))
    # start at vertex 0's entry point
    out = f"M{fmt(pts[0][0][0])},{fmt(pts[0][0][1])}" + pts[0][1]
    for i in range(1, n):
        e, cmd = pts[i]
        out += f"L{fmt(e[0])},{fmt(e[1])}" + cmd
    return out + "Z"


def _offset_line(p, q, d):
    dx, dy = q[0] - p[0], q[1] - p[1]
    L = math.hypot(dx, dy)
    nx, ny = -dy / L, dx / L  # inward for clockwise-on-screen
    return (p[0] + nx * d, p[1] + ny * d), (q[0] + nx * d, q[1] + ny * d)


def _intersect(l1, l2):
    (x1, y1), (x2, y2) = l1
    (x3, y3), (x4, y4) = l2
    den = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4)
    px = ((x1 * y2 - y1 * x2) * (x3 - x4) - (x1 - x2) * (x3 * y4 - y3 * x4)) / den
    py = ((x1 * y2 - y1 * x2) * (y3 - y4) - (y1 - y2) * (x3 * y4 - y3 * x4)) / den
    return (px, py)


def offset(verts, specs, d, min_r=0.0):
    n = len(verts)
    lines = [_offset_line(verts[i], verts[(i + 1) % n], d) for i in range(n)]
    nv = [_intersect(lines[i - 1], lines[i]) for i in range(n)]
    ns = []
    for i in range(n):
        sp = specs[i]
        if sp is None:
            ns.append(None)
            continue
        conv = _convex(verts[i - 1], verts[i], verts[(i + 1) % n])
        if sp[0] == "sq":
            ns.append(("sq", sp[1] - d, sp[2] - d))
        else:
            r = sp[1] - d if conv else sp[1] + d
            ns.append(("arc", r) if r > min_r else (("arc", min_r) if min_r else None))
    return nv, ns


def reach(verts, specs, i):
    """Distance along each edge that corner i consumes."""
    sp = specs[i]
    if sp is None:
        return 0.0
    if sp[0] == "sq":
        return sp[2]
    th = _angle(verts[i - 1], verts[i], verts[(i + 1) % len(verts)])
    return sp[1] / math.tan(th / 2)
