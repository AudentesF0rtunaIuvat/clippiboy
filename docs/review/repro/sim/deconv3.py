exec(open('<review-scratch>/sim/deconv.py').read().split('rows=[]')[0])
def box3(a,r):
    ri=max(1,round((math.sqrt(1+4*r*(r+1)/3)-1)/2)); x=a
    for _ in range(3): x=box_rows(x,ri)
    x=x.T
    for _ in range(3): x=box_rows(x,ri)
    return x.T, ri
def psf_box3(shape,ri):
    K=psf_box(shape,ri); F=np.fft.fft2(K)**3; return np.real(np.fft.ifft2(F))
rows=[]
for size,r in ((14,8),(24,16)):
    f=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf',size)
    W=400;H=max(size*4,4*r+size*2)
    im=Image.new('L',(W,H),255); d=ImageDraw.Draw(im); d.text((3*r,H//2-size//2),"Kx9pQ2vLm7 secret",fill=0,font=f)
    A=np.asarray(im).astype(np.float64)
    B3,ri=box3(A,r)
    out=[A]
    for nsr in (1e-3,1e-4):
        out.append(wiener(B3,psf_box3(A.shape,ri),nsr).clip(0,255))
        out.append(wiener(box(A,r),psf_box(A.shape,r),nsr).clip(0,255))
    rows += out+[np.full((4,W),128.)]
    print(size,r,'ri',ri)
big=np.vstack(rows).astype(np.uint8)
Image.fromarray(big).save(OUT+'deconv3.png')
