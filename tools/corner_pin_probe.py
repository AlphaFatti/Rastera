#!/usr/bin/env python3
"""Installed/core corner-pin assertions and optional owned clean-window pixel samples."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT=Path(__file__).resolve().parents[1]
TITLE='Rastera — Canvas Editor — Operator / UI preview (automated probe)'
parser=argparse.ArgumentParser()
parser.add_argument('--app',type=Path,default=Path.home()/'Applications/Rastera.app')
parser.add_argument('--out',type=Path,default=Path('/tmp/rastera-corner-pin'))
parser.add_argument('--capture',action='store_true')
args=parser.parse_args();folder=args.out.resolve();folder.mkdir(parents=True,exist_ok=True)
subprocess.run(['python3',str(ROOT/'tools/generate_pin_fixtures.py'),str(folder)],check=True)
helper=Path('/tmp/rastera-editor-window')
if not helper.exists():subprocess.run(['clang',str(ROOT/'tools/native_editor_window.m'),'-framework','Cocoa','-framework','ApplicationServices','-o',str(helper)],check=True)
binary=args.app.resolve()/'Contents/MacOS/Rastera';log=folder/'editor.log'
with log.open('w')as f:
 app=subprocess.Popen([str(binary),f'--editor-probe={folder}','--corner-pin-probe','--outputs=0'],cwd='/',env={'HOME':str(Path.home()),'PATH':'/usr/bin:/bin'},stdout=f,stderr=subprocess.STDOUT)
 results={'executable':str(binary),'cwd':'/','minimal_environment':True,'captured':args.capture,'human_picture':'pending'}
 try:
  deadline=time.monotonic()+45
  while time.monotonic()<deadline:
   time.sleep(.2);text=log.read_text()
   if 'corner_pin_probe failed=1'in text:raise RuntimeError(next(x for x in text.splitlines()if 'corner_pin_probe failed=1'in x))
   if 'corner_pin_probe passed=1'in text:break
  if 'corner_pin_probe passed=1'not in log.read_text():raise RuntimeError('Native pin replay timed out')
  time.sleep(.3)
  state=json.loads((folder/'pin-native-state.json').read_text());results['native_pass']=True
  results['outputs']={key:{k:v for k,v in value.items()if k in ['fullscreen','frame','drawable','key','mouse_ignored']}for key,value in state['outputs'].items()}
  if args.capture:
   results['samples']=[]
   for key,value in state['outputs'].items():
    shot=folder/f'clean-{key}.png';start=time.monotonic();subprocess.run(['/usr/sbin/screencapture','-x','-o','-l',str(value['number']),str(shot)],check=True);finish=time.monotonic()
    # Window captures carry display ICC profiles; compare in the fixture's sRGB space.
    normalized=folder/f'clean-{key}-srgb.png'
    subprocess.run(['/usr/bin/sips','--matchTo','/System/Library/ColorSync/Profiles/sRGB Profile.icc',str(shot),'--out',str(normalized)],check=True,stdout=subprocess.DEVNULL)
    ffprobe=shutil.which('ffprobe');ffmpeg=shutil.which('ffmpeg')
    info=json.loads(subprocess.check_output([ffprobe,'-v','error','-show_entries','stream=width,height','-of','json',str(normalized)]))['streams'][0];w=info['width'];h=info['height']
    pixels=subprocess.check_output([ffmpeg,'-v','error','-i',str(normalized),'-f','rawvideo','-pix_fmt','rgb24','-'])
    dw,dh=value['drawable'];top=h-int(dh)
    if w!=dw or top<0:raise RuntimeError('Unexpected screenshot-to-drawable mapping')
    count=0;max_error=0
    for sample in state['samples']:
     x,y=sample['canvas'];output=1 if x<1920 else 2
     if str(output)!=key:continue
     x=int(x if output==1 else x-1920);y=int(y)+top;offset=(y*w+x)*3
     actual=list(pixels[offset:offset+3]);error=max(abs(a-b)for a,b in zip(actual,sample['rgb']));max_error=max(max_error,error);count+=1
    if count==0 or max_error>25:raise RuntimeError(f'Grid sampling differs: output {key}, count {count}, max delta {max_error}')
    results['samples'].append(dict(viewport=int(key),count=count,max_channel_delta=max_error,capture_start=start,capture_return=finish))
 finally:
  subprocess.run([str(helper),str(app.pid),TITLE,'close'],check=True);app.wait(timeout=20)
 text=log.read_text();results['exit_code']=app.returncode;results['drained']='shared_shutdown outstanding_slots=0 capacity=4'in text
 if app.returncode or not results['drained']:raise RuntimeError('Native shutdown did not drain')
(folder/'results.json').write_text(json.dumps(results,indent=2)+'\n');print(json.dumps(results,indent=2))
