#!/usr/bin/env python3
"""Generate minimal placeholder PNG icons for Tauri bundle."""
import struct
import zlib
import os

def make_png(width, height):
    raw = b""
    for _ in range(height):
        raw += bytes([0])
        for _ in range(width):
            raw += bytes([255, 255, 255, 255])
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    def chunk(ct, data):
        c = ct + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)
    png = bytes([137, 80, 78, 71, 13, 10, 26, 10])
    png += chunk(b"IHDR", ihdr)
    png += chunk(b"IDAT", zlib.compress(raw))
    png += chunk(b"IEND", b"")
    return png

icons_dir = os.path.join(os.path.dirname(__file__), "..", "apps", "desktop", "src-tauri", "icons")
icons_dir = os.path.normpath(icons_dir)
os.makedirs(icons_dir, exist_ok=True)

icon_specs = [
    ("32x32.png", 32, 32),
    ("128x128.png", 128, 128),
    ("128x128@2x.png", 256, 256),
]

for name, width, height in icon_specs:
    path = os.path.join(icons_dir, name)
    with open(path, "wb") as f:
        f.write(make_png(width, height))
    print(f"created {name} ({width}x{height})")

print("done")
