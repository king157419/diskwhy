"""Composite small rasters x10 on light/dark grounds next to concept-2 originals -> wip/pix-<tag>.png"""
import sys
from pathlib import Path
from PIL import Image
H = Path(__file__).resolve().parent; C2 = H.parents[1] / "concept-2" / "png"
items = [(C2/"icon-32@32.png","o32"),(H/"png/icon-32.png","32"),(C2/"icon-24@24.png","o24"),(H/"png/icon-24.png","24"),
         (H/"wip/icon-24-alt.png","24b"),(C2/"icon-16@16.png","o16"),(H/"png/icon-16.png","16"),(H/"wip/_mark16.png","t16")]
k=8; pad=8
cols=[Image.open(p).convert("RGBA") for p,_ in items if p.exists()]
W=sum(32*k+pad for _ in cols)+pad; out=Image.new("RGB",(W,2*(32*k+pad)+pad),(200,200,200))
x=pad
for im in cols:
    for row,bg in enumerate([(243,243,243),(32,32,32)]):
        tile=Image.new("RGBA",(32,32),bg+(255,)); tile.alpha_composite(im,(0,0))
        out.paste(tile.convert("RGB").resize((32*k,32*k),Image.NEAREST),(x,pad+row*(32*k+pad)))
    x+=32*k+pad
out.save(H/"wip"/f"pix-{sys.argv[1] if len(sys.argv)>1 else 'cur'}.png")
