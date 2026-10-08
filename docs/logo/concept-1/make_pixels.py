"""Builds the hand-specified 16px / 24px grids (mark-16.txt, mark-24.txt). Edit the specs here, then run pixel2svg.py."""
def build(N, lid_segs, lid_th, lid_x, body, shadow, eyes, mouth, clear, mono=False):
    g=[['.']*N for _ in range(N)]
    def put(x,y,c):
        if 0<=x<N and 0<=y<N: g[y][x]=c
    bx0,by0,bx1,by1=body
    for y in range(by0+shadow,by1+shadow+1):
        for x in range(bx0+shadow,bx1+shadow+1): put(x,y,'K')
    for y in range(by0,by1+1):
        for x in range(bx0,bx1+1):
            put(x,y,'K' if x in (bx0,bx1) or y in (by0,by1) else 'L')
    for x0,x1,t in lid_segs:
        for x in range(x0,x1+1):
            for k in range(lid_th):
                put(x,t+k,'K' if k in (0,lid_th-1) else 'L')
    l0,l1=lid_x
    t0=lid_segs[0][2]; t1=lid_segs[-1][2]
    for k in range(lid_th): put(l0,t0+k,'K'); put(l1,t1+k,'K')
    feat = '.' if mono else 'K'
    for (x,y) in eyes: put(x,y,feat)
    for (x,y) in mouth: put(x,y,feat)
    for (x,y) in clear: put(x,y,'.')
    if mono: g=[['M' if c in 'KL' else c for c in r] for r in g]
    return ["".join(r) for r in g]
# critic pass: lid = one thick bar with a single kink (2 px lime) instead of 3 thin steps,
# which read as a film clapperboard at 16/24; 1 px gap now separates lid from body.
g16=build(16,[(1,7,2),(8,14,1)],4,(1,14),(1,7,13,13),1,
          [(5,9),(5,10),(9,9),(9,10)],[],[(14,1),(1,5),(1,7),(13,7),(1,13)])
g24=build(24,[(2,11,4),(12,21,2)],5,(2,21),(2,10,20,20),1,
          [(x,y) for x in (8,9,13,14) for y in (13,14,15)],[(11,17),(11,18)],
          [(21,2),(2,8),(2,10),(20,10),(2,20)])
for (x,y) in ((11,3),(12,7)):  # close the outline at the lid's kink
    g24[y]=g24[y][:x]+'K'+g24[y][x+1:]
m16=build(16,[(1,7,3),(8,14,2)],3,(1,14),(1,7,13,13),0,
          [(5,9),(5,10),(9,9),(9,10)],[],[(14,2),(1,5),(1,7),(13,7),(1,13),(13,13)],mono=True)
for name,g in (("mark-16.txt",g16),("mark-24.txt",g24),("mark-16-mono.txt",m16)):
    open(name,"w").write(f"# {name} hand-pixelled variant (from make_pixels.py)\n"+"\n".join(g)+"\n")
    print("\n".join(g)); print()
