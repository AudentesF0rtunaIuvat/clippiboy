import math
def pass_radii(radius):
    target = radius*(radius+1)/3.0
    var = lambda r: r*(r+1)/3.0
    low = int(math.floor((-1+math.sqrt(1+4*target))/2))
    for wide in range(4):
        radii = [low+1 if wide>=3 else low, low+1 if wide>=2 else low, low+1 if wide>=1 else low]
        if sum(var(r) for r in radii) + 1e-9 >= target: return radii
    return [low+1]*3
def blur_row(row, r):
    # same as blur_rows: window clamped at the edges, integer division
    n=len(row); out=[0]*n
    for c in range(n):
        a=max(0,c-r); b=min(n-1,c+r)
        out[c]=sum(row[a:b+1])//(b-a+1)
    return out
worst=-1
for radius in [1,2,4,8,16,24]:
    s2 = radius*(radius+1)/3.0
    for period in [3,4,6,8,12,16,24,32,48]:
        w=period*40
        row=[round(127.5+127.5*math.cos(2*math.pi*x/period)) for x in range(w)]
        single=blur_row(row,radius)
        cur=row
        for r in pass_radii(radius):
            if r>0: cur=blur_row(cur,r)
        mid=lambda v: v[w//4:w//4+w//2]
        k1=(max(mid(single))-min(mid(single)))/255
        k3=(max(mid(cur))-min(mid(cur)))/255
        g=math.exp(-2*math.pi**2*s2/period**2)
        excess=k3-g
        worst=max(worst,excess)
        flag = "FAIL" if k3 > g+0.015+2/255 else ""
        if period in (4,6,12,24) or flag: print(f"r={radius:2d} P={period:2d} gauss={g:.3f} 1-box={k1:.3f} 3-box={k3:.3f} {flag}")
print("worst excess of 3-box over gaussian:", round(worst,4))
