#!/usr/bin/env python3
"""Synthetic canvas-editor replay plus PID-scoped native transitions.

No global input, foreground activation or desktop-region captures. The editor's
explicit --editor-probe switch dispatches local NSEvents to its own child view;
this is a programmatic interaction test, not human mouse/keyboard acceptance.
"""
import argparse
import json
import struct
import subprocess
import time
import zlib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = 'Rastera — Canvas Editor — Operator / UI preview (automated probe)'
SIDES = {side: 'Rastera — Native clean output — ' + side.title() + ' (automated probe)' for side in ['left', 'right']}


def fixtures(folder):
    folder.mkdir(parents=True, exist_ok=True)
    width, height = 1200, 600
    rows = b''.join(b'\0' + bytes(c for x in range(width) for c in
        (240, 60, 30, 255 if min(x, y, width-1-x, height-1-y) < 20
         else (180 if (x//100+y//100) % 2 == 0 else 0))) for y in range(height))

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind+data) & 0xffffffff)
    (folder/'alpha.png').write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))
    # Valid schema/reference dimensions, deliberately missing media; live document must survive.
    (folder/'missing.json').write_text(json.dumps(dict(version=1, canvas=[3840, 1080], next_id=3,
        test_background=True, images={'1': dict(path='absent.png', width=1200, height=600)}, texts={},
        layers=[dict(id=2, name='Missing image', rect=dict(x=0., y=0., width=1200., height=600.),
            visible=True, locked=False, opacity=1., aspect_lock=True, content=dict(kind='Image', source=1))])))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--cycles', type=int, default=3)
    parser.add_argument('--leave-open', action='store_true')
    parser.add_argument('--no-capture', action='store_true')
    args = parser.parse_args()
    folder = args.out.resolve()
    fixtures(folder)
    log = folder/'editor.log'
    events = []

    def event(kind, **data):
        row = dict(event=kind, host_s=time.monotonic(), **data)
        events.append(row)
        (folder/'events.json').write_text(json.dumps(events, indent=2))
        print(json.dumps(row), flush=True)

    with log.open('w') as output:
        app = subprocess.Popen([str(Path('target/release/canvas-editor').resolve()), f'--editor-probe={folder}'], stdout=output, stderr=subprocess.STDOUT)
        (folder/'pid').write_text(str(app.pid))
        event('launch', pid=app.pid)

        def window(title, mode, *values):
            return subprocess.check_output(['/tmp/rastera-editor-window', str(app.pid), title, mode, *map(str, values)], text=True).strip()

        def windows():
            return json.loads(subprocess.check_output(['/tmp/rastera-window-info', str(app.pid)]))['windows']

        def capture(title, name):
            target = next(w for w in windows() if w['title'] == title)
            start = time.monotonic()
            subprocess.run(['/usr/sbin/screencapture', '-x', '-o', '-l', str(target['id']), str(folder/name)], check=True)
            event('capture', title=title, file=name, start_host_s=start, end_host_s=time.monotonic())

        try:
            deadline = time.monotonic()+30
            while 'editor_probe passed=1' not in log.read_text():
                if app.poll() is not None or 'editor_probe failed_phase=' in log.read_text():
                    raise RuntimeError('Editor replay failed; retain log')
                if time.monotonic() >= deadline:
                    raise TimeoutError('Editor replay did not finish')
                time.sleep(.1)
            event('replay_passed')
            window(ROOT, 'layout', 20, 50, 1500, 680)
            for side, x in [('left', 20), ('right', 700)]:
                window(SIDES[side], 'layout', x, 740, 640, 350)
            time.sleep(.5)
            if not args.no_capture:
                capture(ROOT, 'operator.png')
                for side in SIDES:
                    capture(SIDES[side], side+'.png')
            # Operator capture is a known adverse condition. Exclude this warmup from transitions.
            time.sleep(3)
            event('idle_start')
            for sample in range(4):
                if not args.no_capture:
                    for side in SIDES:
                        capture(SIDES[side], f'idle-{sample}-{side}.png')
                time.sleep(.2)
            event('idle_end')
            event('measure_start')
            for title, case in [(ROOT, 'operator'), (SIDES['left'], 'left'), (SIDES['right'], 'right')]:
                visible = [s for s in SIDES if SIDES[s] != title]
                for cycle in range(args.cycles):
                    for mode, expected in [('minimize', '1'), ('restore', '0')]:
                        event('native_request', case=case, cycle=cycle+1, mode=mode)
                        with ThreadPoolExecutor(max_workers=1) as executor:
                            action = executor.submit(window, title, mode)
                            for sample in range(4):
                                if not args.no_capture:
                                    for side in visible:
                                        capture(SIDES[side], f'{case}-{cycle+1}-{mode}-{sample}-{side}.png')
                                time.sleep(.06)
                            action.result(timeout=15)
                        state = window(title, 'state')
                        if state != expected:
                            raise RuntimeError(f'Unexpected {case} native state {state}')
                        event('native_confirm', case=case, cycle=cycle+1, mode=mode, minimized=state)
                        time.sleep(.2)
            event('measure_end')
            if not args.no_capture:
                for side, x in [('left', 20), ('right', 820)]:
                    window(SIDES[side], 'layout', x, 120, 960, 568)
                    time.sleep(.15)
                    capture(SIDES[side], f'{side}-seam.png')
                event('seam_captures_complete')
        finally:
            if not args.leave_open and app.poll() is None:
                window(ROOT, 'close')
                app.wait(timeout=15)
                event('closed', returncode=app.returncode)


if __name__ == '__main__':
    main()
