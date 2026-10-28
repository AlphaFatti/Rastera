#!/usr/bin/env python3
"""Decode the native picture's binary stripe from app-window screenshots.

Requires ffmpeg/ffprobe. Capture evidence, not physical scanout verification.
Only native_output_probe's fixed 450x280 Retina window layout is supported.
"""
import argparse
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    result = []
    for path in sorted(args.directory.glob('*.png')):
        info = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-show_entries',
                          'stream=width,height', '-of', 'json', str(path)]))['streams'][0]
        width, height = info['width'], info['height']
        if width != 900 or height != 560:
            raise ValueError(f'Unsupported screenshot dimensions: {width}x{height}')
        raw = subprocess.check_output(['ffmpeg', '-v', 'error', '-i', str(path),
                                       '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'])
        # Captured chrome is 57 px; content is 503 px at this fixed layout.
        y = round(57+(height-57)*0.87)
        value = 0
        for bit in range(32):
            x = round(width*(0.02+0.96*(bit+0.5)/32))
            offset = (y*width+x)*3
            if sum(raw[offset:offset+3]) > 384:
                value |= 1 << bit
        result.append(dict(capture_start_us=int(path.stem), content_id=value))
    print(json.dumps(result))


if __name__ == '__main__':
    main()
