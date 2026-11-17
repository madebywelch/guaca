#!/usr/bin/env python3
"""Package a signed, stapled Guaca.app into the release disk image.

    scripts/package-dmg.py path/to/Guaca.app path/to/Guaca_X.Y.Z_universal.dmg

The image opens in Finder as a drag-to-install window: 660 by 400, large icons,
Guaca on the left and Applications on the right. Finder reads that arrangement
from the volume's .DS_Store. The usual way to get one is to open the mounted
volume in Finder and arrange it with AppleScript, which takes over the screen
of the Mac doing the release and needs permission to control Finder. This
script writes the .DS_Store itself instead, so packaging opens no window and
runs no AppleScript.

The app is copied with ditto and compared afterward, so the image holds it
byte for byte, stapled ticket included. The image is unsigned; the release
procedure signs and notarizes it next.
"""

from __future__ import annotations

import os
import plistlib
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

VOLUME = "Guaca"
WINDOW = "{{10, 657}, {660, 400}}"
ICON_SIZE = 128.0
TEXT_SIZE = 16.0
APP_AT = (180, 170)
APPLICATIONS_AT = (480, 170)

# The .DS_Store is a buddy-allocated file holding one B-tree. Its blocks sit
# where Finder put them in the 0.4.0 image, and its records decode to the ones
# Finder wrote there; the gate holds them to that.
HEADER = (0x0, 0x20)
DSDB = (0x40, 0x20)
NODE = (0x400, 0x400)
ROOT = (0x1000, 0x800)
SPACE = 1 << 31
FILE_SIZE = 0x1800


def address(block: tuple[int, int]) -> int:
    offset, size = block
    return offset | (size.bit_length() - 1)


def free_blocks(offset: int, size: int, used: list[tuple[int, int]]) -> list[tuple[int, int]]:
    """The buddy allocator's free list: every largest aligned block that holds
    nothing in use, found by halving whatever does."""
    inside = [u for u in used if offset <= u[0] < offset + size]
    if not inside:
        return [(offset, size)]
    if inside == [(offset, size)]:
        return []
    half = size // 2
    return free_blocks(offset, half, used) + free_blocks(offset + half, half, used)


def utf16(text: str) -> bytes:
    data = text.encode("utf-16-be")
    return struct.pack(">I", len(data) // 2) + data


def record(name: str, code: str, kind: str, value: bytes) -> bytes:
    if kind == "blob":
        value = struct.pack(">I", len(value)) + value
    return utf16(name) + code.encode() + kind.encode() + value


def ds_store(records: list[tuple[str, str, str, bytes]]) -> bytes:
    ordered = sorted(records, key=lambda r: (r[0].lower(), r[1]))
    node = struct.pack(">II", 0, len(ordered)) + b"".join(record(*r) for r in ordered)
    if len(node) > NODE[1]:
        raise ValueError(f"the layout needs {len(node)} bytes and its node holds {NODE[1]}")

    blocks = [ROOT, DSDB, NODE]
    root = struct.pack(">II", len(blocks), 0)
    root += struct.pack(">256I", *[address(b) for b in blocks], *[0] * (256 - len(blocks)))
    root += struct.pack(">I", 1) + bytes([4]) + b"DSDB" + struct.pack(">I", blocks.index(DSDB))
    free = free_blocks(0, SPACE, [HEADER, *blocks])
    for exponent in range(32):
        offsets = [offset for offset, size in free if size == 1 << exponent]
        root += struct.pack(f">I{len(offsets)}I", len(offsets), *offsets)

    body = bytearray(FILE_SIZE)
    body[0:4] = b"Bud1"
    struct.pack_into(">IIII", body, 4, ROOT[0], ROOT[1], ROOT[0], address(NODE))
    struct.pack_into(">5I", body, DSDB[0], blocks.index(NODE), 0, len(ordered), 1, 0x1000)
    body[NODE[0]:NODE[0] + len(node)] = node
    body[ROOT[0]:ROOT[0] + len(root)] = root
    return b"\0\0\0\1" + bytes(body)


def location(point: tuple[int, int]) -> bytes:
    return struct.pack(">II", *point) + b"\xff" * 6 + b"\0\0"


def layout(app_name: str) -> bytes:
    """The window and icon records, keyed by file name only, so they hold for
    any volume name; nothing in them refers to the volume."""
    window = {
        "ContainerShowSidebar": False,
        "ShowSidebar": False,
        "ShowStatusBar": False,
        "ShowTabView": False,
        "ShowToolbar": False,
        "WindowBounds": WINDOW,
    }
    icons = {
        "arrangeBy": "none",
        "backgroundColorBlue": 1.0,
        "backgroundColorGreen": 1.0,
        "backgroundColorRed": 1.0,
        "backgroundType": 0,
        "gridOffsetX": 0.0,
        "gridOffsetY": 0.0,
        "gridSpacing": 100.0,
        "iconSize": ICON_SIZE,
        "labelOnBottom": True,
        "showIconPreview": True,
        "showItemInfo": False,
        "textSize": TEXT_SIZE,
        "viewOptionsVersion": 1,
    }
    binary = plistlib.FMT_BINARY
    return ds_store([
        (".", "bwsp", "blob", plistlib.dumps(window, fmt=binary)),
        (".", "icvp", "blob", plistlib.dumps(icons, fmt=binary)),
        (".", "vSrn", "long", struct.pack(">I", 1)),
        ("Applications", "Iloc", "blob", location(APPLICATIONS_AT)),
        (app_name, "Iloc", "blob", location(APP_AT)),
    ])


# A volume's own icon shows only when its root carries the custom-icon Finder
# flag (kHasCustomIcon, byte 8 of FinderInfo).
CUSTOM_ICON = "00" * 8 + "04" + "00" * 23


def hdiutil(*args: str) -> str:
    done = subprocess.run(["hdiutil", *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"hdiutil {args[0]} failed: {done.stderr.strip()}")
    return done.stdout


def snapshot(root: Path) -> dict[str, tuple]:
    """Every file, link and directory under root, with what it holds."""
    seen: dict[str, tuple] = {}
    for path in sorted([root, *root.rglob("*")]):
        key = str(path.relative_to(root))
        mode = path.lstat().st_mode
        if path.is_symlink():
            seen[key] = ("link", os.readlink(path))
        elif path.is_dir():
            seen[key] = ("dir", mode)
        else:
            seen[key] = ("file", mode, path.read_bytes())
    return seen


def package(app: Path, out: Path) -> None:
    icon_name = plistlib.loads((app / "Contents/Info.plist").read_bytes()).get("CFBundleIconFile", "icon.icns")
    icon = app / "Contents/Resources" / (icon_name if icon_name.endswith(".icns") else icon_name + ".icns")
    if not icon.is_file():
        raise SystemExit(f"no app icon at {icon}")
    with tempfile.TemporaryDirectory(prefix="guaca-dmg-") as temp:
        work = Path(temp)
        stage = work / "stage"
        stage.mkdir()
        subprocess.run(["ditto", str(app), str(stage / app.name)], check=True)
        (stage / "Applications").symlink_to("/Applications")
        shutil.copyfile(icon, stage / ".VolumeIcon.icns")
        (stage / ".DS_Store").write_bytes(layout(app.name))

        writable = work / "writable.dmg"
        mount = work / "mount"
        mount.mkdir()
        hdiutil("create", "-quiet", "-volname", VOLUME, "-fs", "HFS+", "-format", "UDRW",
                "-srcfolder", str(stage), str(writable))
        hdiutil("attach", "-quiet", "-nobrowse", "-noautoopen", "-mountpoint", str(mount), str(writable))
        try:
            subprocess.run(["xattr", "-wx", "com.apple.FinderInfo", CUSTOM_ICON, str(mount)], check=True)
        finally:
            hdiutil("detach", "-quiet", str(mount))
        hdiutil("convert", "-quiet", str(writable), "-format", "UDZO", "-imagekey", "zlib-level=9",
                "-o", str(out))

        hdiutil("attach", "-quiet", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", str(mount), str(out))
        try:
            if snapshot(mount / app.name) != snapshot(app):
                raise SystemExit("the app in the image differs from the app given")
            if (mount / ".DS_Store").read_bytes() != layout(app.name):
                raise SystemExit("the image's .DS_Store is not the layout written")
        finally:
            hdiutil("detach", "-quiet", str(mount))


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    app, out = Path(argv[0]), Path(argv[1])
    if app.suffix != ".app" or not (app / "Contents/Info.plist").is_file():
        print(f"{app} is not an app bundle", file=sys.stderr)
        return 2
    if out.exists():
        print(f"{out} already exists; a release image is never overwritten in place", file=sys.stderr)
        return 2
    if sys.platform != "darwin":
        print("a disk image can only be made on macOS", file=sys.stderr)
        return 2
    package(app, out)
    print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
