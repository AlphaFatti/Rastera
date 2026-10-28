#!/usr/bin/env python3
"""Separate stack diagnostic; run from repository root after A/B/C.
Uses default animation behavior and the current system Reduce Motion value.
Requires Accessibility and permission for /usr/bin/sample. Not a timing trial.
"""
from pathlib import Path
import subprocess,time,sys
sys.path.insert(0,'tools')
from presentation_probe import apple_event
base=Path('/tmp/rastera-animation/stack')
base.mkdir(parents=True, exist_ok=True)
with (base/'sample-transition.log').open('w') as log:
 p=subprocess.Popen([str(Path('target/release/native-output-control').resolve()),'--window-animation=default'],stdout=log,stderr=subprocess.STDOUT)
 try:
  time.sleep(2)
  apple_event(p.pid,'set frontmost to true\nset position of window "Rastera" to {20,40}\nset position of window "Rastera — Native Output" to {20,500}')
  sampler=subprocess.Popen(['/usr/bin/sample',str(p.pid),'10','1','-file',str(base/'transition.sample.txt')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  time.sleep(1)
  print('native minimize:',apple_event(p.pid,'perform action "AXPress" of (first button of window "Rastera — Native Output" whose subrole is "AXMinimizeButton")'),flush=True)
  sampler.wait(timeout=15)
  apple_event(p.pid,'set value of attribute "AXMinimized" of window "Rastera — Native Output" to false')
  time.sleep(2)
  apple_event(p.pid,'perform action "AXPress" of (first button of window "Rastera" whose subrole is "AXCloseButton")')
  p.wait(timeout=8)
  print('exit',p.returncode)
 finally:
  if p.poll() is None:p.terminate();p.wait(timeout=5)
