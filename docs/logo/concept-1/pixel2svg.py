"""Turn a hand-drawn character grid into a crisp pixel SVG (K=ink, L=lime, W=white, M=currentColor, .=clear)."""
import sys
COL = {"K": "#141412", "L": "#d4f24a", "W": "#ffffff", "M": "currentColor"}
def grid_to_svg(rows):
    n = len(rows); assert all(len(r) == n for r in rows), [len(r) for r in rows]
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n} {n}" shape-rendering="crispEdges">']
    for y, r in enumerate(rows):
        x = 0
        while x < n:
            ch = r[x]
            if ch in COL:
                x2 = x
                while x2 < n and r[x2] == ch: x2 += 1
                out.append(f'<rect x="{x}" y="{y}" width="{x2-x}" height="1" fill="{COL[ch]}"/>')
                x = x2
            else: x += 1
    out.append("</svg>")
    return "\n".join(out)
if __name__ == "__main__":
    src, dst = sys.argv[1], sys.argv[2]
    rows = [l.rstrip("\n") for l in open(src, encoding="utf-8") if l.strip() and not l.startswith("#")]
    open(dst, "w", encoding="utf-8").write(grid_to_svg(rows))
