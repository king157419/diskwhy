"""Path geometry (adapted from round2/variant-1/geom.py; adds exact offset corners).

contour(verts) builds a closed SVG path from polygon vertices, each with a corner spec:
  None                 sharp vertex
  ("arc", r)           true tangent circular fillet (any angle, convex or concave)
  ("pts", [p, ...])    explicit corner polyline from the incoming edge to the outgoing edge
                       (used for the exact inward offset of a smoothed corner)
  ("smooth", r, xi)    Figma corner smoothing (xi = 0..1) on a 90-degree vertex;
                       follows figma-squircle's getPathParamsForCorner, smoothing is
                       clamped to the available edge budget (preserveSmoothing = false).
Vertices go clockwise on screen (y down).
"""
import math


def f(v):
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def _unit(dx, dy):
    L = math.hypot(dx, dy)
    return dx / L, dy / L


def smooth_params(R, xi, budget):
    p = (1 + xi) * R
    if p > budget:
        xi = max(0.0, min(xi, budget / R - 1))
        p = min(p, budget)
    arc_measure = 90 * (1 - xi)
    arc_sec = math.sin(math.radians(arc_measure / 2)) * R * math.sqrt(2)
    alpha = (90 - arc_measure) / 2
    p34 = R * math.tan(math.radians(alpha / 2))
    beta = 45 * xi
    c = p34 * math.cos(math.radians(beta))
    d = c * math.tan(math.radians(beta))
    b = (p - arc_sec - c - d) / 3
    a = 2 * b
    return dict(a=a, b=b, c=c, d=d, p=p, arc=arc_sec, R=R, xi=xi)


def _extent(spec, theta, budget):
    """distance from vertex along each edge where the corner starts."""
    if spec is None:
        return 0.0
    if spec[0] == "arc":
        return spec[1] / math.tan(theta / 2)
    if spec[0] == "pts":
        return None
    if spec[0] == "smooth":
        return smooth_params(spec[1], spec[2], budget)["p"]
    raise ValueError(spec)


def _bez(p0, p1, p2, p3, n):
    out = []
    for i in range(1, n + 1):
        u = i / n
        a, b, c, d = (1 - u) ** 3, 3 * u * (1 - u) ** 2, 3 * u * u * (1 - u), u ** 3
        out.append((a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]))
    return out


def smooth_corner_pts(prv, v, nxt, R, xi, n=24):
    """Sample a Figma-smoothed 90-degree corner (same construction as contour) from the point
    where it leaves the incoming edge to where it joins the outgoing edge. No budget clamping."""
    P = smooth_params(R, xi, 1e9)
    a, b, c, dd, arc = P["a"], P["b"], P["c"], P["d"], P["arc"]
    uin = _unit(v[0] - prv[0], v[1] - prv[1])
    uout = _unit(nxt[0] - v[0], nxt[1] - v[1])
    s = (v[0] - uin[0] * P["p"], v[1] - uin[1] * P["p"])
    L = lambda ss, tt: (s[0] + uin[0] * ss + uout[0] * tt, s[1] + uin[1] * ss + uout[1] * tt)
    p1, p2, p3 = L(a, 0), L(a + b, 0), L(a + b + c, dd)
    q = (a + b + c + arc, dd + arc)
    p4 = L(*q)
    p5, p6, p7 = L(q[0] + dd, q[1] + c), L(q[0] + dd, q[1] + b + c), L(q[0] + dd, q[1] + a + b + c)
    pts = [s] + _bez(s, p1, p2, p3, n)
    # arc p3 -> p4, radius R, centre on the inside of the corner
    mx, my = (p3[0] + p4[0]) / 2, (p3[1] + p4[1]) / 2
    h = math.dist(p3, p4) / 2
    k = math.sqrt(max(R * R - h * h, 0))
    w = _unit(uout[0] - uin[0], uout[1] - uin[1])
    cx, cy = mx + w[0] * k, my + w[1] * k
    a0 = math.atan2(p3[1] - cy, p3[0] - cx)
    a1 = math.atan2(p4[1] - cy, p4[0] - cx)
    da = (a1 - a0 + math.pi) % (2 * math.pi) - math.pi
    m = max(2, int(n * abs(da) / (math.pi / 2)) + 1)
    pts += [(cx + R * math.cos(a0 + da * i / m), cy + R * math.sin(a0 + da * i / m)) for i in range(1, m + 1)]
    pts += _bez(p4, p5, p6, p7, n)
    return pts


def offset_pts(pts, d):
    """Offset an open clockwise-on-screen polyline by d toward the shape interior."""
    out = []
    for i, p in enumerate(pts):
        q0 = pts[max(i - 1, 0)]
        q1 = pts[min(i + 1, len(pts) - 1)]
        tx, ty = _unit(q1[0] - q0[0], q1[1] - q0[1])
        out.append((p[0] - ty * d, p[1] + tx * d))
    return out


def contour(verts):
    pts = [v[0] for v in verts]
    specs = [v[1] for v in verts]
    n = len(pts)
    info = []
    # first pass: fixed extents of non-smooth corners
    ext = [None] * n
    thetas = []
    for i in range(n):
        prv, v, nxt = pts[i - 1], pts[i], pts[(i + 1) % n]
        u1 = _unit(prv[0] - v[0], prv[1] - v[1])
        u2 = _unit(nxt[0] - v[0], nxt[1] - v[1])
        cosang = max(-1, min(1, u1[0] * u2[0] + u1[1] * u2[1]))
        theta = math.acos(cosang)
        thetas.append(theta)
        if specs[i] is None or specs[i][0] == "arc":
            ext[i] = _extent(specs[i], theta, 0)
        elif specs[i][0] == "pts":
            ext[i] = math.dist(pts[i], specs[i][1][0])
    elen = [math.dist(pts[i], pts[(i + 1) % n]) for i in range(n)]  # edge i: vertex i -> i+1
    for i in range(n):
        if ext[i] is None:  # smooth: budget = what is left on both adjacent edges
            bprev = elen[i - 1] - (ext[i - 1] or 0)
            bnext = elen[i] - (ext[(i + 1) % n] or 0)
            ext[i] = _extent(specs[i], thetas[i], min(bprev, bnext))
            info.append((i, min(bprev, bnext)))
    budgets = dict(info)
    segs = []
    starts, ends = [], []
    for i in range(n):
        prv, v, nxt = pts[i - 1], pts[i], pts[(i + 1) % n]
        uin = _unit(v[0] - prv[0], v[1] - prv[1])
        uout = _unit(nxt[0] - v[0], nxt[1] - v[1])
        e = ext[i]
        s = (v[0] - uin[0] * e, v[1] - uin[1] * e)
        t = (v[0] + uout[0] * e, v[1] + uout[1] * e)
        starts.append(s)
        ends.append(t)
    d = f"M{f(ends[0][0])},{f(ends[0][1])}"
    for k in range(1, n + 1):
        i = k % n
        prv, v, nxt = pts[i - 1], pts[i], pts[(i + 1) % n]
        s, t = starts[i], ends[i]
        d += f"L{f(s[0])},{f(s[1])}"
        spec = specs[i]
        if spec is None:
            continue
        uin = _unit(v[0] - prv[0], v[1] - prv[1])
        uout = _unit(nxt[0] - v[0], nxt[1] - v[1])
        cross = uin[0] * uout[1] - uin[1] * uout[0]
        sweep = 1 if cross > 0 else 0
        if spec[0] == "arc":
            r = spec[1]
            d += f"A{f(r)},{f(r)} 0 0 {sweep} {f(t[0])},{f(t[1])}"
        elif spec[0] == "pts":
            d += "".join(f"L{f(x)},{f(y)}" for x, y in spec[1][1:])
        else:
            P = smooth_params(spec[1], spec[2], budgets[i])
            a, b, c, dd, arc, R = P["a"], P["b"], P["c"], P["d"], P["arc"], P["R"]
            # local frame: s along uin, t along uout (outgoing is the "turn" direction)
            L = lambda ss, tt: (s[0] + uin[0] * ss + uout[0] * tt, s[1] + uin[1] * ss + uout[1] * tt)
            p1, p2, p3 = L(a, 0), L(a + b, 0), L(a + b + c, dd)
            q = (a + b + c + arc, dd + arc)
            p4 = L(*q)
            p5, p6, p7 = L(q[0] + dd, q[1] + c), L(q[0] + dd, q[1] + b + c), L(q[0] + dd, q[1] + a + b + c)
            d += (f"C{f(p1[0])},{f(p1[1])} {f(p2[0])},{f(p2[1])} {f(p3[0])},{f(p3[1])}"
                  f"A{f(R)},{f(R)} 0 0 {sweep} {f(p4[0])},{f(p4[1])}"
                  f"C{f(p5[0])},{f(p5[1])} {f(p6[0])},{f(p6[1])} {f(p7[0])},{f(p7[1])}")
    return d + "Z"


def rect(x0, y0, x1, y1, tl, tr, br, bl):
    """rect with per-corner spec (clockwise from top-left)."""
    return contour([((x0, y0), tl), ((x1, y0), tr), ((x1, y1), br), ((x0, y1), bl)])
