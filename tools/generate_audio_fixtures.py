#!/usr/bin/env python3
"""Synthetic channels only; ffmpeg is fixture tooling, never an app dependency."""
from pathlib import Path
import subprocess,sys,json
out=Path(sys.argv[1]);out.mkdir(parents=True,exist_ok=True)
def run(name,args):subprocess.run(['ffmpeg','-nostdin','-hide_banner','-loglevel','error','-y',*args,str(out/name)],check=True)
video=['-f','lavfi','-i','testsrc2=size=320x180:rate=30:duration=6']
freq=[220,330,440,550]
for count in [2,4]:
 expr='|'.join(f'0.1*sin(2*PI*{f}*t)'for f in freq[:count])
 args=['-f','lavfi','-i',f'aevalsrc={expr}:s=48000:d=6:c={"stereo"if count==2 else"4.0"}']
 run(f'{count}ch.mov',video+args+['-map','0:v','-map','1:a','-c:v','libx264','-pix_fmt','yuv420p','-c:a','pcm_f32le'])
 if count==2:
  for ext,codec in [('wav','pcm_s16le'),('aiff','pcm_s16be')]:run(f'stereo.{ext}',args+['-c:a',codec])
for kind,groups in [('dual-stereo',[[220,330],[440,550]]),('quad-mono',[[220],[330],[440],[550]])]:
 args=video.copy()
 for i,g in enumerate(groups):args+=['-itsoffset',str(i*.125),'-f','lavfi','-i','aevalsrc='+('|'.join(f'0.1*sin(2*PI*{f}*t)'for f in g))+f':s=48000:d=6:c={"mono"if len(g)==1 else"stereo"}']
 args+=['-map','0:v']
 for i in range(len(groups)):args+=['-map',f'{i+1}:a']
 run(kind+'.mov',args+['-c:v','libx264','-pix_fmt','yuv420p','-c:a','pcm_f32le'])
(out/'expected.json').write_text(json.dumps({'rate':48000,'frequencies':freq,'track_offset_seconds':.125},indent=2))
