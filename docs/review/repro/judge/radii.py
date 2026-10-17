import math
def var(q): return q*(q+1)/3.0
def inc(r):
    t=var(r); low=int(math.floor((-1+math.sqrt(1+4*t))/2))
    for k in range(4):
        rs=[low+(1 if k>=3 else 0), low+(1 if k>=2 else 0), low+(1 if k>=1 else 0)]
        if sum(var(q) for q in rs)+1e-9>=t: return rs
    return [low+1]*3
def arch(r):
    q=max(1, math.floor((math.sqrt(1+(4*r*(r+1))/3)-1)/2+0.5)); return [q]*3
def gamer(r):
    s2=var(r); q=max(1, math.ceil((math.sqrt(1+4*s2)-1)/2)); return [q]*3
def box(q,f):
    w=2*q+1; s=math.sin(math.pi*f)
    return abs(math.sin(w*math.pi*f)/(w*s)) if s else 1.0
def g(s2,f): return math.exp(-2*math.pi**2*s2*f*f)
for name,fn in [("incremental",inc),("architecture",arch),("gamer",gamer)]:
    worst=(0,None,None); varshort=0
    for r in range(1,201):
        rs=fn(r); s2=var(r)
        if sum(var(q) for q in rs) < s2-1e-9: varshort+=1
        for i in range(300,6400):
            P=i/100; f=1/P
            ex=math.prod(box(q,f) for q in rs)-g(s2,f)
            if ex>worst[0]: worst=(ex,r,P)
    print(f"{name:13s} radii(8)={fn(8)} radii(4)={fn(4)} radii(2)={fn(2)} var<preview for {varshort}/200 radii; worst MTF excess {worst[0]*100:.2f} pp at r={worst[1]} P={worst[2]}")
