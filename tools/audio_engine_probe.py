#!/usr/bin/env python3
"""Installed, isolated native audio workflow; optional native operator controls, no screenshots/PCM recordings."""
from pathlib import Path
import argparse,json,subprocess,time,os
ROOT=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser();p.add_argument('--app',type=Path,default=Path.home()/'Applications/Rastera.app');p.add_argument('--out',type=Path,default=Path('/tmp/rastera-audio-native'));p.add_argument('--seconds',type=int,default=600);p.add_argument('--input',help='Explicit durable input UID; may request microphone authorization');p.add_argument('--transitions',action='store_true');p.add_argument('--skip-fixtures',action='store_true');a=p.parse_args()
out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
if not a.skip_fixtures:subprocess.run(['python3',str(ROOT/'tools/generate_audio_fixtures.py'),str(out)],check=True)
helper=Path('/tmp/rastera-audio-editor-window');subprocess.run(['xcrun','clang',str(ROOT/'tools/native_editor_window.m'),'-framework','Cocoa','-framework','ApplicationServices','-o',str(helper)],check=True)
binary=a.app.resolve()/'Contents/MacOS/Rastera';args=[str(binary),f'--editor-probe={out}','--audio-probe',f'--audio-seconds={a.seconds}','--outputs=0']
if a.input:args.append('--audio-input='+a.input)
title='Rastera — Canvas Editor — Operator / UI preview (automated probe)';events=[];rss=[];cycles=0;trial=None
with (out/'editor.log').open('w')as f:
 app=subprocess.Popen(args,cwd='/',env={'HOME':str(Path.home()),'PATH':'/usr/bin:/bin'},stdout=f,stderr=subprocess.STDOUT)
 try:
  deadline=time.monotonic()+a.seconds+150;last=0
  while time.monotonic()<deadline:
   log=(out/'editor.log').read_text(errors='replace')
   if 'audio_probe failed=1'in log:raise RuntimeError(next(s for s in log.splitlines()if 'audio_probe failed=1'in s))
   if 'audio_trial started=1'in log and trial is None:trial=time.monotonic()
   if trial is not None and a.transitions and cycles<3 and time.monotonic()-trial>10+cycles*20:
    for action in ['minimize','restore']:
     start=time.monotonic();r=subprocess.run([str(helper),str(app.pid),title,action],capture_output=True,text=True);events.append({'action':action,'start_monotonic':start,'return_monotonic':time.monotonic(),'returncode':r.returncode,'result':r.stdout.strip(),'stderr':r.stderr.strip()});time.sleep(.4)
    cycles+=1
   if time.monotonic()-last>1:
    r=subprocess.run(['ps','-p',str(app.pid),'-o','rss='],capture_output=True,text=True);rss.append({'monotonic':time.monotonic(),'rss_kib':int(r.stdout.strip())if r.stdout.strip()else None});last=time.monotonic()
   if 'audio_probe passed=1'in log:break
   if app.poll()is not None:raise RuntimeError(f'Application exited early: {app.returncode}')
   time.sleep(.2)
  else:raise RuntimeError('Native audio workflow timed out')
 finally:
  if app.poll()is None:
   close=subprocess.run([str(helper),str(app.pid),title,'close'],capture_output=True,text=True);events.append({'action':'normal close','returncode':close.returncode,'stdout':close.stdout.strip(),'stderr':close.stderr.strip()})
   try:app.wait(timeout=25)
   except subprocess.TimeoutExpired:raise RuntimeError('Normal shutdown timed out; process left for inspection')
  (out/'driver-results.json').write_text(json.dumps({'exit_code':app.returncode,'native_controls':events,'resident_memory':rss,'screenshot_capture':False,'audio_recording':False,'physical_listening':'pending','physical_av_loopback':'unavailable'},indent=2)+'\n')
if app.returncode!=0:raise SystemExit(f'App exit {app.returncode}')
print(json.dumps({'results':str(out/'audio-trial-results.json'),'driver':str(out/'driver-results.json'),'exit_code':app.returncode},indent=2))
