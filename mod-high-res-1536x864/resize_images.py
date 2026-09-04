#!/usr/bin/env python3
"""Produce this mod's three 8-bit BMPs from the 1600 x 900 set next door.

The game's screen art is 8-bit paletted, uncompressed BMP, and the three pieces follow
the screen size: the full map background is WIDTH x HEIGHT, the top bar (HauptscreenA)
is (WIDTH - 284) x 42, and the right-hand bottom piece (HauptscreenE) is
284 x (HEIGHT - 600). The palette is kept: with Pillow installed the pixels are resampled
in RGB and quantised back to the source palette; without it a nearest-neighbour resample
of the palette indices is used.

Usage: resize_images.py [width height]   (default 1536 864)
"""
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = os.path.join(HERE, "..", "mod-high-res-1600x900", "src")
WIDTH, HEIGHT = (int(sys.argv[1]), int(sys.argv[2])) if len(sys.argv) == 3 else (1536, 864)
TAG = str(WIDTH)

JOBS = [
    ("Vollansichtskarte1600.bmp", f"Vollansichtskarte{TAG}.bmp", WIDTH, HEIGHT),
    ("HauptscreenA1600.bmp", f"HauptscreenA{TAG}.bmp", WIDTH - 284, 42),
    ("HauptscreenE1600.bmp", f"HauptscreenE{TAG}.bmp", 284, HEIGHT - 600),
]


def read_bmp8(path):
    d = open(path, "rb").read()
    off = struct.unpack_from("<I", d, 10)[0]
    w, h = struct.unpack_from("<ii", d, 18)
    bpp = struct.unpack_from("<H", d, 28)[0]
    assert bpp == 8, f"{path}: {bpp} bpp, expected 8"
    palette = d[54:54 + 1024]
    stride = (w + 3) & ~3
    rows = [d[off + y * stride: off + y * stride + w] for y in range(abs(h))]
    if h > 0:  # bottom-up storage
        rows.reverse()
    return palette, rows


def write_bmp8(path, palette, rows):
    w, h = len(rows[0]), len(rows)
    stride = (w + 3) & ~3
    pixels = b"".join(bytes(r) + b"\0" * (stride - w) for r in reversed(rows))
    size = 54 + 1024 + len(pixels)
    header = struct.pack("<2sIHHI", b"BM", size, 0, 0, 54 + 1024)
    info = struct.pack("<IiiHHIIiiII", 40, w, h, 1, 8, 0, len(pixels), 2835, 2835, 256, 256)
    open(path, "wb").write(header + info + palette + pixels)


def nearest(rows, w, h):
    sh, sw = len(rows), len(rows[0])
    xs = [x * sw // w for x in range(w)]
    return [bytes(rows[y * sh // h][x] for x in xs) for y in range(h)]


def resample(palette, rows, w, h):
    try:
        from PIL import Image
    except ImportError:
        print("  Pillow not installed - nearest-neighbour resample (install python3-pil for Lanczos)")
        return nearest(rows, w, h)
    src = Image.frombytes("P", (len(rows[0]), len(rows)), b"".join(rows))
    # BMP palettes are BGRX; Pillow wants RGB triples.
    rgb = b"".join(bytes((palette[i + 2], palette[i + 1], palette[i])) for i in range(0, 1024, 4))
    src.putpalette(rgb)
    resized = src.convert("RGB").resize((w, h), Image.LANCZOS)
    quantised = resized.quantize(palette=src, dither=Image.NONE)
    data = quantised.tobytes()
    return [data[y * w:(y + 1) * w] for y in range(h)]


for source, target, w, h in JOBS:
    palette, rows = read_bmp8(os.path.join(SOURCE, source))
    print(f"{source} {len(rows[0])}x{len(rows)} -> {target} {w}x{h}")
    write_bmp8(os.path.join(HERE, "src", target), palette, resample(palette, rows, w, h))
