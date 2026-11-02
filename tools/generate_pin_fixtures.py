#!/usr/bin/env python3
"""Numbered RGBA grid fixture; no optional Python imaging dependency."""
import struct
import sys
from pathlib import Path
import zlib
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
folder = Path(sys.argv[1]).resolve()
folder.mkdir(parents=True, exist_ok=True)
w, h = 960, 540
data = bytearray([240, 240, 240, 255]) * (w*h)
# Compact 3x5 number glyphs, enlarged eight times.
digits = ['111101101101111','010110010010111','111001111100111','111001111001111','101101111001001','111100111001111','111100111101111','111001001001001','111101111101111','111101111001111']
for y in range(h):
    for x in range(w):
        i = (y*w+x)*4
        if x % 120 < 2 or y % 90 < 2:
            data[i:i+4] = bytes([15, 20, 30, 255])
        elif (x//120+y//90) % 2:
            data[i:i+4] = bytes([100, 210, 230, 255])
for row in range(6):
    for col in range(8):
        for n, char in enumerate(str(row*8+col+1)):
            glyph = digits[int(char)]
            for gy in range(5):
                for gx in range(3):
                    if glyph[gy*3+gx] == '1':
                        for dy in range(8):
                            for dx in range(8):
                                x=col*120+20+n*32+gx*8+dx;y=row*90+20+gy*8+dy;i=(y*w+x)*4;data[i:i+4]=bytes([30,10,80,255])
def chunk(tag, payload):
    return struct.pack('!I',len(payload))+tag+payload+struct.pack('!I',zlib.crc32(tag+payload)&0xffffffff)
raw=b''.join(b'\0'+data[y*w*4:(y+1)*w*4] for y in range(h))
(folder/'grid.png').write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!2I5B',w,h,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b''))
shutil.copy2(root/'examples/alpha.png',folder/'alpha.png')
subprocess.run([str(root/'tools/generate_media_fixtures.sh'),str(folder)],check=True)
