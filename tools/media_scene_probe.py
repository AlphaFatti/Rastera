#!/usr/bin/env python3
"""Owned native replay, isolated journals/destinations, no global input or camera pictures."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / 'target/Rastera.app/Contents/MacOS/Rastera'
HELPER = '/tmp/rastera-editor-window'
TITLE = 'Rastera — Canvas Editor — Operator / UI preview'


def run(folder, name, args, marker=None, cycles=0):
    log = folder / f'{name}.log'
    title = TITLE + (' (automated probe)' if any(a.startswith('--editor-probe=') for a in args) else '')
    events = []
    with log.open('w') as output:
        app = subprocess.Popen([str(APP), *args], cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + (45 if marker else 4)
            while time.monotonic() < deadline and app.poll() is None:
                time.sleep(.25)
                text = log.read_text()
                if 'media_probe failed=1' in text:
                    raise RuntimeError(next(x for x in text.splitlines() if 'media_probe failed=1' in x))
                if marker and marker in text:
                    break
            if marker and marker not in log.read_text():
                raise RuntimeError(f'{name}: missing success marker')
            for cycle in range(cycles):
                for command in ['minimize', 'restore']:
                    events.append(dict(cycle=cycle, command=command, start=time.monotonic()))
                    subprocess.run([HELPER, str(app.pid), title, command], check=True)
                    events[-1]['return'] = time.monotonic()
                    time.sleep(.6)
        finally:
            if app.poll() is None:
                subprocess.run([HELPER, str(app.pid), title, 'close'], check=True)
                app.wait(timeout=20)
    text = log.read_text()
    if app.returncode or 'shared_shutdown outstanding_slots=0 capacity=4' not in text or 'capture_shutdown drained=1' not in text:
        raise RuntimeError(f'{name}: shutdown did not drain')
    return text, events


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, default=Path('/tmp/rastera-media-scenes'))
    args = parser.parse_args()
    folder = args.out.resolve()
    folder.mkdir(parents=True, exist_ok=True)
    if not Path(HELPER).exists():
        subprocess.run(['clang', str(ROOT/'tools/native_editor_window.m'), '-framework', 'Cocoa', '-framework', 'ApplicationServices', '-o', HELPER], check=True)
    subprocess.run([str(ROOT/'tools/generate_media_fixtures.sh'), str(folder)], check=True)
    text, events = run(folder, 'combined', [f'--editor-probe={folder}', '--media-probe', '--outputs=0'], 'media_scenes_probe passed=1', 2)
    source = folder/'session'
    original = json.loads((source/'current.json').read_text())
    results = dict(combined=True, camera='camera=1' in text, native_crop_undo_redo=True, minimize_events=events,
                   physical_picture_verified=False, native_drain=True)
    for name, extra, enabled in [('cold', [], False), ('enabled', [], True), ('no-restore', ['--no-restore'], False), ('explicit', [f'--project={folder}/combined.rastera.json'], False)]:
        journal = folder/f'{name}-session'
        shutil.copytree(source, journal, dirs_exist_ok=True)
        shutil.copyfile(folder/'outputs.local.json', journal/'outputs.local.json')
        prefs = json.loads((journal/'preferences.json').read_text())
        prefs['restore_outputs'] = enabled
        (journal/'preferences.json').write_text(json.dumps(prefs))
        log, _ = run(folder, name, [f'--session-dir={journal}', *extra])
        snapshot = json.loads((journal/'current.json').read_text())
        if name in ('cold', 'enabled'):
            assert snapshot['document'] == original['document'], 'Unsaved recovered document changed'
            assert snapshot['project'] == original['project'] and len(snapshot['playheads']) == 2
            assert 'session_restored' in log
            assert len(snapshot['enabled']) == (len(original['enabled']) if enabled else 0)
        else:
            assert 'session_restored' not in log
            assert len(snapshot['document']['scenes']) == (1 if name == 'no-restore' else 3)
        results[name] = True
    (folder/'results.json').write_text(json.dumps(results, indent=2)+'\n')
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
