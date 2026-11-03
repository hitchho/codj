#!/usr/bin/env python3
"""Draw a DJ-mode snapshot (cargo test snapshot) as a PNG, to look at the screen as it is:

    CODJ_SNAP=/tmp/snap cargo test snapshot -- --ignored
    for f in /tmp/snap/*.cells; do python3 tools/cells2png.py $f ${f%.cells}.png; done

Needs Pillow and the JetBrainsMono Nerd Font."""
import sys, re
from PIL import Image, ImageDraw, ImageFont
NAMED = {'Black':(0,0,0),'Red':(205,49,49),'Green':(13,188,121),'Yellow':(229,229,16),'Blue':(36,114,200),
 'Magenta':(188,63,188),'Cyan':(17,168,205),'Gray':(204,204,204),'DarkGray':(118,118,118),'LightRed':(241,76,76),
 'LightGreen':(35,209,139),'LightYellow':(245,245,67),'LightBlue':(59,142,234),'LightMagenta':(214,112,214),
 'LightCyan':(41,184,219),'White':(242,242,242)}
DFG, DBG = (210,210,210), (24,24,28)
def idx(n):
    base=list(NAMED.values()); order=['Black','Red','Green','Yellow','Blue','Magenta','Cyan','Gray','DarkGray','LightRed','LightGreen','LightYellow','LightBlue','LightMagenta','LightCyan','White']
    if n<16: return NAMED[order[n]]
    if n<232:
        n-=16; v=[0,95,135,175,215,255]; return (v[n//36],v[n//6%6],v[n%6])
    g=8+(n-232)*10; return (g,g,g)
def col(s, d):
    if s=='Reset': return d
    m=re.match(r'Rgb\((\d+), (\d+), (\d+)\)',s)
    if m: return tuple(map(int,m.groups()))
    m=re.match(r'Indexed\((\d+)\)',s)
    if m: return idx(int(m.group(1)))
    return NAMED.get(s,d)
src,dst=sys.argv[1],sys.argv[2]
lines=open(src,encoding='utf-8').read().split('\n')
w,h=map(int,lines[0].split())
F=ImageFont.truetype('/usr/share/fonts/TTF/JetBrainsMonoNerdFontMono-Regular.ttf',16)
FB=ImageFont.truetype('/usr/share/fonts/TTF/JetBrainsMonoNerdFontMono-Bold.ttf',16)
cw,ch=10,20
img=Image.new('RGB',(w*cw,h*ch),DBG); d=ImageDraw.Draw(img)
for i,l in enumerate(lines[1:w*h+1]):
    sym,fg,bg,mod=l.split('\t')
    x,y=i%w,i//w
    f,b=col(fg,DFG),col(bg,DBG)
    if 'REVERSED' in mod: f,b=b,f
    if 'DIM' in mod: f=tuple(int(c*0.55+b[j]*0.45) for j,c in enumerate(f))
    d.rectangle([x*cw,y*ch,x*cw+cw-1,y*ch+ch-1],fill=b)
    if sym.strip():
        if sym in '█▇▆▅▄▃▂▁':
            k='▁▂▃▄▅▆▇█'.index(sym)+1; d.rectangle([x*cw,y*ch+ch-ch*k//8,x*cw+cw-1,y*ch+ch-1],fill=f)
        elif sym in '▀': d.rectangle([x*cw,y*ch,x*cw+cw-1,y*ch+ch//2-1],fill=f)
        else: d.text((x*cw,y*ch+1),sym,font=FB if 'BOLD' in mod else F,fill=f)
img.save(dst)
