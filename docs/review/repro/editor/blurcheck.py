import math
def box(r, f):
    w = 2*r+1
    if r == 0: return 1.0
    s = math.sin(math.pi*f)
    return abs(math.sin(w*math.pi*f)/(w*s)) if s else 1.0
def radii(r):
    s2 = r*(r+1)/3.0               # preview variance (sigmaOf^2)
    rl = int(math.floor((-1+math.sqrt(1+4*s2))/2))
    var = lambda q: q*(q+1)/3.0
    for k in range(4):
        rs = [rl]*(3-k)+[rl+1]*k
        if sum(var(q) for q in rs) >= s2 - 1e-9: return rs, s2
def gauss(s2, f): return math.exp(-2*math.pi**2*s2*f*f)
for r in [2,4,8,16,24,48,96]:
    rs, s2 = radii(r)
    worst1=worst3=worstg=0; p1=p3=0
    # periods from 3 px to 64 px: text strokes / letters
    for i in range(300,6400):
        P = i/100; f = 1/P
        b1 = box(r,f); b3 = math.prod(box(q,f) for q in rs); g = gauss(s2,f)
        # only beyond the main lobe matters (P < 2r+1 for single box)
        if P < 2*r+1:
            if b1>worst1: worst1,p1=b1,P
            if b3-g>worst3: worst3,p3=b3-g,P
    print(f"r={r:3d} sigma={math.sqrt(s2):6.2f} passes={rs} margin={sum(rs)}  single-box max residual={worst1*100:5.1f}% @P={p1:5.1f}px   3-pass max excess over gaussian={worst3*100:5.2f}%")
