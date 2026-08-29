#!/usr/bin/env python3
"""Generate the three menu bar glyphs.

Pure stdlib, for the same reason as `make-icon.py`: the repo builds its own
assets on a clean machine. Writes 36x36 RGBA PNGs into `src-tauri/icons/`,
which `tray.rs` embeds at compile time.

36 pixels because the tray crate scales whatever it is given to 18 points tall,
so 36 is exactly 2x and anything larger is a resample for nothing.

One agent, not the two the app icon carries. The dock has the room to say what
the app is; the menu bar gets the protagonist alone, because two outlined
figures at 18 points is a texture rather than a glyph. Same species and the same
lean, imported from `make-icon.py` rather than re-derived here, so the two marks
stay one family and editing the icon's shape moves this one with it.

Three glyphs, and the difference between them has to survive being 18 points
tall on a strip shared with a dozen other apps:

  idle        an outline. Present, quiet, nothing happening.
  working     the same shape filled, with the pit punched out of it. Mass is
              the one difference that reads at this size without color.
  attention   filled, in warm red, and the only one that is not a template
              image. macOS tints a template image to match the menu bar, so a
              template glyph cannot be a color; giving up the tint is what
              buys the one state that must not be missed. The count beside the
              icon says the same thing in text, for anyone the color does not
              reach.

Template tinting is macOS-only. When there is a Windows build, `idle` and
`working` need a light variant and something to pick between them: pure black
on a dark taskbar is an icon nobody can see, and there is nothing to tint it.
`attention` already carries its own color and needs nothing.

  ./scripts/make-tray.py            writes the three glyphs
  ./scripts/make-tray.py --preview  also writes 8x copies to eyeball
"""

from __future__ import annotations

import importlib.util
import math
import struct
import sys
import zlib
from pathlib import Path

# The species lives in the icon's generator, and the glyph is the same shape at
# another size. Loaded by path because the file it is in has a hyphen in its
# name, which is right for something run as a script and not importable as one.
# Bytecode off first: this is a two-file build script, and it should not leave a
# __pycache__ in a directory that has never had one.
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location(
    "make_icon", Path(__file__).resolve().parent / "make-icon.py"
)
make_icon = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(make_icon)

SIZE = 36
SUPERSAMPLE = 4

# Warm red rather than either of the app's own alarm tokens. The light one is
# too dark to read against a dark menu bar and the dark one is too pale against
# a light one, and the menu bar is the one surface that is both.
ALARM = (0xD9, 0x5F, 0x43)

INK = (0x00, 0x00, 0x00)

# How much of the canvas the glyph fills, in the 64-unit box the species is
# drawn in. Short of the full height so the shape does not touch the edges of
# the menu bar, which is what makes a glyph look pasted on rather than set.
SPAN = 52.0

# The outline's weight, in box units. 3 pixels of 36 is 1.5 points, which is the
# line the rest of the menu bar is drawn with.
STROKE = 3.0 / SIZE * make_icon.BOX

# The app icon's protagonist, re-seated to fill this canvas. The lean, the
# taper and the eye all come from there; only the size and the seat are local.
GLYPH = make_icon.Body({**make_icon.BIG, "span": SPAN, "at": (32.0, 32.0)})


def to_box(x: float, y: float) -> tuple[float, float]:
    """Canvas fractions into the 64-unit box the species is drawn in."""
    return x * make_icon.BOX, y * make_icon.BOX


def inside(x: float, y: float, shrink: float) -> bool:
    """Point in the silhouette, optionally inset by `shrink` box units.

    The inset is a fixed distance taken off the radius, not a smaller copy of
    the whole shape. Scaling the shape down about its own center takes off a
    share of the local radius, so the band it leaves is thick where the body is
    wide and thin where it tapers, which on a leaning body is a brush stroke
    rather than an outline and breaks first at the thin end. Here the radius at
    each angle is reduced by the same amount, corrected for the body's own
    stretch so that a constant in the species' space is a constant on screen.
    """
    bx, by = to_box(x, y)
    u = (GLYPH.mx + (bx - GLYPH.tx) / GLYPH.k - GLYPH.cx) / make_icon.AX
    v = (GLYPH.my + (by - GLYPH.ty) / GLYPH.k - GLYPH.cy) / make_icon.AY
    d = math.hypot(u, v)
    if d == 0.0:
        return True
    limit = make_icon.RADIUS * make_icon.radius(math.atan2(v, u), GLYPH.gaze)
    if shrink:
        # Box units per unit of radius in this direction, so the stroke is the
        # width it was asked for on every side of a body that is not a circle.
        stretch = math.hypot(make_icon.AX * u / d, make_icon.AY * v / d) * GLYPH.k
        limit -= shrink / stretch
    return d <= limit


def in_pit(x: float, y: float) -> bool:
    """The eye, which is the pit. One hole, and the same one in both marks."""
    return GLYPH.in_eye(*to_box(x, y))


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
                    # Premultiplied, so a transparent sample drags no color in.
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
