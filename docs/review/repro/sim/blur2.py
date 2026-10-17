import sys
sys.argv=['x']
exec(open('<review-scratch>/sim/blur.py').read().split('for r in (4,8,16):')[0])
rows=[]
for size,r in ((14,4),(14,8),(24,8),(32,8)):
    f=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf',size)
    im=Image.new('L',(360,size*3),255); d=ImageDraw.Draw(im); d.text((30,size),"Kx9pQ2vLm7 secret",fill=0,font=f)
    A=np.asarray(im).astype(np.float64)
    s=math.sqrt(r*(r+1)/3)
    sep=np.full((4,360),128.)
    rows += [A, box(A,r), gauss(A,s), sep]
big=np.vstack(rows).clip(0,255).astype(np.uint8)
Image.fromarray(big).resize((big.shape[1]*2,big.shape[0]*2),Image.NEAREST).save(OUT+'cmp2.png')
