#!/usr/bin/env python3
"""Generate the Guaca source icon: two agents, looking at each other.

Pure stdlib on purpose: the repo should build its own assets on a clean machine
without Pillow, ImageMagick, or a design tool in the path. Writes a 1024x1024
RGBA PNG; `cargo tauri icon` derives every platform size from it.

What the icon is of, which was the question and not the drawing: the name says
avocado, the app says a round creature made of clay whose eyes do the acting,
and the product says agents talking to agents. It says the third. An avocado is
a fruit anybody can draw and a creature is a mascot anybody can draw; two agents
aimed at each other is the only one of the three that is this app and not some
other one. `scripts/icon-variants.html` is the comparison that settled it and
holds the nine it was chosen over.

The bodies are the app's own species, not a picture of it. `radius` below is
`bodyPoints` from `src/avatars/form.ts` with the gaze term kept and the animated
terms dropped, so the two shapes here are two agents at rest leaning after a
look, drawn by the same numbers that draw them in the window. That is the whole
reason the icon cannot drift from the product: change the species and the icon
changes with it.

One difference from the webview, and it is in this file's favor. The app strokes
a Catmull-Rom through 28 sampled radii because it is redrawing every creature
every frame and needs a path. Nothing here is animated, so a point is tested
against the exact radial curve instead: `inside` inverts the seating transform
and compares one distance to one radius. No polygon, no facets at 1024px.

  ./scripts/make-icon.py                 writes scripts/icon-source.png
  ./scripts/make-icon.py path/to/out.png writes somewhere else
  ./scripts/make-icon.py --preview       also writes 128, 64, 32 and 16 next to it
"""

from __future__ import annotations

import math
import struct
import sys
import zlib
from pathlib import Path

SIZE = 1024
SUPERSAMPLE = 3  # 3x3 samples per pixel, enough to kill visible stair-stepping

# Every color is a token from `src/styles.css`. The tile is the app's ink, the
# talking agent is the one amber the app spends on anything that wants a person,
# and the listening one is the rail: ink, paper and amber, which is the entire
# color system. A fourth color here would be a color the app does not have.
INK = (0x0B, 0x0B, 0x0A, 255)
AMBER = (0xB4, 0x53, 0x0A, 255)
RAIL = (0xF5, 0xF3, 0xEE, 255)

# macOS draws app icons into 824 of a 1024 canvas and every native neighbor in
# the dock obeys it. Without the padding this icon is simply bigger than the
# ones beside it, which is the most common way a hand-built icon looks wrong
# while every individual decision inside it looks right.
CONTENT = 824 / 1024

# Everything below is in the 64-unit box `src/avatars/form.ts` draws a creature
# in, so the numbers here and the numbers in the preview page are the same
# numbers. FORM.box, FORM.center, FORM.radius.
BOX = 64.0
CENTER = 32.0
RADIUS = 20.0

# The corner. n = 5 is close to the macOS continuous corner, and closer than the
# circular arc this file used to draw.
CORNER_N = 5.0

# The species, tuned for the icon: a k=1 lobe at phase 0 is wide at the bottom
# and narrow at the top, which is an avocado without anybody having drawn one.
AX, AY = 0.80, 1.10
SIG = ((1, 0.105, 0.0), (2, 0.022, 0.0))

# How hard the mass follows a look. `PULL` in form.ts, and it has to stay equal
# to it: the lean and the swell being one displacement is what makes a body read
# as pulled rather than moved.
SWELL, FLATTEN, LEAN, WIDTH = 0.8, 0.32, 0.1, 0.7

# The composition, and the only part of this file that is a decision about *this*
# icon rather than about the species.
#
# Two agents. What the icon says lives in the relationship rather than in either
# shape, so three numbers carry it and the rest are consequences: the masses are
# clearly unequal, or the pair reads as one thing split in half; they sit on a
# diagonal, so neither is the subject; and the gap between them is real, because
# a gap is what makes two shapes two shapes at 32 pixels.
#
# `scripts/icon-variants.html` previews the same numbers in the browser. This
# file is the one that ships, so it is the one to edit; the page is a viewer.
BIG = {
    "span": 29.0,
    "at": (25.5, 36.2),
    "gaze": (0.22, -0.16),
    "eye": 0.20,
    "look": (0.15, -0.11),
    "color": AMBER,
}
SMALL = {
    "span": 18.5,
    "at": (42.6, 23.4),
    "gaze": (-0.22, 0.16),
    "eye": 0.22,
    "look": (-0.13, 0.10),
    "color": RAIL,
}

TAU = math.tau


def press(a: float, th: float, w: float, amp: float) -> float:
    """A gaussian thumbprint at one angle. form.ts, verbatim."""
    d = a - th * TAU
    while d > math.pi:
        d -= TAU
    while d < -math.pi:
        d += TAU
    return amp * math.exp(-(d * d) / (w * w))


def radius(a: float, gaze: tuple[float, float]) -> float:
    """The body's radius at angle `a`, as a multiple of `RADIUS`."""
    gx, gy = gaze
    reach = math.hypot(gx, gy)
    rr = 1.0
    for k, amp, phase in SIG:
        rr += amp * math.sin(k * a + phase)
    if reach > 0.004:
        towards = math.atan2(gy, gx) / TAU
        rr += press(a, towards, WIDTH, reach * SWELL)
        rr += press(a, towards + 0.5, WIDTH + 0.1, -reach * FLATTEN)
    return rr


class Body:
    """One agent, seated in the box.

    `seat` in the preview page: the outline is scaled so its longest dimension
    is `span` and its bounding box is centered on `at`. Sizing every body
    through one span rather than through a hand-picked scale is what keeps two
    of them honestly comparable, and it is why changing the species cannot
    quietly make one of the two bigger than the other.
    """

    def __init__(self, spec: dict) -> None:
        self.gaze = spec["gaze"]
        gx, gy = self.gaze
        self.cx = CENTER + gx * RADIUS * LEAN
        self.cy = CENTER + gy * RADIUS * LEAN

        xs, ys = [], []
        for i in range(720):
            a = (i / 720) * TAU
            rr = radius(a, self.gaze) * RADIUS
            xs.append(self.cx + math.cos(a) * rr * AX)
            ys.append(self.cy + math.sin(a) * rr * AY)
        x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)

        self.k = spec["span"] / max(x1 - x0, y1 - y0)
        self.mx, self.my = (x0 + x1) / 2, (y0 + y1) / 2
        self.tx, self.ty = spec["at"]
        self.color = spec["color"]

        self.w = (x1 - x0) * self.k
        self.h = (y1 - y0) * self.k
        self.eye = (
            self.tx + self.w * spec["look"][0],
            self.ty + self.h * spec["look"][1],
            self.w * spec["eye"],
        )

    def inside(self, x: float, y: float) -> bool:
        """Point in the body. Inverts the seating, then one distance test."""
        u = (self.mx + (x - self.tx) / self.k - self.cx) / AX
        v = (self.my + (y - self.ty) / self.k - self.cy) / AY
        d = math.hypot(u, v)
        if d > RADIUS * 1.4:  # Cheap reject; nothing is ever out this far.
            return False
        return d <= RADIUS * radius(math.atan2(v, u), self.gaze)

    def in_eye(self, x: float, y: float) -> bool:
        ex, ey, er = self.eye
        return (x - ex) ** 2 + (y - ey) ** 2 <= er * er


BODIES = [Body(BIG), Body(SMALL)]


def in_tile(x: float, y: float) -> bool:
    """Superellipse, in box units."""
    a = BOX / 2
    return (abs(x - CENTER) / a) ** CORNER_N + (abs(y - CENTER) / a) ** CORNER_N <= 1.0


def sample(px: float, py: float) -> tuple[int, int, int, int]:
    """Color at one sub-pixel position, in pixels."""
    # Into the 64-unit box, then back out of the macOS content inset.
    off = (1.0 - CONTENT) * BOX / 2.0
    x = (px / SIZE * BOX - off) / CONTENT
    y = (py / SIZE * BOX - off) / CONTENT

    if not in_tile(x, y):
        return (0, 0, 0, 0)
    # Drawn back to front: the smaller agent is nearer, so it is tested first.
    for body in reversed(BODIES):
        if body.inside(x, y):
            # An eye is a hole punched through to the tile, not a dark shape
            # laid on top of one. Two colors per body, and none of them a third.
            return INK if body.in_eye(x, y) else body.color
    return INK


def render(size: int) -> bytes:
    rows = []
    step = 1.0 / SUPERSAMPLE
    offset = step / 2.0
    weight = SUPERSAMPLE * SUPERSAMPLE
    scale = SIZE / size

    for py in range(size):
        row = bytearray()
        for px in range(size):
            r = g = b = a = 0
            for sy in range(SUPERSAMPLE):
                yy = (py + offset + sy * step) * scale
                for sx in range(SUPERSAMPLE):
                    xx = (px + offset + sx * step) * scale
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


def write(path: Path, size: int) -> None:
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(render(size), 9))
        + chunk(b"IEND", b"")
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)
    print(f"wrote {path} ({size}x{size}, {len(png)} bytes)")


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    out = Path(args[0] if args else "scripts/icon-source.png")
    write(out, SIZE)

    if "--preview" in sys.argv:
        # Supersampled from the source geometry rather than downscaled from the
        # 1024, which is the more pessimistic of the two and the one worth
        # looking at: if the composition holds here it holds in the dock.
        for size in (128, 64, 32, 16):
            write(out.with_name(f"{out.stem}-{size}{out.suffix}"), size)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
