#!/usr/bin/env python3
"""Generate the three menu bar glyphs.

Pure stdlib, for the same reason as `make-icon.py`: the repo builds its own
assets on a clean machine. Writes 36x36 RGBA PNGs into `src-tauri/icons/`,
which `tray.rs` embeds at compile time.

36 pixels because the tray crate scales whatever it is given to 18 points tall,
so 36 is exactly 2x and anything larger is a resample for nothing.

Three glyphs, and the difference between them has to survive being 18 points
tall on a strip shared with a dozen other apps:

  idle        an outline. Present, quiet, nothing happening.
  working     the same shape filled, with the pit punched out of it. Mass is
              the one difference that reads at this size without colour.
  attention   filled, in warm red, and the only one that is not a template
              image. macOS tints a template image to match the menu bar, so a
              template glyph cannot be a colour; giving up the tint is what
              buys the one state that must not be missed. The count beside the
              icon says the same thing in text, for anyone the colour does not
              reach.

Template tinting is macOS-only. When there is a Windows build, `idle` and
`working` need a light variant and something to pick between them: pure black
on a dark taskbar is an icon nobody can see, and there is nothing to tint it.
`attention` already carries its own colour and needs nothing.

  ./scripts/make-tray.py            writes the three glyphs
  ./scripts/make-tray.py --preview  also writes 8x copies to eyeball
"""

from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path

SIZE = 36
SUPERSAMPLE = 4

# Warm red rather than either of the app's own alarm tokens. The light one is
# too dark to read against a dark menu bar and the dark one is too pale against
# a light one, and the menu bar is the one surface that is both.
ALARM = (0xD9, 0x5F, 0x43)

INK = (0x00, 0x00, 0x00)

# The glyph, in fractions of the canvas. Inset top and bottom so the shape does
# not touch the edges of the menu bar.
TOP = 0.085
BOTTOM = 0.915
WIDEST = 0.315

# Where the pit sits, and how big it is. Low, like the stone in the fruit.
PIT_AT = 0.615
PIT_R = 0.098

# The outline's weight. 3 pixels of 36 is 1.5 points, which is the line the rest
# of the menu bar is drawn with.
STROKE = 3.0 / SIZE


def profile(t: float) -> float:
    """Half-width at height `t`, 0 at the top and 1 at the bottom.

    A skewed superellipse: the exponent rounds the ends so the shape does not
    come to a point, and the skew makes the bottom heavier than the top, which
    is the whole difference between an avocado and an egg.
    """
    if t <= 0.0 or t >= 1.0:
        return 0.0
    span = abs(2.0 * t - 1.0)
    return (1.0 - span**2.4) ** 0.5 * (0.60 + 0.40 * t**0.9)


def inside(x: float, y: float, shrink: float) -> bool:
    """Point in the silhouette, optionally eroded by `shrink` of the canvas."""
    top, bottom = TOP + shrink, BOTTOM - shrink
    if not (top <= y <= bottom):
        return False
    t = (y - top) / (bottom - top)
    return abs(x - 0.5) <= profile(t) * (WIDEST - shrink)


def in_pit(x: float, y: float) -> bool:
    cy = TOP + (BOTTOM - TOP) * PIT_AT
    return (x - 0.5) ** 2 + (y - cy) ** 2 <= PIT_R**2


def idle(x: float, y: float) -> tuple[int, int, int, int]:
    return (*INK, 255) if inside(x, y, 0.0) and not inside(x, y, STROKE) else (0, 0, 0, 0)


def working(x: float, y: float) -> tuple[int, int, int, int]:
    return (*INK, 255) if inside(x, y, 0.0) and not in_pit(x, y) else (0, 0, 0, 0)


def attention(x: float, y: float) -> tuple[int, int, int, int]:
    return (*ALARM, 255) if inside(x, y, 0.0) and not in_pit(x, y) else (0, 0, 0, 0)


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
                    # Premultiplied, so a transparent sample drags no colour in.
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
    out = here / "src-tauri" / "icons"
    glyphs = {"tray-idle": idle, "tray-working": working, "tray-attention": attention}

    for name, sample in glyphs.items():
        write(out / f"{name}.png", sample, SIZE)

    if "--preview" in sys.argv:
        for name, sample in glyphs.items():
            write(here / ".context" / f"{name}-preview.png", sample, SIZE * 8)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
