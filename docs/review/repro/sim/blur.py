import numpy as np
from PIL import Image, ImageDraw, ImageFont
import math, sys
OUT='<review-scratch>/sim/'
font=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf',14)
def render(s,W=220,H=40):
    im=Image.new('L',(W,H),255); d=ImageDraw.Draw(im); d.text((20,12),s,fill=0,font=font)
    return np.asarray(im).astype(np.float64)
def box_rows(a,r):
    # same as blur_rows: truncated window at edges, integer floor
    h,w=a.shape; out=np.zeros_like(a)
    c=np.cumsum(np.pad(a,((0,0),(1,0))),axis=1)
    for x in range(w):
        f=max(0,x-r); l=min(w-1,x+r)
        out[:,x]=np.floor((c[:,l+1]-c[:,f])/(l-f+1))
    return out
def box(a,r): return box_rows(box_rows(a,r).T,r).T
def gauss1d(s):
    k=int(math.ceil(4*s)); x=np.arange(-k,k+1); g=np.exp(-x*x/(2*s*s)); return g/g.sum()
def conv_rows(a,k):
    r=len(k)//2; p=np.pad(a,((0,0),(r,r)),mode='reflect'); out=np.zeros_like(a)
    for i,v in enumerate(k): out+=v*p[:,i:i+a.shape[1]]
    return out
def gauss(a,s):
    k=gauss1d(s); return np.round(conv_rows(conv_rows(a,k).T,k).T)
def H_box(f,r):
    N=2*r+1; return np.abs(np.sin(np.pi*f*N)/(N*np.sin(np.pi*f)))
def H_g(f,s): return np.exp(-2*np.pi**2*s*s*f*f)
for r in (4,8,16):
    s=math.sqrt(r*(r+1)/3); N=2*r+1
    f=1.4303/N
    print(f"r={r} sigma={s:.2f} first sidelobe period={1/f:.1f}px box={H_box(f,r):.3f} gauss={H_g(f,s):.4f}")
    for per in (3,4,6,8):
        f=1/per; print(f"   period {per}px: box={H_box(f,r):.3f} gauss={H_g(f,s):.2e}")
words=["hunter2@mail.com","hunter3@mail.com","hanter2@mail.com","Kx9pQ2vLm7","Kx9pQ2vLm1"]
imgs={w:render(w) for w in words}
for r in (4,8,16):
    s=math.sqrt(r*(r+1)/3)
    for a,b in [(0,1),(0,2),(3,4)]:
        A=imgs[words[a]];B=imgs[words[b]]
        db=np.abs(box(A,r)-box(B,r)); dg=np.abs(gauss(A,s)-gauss(B,s))
        print(f"r={r} {words[a]} vs {words[b]}: box maxdiff={db.max():.0f} sumdiff={db.sum():.0f}  gauss maxdiff={dg.max():.0f} sumdiff={dg.sum():.0f}")
rows=[]
for w in words[:1]+words[3:4]:
    A=imgs[w]; rows.append(A)
    for r in (4,8):
        s=math.sqrt(r*(r+1)/3); rows.append(box(A,r)); rows.append(gauss(A,s))
big=np.vstack(rows).clip(0,255).astype(np.uint8)
Image.fromarray(big).resize((big.shape[1]*3,big.shape[0]*3),Image.NEAREST).save(OUT+'cmp.png')
# contrast stretched version to show what an attacker sees with levels adjust
rows2=[]
for w in words[:1]+words[3:4]:
    A=imgs[w]
    for r in (4,8):
        s=math.sqrt(r*(r+1)/3)
        for X in (box(A,r),gauss(A,s)):
            # high-pass: subtract heavy blur, stretch
            hp=X-gauss(X,6); hp=(hp-hp.min())/(hp.max()-hp.min()+1e-9)*255
            rows2.append(hp)
big2=np.vstack(rows2).astype(np.uint8)
Image.fromarray(big2).resize((big2.shape[1]*3,big2.shape[0]*3),Image.NEAREST).save(OUT+'hp.png')
