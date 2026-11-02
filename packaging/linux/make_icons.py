#!/usr/bin/env python3
"""Generates the Linux icons (hicolor PNGs and an SVG) from the same geometry as src/icon.rs:
a red circle with white headphones. Run it again after changing src/icon.rs:

    python3 packaging/linux/make_icons.py
"""
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
RED = (0xE3, 0x18, 0x37)
SIZES = [16, 24, 32, 48, 64, 128, 256, 512]


def pixels(size):
    """RGBA rows, same supersampled shape as icon::pixels."""
    c, s, ss = size / 2, float(size), 4
    rows = []
    for y in range(size):
        row = bytearray()
        for x in range(size):
            acc, n = [0.0, 0.0, 0.0], 0
            for sy in range(ss):
                for sx in range(ss):
                    px = x + (sx + 0.5) / ss
                    py = y + (sy + 0.5) / ss
                    if ((px - c) ** 2 + (py - c) ** 2) ** 0.5 > c - 0.5:
                        continue
                    hy = c + s * 0.06
                    r = ((px - c) ** 2 + (py - hy) ** 2) ** 0.5
                    band = py < hy and s * 0.27 < r < s * 0.35
                    cup = s * 0.42 < py < s * 0.72 and (s * 0.17 < px < s * 0.32 or s * 0.68 < px < s * 0.83)
                    col = (255, 255, 255) if band or cup else RED
                    for k in range(3):
                        acc[k] += col[k]
                    n += 1
            if n:
                row += bytes(int(a / n) for a in acc) + bytes([255 * n // (ss * ss)])
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    return rows


def png(size, path):
    raw = b"".join(b"\0" + r for r in pixels(size))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def svg(path):
    s = 256.0
    c, hy = s / 2, s / 2 + s * 0.06
    ro, ri = s * 0.35, s * 0.27
    band = (f"M {c - ro:g} {hy:g} A {ro:g} {ro:g} 0 0 1 {c + ro:g} {hy:g} "
            f"L {c + ri:g} {hy:g} A {ri:g} {ri:g} 0 0 0 {c - ri:g} {hy:g} Z")
    cup = lambda x0, x1: (f'<rect x="{s * x0:g}" y="{s * 0.42:g}" width="{s * (x1 - x0):g}" '
                          f'height="{s * 0.30:g}" fill="#fff"/>')
    text = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">\n'
            f'  <circle cx="{c:g}" cy="{c:g}" r="{c - 0.5:g}" fill="#E31837"/>\n'
            f'  <path d="{band}" fill="#fff"/>\n'
            f'  {cup(0.17, 0.32)}\n  {cup(0.68, 0.83)}\n</svg>\n')
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", newline="\n") as f:
        f.write(text)


if __name__ == "__main__":
    for size in SIZES:
        png(size, os.path.join(HERE, "icons", "hicolor", f"{size}x{size}", "apps", "nogenuity.png"))
    svg(os.path.join(HERE, "icons", "hicolor", "scalable", "apps", "nogenuity.svg"))
    print("icons written to", os.path.join(HERE, "icons"))
