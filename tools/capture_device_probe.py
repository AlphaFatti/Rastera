#!/usr/bin/env python3
"""Real capture integration in the signed Rastera.app identity. No camera pictures are saved.
The explicit --capture-probe action may show macOS camera consent. Uses PID-owned window controls.
"""
import argparse
import json
import subprocess
import time
from pathlib import Path
ROOT='Rastera — Canvas Editor — Operator / UI preview (automated probe)'

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--project',type=Path,help='Fresh-process reopen check of an existing capture project')
    args=parser.parse_args();folder=args.out.resolve();folder.mkdir(parents=True,exist_ok=True)
    command=[str(Path('target/Rastera.app/Contents/MacOS/Rastera').resolve()),f'--editor-probe={folder}','--outputs=0','--capture-reopen-probe' if args.project else '--capture-probe']
    if args.project:command.append(f'--project={args.project.resolve()}')
    events=[]
    def record(event,**fields):events.append(dict(event=event,host_s=time.monotonic(),**fields));(folder/'driver-events.json').write_text(json.dumps(events,indent=2))
    with (folder/'editor.log').open('w') as log:
        app=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT)
        (folder/'pid').write_text(str(app.pid));record('launch',pid=app.pid)
        try:
            deadline=time.monotonic()+150
            while True:
                text=(folder/'editor.log').read_text()
                if 'capture_probe failed=1' in text or 'panicked' in text or app.poll() is not None:raise RuntimeError('Capture replay failed; inspect editor.log')
                if 'capture_probe passed=1' in text or 'capture_reopen_probe passed=1' in text:break
                if time.monotonic()>deadline:raise TimeoutError('Capture replay timed out; check camera permission/status')
                time.sleep(.1)
            record('passed')
            for cycle in range(2):
                record('minimize_start',cycle=cycle);subprocess.run(['/tmp/rastera-editor-window',str(app.pid),ROOT,'minimize'],check=True);time.sleep(.7);record('restore_start',cycle=cycle);subprocess.run(['/tmp/rastera-editor-window',str(app.pid),ROOT,'restore'],check=True);time.sleep(.4)
        finally:
            if app.poll() is None:subprocess.run(['/tmp/rastera-editor-window',str(app.pid),ROOT,'close'],check=True);app.wait(timeout=20)
            record('closed',returncode=app.returncode)
        if app.returncode:raise RuntimeError(f'Application exited {app.returncode}')
        print(json.dumps(dict(passed=True,out=str(folder),returncode=app.returncode)))
if __name__=='__main__':main()
