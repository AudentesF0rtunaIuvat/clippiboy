import numpy as np, sys
from PIL import Image, ImageDraw, ImageFont
exec(open('disc.py').read().split('base="hunter2pass"')[0].replace("font = ImageFont","_f = ImageFont").replace("W,H=200,30",""))
def render(t,size,W,H):
    f=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf', size)
    img=Image.new('L',(W,H),255); ImageDraw.Draw(img).text((10,H//4),t,font=f,fill=0); return np.asarray(img).astype(float)
def stretch(x): return (x-x.min())/max(1,x.max()-x.min())*255
rows=[]
for size,r,W,H in ((20,8,360,40),(28,16,500,60)):
    a=render("hunter2pass@mail.com",size,W,H)
    for img in (a,box(a,r),gauss(a,r)):
        rows.append(np.hstack([img,np.full((H,6),128),stretch(img)]))
    rows.append(np.full((6,rows[-1].shape[1]),128))
w=max(x.shape[1] for x in rows)
rows=[np.pad(x,((0,0),(0,w-x.shape[1])),constant_values=255) for x in rows]
Image.fromarray(np.vstack(rows).astype(np.uint8)).resize((w*2,sum(x.shape[0] for x in rows)*2),Image.NEAREST).save('out2.png')
