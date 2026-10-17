import sys, numpy as np
from PIL import Image
f=np.float32
img=np.array(Image.open(sys.argv[1]).convert('RGBA')).astype(np.float32)
H,W=img.shape[:2]
a=img[...,3]/255.0
def windows(n, to):
    step=f(n)/f(to); out=[]
    for t in range(to):
        lo=int(f(t)*step); hi=int(np.ceil(f(t+1)*step)); hi=min(max(hi,lo+1),n); out.append((lo,hi))
    return out
def scale(th):
    tw=int(th*W//H)
    ys=windows(H,th); xs=windows(W,tw)
    out=np.zeros((th,tw))
    # alpha only
    cs=np.cumsum(np.cumsum(np.pad(a,((1,0),(1,0))),0),1)
    for i,(t,b) in enumerate(ys):
        for j,(l,r) in enumerate(xs):
            s=cs[b,r]-cs[t,r]-cs[b,l]+cs[t,l]
            out[i,j]=s/((b-t)*(r-l))
    return out,tw
for th in [202,302,304,269]:
    o,tw=scale(th)
    ref=np.array(Image.fromarray((a*255).astype(np.uint8)).resize((tw,th),Image.BILINEAR)).astype(np.float32)/255
    d=np.abs(o-ref)
    print(th, 'mean|diff| alpha', round(float(d.mean()),4),'p99',round(float(np.percentile(d,99)),3),'max',round(float(d.max()),3))
