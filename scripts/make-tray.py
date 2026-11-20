#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import math
import struct
import sys
import zlib
from pathlib import Path

sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location(
    "make_icon", Path(__file__).resolve().parent / "make-icon.py"
)
make_icon = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(make_icon)

SIZE = 36
SUPERSAMPLE = 4

ATTENTION = (0xFF, 0xB3, 0x46)

INK = (0x00, 0x00, 0x00)

SPAN = 52.0

STROKE = 3.0 / SIZE * make_icon.BOX

GLYPH = make_icon.Body({**make_icon.BIG, "span": SPAN, "at": (32.0, 32.0)})


def to_box(x: float, y: float) -> tuple[float, float]:
    return x * make_icon.BOX, y * make_icon.BOX


def inside(x: float, y: float, shrink: float) -> bool:
    bx, by = to_box(x, y)
    u = (GLYPH.mx + (bx - GLYPH.tx) / GLYPH.k - GLYPH.cx) / make_icon.AX
    v = (GLYPH.my + (by - GLYPH.ty) / GLYPH.k - GLYPH.cy) / make_icon.AY
    d = math.hypot(u, v)
    if d == 0.0:
        return True
    limit = make_icon.RADIUS * make_icon.radius(math.atan2(v, u), GLYPH.gaze)
    if shrink:
        stretch = math.hypot(make_icon.AX * u / d, make_icon.AY * v / d) * GLYPH.k
        limit -= shrink / stretch
    return d <= limit


def in_pit(x: float, y: float) -> bool:
    return GLYPH.in_eye(*to_box(x, y))


def idle(x: float, y: float) -> tuple[int, int, int, int]:
    return (*INK, 255) if inside(x, y, 0.0) and not inside(x, y, STROKE) else (0, 0, 0, 0)


def working(x: float, y: float) -> tuple[int, int, int, int]:
    return (*INK, 255) if inside(x, y, 0.0) and not in_pit(x, y) else (0, 0, 0, 0)


def attention(x: float, y: float) -> tuple[int, int, int, int]:
    return (*ATTENTION, 255) if inside(x, y, 0.0) and not in_pit(x, y) else (0, 0, 0, 0)


def render(sample, size: int) -> bytes:
    rows = []
    step = 1.0 / SUPERSAMPLE
    offset = step / 2.0
    weight = SUPERSAMPLE * SUPERSAMPLE

    for py in range(size):
        row = bytearray()
        for px in range(size):
            r = g = b = a = 0
            for sy in range(SUPERSAMPLE):
                yy = (py + offset + sy * step) / size
                for sx in range(SUPERSAMPLE):
                    xx = (px + offset + sx * step) / size
                    cr, cg, cb, ca = sample(xx, yy)
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


def write(path: Path, sample, size: int) -> None:
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(render(sample, size), 9))
        + chunk(b"IEND", b"")
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)
    print(f"wrote {path} ({size}x{size}, {len(png)} bytes)")


def main() -> int:
    here = Path(__file__).resolve().parent.parent
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    out = Path(args[0]) if args else here / "src-tauri" / "icons"
    glyphs = {"tray-idle": idle, "tray-working": working, "tray-attention": attention}

    for name, sample in glyphs.items():
        write(out / f"{name}.png", sample, SIZE)

    if "--preview" in sys.argv:
        for name, sample in glyphs.items():
            write(here / ".context" / f"{name}-preview.png", sample, SIZE * 8)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
