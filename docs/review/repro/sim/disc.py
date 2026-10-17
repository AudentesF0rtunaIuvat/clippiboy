import numpy as np, sys
from PIL import Image, ImageDraw, ImageFont, ImageFilter

def gaussian_filter1d(a,s,axis,mode):
    R=int(4*s+0.5); k=np.exp(-0.5*(np.arange(-R,R+1)/s)**2); k/=k.sum()
    a=np.moveaxis(a,axis,-1); pad=np.pad(a,[(0,0)]*(a.ndim-1)+[(R,R)],mode='edge')
    out=np.apply_along_axis(lambda v: np.convolve(v,k,mode='valid'),-1,pad)
    return np.moveaxis(out,-1,axis)
font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf', 14)
W,H=200,30
def render(t):
    img = Image.new('L',(W,H),255); ImageDraw.Draw(img).text((10,7),t,font=font,fill=0)
    return np.asarray(img).astype(float)
def box1(a,r):
    h,w=a.shape; c=np.concatenate([np.zeros((h,1)),np.cumsum(a,1)],1)
    x=np.arange(w); lo=np.maximum(0,x-r); hi=np.minimum(w-1,x+r)
    return np.floor((c[:,hi+1]-c[:,lo])/(hi-lo+1))
def box(a,r): return box1(box1(a,r).T,r).T
def gauss(a,r):
    s=(r*(r+1)/3)**0.5
    g=gaussian_filter1d(gaussian_filter1d(a,s,axis=1,mode='nearest'),s,axis=0,mode='nearest')
    return np.round(g)
base="hunter2pass"
import string
alts=[]
for i in range(len(base)):
    for ch in "aeosnrtcx1":
        if ch!=base[i]: alts.append(base[:i]+ch+base[i+1:])
B=render(base)
for r in (4,8,16):
    for name,f in (("box",box),("gauss",gauss)):
        fb=f(B,r)
        d=[np.sqrt(((f(render(t),r)-fb)**2).sum()) for t in alts]
        d=np.array(d)
        print(r,name,"median L2 %.1f  min %.1f  frac<16(=1 level/256px): %.2f"%(np.median(d),d.min(),(d<16).mean()))
