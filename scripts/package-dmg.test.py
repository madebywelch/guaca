"""The release image's Finder layout, held without a Mac.

A disk image needs hdiutil, so the gate cannot make one on Linux. What decides
how the image opens is the .DS_Store the script writes, and that is plain
bytes: this reads them back with a reader of its own and holds them to the
records Finder wrote for the 0.4.0 image, which opened arranged. Opening a
packaged image in Finder is a release step in docs/RELEASING.md.
"""

import importlib.util
import plistlib
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("package_dmg", Path(__file__).with_name("package-dmg.py"))
package_dmg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package_dmg)


def read_ds_store(data: bytes) -> list[tuple[str, str, object]]:
    """Every record in a .DS_Store, decoded independently of the writer."""
    assert data[:8] == b"\0\0\0\1Bud1", "not a .DS_Store"
    body = data[4:]
    offset, size, again = struct.unpack(">III", body[4:16])
    assert offset == again
    root = body[offset:offset + size]
    count, _ = struct.unpack(">II", root[:8])
    addresses = struct.unpack(f">{count}I", root[8:8 + 4 * count])
    at = 8 + 4 * ((count + 255) // 256 * 256)
    (directories,) = struct.unpack(">I", root[at:at + 4])
    at += 4
    names = {}
    for _ in range(directories):
        length = root[at]
        names[root[at + 1:at + 1 + length]] = struct.unpack(">I", root[at + 1 + length:at + 5 + length])[0]
        at += 5 + length

    def block(index: int) -> bytes:
        address = addresses[index]
        start = address & ~0x1F
        return body[start:start + (1 << (address & 0x1F))]

    (top,) = struct.unpack(">I", block(names[b"DSDB"])[:4])
    node = block(top)
    leaf, records = struct.unpack(">II", node[:8])
    assert leaf == 0, "the layout is one leaf"
    at, out = 8, []
    for _ in range(records):
        (length,) = struct.unpack(">I", node[at:at + 4])
        name = node[at + 4:at + 4 + 2 * length].decode("utf-16-be")
        at += 4 + 2 * length
        code, kind = node[at:at + 4].decode(), node[at + 4:at + 8].decode()
        at += 8
        if kind == "blob":
            (length,) = struct.unpack(">I", node[at:at + 4])
            value = node[at + 4:at + 4 + length]
            at += 4 + length
            if value.startswith(b"bplist"):
                value = plistlib.loads(value)
        elif kind == "long":
            (value,) = struct.unpack(">I", node[at:at + 4])
            at += 4
        else:
            raise AssertionError(f"unexpected record type {kind}")
        out.append((name, code, value))
    return out


def icon_at(x: int, y: int) -> bytes:
    return struct.pack(">II", x, y) + bytes.fromhex("ffffffffffff0000")


# Decoded from the .DS_Store in Guaca_0.4.0_universal.dmg, the last image
# whose window opened arranged.
ARRANGED = [
    (".", "bwsp", {
        "ContainerShowSidebar": False, "ShowSidebar": False, "ShowStatusBar": False,
        "ShowTabView": False, "ShowToolbar": False, "WindowBounds": "{{10, 657}, {660, 400}}",
    }),
    (".", "icvp", {
        "arrangeBy": "none", "backgroundColorBlue": 1.0, "backgroundColorGreen": 1.0,
        "backgroundColorRed": 1.0, "backgroundType": 0, "gridOffsetX": 0.0, "gridOffsetY": 0.0,
        "gridSpacing": 100.0, "iconSize": 128.0, "labelOnBottom": True, "showIconPreview": True,
        "showItemInfo": False, "textSize": 16.0, "viewOptionsVersion": 1,
    }),
    (".", "vSrn", 1),
    ("Applications", "Iloc", icon_at(480, 170)),
    ("Guaca.app", "Iloc", icon_at(180, 170)),
]


class RefusalTests(unittest.TestCase):
    def setUp(self):
        self.temp = Path(tempfile.mkdtemp(prefix="package-dmg-"))
        self.addCleanup(lambda: __import__("shutil").rmtree(self.temp))
        self.app = self.temp / "Guaca.app"
        (self.app / "Contents").mkdir(parents=True)
        (self.app / "Contents/Info.plist").write_bytes(plistlib.dumps({}))
        run = patch.object(package_dmg.subprocess, "run", side_effect=AssertionError("ran a command"))
        run.start()
        self.addCleanup(run.stop)

    def refused(self, *argv: str, platform: str = "darwin") -> str:
        with patch.object(sys, "platform", platform), patch("sys.stderr") as err:
            self.assertEqual(package_dmg.main(list(argv)), 2)
        return "".join(call.args[0] for call in err.write.call_args_list)

    def test_refuses_what_is_not_an_app_bundle(self):
        folder = self.temp / "Guaca"
        folder.mkdir()
        self.assertIn("not an app bundle", self.refused(str(folder), str(self.temp / "out.dmg")))
        self.assertIn("not an app bundle", self.refused(str(self.temp / "Missing.app"), str(self.temp / "out.dmg")))

    def test_never_overwrites_an_image(self):
        out = self.temp / "out.dmg"
        out.write_bytes(b"published")
        self.assertIn("already exists", self.refused(str(self.app), str(out)))
        self.assertEqual(out.read_bytes(), b"published")

    def test_refuses_off_macos_before_running_anything(self):
        self.assertIn("only be made on macOS", self.refused(str(self.app), str(self.temp / "out.dmg"), platform="linux"))

    def test_needs_both_paths(self):
        self.refused(str(self.app))


class LayoutTests(unittest.TestCase):
    def test_the_image_opens_arranged_like_0_4_0(self):
        self.assertEqual(read_ds_store(package_dmg.layout("Guaca.app")), ARRANGED)

    def test_the_layout_follows_the_app_name_and_never_names_the_volume(self):
        records = read_ds_store(package_dmg.layout("Guaca Beta.app"))
        self.assertIn(("Guaca Beta.app", "Iloc", icon_at(180, 170)), records)
        self.assertNotIn(b"Guaca", package_dmg.layout("Other.app"))

    def test_the_allocator_accounts_for_every_byte_it_hands_out(self):
        used = [package_dmg.HEADER, package_dmg.DSDB, package_dmg.NODE, package_dmg.ROOT]
        free = package_dmg.free_blocks(0, package_dmg.SPACE, used)
        self.assertEqual(sum(size for _, size in free) + sum(size for _, size in used), package_dmg.SPACE)
        for offset, size in free:
            self.assertEqual(offset % size, 0)
            self.assertFalse(any(offset <= u[0] < offset + size for u in used))

    def test_a_layout_too_large_for_its_node_is_refused(self):
        with self.assertRaises(ValueError):
            package_dmg.ds_store([("x" * 300, "Iloc", "blob", b"\0" * 400)] * 2)


if __name__ == "__main__":
    unittest.main()
