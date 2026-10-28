#!/usr/bin/env python3
"""Shared producer/native consumer evidence; screenshots are not physical scanout.
Reads only owned app-window captures; requires ffmpeg/ffprobe for --images.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
from summarize_native_output import parse_log, max_gap, ms


def summarize(path):
    rows=parse_log(path)
    driver=[json.loads(l) for l in path.with_suffix('.events.jsonl').read_text().splitlines()]
    lo=next(e['host_s'] for e in driver if e['event']=='measure_start')
    hi=next(e['host_s'] for e in driver if e['event']=='measure_end')
    gaps=[r for r in rows if r['record_type']=='shared_ui_gap' and lo<=float(r['host_s'])-int(r['gap_us'])/1e6<=float(r['host_s'])<=hi]
    result=dict(case=path.stem,seconds=hi-lo,ui_max_gap_ms=max((int(r['gap_us'])/1000 for r in gaps),default=0),roles={})
    events=[r for r in rows if r['record_type']=='shared_event']
    producer=[r for r in rows if r['record_type']=='shared_producer']
    epoch=float(producer[0]['host_s'])-int(producer[0]['t_us'])/1e6
    result['producer_stage_max_us']={stage:max((int(r['max_us']) for r in rows if r['record_type']=='stage' and r['name']==stage and lo<=epoch+int(r['max_start_us'])/1e6 and epoch+(int(r['max_start_us'])+int(r['max_us']))/1e6<=hi),default=None) for stage in ['cadence','record','finish','submit','wait']}
    result['max_leased_slots']=max(int(r['leased_slots']) for r in producer)
    result['skipped_pool_full']=max(int(r['skipped']) for r in producer)
    result['shutdown']=[r for r in rows if r['record_type']=='shared_shutdown']
    result['device_checks']=[r for r in rows if r['record_type']=='shared_device']
    result['native_controls']=[r for r in rows if r['record_type']=='shared_open']
    for role in sorted({r['role'] for r in events}):
        records=[r for r in events if r['role']==role]
        key=lambda r:(r['seq'],r['frame']) # Reopened consumers restart seq; core frame IDs keep advancing.
        commits={key(r) for r in records if r['kind']=='commit'}
        completed={key(r) for r in records if r['kind']=='complete'}
        handlers=[r for r in records if r['kind']=='presented']
        valid=lambda r:math.isfinite(float(r['gpu_or_presented_host_s'])) and float(r['gpu_or_presented_host_s'])>0
        positive={key(r) for r in handlers if valid(r)}
        presented=[float(r['gpu_or_presented_host_s']) for r in handlers if valid(r) and lo<=float(r['gpu_or_presented_host_s'])<=hi]
        schedule=[float(r['callback_host_s']) for r in records if r['kind']=='schedule' and lo<=float(r['callback_host_s'])<=hi]
        intervals=[r for r in rows if r['record_type']=='shared_interval' and r['role']==role]
        pauses=[]
        for gap in gaps:
            end=float(gap['host_s']);start=end-int(gap['gap_us'])/1e6
            generated=[int(r['frame']) for r in records if r['kind']=='commit' and start<=float(r['callback_host_s'])<=end]
            shown=[r for r in handlers if valid(r) and start<=float(r['gpu_or_presented_host_s'])<=end]
            pauses.append(dict(start_host_s=start,end_host_s=end,ui_gap_ms=int(gap['gap_us'])/1000,
                               committed_frames=len(set(generated)),source_frame_range=[min(generated),max(generated)] if generated else None,
                               positive_presentation_reports=len(shown)))
        result['roles'][role]=dict(schedule_gap_ms=ms(max_gap(schedule)),presentation_gap_ms=ms(max_gap(presented)),positive_presentation_samples=len(presented),
            zero_callbacks_scored=sum(float(r['gpu_or_presented_host_s'])==0 and lo<=float(r['callback_host_s'])<=hi for r in handlers),
            zero_callbacks_whole=sum(float(r['gpu_or_presented_host_s'])==0 for r in handlers),
            committed_whole=len(commits),missing_completions=sorted(commits-completed),missing_presented_handlers=sorted(commits-{key(r) for r in handlers}),
            without_positive_presentation=sorted(commits-positive),max_inflight=max(int(r['max_inflight']) for r in intervals),
            busy=max(int(r['busy']) for r in intervals),errors=max(int(r['errors']) for r in intervals),lost=max(int(r['lost']) for r in intervals),
            stop=[r for r in rows if r['record_type']=='shared_stop' and r['role']==role],during_ui_gaps=pauses)
    return result


def image_samples(folder):
    timing=[json.loads(l) for l in (folder/'timing.jsonl').read_text().splitlines()]
    samples=[]
    for r in timing:
        if r['returncode']:continue
        path=folder/r['file']
        info=json.loads(subprocess.check_output(['ffprobe','-v','error','-show_entries','stream=width,height','-of','json',str(path)]))['streams'][0]
        w,h=info['width']//4,info['height']//4
        raw=subprocess.check_output(['ffmpeg','-v','error','-i',str(path),'-vf',f'scale={w}:{h}','-f','rawvideo','-pix_fmt','rgb24','-'])
        points=[(i//3)%w for i in range(20*w*3,len(raw),3) if raw[i]>200 and raw[i+1]>200 and raw[i+2]<140]
        samples.append(dict(**r,marker_mean_x=round(sum(points)/len(points),4) if points else None))
    repeated=[dict(first=a['file'],last=b['file'],request_span_ms=(b['start_host_s']-a['start_host_s'])*1000)
              for a,b in zip(samples,samples[1:]) if a['marker_mean_x'] is not None and a['marker_mean_x']==b['marker_mean_x']]
    return dict(samples=len(samples),missing_marker=sum(r['marker_mean_x'] is None for r in samples),
                repeated_marker_pairs=repeated,max_capture_spacing_ms=ms(max_gap([r['start_host_s'] for r in samples])),records=samples)


def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('directory',type=Path);ap.add_argument('--images',action='store_true');args=ap.parse_args()
    result={}
    for case in ['preview','single','idle','operator','left','right','geometry','lifecycle']:
        path=args.directory/(case+'.log')
        if path.exists():result[case]=summarize(path)
        if args.images and case in result:
            result[case]['captures']={}
            for folder in args.directory.glob(case+'.*.frames'):
                role=folder.name.split('.')[1]
                result[case]['captures'][role]=image_samples(folder)
    print(json.dumps(result,indent=2))


if __name__=='__main__':main()
