#!/usr/bin/env python3
"""Generate the Guac source icon.

Pure stdlib on purpose: the repo should build its own assets on a clean machine
without Pillow, ImageMagick, or a design tool in the path. Writes a 1024x1024
RGBA PNG; `cargo tauri icon` derives every platform size from it.
"""

from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path

SIZE = 1024
SUPERSAMPLE = 3  # 3x3 samples per pixel, enough to kill visible stair-stepping

TILE = (0x14, 0x28, 0x1D, 255)
SKIN = (0x38, 0x66, 0x41, 255)
FLESH = (0xA7, 0xC9, 0x57, 255)
PIT = (0x8B, 0x5E, 0x34, 255)


def rounded_rect(x: float, y: float, w: float, h: float, r: float) -> bool:
    """Point-in-rounded-rectangle for a rect anchored at the origin."""
    if not (0 <= x <= w and 0 <= y <= h):
        return False
    cx = min(max(x, r), w - r)
    cy = min(max(y, r), h - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def ellipse(x: float, y: float, cx: float, cy: float, rx: float, ry: float) -> bool:
    return ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2 <= 1.0


def sample(x: float, y: float) -> tuple[int, int, int, int]:
    """Color at one sub-pixel position."""
    if not rounded_rect(x, y, SIZE, SIZE, SIZE * 0.225):
        return (0, 0, 0, 0)

    # The avocado sits slightly high so the pit reads as the optical center.
    cx, cy = SIZE * 0.5, SIZE * 0.52
    if ellipse(x, y, cx, cy, SIZE * 0.30, SIZE * 0.365):
        if ellipse(x, y, cx, cy, SIZE * 0.235, SIZE * 0.295):
            if ellipse(x, y, cx, cy - SIZE * 0.015, SIZE * 0.105, SIZE * 0.105):
                return PIT
            return FLESH
        return SKIN
    return TILE


def render() -> bytes:
    rows = []
    step = 1.0 / SUPERSAMPLE
    offset = step / 2.0
    weight = SUPERSAMPLE * SUPERSAMPLE

    for py in range(SIZE):
        row = bytearray()
        for px in range(SIZE):
            r = g = b = a = 0
            for sy in range(SUPERSAMPLE):
                yy = py + offset + sy * step
                for sx in range(SUPERSAMPLE):
                    xx = px + offset + sx * step
                    cr, cg, cb, ca = sample(xx, yy)
                    # Premultiply so transparent samples do not drag color in.
                    r += cr * ca
                    g += cg * ca
                    b += cb * ca
                    a += ca
            if a == 0:
                row += b"\x00\x00\x00\x00"
            else:
                row += bytes((r // a, g // a, b // a, a // weight))
        rows.append(bytes(row))
    return b"".join(b"\x00" + row for row in rows)


def chunk(tag: bytes, payload: bytes) -> bytes:
    return (
        struct.pack(">I", len(payload))
        + tag
        + payload
        + struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
    )


def main() -> int:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "scripts/icon-source.png")
    out.parent.mkdir(parents=True, exist_ok=True)

    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(render(), 9))
        + chunk(b"IEND", b"")
    )
    out.write_bytes(png)
    print(f"wrote {out} ({len(png)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
