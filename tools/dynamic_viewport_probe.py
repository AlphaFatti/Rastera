#!/usr/bin/env python3
"""ID-scoped native dynamic-output replay; no global mouse/keyboard input.

--foreground opts into app activation for an unobscured trial and restores focus.
--capture records only owned clean windows. It is not physical scanout evidence.
"""
import argparse
import json
import re
import subprocess
import time
from pathlib import Path

ROOT = 'Rastera — Canvas Editor — Operator / UI preview (automated probe)'

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--foreground', action='store_true')
    parser.add_argument('--capture', action='store_true')
    args = parser.parse_args()
    folder = args.out.resolve()
    folder.mkdir(parents=True, exist_ok=True)
    (folder/'layout-ready').unlink(missing_ok=True)
    events = []
    def event(kind, **fields):
        row = dict(event=kind, host_s=time.monotonic(), **fields)
        events.append(row)
        (folder/'events.json').write_text(json.dumps(events, indent=2))
        print(json.dumps(row), flush=True)
    def activate(pid):
        subprocess.run(['osascript','-l','JavaScript','-e', f'ObjC.import("AppKit"); $.NSRunningApplication.runningApplicationWithProcessIdentifier({pid}).activateWithOptions(2)'], check=True, stdout=subprocess.DEVNULL)
    prior = None
    if args.foreground:
        prior = int(subprocess.check_output(['osascript','-l','JavaScript','-e','ObjC.import("AppKit"); $.NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier'], text=True))
    with (folder/'editor.log').open('w') as output:
        app = subprocess.Popen([str(Path('target/release/canvas-editor').resolve()), f'--editor-probe={folder}', '--dynamic-probe','--dynamic-layout-driver'], stdout=output, stderr=subprocess.STDOUT)
        (folder/'pid').write_text(str(app.pid)); event('launch', pid=app.pid)
        def windows():
            return json.loads(subprocess.check_output(['/tmp/rastera-window-info', str(app.pid)]))['windows']
        def action(title, mode, *values):
            subprocess.run(['/tmp/rastera-editor-window',str(app.pid),title,mode,*map(str,values)],check=True)
        def capture(w, name):
            start=time.monotonic()
            subprocess.run(['screencapture','-x','-o','-l',str(w['id']),str(folder/name)],check=True)
            event('capture', file=name, title=w['title'], start_host_s=start, end_host_s=time.monotonic())
        activated = False
        arranged = False
        try:
            deadline = time.monotonic()+180
            while True:
                log=(folder/'editor.log').read_text()
                if 'dynamic_viewport_probe failed=1' in log or 'panicked' in log or app.poll() is not None:
                    raise RuntimeError('Native dynamic replay failed; inspect editor.log')
                if 'dynamic_viewport_probe passed=1' in log:
                    event('replay_passed'); break
                wins = windows()
                if any(w['title']==ROOT for w in wins) and not activated:
                    if args.foreground: activate(app.pid)
                    action(ROOT,'layout',20,50,1500,500)
                    activated=True
                if (folder/'dynamic-open.json').exists() and not arranged:
                    clean={int(m[1]):w for w in wins if (m:=re.search(r'\[ID (\d+)\](?: \(automated probe\))?$',w['title']))}
                    doc=json.loads((folder/'dynamic-open.json').read_text())
                    ids=[v['id'] for v in doc['viewports']]
                    if all(i in clean for i in ids):
                        for i, bounds in zip(ids,[(20,620,400,260),(450,570,260,490),(730,620,400,330),(1160,620,600,180)]):
                            action(clean[i]['title'],'layout',*bounds)
                        event('four_aspects_arranged', viewports=[dict(id=v['id'],size=v['size']) for v in doc['viewports']])
                        if args.capture:
                            for w in windows():
                                if (m:=re.search(r'\[ID (\d+)\](?: \(automated probe\))?$',w['title'])) and int(m[1]) in ids:
                                    capture(w,f"output-{m[1]}.png")
                        (folder/'layout-ready').write_text('ready\n')
                        arranged=True
                if time.monotonic()>deadline: raise TimeoutError('Native replay timed out')
                time.sleep(.05)
            if args.capture:
                for w in windows():
                    if m:=re.search(r'\[ID (\d+)\](?: \(automated probe\))?$',w['title']):capture(w,f'final-output-{m[1]}.png')
        finally:
            if app.poll() is None:
                action(ROOT,'close'); app.wait(timeout=20);event('closed', returncode=app.returncode)
            if prior is not None:activate(prior);event('focus_restored')

if __name__=='__main__':main()
