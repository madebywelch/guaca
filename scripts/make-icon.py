#!/usr/bin/env python3

from __future__ import annotations

import math
import re
import struct
import sys
import zlib
from pathlib import Path

SIZE = 1024
SUPERSAMPLE = 3

ROOT = Path(__file__).resolve().parent.parent


def light_tokens() -> dict[str, str]:
    css = (ROOT / "src" / "styles.css").read_text()
    root = re.search(r"^:root \{\n(.*?)^\}", css, re.M | re.S)
    if root is None:
        raise SystemExit("src/styles.css has no :root block to read the icon's colors from")
    return dict(re.findall(r"^\s*--([a-z-]+): (#[0-9a-fA-F]{6});$", root.group(1), re.M))


def token(tokens: dict[str, str], name: str) -> tuple[int, int, int, int]:
    if name not in tokens:
        raise SystemExit(f"src/styles.css has no --{name} in its :root block, and the icon is drawn in it")
    value = tokens[name]
    return (int(value[1:3], 16), int(value[3:5], 16), int(value[5:7], 16), 255)


TOKENS = light_tokens()
PAPER = token(TOKENS, "ground")
INK = token(TOKENS, "text")
AMBER = token(TOKENS, "attention")

CONTENT = 824 / 1024

BOX = 64.0
CENTER = 32.0
RADIUS = 20.0

CORNER_N = 5.0

RIM = 0.6

AX, AY = 0.80, 1.10
SIG = ((1, 0.105, 0.0), (2, 0.022, 0.0))

SWELL, FLATTEN, WIDTH = 0.8, 0.32, 0.7

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
    "color": INK,
}

TAU = math.tau


def press(a: float, th: float, w: float, amp: float) -> float:
    d = a - th * TAU
    while d > math.pi:
        d -= TAU
    while d < -math.pi:
        d += TAU
    return amp * math.exp(-(d * d) / (w * w))


def radius(a: float, gaze: tuple[float, float]) -> float:
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

    def __init__(self, spec: dict) -> None:
        self.gaze = spec["gaze"]
        self.cx = CENTER
        self.cy = CENTER

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
        u = (self.mx + (x - self.tx) / self.k - self.cx) / AX
        v = (self.my + (y - self.ty) / self.k - self.cy) / AY
        d = math.hypot(u, v)
        if d > RADIUS * 1.4:
            return False
        return d <= RADIUS * radius(math.atan2(v, u), self.gaze)

    def in_eye(self, x: float, y: float) -> bool:
        ex, ey, er = self.eye
        return (x - ex) ** 2 + (y - ey) ** 2 <= er * er


BODIES = [Body(BIG), Body(SMALL)]


def in_tile(x: float, y: float, inset: float = 0.0) -> bool:
    a = BOX / 2 - inset
    return (abs(x - CENTER) / a) ** CORNER_N + (abs(y - CENTER) / a) ** CORNER_N <= 1.0


def sample(px: float, py: float) -> tuple[int, int, int, int]:
    off = (1.0 - CONTENT) * BOX / 2.0
    x = (px / SIZE * BOX - off) / CONTENT
    y = (py / SIZE * BOX - off) / CONTENT

    if not in_tile(x, y):
        return (0, 0, 0, 0)
    if not in_tile(x, y, RIM):
        return INK
    for body in reversed(BODIES):
        if body.inside(x, y):
            return PAPER if body.in_eye(x, y) else body.color
    return PAPER


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
        for size in (128, 64, 32, 16):
            write(out.with_name(f"{out.stem}-{size}{out.suffix}"), size)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
