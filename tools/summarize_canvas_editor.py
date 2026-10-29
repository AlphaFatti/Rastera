#!/usr/bin/env python3
"""Summarize owned editor trials; presentation reports are not physical scanout."""
import argparse
import json
import math
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from summarize_native_output import parse_log, max_gap, ms


def summarize(folder, images=False):
    rows = parse_log(folder/'editor.log')
    driver = json.loads((folder/'events.json').read_text())
    lo = next(r['host_s'] for r in driver if r['event'] == 'measure_start')
    hi = next(r['host_s'] for r in driver if r['event'] == 'measure_end')
    native = [r for r in rows if r['record_type'] == 'shared_event']
    ui = [r for r in rows if r['record_type'] == 'shared_ui_gap']
    positive = lambda r: math.isfinite(float(r['gpu_or_presented_host_s'])) and float(r['gpu_or_presented_host_s']) > 0
    result = dict(seconds=hi-lo, replay_passed=any(r.get('passed') == '1' for r in rows if r['record_type'] == 'editor_probe'),
        shutdown=[r for r in rows if r['record_type'] == 'shared_shutdown'], cases={}, roles={})
    for role in ['preview', 'left', 'right']:
        records = [r for r in native if r['role'] == role]
        key = lambda r: (int(r['seq']), int(r['frame']))
        commits = {key(r) for r in records if r['kind'] == 'commit'}
        completes = {key(r) for r in records if r['kind'] == 'complete'}
        handlers = [r for r in records if r['kind'] == 'presented']
        samples = [r for r in rows if r['record_type'] == 'shared_interval' and r['role'] == role]
        result['roles'][role] = dict(committed=len(commits), missing_completions=sorted(commits-completes),
            missing_presentation_handlers=sorted(commits-{key(r) for r in handlers}),
            without_positive_presentation=sorted(commits-{key(r) for r in handlers if positive(r)}),
            zero_timestamps=sum(float(r['gpu_or_presented_host_s']) == 0 for r in handlers),
            nonfinite_timestamps=sum(not math.isfinite(float(r['gpu_or_presented_host_s'])) for r in handlers),
            max_inflight=max(int(r['max_inflight']) for r in samples), errors=max(int(r['errors']) for r in samples), lost=max(int(r['lost']) for r in samples))
    for case in ['operator', 'left', 'right']:
        requests = [r for r in driver if r.get('case') == case and r['event'] == 'native_request']
        confirms = [r for r in driver if r.get('case') == case and r['event'] == 'native_confirm']
        start, end = requests[0]['host_s'], confirms[-1]['host_s']
        groups = {}
        for role in ['left', 'right']:
            if role == case:
                continue
            records = [r for r in native if r['role'] == role]
            times = sorted(float(r['gpu_or_presented_host_s']) for r in records if r['kind'] == 'presented' and positive(r) and start <= float(r['gpu_or_presented_host_s']) <= end)
            scheduled = [float(r['callback_host_s']) for r in records if r['kind'] == 'schedule' and start <= float(r['callback_host_s']) <= end]
            groups[role] = dict(valid_presentation_gap_ms=ms(max_gap(times)), positive_reports=len(times),
                first_positive_after_start_ms=ms(times[0]-start) if times else None,
                end_after_last_positive_ms=ms(end-times[-1]) if times else None,
                schedule_gap_ms=ms(max_gap(scheduled)), zero_callbacks=sum(r['kind'] == 'presented' and float(r['gpu_or_presented_host_s']) == 0 and start <= float(r['callback_host_s']) <= end for r in records))
        result['cases'][case] = dict(confirmed_actions=len(confirms), visible_outputs=groups,
            ui_gap_ms=max((int(r['gap_us'])/1000 for r in ui if start <= float(r['host_s']) <= end), default=0))
    producer = [r for r in rows if r['record_type'] == 'shared_producer']
    result['pool'] = dict(capacity=4, max_logged_leased_slots=max(int(r['leased_slots']) for r in producer), max_logged_skips=max(int(r['skipped']) for r in producer))
    result['logged_stage_max_us'] = {stage:max(int(r['max_us']) for r in rows if r['record_type'] == 'stage' and r['name'] == stage) for stage in ['cadence','record','finish','submit','wait']}
    if images:
        captures = [r for r in driver if r['event'] == 'capture' and r['file'].endswith(('-left.png', '-right.png'))]

        def detect(row):
            raw = subprocess.check_output(['ffmpeg','-v','error','-i',str(folder/row['file']),'-vf','scale=320:175','-frames:v','1','-f','rawvideo','-pix_fmt','rgb24','-'])
            xs = [(i//3) % 320 for i in range(20*320*3,len(raw),3) if raw[i]>200 and raw[i+1]>200 and raw[i+2]<140]
            return dict(**row, marker_x=round(sum(xs)/len(xs),4) if xs else None)
        with ThreadPoolExecutor(max_workers=2) as pool:
            samples = list(pool.map(detect, captures))
        result['images'] = dict(samples=samples, groups={})
        for case in ['idle','operator','left','right']:
            for role in ['left','right']:
                group = [r for r in samples if r['file'].startswith(case+'-') and r['file'].endswith('-'+role+'.png')]
                if not group:
                    continue
                repeated = [dict(first=a['file'],last=b['file'],first_start=a['start_host_s'],first_end=a['end_host_s'],last_start=b['start_host_s'],last_end=b['end_host_s']) for a,b in zip(group,group[1:]) if a['marker_x'] is not None and a['marker_x'] == b['marker_x']]
                result['images']['groups'][case+'-'+role] = dict(count=len(group),missing_marker=sum(r['marker_x'] is None for r in group),repeated_pairs=repeated,max_request_spacing_ms=ms(max_gap([r['start_host_s'] for r in group])))
    return result


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('folder',type=Path)
    parser.add_argument('--images',action='store_true')
    args=parser.parse_args()
    print(json.dumps(summarize(args.folder,args.images),indent=2))
