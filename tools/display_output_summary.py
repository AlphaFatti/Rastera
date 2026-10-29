#!/usr/bin/env python3
"""Summarize logged native evidence by logical viewport and consumer instance.
Zero timestamps and commits without a logged presented callback are retained.
Callback arrival times are never interpreted as presentation times.
"""
import argparse
import collections
import hashlib
import json
import re
import struct
import subprocess
from pathlib import Path

def marker(path):
    width,_=struct.unpack('>II',path.read_bytes()[16:24])
    raw=subprocess.check_output(['ffmpeg','-v','error','-i',str(path),'-f','rawvideo','-pix_fmt','rgb24','-'])
    points=[i for i,(r,g,b) in enumerate(zip(raw[::3],raw[1::3],raw[2::3])) if r>220 and g>180 and b<60]
    return dict(pixels=len(points),mean_x=sum(i%width for i in points)/len(points) if points else None,bounds=[min(i%width for i in points),min(i//width for i in points),max(i%width for i in points),max(i//width for i in points)] if points else None)

def summarize(folder,markers=False):
    instances={}
    gaps=[]
    shutdown=[]
    lines=(folder/'editor.log').read_text().splitlines()
    for line in lines:
        fields=dict(re.findall(r'(\w+)=([^ ]+)',line))
        if line.startswith('shared_event '):
            key=(int(fields['viewport_id']),int(fields['instance_id']))
            item=instances.setdefault(key,collections.defaultdict(list))
            item[fields['kind']].append(fields)
        elif line.startswith('shared_ui_gap '):gaps.append(int(fields['gap_us'])/1000)
        elif line.startswith('shared_shutdown '):shutdown.append(int(fields['outstanding_slots']))
    rows=[]
    for (viewport,instance),events in sorted(instances.items()):
        presented=events.get('presented',[])
        times=sorted(float(e['gpu_or_presented_host_s']) for e in presented if float(e['gpu_or_presented_host_s'])>0)
        callbacks=[float(e['callback_host_s']) for e in presented]
        commit_keys={e['seq'] for e in events.get('commit',[])}
        callback_keys={e['seq'] for e in presented}
        rows.append(dict(viewport_id=viewport,instance_id=instance,event_counts={k:len(v) for k,v in events.items()},zero_presentation_timestamps=sum(float(e['gpu_or_presented_host_s'])==0 for e in presented),positive_presentation_timestamps=len(times),unmatched_logged_drawable_commits=len(commit_keys-callback_keys),max_positive_api_presentation_gap_ms=max(((b-a)*1000 for a,b in zip(times,times[1:])),default=None),max_presentation_callback_arrival_gap_ms=max(((b-a)*1000 for a,b in zip(callbacks,callbacks[1:])),default=None)))
    return dict(command=f'python3 tools/display_output_probe.py --out {folder}'+(' --capture' if (folder/'idle.png').exists() else ''),passed=any('display_output_probe passed=1' in line for line in lines),native_assertions=json.loads((folder/'display-results.json').read_text()),instances=rows,max_logged_ui_gap_over_100ms_ms=max(gaps,default=None),shutdown_outstanding_slots=shutdown,captures=[dict(file=p.name,sha256=hashlib.sha256(p.read_bytes()).hexdigest(),**({'yellow_marker':marker(p)} if markers else {})) for p in sorted(folder.glob('*.png'))],driver_events=json.loads((folder/'driver-events.json').read_text()),limitations=['API evidence is not physical scanout or human picture acceptance.','Whole-run gaps include intentional mode changes, retirement and application switching.','Missing logged callbacks do not establish physical frame drops.','Capture request/return times bound observations and can perturb playback.'])

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('folders',nargs='+',type=Path)
    parser.add_argument('--markers',action='store_true',help='Offline yellow-marker analysis using installed ffmpeg; no runtime app dependency')
    args=parser.parse_args()
    print(json.dumps([summarize(p,args.markers) for p in args.folders],indent=2))

if __name__=='__main__':main()
