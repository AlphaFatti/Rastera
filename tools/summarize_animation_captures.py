#!/usr/bin/env python3
"""Summarize controls timestamp-region holds with capture acquisition bounds.

Requires ffmpeg; fixed 1800x800 controls-window captures only. Hash equality
means the same captured timestamp/callback pixels, not physical scanout timing.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def holds(samples):
    groups = []
    for sample in samples:
        if groups and groups[-1][-1]['sha256'] == sample['sha256']:
            groups[-1].append(sample)
        else:
            groups.append([sample])
    result = []
    for index, group in enumerate(groups):
        if len(group) < 2:
            continue
        # Adjacent different images bound the entire hold, when both exist.
        previous = groups[index-1][-1] if index else None
        following = groups[index+1][0] if index+1 < len(groups) else None
        result.append(dict(first_file=group[0]['file'], last_file=group[-1]['file'], samples=len(group),
                           request_start_span_ms=(group[-1]['start_us']-group[0]['start_us'])/1000,
                           acquisition_lower_bound_ms=max(0, group[-1]['start_us']-group[0]['end_us'])/1000,
                           freeze_upper_bound_ms=(following['end_us']-previous['start_us'])/1000
                           if previous and following else None))
    return result


def summarize(directory):
    timing = [json.loads(line) for line in (directory/'timing.jsonl').read_text().splitlines()]
    samples = []
    for record in timing:
        if record['returncode']:
            continue
        path = directory/record['file']
        raw = subprocess.check_output(['ffmpeg', '-v', 'error', '-i', str(path),
                                      '-vf', 'crop=1700:130:10:100',
                                      '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'])
        samples.append(dict(**record, sha256=hashlib.sha256(raw).hexdigest()))
    return dict(samples=len(samples), failed_captures=sum(r['returncode'] != 0 for r in timing),
                max_capture_duration_ms=max((r['end_us']-r['start_us'] for r in samples), default=0)/1000,
                max_request_spacing_ms=max((b['start_us']-a['start_us'] for a,b in zip(samples,samples[1:])), default=0)/1000,
                repeated_timestamp_regions=holds(samples), records=samples)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    print(json.dumps(summarize(args.directory)))


if __name__ == '__main__':
    main()
