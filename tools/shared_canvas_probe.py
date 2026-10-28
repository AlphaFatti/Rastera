#!/usr/bin/env python3
"""Owned-process native controls and app-window captures for shared-canvas-native.
Requires Accessibility, Screen Recording, and /tmp/rastera-window-info.
"""
import argparse
import json
from pathlib import Path
import subprocess
import threading
import time
from presentation_probe import apple_event

TITLES = {'preview':'Rastera — Operator / UI preview',
          'left':'Rastera — Native clean output — Left',
          'right':'Rastera — Native clean output — Right'}


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('case',choices=['preview','single','idle','operator','left','right','lifecycle','geometry'])
    ap.add_argument('--region',action='store_true',help='Capture only visible clean-output content rectangles, not cached window images')
    ap.add_argument('--no-capture',action='store_true')
    ap.add_argument('--clean-only',action='store_true',help='Capture only unaffected clean outputs, excluding the composited operator view')
    ap.add_argument('--cycles',type=int,default=3)
    ap.add_argument('--out',type=Path,required=True)
    args=ap.parse_args()
    if args.region:
        if args.case in ['preview','geometry','lifecycle']:
            ap.error('--region applies to clean-output continuity captures only')
        args.clean_only=True
    args.out.parent.mkdir(parents=True,exist_ok=True)
    count=0 if args.case=='preview' else 1 if args.case=='single' else 2
    origin=time.monotonic()
    stop=threading.Event(); threads=[]
    with args.out.open('w') as log,args.out.with_suffix('.events.jsonl').open('w') as trace:
        def event(kind,**fields):
            row=dict(event=kind,host_s=time.monotonic(),**fields)
            trace.write(json.dumps(row)+'\n');trace.flush();print(json.dumps(row),flush=True)
        def capture(role,wid,bounds):
            folder=args.out.with_suffix('.'+role+'.frames');folder.mkdir(exist_ok=True)
            with (folder/'timing.jsonl').open('w') as timing:
                while not stop.is_set():
                    start=time.monotonic();dest=folder/f'{round((start-origin)*1e6):012d}.png'
                    capture_args=['-l',str(wid)]
                    if args.region:
                        if role=='preview':raise RuntimeError('Region capture is restricted to clean output interiors')
                        rect=','.join(str(round(v)) for v in [bounds['X']+5,bounds['Y']+38,bounds['Width']-10,bounds['Height']-48])
                        capture_args=['-R',rect]
                    result=subprocess.run(['/usr/sbin/screencapture','-x','-o',*capture_args,str(dest)],capture_output=True)
                    end=time.monotonic()
                    timing.write(json.dumps(dict(file=dest.name,start_host_s=start,end_host_s=end,returncode=result.returncode))+'\n');timing.flush()
                    if result.returncode:
                        (folder/'error.txt').write_bytes(result.stderr);break
                    stop.wait(.08)
        proc=subprocess.Popen([str(Path('target/release/shared-canvas-native').resolve()),f'--outputs={count}'],stdout=log,stderr=subprocess.STDOUT)
        event('launch',pid=proc.pid,outputs=count)
        try:
            time.sleep(2)
            if proc.poll() is not None:raise RuntimeError(f'startup exit {proc.returncode}')
            root=TITLES['preview']; names=['preview']+(['left'] if count>=1 else [])+(['right'] if count==2 else [])
            layout=[f'set frontmost to true',f'set position of window "{root}" to {{20,40}}',f'set size of window "{root}" to {{1000,430}}']
            for i,role in enumerate(names[1:]):
                layout += [f'set position of window "{TITLES[role]}" to {{{20+i*500},520}}',f'set size of window "{TITLES[role]}" to {{480,300}}']
            apple_event(proc.pid,'\n'.join(layout));time.sleep(2)
            info=json.loads(subprocess.check_output(['/tmp/rastera-window-info',str(proc.pid)],text=True))
            if not info['screen_capture_allowed']:raise RuntimeError('Screen Recording is unavailable')
            windows={role:next(w['id'] for w in info['windows'] if w['title']==TITLES[role]) for role in names}
            event('layout',capture_mode='visible-content-region' if args.region else 'window-image',windows=windows,titles={role:TITLES[role] for role in names})
            observed=names if args.case in ['preview','single','idle'] else ['left','right'] if args.case=='operator' else [n for n in names if n!=args.case]
            if args.clean_only:observed=[role for role in observed if role!='preview']
            if args.case not in ['geometry','lifecycle'] and not args.no_capture:
                for role in observed:
                    t=threading.Thread(target=capture,args=(role,windows[role],next(w['bounds'] for w in info['windows'] if w['id']==windows[role])));t.start();threads.append(t)
            event('measure_start')
            if args.case in ['operator','left','right']:
                target='preview' if args.case=='operator' else args.case
                for cycle in range(args.cycles):
                    for minimized in [True,False]:
                        event('native_request',target=target,minimized=minimized,cycle=cycle+1)
                        action='perform action "AXPress" of (first button of targetWindow whose subrole is "AXMinimizeButton")' if minimized else 'set value of attribute "AXMinimized" of targetWindow to false'
                        expected=str(minimized).lower()
                        state=apple_event(proc.pid,f'''set targetWindow to window "{TITLES[target]}"
{action}
repeat 50 times
if value of attribute "AXMinimized" of targetWindow is {expected} then exit repeat
delay 0.1
end repeat
return value of attribute "AXMinimized" of targetWindow''')
                        if state!=expected:raise RuntimeError('State not confirmed: '+state)
                        for other in names:
                            if other!=target:
                                state=apple_event(proc.pid,f'return value of attribute "AXMinimized" of window "{TITLES[other]}"')
                                if state!='false':raise RuntimeError('Other window minimized: '+other)
                        snapshot=json.loads(subprocess.check_output(['/tmp/rastera-window-info',str(proc.pid)],text=True))
                        event('native_confirm',target=target,minimized=minimized,cycle=cycle+1,windows=[w for w in snapshot['windows'] if w['title'] in TITLES.values()])
                        time.sleep(2)
            elif args.case=='geometry':
                # 960x540 point content = 1920x1080 drawable on this 2x host.
                for role in ['left','right']:
                    title=TITLES[role]
                    apple_event(proc.pid,f'set position of window "{title}" to {{20,350}}\nset size of window "{title}" to {{960,568}}\nperform action "AXRaise" of window "{title}"')
                    time.sleep(1)
                    dest=args.out.with_suffix('.'+role+'.png')
                    subprocess.run(['/usr/sbin/screencapture','-x','-o','-l',str(windows[role]),str(dest)],check=True)
                    event('geometry_capture',role=role,file=str(dest))
            elif args.case=='lifecycle':
                # Separate native resize/close and UI reopen; checkbox locations are
                # derived from this fixed operator layout (verify with saved image).
                for role in ['left','right']:
                    title=TITLES[role]
                    for w,h in [(700,360),(350,500),(480,300)]:
                        apple_event(proc.pid,f'set size of window "{title}" to {{{w},{h}}}');time.sleep(.5)
                        event('resize',role=role,width=w,height=h)
                    apple_event(proc.pid,f'perform action "AXPress" of (first button of window "{title}" whose subrole is "AXCloseButton")')
                    time.sleep(1)
                    state=apple_event(proc.pid,'return name of every window')
                    if title in state:raise RuntimeError('Native close did not remove window')
                    event('closed',role=role)
                    subprocess.run(['/usr/sbin/screencapture','-x','-o','-l',str(windows['preview']),str(args.out.with_suffix('.before-'+role+'.png'))],check=True)
                    # Native click into the operator checkbox. Coordinates use the
                    # title bar plus the fixed top-panel label layout on this host.
                    position=apple_event(proc.pid,f'return position of window "{root}"')
                    root_x,root_y=[int(v.strip()) for v in position.split(',')]
                    x=root_x+15
                    y=root_y+(97 if role=='left' else 118)
                    apple_event(proc.pid,f'set frontmost to true\nperform action "AXRaise" of window "{root}"')
                    subprocess.run(['/tmp/rastera-native-click',str(proc.pid),root,str(root_x+250),str(root_y+12)],check=True)
                    time.sleep(.2)
                    subprocess.run(['/tmp/rastera-native-click',str(proc.pid),root,str(x),str(y)],check=True)
                    time.sleep(2)
                    state=apple_event(proc.pid,'return name of every window')
                    if title not in state:raise RuntimeError('Checkbox reopen unconfirmed: '+role)
                    event('reopened',role=role)
                time.sleep(3)
            else:time.sleep(6)
            stop.set()
            for t in threads:t.join(timeout=5)
            for role in observed:
                failure=args.out.with_suffix('.'+role+'.frames')/'error.txt'
                if failure.exists():raise RuntimeError('Capture failed: '+str(failure))
            event('measure_end')
            apple_event(proc.pid,f'perform action "AXPress" of (first button of window "{root}" whose subrole is "AXCloseButton")')
            proc.wait(timeout=10)
            if proc.returncode:raise RuntimeError('exit '+str(proc.returncode))
            event('done',returncode=proc.returncode)
        finally:
            stop.set()
            for t in threads:t.join(timeout=5)
            if proc.poll() is None:proc.terminate();proc.wait(timeout=5)


if __name__=='__main__':main()
