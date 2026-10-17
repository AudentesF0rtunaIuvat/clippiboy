exec(open('<review-scratch>/sim/blur.py').read().split('for r in (4,8,16):')[0])
def psf_box(shape,r):
    h,w=shape; k=np.zeros(shape); 
    for dy in range(-r,r+1):
        for dx in range(-r,r+1): k[dy%h,dx%w]=1
    return k/k.sum()
def psf_g(shape,s):
    h,w=shape; y=np.minimum(np.arange(h),h-np.arange(h)); x=np.minimum(np.arange(w),w-np.arange(w))
    k=np.exp(-(y[:,None]**2+x[None,:]**2)/(2*s*s)); return k/k.sum()
def wiener(B,K,nsr):
    Hf=np.fft.fft2(K); Bf=np.fft.fft2(B-255)
    return np.real(np.fft.ifft2(Bf*np.conj(Hf)/(np.abs(Hf)**2+nsr)))+255
rows=[]
for size,r in ((14,8),(14,16),(24,16)):
    f=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf',size)
    W=400;H=max(size*4,4*r+size*2)
    im=Image.new('L',(W,H),255); d=ImageDraw.Draw(im); d.text((3*r,H//2-size//2),"Kx9pQ2vLm7 secret",fill=0,font=f)
    A=np.asarray(im).astype(np.float64); s=math.sqrt(r*(r+1)/3)
    Bb=box(A,r); Bg=gauss(A,s)
    best=[]
    for B,K in ((Bb,psf_box(A.shape,r)),(Bg,psf_g(A.shape,s))):
        R=wiener(B,K,1e-3)
        err=np.sqrt(np.mean((R.clip(0,255)-A)**2))
        best.append(R.clip(0,255)); print(size,r,'rmse',round(err,1))
    rows += [A,best[0],best[1],np.full((4,W),128.)]
big=np.vstack(rows).astype(np.uint8)
Image.fromarray(big).resize((big.shape[1]*2,big.shape[0]*2),Image.NEAREST).save(OUT+'deconv.png')
