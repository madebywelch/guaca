# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools>=4.60", "brotli>=1.1"]
# ///
"""
Five wordmark prototypes for Guaca, and the page that compares them.

The rail's wordmark is `GUACA` in Instrument Sans at 600 with the section
heads' tracking, which is why it reads as one more label in the chrome rather
than as the thing the chrome belongs to. Each prototype here makes one drawn
move, and every move is taken from something the app already has rather than
invented for the logo:

  conversation-g  the Paper scratchpad's icon, whose G is a reply bubble
  eye-contact     the shipped icon, two creatures looking at each other
  crew            the five silhouettes every creature is cut from
  mashed          the name, which is something mashed together
  drawn           the drawn cast's brush

Everything is written out as outlines. A wordmark set as live text is a
wordmark that changes with the font file, the renderer and the kerning table,
and the point of drawing one is that it does not.

Run through ./design/wordmarks/make.sh, which samples the silhouettes first.
"""

import base64
import json
import math
import sys
from pathlib import Path

from fontTools.misc.transform import Transform
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = Path(__file__).resolve().parents[2]
HERE = ROOT / "design" / "wordmarks"
FONTS = ROOT / "node_modules" / "@fontsource-variable"
SANS = FONTS / "instrument-sans" / "files" / "instrument-sans-latin-standard-normal.woff2"
SILHOUETTES = json.loads((ROOT / "node_modules" / ".cache" / "silhouettes.json").read_text())

# The rail's two surfaces, from the token blocks at the top of src/styles.css.
PAPER = {"ground": "#F5F3EE", "edge": "#E5E2DA", "raised": "#FFFFFF", "ink": "#0B0B0A",
         "muted": "#54524D", "amber": "#B4530A"}
DARK = {"ground": "#151513", "edge": "#2B2A27", "raised": "#1F1F1C", "ink": "#F7F5F0",
        "muted": "#A8A49C", "amber": "#F0A63C"}


def num(v: float) -> str:
    return f"{v:.1f}".rstrip("0").rstrip(".")


def bold(wdth: float = 100):
    return instancer.instantiateVariableFont(TTFont(SANS), {"wght": 700, "wdth": wdth})


B700 = bold()


def glyph(font, ch: str, x: float = 0, baseline: float = 0, scale: float = 1):
    """One glyph's outline in SVG space (y down), baked at a position, plus its metrics."""
    gs = font.getGlyphSet()
    g = gs[font.getBestCmap()[ord(ch)]]
    pen = SVGPathPen(gs, ntos=num)
    g.draw(TransformPen(pen, Transform(scale, 0, 0, -scale, x, baseline)))
    bounds = BoundsPen(gs)
    g.draw(bounds)
    return pen.getCommands(), g.width, bounds.bounds


def svg(x: float, y: float, w: float, h: float, body: str) -> str:
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{num(x)} {num(y)} {num(w)} {num(h)}" '
            f'width="{num(w)}" height="{num(h)}">{body}</svg>')


# --- 1. Conversation G -------------------------------------------------------
#
# The font's own C, with the icon's bubble laid in as the crossbar. Drawing a
# G to match the icon was tried first: its terminals were cut radially, the
# typeface cuts them flat, and the one letter that was not the typeface was the
# one everybody looked at. In one color the bubble is only a crossbar, which is
# the test the mark has to pass on a fax, a favicon or an engraving.

def conversation_g(c=PAPER) -> str:
    d, _, (x0, _, x1, _) = glyph(B700, "C", 0, 720)
    body = f'<path d="{d}" fill="{c["ink"]}"/>'
    top, bot = 295, 458
    left, right, r = (x0 + x1) / 2 - 20, x1 + 70, 52
    # The tail's left edge is vertical and its point hangs into the aperture, as
    # on the icon, with paper left between it and the C's lower lip.
    tail0, tail1, tip = right - 92, right - 160, bot + 96
    bubble = (f"M{num(left + r)} {top}H{num(right - r)}Q{num(right)} {top} {num(right)} {top + r}"
              f"V{bot - r}Q{num(right)} {bot} {num(right - r)} {bot}H{num(tail0)}L{num(tail1)} {tip}"
              f"V{bot}H{num(left + r)}Q{num(left)} {bot} {num(left)} {bot - r}V{top + r}"
              f"Q{num(left)} {top} {num(left + r)} {top}Z")
    body += f'<path d="{bubble}" fill="{c["amber"]}"/>'
    x, track, rest = right + 6, -22, []
    for ch in "uaca":
        d, adv, _ = glyph(B700, ch, x, 720)
        rest.append(f'<path d="{d}"/>')
        x += adv + track
    body += f'<g fill="{c["ink"]}">{"".join(rest)}</g>'
    return svg(x0 - 10, -20, x - track - x0 + 10, 760, body)


# --- 2. Eye contact ----------------------------------------------------------
#
# Circles and one round stroke, which is what the cast is made of. The bowls of
# the two a's are eyes, and they look at each other across the c: agents
# talking to agents, which is what the shipped icon says with two bodies.
# `look` is where the pupils sit, in x-heights; the page moves it.

EYES = ((280, 1), (489, -1))  # each a's center, and which way it faces the other


def eye_contact(c=PAPER, look=(10, 0)) -> str:
    r = 38
    lines = [
        f'<circle cx="50" cy="50" r="{r}"/>',
        '<path d="M88 12V94A38 38 0 0 1 17.1 113"/>',
        '<path d="M128 12V50A38 38 0 0 0 204 50M204 12V88"/>',
        f'<circle cx="280" cy="50" r="{r}"/><path d="M318 12V88"/>',
    ]
    # The c opens 52 degrees either side of level: round caps close an aperture
    # by their own radius, so what is drawn at 45 reads as 30.
    a, cx = math.radians(52), 394
    ux, uy = cx + r * math.cos(a), 50 - r * math.sin(a)
    lines.append(f'<path d="M{num(ux)} {num(uy)}A38 38 0 1 0 {num(ux)} {num(100 - uy)}"/>')
    lines.append(f'<circle cx="489" cy="50" r="{r}"/><path d="M527 12V88"/>')
    strokes = (f'<g fill="none" stroke="{c["ink"]}" stroke-width="24" stroke-linecap="round">'
               + "".join(lines) + "</g>")
    dx, dy = look
    pupils = "".join(f'<circle class="pupil" data-face="{face}" cx="{num(x + dx * face)}" cy="{num(50 + dy)}" r="12.5"/>'
                     for x, face in EYES)
    return svg(-2, -2, 543, 148, strokes + f'<g fill="{c["ink"]}">{pupils}</g>')


# --- 3. Crew -----------------------------------------------------------------
#
# Each letter knocked out of one of the five silhouettes, sampled from
# src/avatars/silhouette.ts rather than traced, so the outlines are the ones
# the cast is cut from. The A that stands in the drop is the one whose apex
# agrees with it.

CREW = (("G", "circle"), ("U", "square"), ("A", "drop"), ("C", "cloud"), ("A", "octagon"))
# Where the mass of each shape sits below its box's middle: a drop and a cloud
# carry their weight low, and a letter centered on the box floats in them.
SETTLE = {"circle": 0, "square": 0, "octagon": 0, "drop": 6, "cloud": 3}


def shape(name: str, dx: float) -> tuple[str, float, float]:
    pts = SILHOUETTES[name]
    xs = [x for x, _ in pts]
    d = "M" + "L".join(f"{num(x + dx)} {num(y)}" for x, y in pts) + "Z"
    return d, min(xs), max(xs)


def crew(c=PAPER, gap=9, cap=44) -> str:
    s, edge, body = cap / 720, 0.0, []
    for ch, name in CREW:
        _, left, right = shape(name, 0)
        dx = edge - left
        d, _, _ = shape(name, dx)
        body.append(f'<path d="{d}" fill="{c["ink"]}"/>')
        _, _, (gx0, _, gx1, _) = glyph(B700, ch)
        x = dx + 50 - (gx1 - gx0) * s / 2 - gx0 * s
        d, _, _ = glyph(B700, ch, x, 50 + cap / 2 + SETTLE[name], s)
        body.append(f'<path d="{d}" fill="{c["ground"]}"/>')
        edge = dx + right + gap
    return svg(-2, -4, edge - gap + 4, 110, "".join(body))


# --- 4. Mashed ---------------------------------------------------------------
#
# Instrument Sans Bold, thickened by a round-joined stroke of its own color,
# which softens every corner toward the cast's geometry, then kerned until the
# round letters touch. Two straights never touch: g against u would fuse into
# one slab the height of the x-height, so that pair keeps a gap and every other
# pair kisses.

SOFTEN = 26
KERN = {"gu": -70, "ua": -78, "ac": -66, "ca": -70}


def mashed(c=PAPER) -> str:
    x, prev, parts = 0.0, "", []
    for ch in "guaca":
        x += KERN.get(prev + ch, 0)
        d, adv, _ = glyph(B700, ch, x, 520)
        parts.append(f'<path d="{d}"/>')
        x, prev = x + adv, ch
    body = (f'<g fill="{c["ink"]}" stroke="{c["ink"]}" stroke-width="{SOFTEN}" stroke-linejoin="round">'
            + "".join(parts) + "</g>")
    return svg(0, -30, x + 20, 775, body)


# --- 5. Drawn ----------------------------------------------------------------
#
# Lettered with the drawn cast's brush: `stroke` below is src/avatars/drawn.ts's
# own, ported line for line, so a change of mind about the brush there is a
# change here. Pressure follows direction (a downstroke is the full width, an
# upstroke under a third of it), the ends come to a point rather than a knob,
# and the pigment is printed a little down and right of the line inside each
# bowl, which is what the cast does with a body.

def spline(pts, per):
    n = len(pts)
    at = lambda i: pts[max(0, min(n - 1, i))]
    out = []
    for i in range(n - 1):
        p0, p1, p2, p3 = at(i - 1), at(i), at(i + 1), at(i + 2)
        for j in range(per):
            s = j / per
            f = lambda a, b, c, d: 0.5 * (2 * b + (-a + c) * s + (2 * a - 5 * b + 4 * c - d) * s * s
                                          + (-a + 3 * b - 3 * c + d) * s ** 3)
            out.append((f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])))
    out.append(pts[-1])
    return out


def loop(pts, per):
    n = len(pts)
    out = []
    for i in range(n):
        p0, p1, p2, p3 = pts[(i - 1) % n], pts[i], pts[(i + 1) % n], pts[(i + 2) % n]
        for j in range(per):
            s = j / per
            f = lambda a, b, c, d: 0.5 * (2 * b + (-a + c) * s + (2 * a - 5 * b + 4 * c - d) * s * s
                                          + (-a + 3 * b - 3 * c + d) * s ** 3)
            out.append((f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])))
    return out


def stroke(center, widths) -> str:
    """drawn.ts `stroke`: a filled stroke along a centerline, its own width at every point, round ends."""
    n = len(center)

    def tangent(i):
        a, b = center[max(0, i - 1)], center[min(n - 1, i + 1)]
        l = math.hypot(b[0] - a[0], b[1] - a[1]) or 1
        return (b[0] - a[0]) / l, (b[1] - a[1]) / l

    left, right = [], []
    for i in range(n):
        tx, ty = tangent(i)
        w, p = widths[i] / 2, center[i]
        left.append((p[0] - ty * w, p[1] + tx * w))
        right.append((p[0] + ty * w, p[1] - tx * w))

    def cap(i, forward):
        tx, ty = tangent(i)
        d, w, p = (1 if forward else -1), widths[i] / 2, center[i]
        nx, ny = (-ty, tx) if forward else (ty, -tx)
        return [(p[0] + (nx * math.cos(k / 6 * math.pi) + tx * d * math.sin(k / 6 * math.pi)) * w,
                 p[1] + (ny * math.cos(k / 6 * math.pi) + ty * d * math.sin(k / 6 * math.pi)) * w)
                for k in range(1, 6)]

    pts = left + cap(n - 1, True) + right[::-1] + cap(0, False)
    return "M" + "L".join(f"{num(x)} {num(y)}" for x, y in pts) + "Z"


BOWL = [(80, 24), (62, 6), (38, 2), (17, 16), (8, 44), (13, 74), (32, 95), (56, 97), (75, 82), (84, 58)]
LETTERS = {
    "g": [BOWL, [(83, -2), (86, 8), (87, 60), (86, 118), (78, 144), (58, 158), (34, 156), (16, 142)]],
    "u": [[(6, -1), (9, 8), (9, 58), (15, 86), (34, 100), (57, 95), (72, 74)],
          [(76, -1), (79, 8), (80, 60), (81, 94), (89, 102)]],
    "a": [BOWL, [(82, -1), (85, 8), (86, 60), (86, 94), (94, 102)]],
    "c": [[(78, 22), (62, 5), (38, 1), (16, 15), (7, 44), (11, 74), (30, 95), (55, 99), (77, 86)]],
}
ADVANCE = {"g": 104, "u": 100, "a": 104, "c": 90}
# A hand does not set five letters on one line at one angle.
BOUNCE = (0, -2, 1, -1, 2)
TILT = (-1.5, 1.0, -0.5, 1.5, -1.0)


def brushed(ctrl, weight):
    c = spline(ctrl, 8)
    n, widths = len(c), []
    for i in range(n):
        a, b = c[max(0, i - 1)], c[min(n - 1, i + 1)]
        ty = (b[1] - a[1]) / (math.hypot(b[0] - a[0], b[1] - a[1]) or 1)
        s = i / (n - 1)
        press = 0.3 + 0.7 * max(0.0, min(1.0, ty + 0.1))
        taper = min(1, 0.35 + s / 0.06, 0.3 + (1 - s) / 0.1)
        widths.append(weight * press * taper)
    return c, widths


def drawn(c=PAPER, pigment="#F2D9BE", weight=17) -> str:
    x, ink, color = 0, [], []
    for i, ch in enumerate("guaca"):
        t = Transform().translate(x, BOUNCE[i]).translate(50, 50).rotate(math.radians(TILT[i])).translate(-50, -50)
        move = lambda pts: [t.transformPoint(p) for p in pts]
        for line in LETTERS[ch]:
            center, widths = brushed(line, weight)
            ink.append(f'<path d="{stroke(move(center), widths)}"/>')
        if ch in "ga":
            bowl = [(px + 5, py + 4) for px, py in move(loop(BOWL[1:], 6))]
            color.append('<path d="M' + "L".join(f"{num(px)} {num(py)}" for px, py in bowl) + 'Z"/>')
        x += ADVANCE[ch]
    return svg(-10, -12, x + 16, 184, f'<g fill="{pigment}">{"".join(color)}</g><g fill="{c["ink"]}">{"".join(ink)}</g>')


# --- the set -----------------------------------------------------------------

# How tall each one stands in the rail, in CSS pixels: set by eye against the
# plus beside it and the names under it, so the five carry the same weight.
MARKS = [
    {
        "key": "conversation-g", "name": "Conversation G", "rail": 20,
        "paper": conversation_g, "dark": lambda: conversation_g(DARK),
        "title": "The icon starts the word.",
        "why": "Instrument Sans Bold's own C, with the icon's reply bubble as the crossbar. The mark "
               "and the word stop being two things set side by side. In one color the bubble is just "
               "a crossbar, so it still reads as a G. The only one of the five that spends amber, and "
               "amber is the color that means the app wants something from you.",
    },
    {
        "key": "eye-contact", "name": "Eye contact", "rail": 23,
        "paper": eye_contact, "dark": lambda: eye_contact(DARK),
        "title": "The two a's look at each other.",
        "why": "Drawn from circles and one round stroke, like the cast. Each a's bowl is an eye, and "
               "the pupils meet across the c: agents talking to agents, which is what the shipped "
               "icon says with two bodies. The pupils can move. Put the pointer on this page and they "
               "follow it; in the rail they would turn to the operator when something is waiting.",
    },
    {
        "key": "crew", "name": "Crew", "rail": 28,
        "paper": crew, "dark": lambda: crew(DARK),
        "title": "Five letters, five shapes.",
        "why": "Each letter is knocked out of one of the silhouettes every creature is cut from: "
               "circle, square, drop, cloud, octagon. The outlines are sampled from silhouette.ts, not "
               "traced, so the wordmark is the cast standing in a row. The loudest of the five, and "
               "the one that needs the most height to read.",
    },
    {
        "key": "mashed", "name": "Mashed", "rail": 25,
        "paper": mashed, "dark": lambda: mashed(DARK),
        "title": "One mass, not five letters.",
        "why": "Instrument Sans Bold, softened by a round-joined stroke and kerned until the round "
               "letters touch. Straights keep a gap; rounds kiss. The most conventional of the five, "
               "and the one that holds up best at 16 pixels and on a favicon.",
    },
    {
        "key": "drawn", "name": "Drawn", "rail": 27,
        "paper": drawn, "dark": lambda: drawn(DARK, pigment="#5A4A3A"),
        "small": lambda: drawn(weight=23), "small_dark": lambda: drawn(DARK, pigment="#5A4A3A", weight=23),
        "title": "Lettered with the cast's brush.",
        "why": "Every stroke goes through the drawn cast's own stroke(): heavy downstrokes, pointed "
               "ends, pigment printed off the line in the bowls. The rail cut uses a heavier line, "
               "which is what hint() does for a creature drawn small. Only fits a workspace that has "
               "chosen the drawn cast.",
    },
]


def sized(markup: str, height: float) -> str:
    """The same drawing at a height, with the width that keeps its proportions."""
    head, rest = markup.split(">", 1)
    vb = head.split('viewBox="')[1].split('"')[0].split()
    w = float(vb[2]) * height / float(vb[3])
    head = head.split(' width="')[0] + f' width="{num(w)}" height="{num(height)}"'
    return f"{head}>{rest}"


def font_face(family: str, path: Path) -> str:
    data = base64.b64encode(path.read_bytes()).decode()
    return (f'@font-face{{font-family:"{family}";font-style:normal;font-weight:100 900;'
            f'src:url(data:font/woff2;base64,{data}) format("woff2");}}')


def rail(markup: str, surface: dict, label: str) -> str:
    return (f'<div class="rail" style="--g:{surface["ground"]};--e:{surface["edge"]};--r:{surface["raised"]};'
            f'--t:{surface["ink"]};--m:{surface["muted"]}" aria-label="{label}">'
            f'<div class="brand">{markup}<span class="plus">+</span></div>'
            '<div class="row">Research crew</div><div class="row muted">Scout</div></div>')


CURRENT = '<span class="current">GUACA</span>'


def page(marks) -> str:
    sections = []
    for i, m in enumerate(marks):
        small, small_dark = m.get("small", m["paper"])(), m.get("small_dark", m["dark"])()
        sections.append(f"""
<section id="{m['key']}">
  <header><span>GUACA / WORDMARKS</span><span>{i + 3:02d}&nbsp;&nbsp;&nbsp;{m['name']}</span></header>
  <div class="hero">{sized(m['paper'](), 200)}</div>
  <div class="foot">
    <div class="why"><h2>{m['title']}</h2><p>{m['why']}</p></div>
    <div class="in-rail"><h3>IN THE RAIL, 1X</h3><div class="rails">
      {rail(sized(small, m['rail']), PAPER, 'Paper')}{rail(sized(small_dark, m['rail']), DARK, 'Dark')}
    </div></div>
  </div>
</section>""")
    cast = [("Current", CURRENT)] + [(m["name"], sized(m.get("small", m["paper"])(), m["rail"])) for m in marks]
    lineup = "".join(f"<figure><figcaption>{name}</figcaption>{rail(markup, PAPER, name)}</figure>"
                     for name, markup in cast)
    fonts = "\n".join([
        font_face("Instrument Sans", FONTS / "instrument-sans" / "files" / "instrument-sans-latin-wght-normal.woff2"),
        font_face("Inter", FONTS / "inter" / "files" / "inter-latin-wght-normal.woff2"),
    ])
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Guaca wordmarks</title>
<style>
{fonts}
* {{ box-sizing: border-box; }}
body {{ margin: 0; background: #fff; color: #0B0B0A; font-family: Inter, system-ui, sans-serif;
  -webkit-font-smoothing: antialiased; }}
section, .lineup {{ width: 1440px; padding: 48px; display: flex; flex-direction: column; gap: 32px; }}
section + section, .lineup {{ border-top: 1px solid #E5E2DA; }}
header {{ display: flex; justify-content: space-between; font-size: 14px; line-height: 20px; color: #54524D; }}
header span:first-child {{ letter-spacing: .12em; }}
.hero {{ height: 440px; background: #F5F3EE; display: flex; align-items: center; padding: 64px 96px; }}
.foot {{ display: flex; justify-content: space-between; gap: 64px; padding-top: 8px; }}
.why {{ width: 560px; }}
h2 {{ margin: 0 0 14px; font: 600 28px/34px "Instrument Sans"; }}
p {{ margin: 0; font-size: 18px; line-height: 28px; color: #54524D; }}
h3 {{ margin: 0 0 14px; font-weight: 400; font-size: 14px; line-height: 20px; letter-spacing: .08em; color: #54524D; }}
.rails, .lineup .rails {{ display: flex; gap: 16px; }}
.rail {{ width: 248px; background: var(--g); border: 1px solid var(--e); padding-bottom: 14px; color: var(--t); }}
.brand {{ display: flex; align-items: center; justify-content: space-between; padding: 36px 16px 14px; }}
.brand svg {{ display: block; }}
.plus {{ width: 26px; height: 26px; display: inline-flex; align-items: center; justify-content: center;
  border: 1px solid var(--e); background: var(--r); border-radius: 3px; font-size: 17px; }}
.row {{ padding: 6px 16px; font-size: 15px; line-height: 20px; }}
.row.muted {{ color: var(--m); }}
.current {{ font: 600 22px/26px "Instrument Sans"; letter-spacing: .18em; }}
.lineup .rails {{ flex-wrap: wrap; gap: 40px 48px; }}
figure {{ margin: 0; }}
figcaption {{ margin-bottom: 12px; font-size: 14px; line-height: 20px; color: #54524D; }}
</style>
</head>
<body>
{"".join(sections)}
<div class="lineup" id="lineup">
  <header><span>GUACA / WORDMARKS</span><span>08&nbsp;&nbsp;&nbsp;All five, where they live</span></header>
  <div class="rails">{lineup}</div>
</div>
<script>
/* Eye contact: the pupils meet across the c until the pointer is on the page,
   and then both follow it. Clamped to the room a pupil has inside its bowl. */
const ROOM = 10;
const pupils = [...document.querySelectorAll("#eye-contact .pupil, .rail .pupil")];
const rest = pupils.map((p) => [Number(p.getAttribute("cx")), Number(p.getAttribute("cy")), Number(p.dataset.face)]);
function look(event) {{
  pupils.forEach((p, i) => {{
    const [cx, cy, face] = rest[i];
    const home = cx - ROOM * face;
    if (!event) {{ p.setAttribute("cx", cx); p.setAttribute("cy", cy); return; }}
    const box = p.ownerSVGElement.getBoundingClientRect();
    const vb = p.ownerSVGElement.viewBox.baseVal;
    const k = vb.width / box.width;
    const dx = (event.clientX - box.left) * k + vb.x - home;
    const dy = (event.clientY - box.top) * k + vb.y - 50;
    const l = Math.hypot(dx, dy) || 1;
    const r = Math.min(ROOM, l);
    p.setAttribute("cx", home + (dx / l) * r);
    p.setAttribute("cy", 50 + (dy / l) * r);
  }});
}}
addEventListener("pointermove", look);
document.addEventListener("pointerleave", () => look(null));
</script>
</body>
</html>
"""


def main():
    for m in MARKS:
        (HERE / f"{m['key']}.svg").write_text(m["paper"]() + "\n")
        (HERE / f"{m['key']}.dark.svg").write_text(m["dark"]() + "\n")
        if "small" in m:
            (HERE / f"{m['key']}.small.svg").write_text(m["small"]() + "\n")
            (HERE / f"{m['key']}.small.dark.svg").write_text(m["small_dark"]() + "\n")
    (ROOT / "design" / "wordmarks.html").write_text(page(MARKS))
    print(f"design/wordmarks.html and {len(list(HERE.glob('*.svg')))} SVGs in design/wordmarks/", file=sys.stderr)


if __name__ == "__main__":
    main()
