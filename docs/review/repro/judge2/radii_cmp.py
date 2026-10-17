import math, numpy as np
def var(r): return r*(r+1)/3.0
def inc(radius):  # incremental plan pass_radii
    target=var(radius); low=int(math.floor((-1+math.sqrt(1+4*target))/2))
    for wide in range(4):
        rs=[low+(wide>=3), low+(wide>=2), low+(wide>=1)]
        if sum(var(r) for r in rs)+1e-9>=target: return rs
    return [low+1]*3
def arch(r):  # architecture boxRadii (round to nearest), 3 equal passes
    ri=max(1, math.floor((math.sqrt(1+(4*r*(r+1))/3)-1)/2+0.5)); return [ri]*3
def report1(r):  # report 01 formula, "rounded and >=1" -> same as arch
    ri=max(1, round(((1.0+4.0*r*(r+1)/3.0)**0.5-1.0)/2.0)); return [ri]*3
def gamer(r):  # gamer three_pass_radius(sigma=sqrt(var(r))) ceil, no floor applied
    s=math.sqrt(var(r)); ri=max(1, math.ceil((math.sqrt(1+4*s*s)-1)/2)); return [ri]*3
def blur_row(row, r):
    n=len(row); c=np.concatenate([[0],np.cumsum(row)])
    idx=np.arange(n); a=np.maximum(0,idx-r); b=np.minimum(n-1,idx+r)
    return (c[b+1]-c[a])//(b-a+1)
def kept(radius, period, radii):
    w=period*40
    row=np.round(127.5+127.5*np.cos(2*np.pi*np.arange(w)/period)).astype(np.int64)
    cur=row
    for r in radii:
        if r>0: cur=blur_row(cur,r)
    mid=cur[w//4:w//4+w//2]
    return (mid.max()-mid.min())/255
print("r   inc            arch/rep01      gamer        var_pre  var_inc var_arch var_gamer")
worst={}
for name,f in [("inc",inc),("arch",arch),("gamer",gamer)]:
    worst[name]=(-9,None)
for radius in list(range(1,25))+[32,48,64,96,128,192]:
    rows=[]
    for name,f in [("inc",inc),("arch",arch),("gamer",gamer)]:
        rs=f(radius)
        for P in [3,4,5,6,8,10,12,16,20,24,32,48,64]:
            g=math.exp(-2*math.pi**2*var(radius)/P**2)
            ex=kept(radius,P,rs)-g
            if ex>worst[name][0]: worst[name]=(ex,(radius,P,rs))
    if radius in (2,4,8,16,24,48,96,192):
        print(radius, inc(radius), arch(radius), gamer(radius), round(var(radius),1), round(sum(map(var,inc(radius))),1), round(sum(map(var,arch(radius))),1), round(sum(map(var,gamer(radius))),1))
for k,v in worst.items(): print("worst excess over preview gaussian", k, round(v[0],4), v[1])
# single box baseline worst
wb=(-9,None)
for radius in [2,4,8,16,24]:
    for P in [3,4,5,6,8,10,12,16,20,24,32,48]:
        g=math.exp(-2*math.pi**2*var(radius)/P**2); ex=kept(radius,P,[radius])
        if ex>wb[0]: wb=(ex,(radius,P,g))
print("single box worst kept contrast", round(wb[0],3), wb[1])
print("--- single box excess over equal-variance gaussian")
for radius in [4,8,16]:
    best=(-9,None)
    for P10 in range(30,500):
        P=P10/10
        if abs(P-round(P))>1e-9: continue
        P=int(round(P))
        g=math.exp(-2*math.pi**2*var(radius)/P**2); k=kept(radius,P,[radius])
        if k-g>best[0]: best=(k-g,(P,round(k,3),round(g,3)))
    print(radius,best)
# arch-at-scale: worst-case per-radius excess for arch
print("--- arch per radius worst excess")
for radius in [2,3,4,5,6,8,10,12,16,24,48,96]:
    best=-9
    for P in range(3,80):
        g=math.exp(-2*math.pi**2*var(radius)/P**2); best=max(best,kept(radius,P,arch(radius))-g)
    print(radius, arch(radius), round(best,4))
