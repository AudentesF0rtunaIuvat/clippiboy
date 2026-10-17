import numpy as np, math
from fractions import Fraction
PW,PH=535,152
def rround32(x):  # x is np.float32 positive; round half away from zero exactly
    f=Fraction(float(x))
    fl=math.floor(f)
    return fl+1 if f-fl>=Fraction(1,2) else fl
def rust_rect(fw,fh):
    h=rround32(np.float32(fh)*np.float32(0.07))
    if h<14: return None
    w=h*PW//PH
    m=rround32(np.float32(fh)*np.float32(0.015))
    if w+2*m > fw//2: return None
    return (m,fh-m-h,w,h)
def rust_kept(fw,fh):
    r=rust_rect(fw,fh)
    if r: return r
    m=rround32(np.float32(min(fw,fh))*np.float32(0.015))
    h=max(rround32(np.float32(fh)*np.float32(0.07)),14)
    w=h*PW//PH
    rw=max(fw-2*m,0); rh=max(fh-2*m,0)
    if w>rw: w=rw; h=w*PH//PW
    if h>rh: h=rh; w=h*PW//PH
    if w==0 or h==0: return None
    return (m,fh-m-h,w,h)
def jsround(x): return math.floor(x+0.5)
def ts(fw,fh):
    m=jsround(fh*0.015)
    wf=lambda h: math.floor(h*PW/PH); hf=lambda w: math.floor(w*PH/PW)
    h=jsround(fh*0.07); w=wf(h)
    if h>=14 and w+2*m<=math.floor(fw/2): return (m,fh-m-h,w,h)
    km=jsround(min(fw,fh)*0.015)
    h=max(h,14); w=wf(h)
    rw=max(fw-2*km,0); rh=max(fh-2*km,0)
    if w>rw: w=rw; h=hf(w)
    if h>rh: h=rh; w=wf(h)
    if w==0 or h==0: return None
    return (km,fh-km-h,w,h)
mis=0; n=0
import itertools
ws=list(range(1,400))+list(range(400,8000,7))
hs=list(range(1,400))+list(range(400,8000,7))
for fw in ws:
    for fh in hs:
        n+=1
        a=rust_kept(fw,fh); b=ts(fw,fh)
        if a!=b:
            mis+=1
            if mis<10: print(fw,fh,a,b)
print(n,mis)
