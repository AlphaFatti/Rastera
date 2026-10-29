#!/usr/bin/env python3
"""Owned-window fullscreen assertions. Briefly covers connected displays; restores focus.
No global input or screen captures. Requires compiled PID-scoped window helpers.
"""
import argparse
import json
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = 'Rastera — Canvas Editor — Operator / UI preview (automated probe)'

def activate(pid):
    subprocess.run(['osascript','-l','JavaScript','-e',f'ObjC.import("AppKit"); $.NSRunningApplication.runningApplicationWithProcessIdentifier({pid}).activateWithOptions(2)'],check=True,stdout=subprocess.DEVNULL)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--capture',action='store_true',help='Capture only owned clean windows; known capture perturbation applies')
    args=parser.parse_args();folder=args.out.resolve();folder.mkdir(parents=True,exist_ok=True)
    for name in ['display-start','transition-ready','transitions-done']:(folder/name).unlink(missing_ok=True)
    events=[]
    def event(kind,**fields):
        events.append(dict(event=kind,host_s=time.monotonic(),**fields));(folder/'driver-events.json').write_text(json.dumps(events,indent=2))
    prior=int(subprocess.check_output(['osascript','-l','JavaScript','-e','ObjC.import("AppKit"); $.NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier'],text=True))
    with (folder/'editor.log').open('w') as log:
        app=subprocess.Popen([str(Path('target/release/canvas-editor').resolve()),f'--editor-probe={folder}','--display-probe','--outputs=0'],stdout=log,stderr=subprocess.STDOUT)
        (folder/'pid').write_text(str(app.pid));started=False;transitions=False
        try:
            deadline=time.monotonic()+120
            while True:
                contents=(folder/'editor.log').read_text()
                if 'display_output_probe failed=1' in contents or 'panicked' in contents or app.poll() is not None:raise RuntimeError('Native display replay failed; inspect editor.log')
                if 'display_output_probe passed=1' in contents:
                    event('replay_passed');print((folder/'display-results.json').read_text());break
                if (folder/'transition-ready').exists() and not transitions:
                    def action(mode):
                        event(mode+'_start');subprocess.run(['/tmp/rastera-editor-window',str(app.pid),ROOT,mode],check=True);event(mode+'_return')
                    def clean():
                        rows=json.loads(subprocess.check_output(['/tmp/rastera-window-info',str(app.pid)]))['windows']
                        return next(w for w in rows if '[ID 1]' in w['title'])
                    def snapshot(name):
                        window=clean();start=time.monotonic();subprocess.run(['screencapture','-x','-o','-l',str(window['id']),str(folder/name)],check=True);event('capture',file=name,start_host_s=start,end_host_s=time.monotonic())
                    event('idle_start');time.sleep(.7)
                    if args.capture:snapshot('idle.png')
                    for cycle in range(3):
                        event('cycle',cycle=cycle)
                        with ThreadPoolExecutor(max_workers=1) as pool:
                            future=pool.submit(action,'minimize')
                            if args.capture:
                                time.sleep(.2);snapshot(f'minimize-{cycle}.png')
                            future.result()
                        assert clean()['onscreen'],'Fullscreen disappeared on operator minimize'
                        time.sleep(.3);action('restore');time.sleep(.3)
                    event('application_switch_start');activate(prior);time.sleep(.7)
                    assert clean()['onscreen'],'Fullscreen disappeared during application switch'
                    if args.capture:snapshot('other-application.png')
                    event('application_switch_end');activate(app.pid);time.sleep(.2)
                    (folder/'transitions-done').write_text('done');transitions=True
                if not started:
                    windows=json.loads(subprocess.check_output(['/tmp/rastera-window-info',str(app.pid)]))['windows']
                    if any(w['title']==ROOT for w in windows):
                        activate(app.pid);time.sleep(.2);(folder/'display-start').write_text('ready\n');started=True
                if time.monotonic()>deadline:raise TimeoutError('Native display replay timed out')
                time.sleep(.05)
        finally:
            if app.poll() is None:
                # UI close is scoped strictly to this owned PID, including failure recovery.
                subprocess.run(['/tmp/rastera-editor-window',str(app.pid),ROOT,'close'],check=True);app.wait(timeout=20)
            activate(prior)
        if app.returncode:raise RuntimeError(f'Application exit {app.returncode}')

if __name__=='__main__':main()
