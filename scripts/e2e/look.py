from __future__ import annotations

from dataclasses import dataclass

ROLES = {
    (12, 400): "meta",
    (13, 400): "support",
    (15, 400): "UI",
    (15, 600): "UI strong",
    (16, 400): "reading",
    (20, 600): "title",
}
STYLE_BUDGET = 8
CONTROL_HEIGHTS = (28, 32)
RADII = (0, 6, 10)
FRAME = (1280, 820)
CODE = "code, pre, .diff, .file__text, .connector-failure__detail > pre, .textarea--code, .webhook, .devicecode__code, .skill__body"

SETTLED = ("document.getAnimations().every(a => a.playState !== 'running' "
           "|| a.effect.getComputedTiming().iterations === Infinity)")

MEASURE = """
(() => {
  const CODE = __CODE__;
  const px = (v) => Math.round(parseFloat(v) * 100) / 100;
  const scale = parseFloat(getComputedStyle(document.documentElement).fontSize) / 16;
  const read = getComputedStyle(document.documentElement).getPropertyValue('--type-read').trim();
  const reading = read.endsWith('rem') ? parseFloat(read) * 16 : 16;
  const oneLine = (el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const tops = new Set([...range.getClientRects()].filter((r) => r.width > 0).map((r) => Math.round(r.top)));
    return tops.size <= 1;
  };
  const words = (el) => el.matches('select, input') || el.textContent.trim() !== '' || !!el.querySelector('svg.icon');
  const seen = (el) => {
    if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    const r = el.getBoundingClientRect();
    return r.width >= 1 && r.height >= 1 && r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
  };
  const named = (el) => (el.className && typeof el.className === 'string' && el.className.split(' ')[0]) || el.tagName.toLowerCase();
  const text = new Map();
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const words = node.textContent.replace(/\\s+/g, ' ').trim();
    const el = node.parentElement;
    if (!words || !el || !seen(el)) continue;
    const range = document.createRange();
    range.selectNodeContents(node);
    const box = range.getBoundingClientRect();
    if (box.width < 1 || box.height < 1 || box.bottom <= 0 || box.top >= innerHeight) continue;
    const s = getComputedStyle(el);
    const style = {
      family: s.fontFamily.split(',')[0].replace(/["']/g, '').trim(),
      size: px(parseFloat(s.fontSize) / scale),
      weight: Number(s.fontWeight),
      tracking: s.letterSpacing === 'normal' ? 0 : px(s.letterSpacing),
      italic: s.fontStyle !== 'normal',
      caps: s.textTransform === 'uppercase',
      code: !!el.closest(CODE),
    };
    const key = JSON.stringify(style);
    const entry = text.get(key) || { ...style, count: 0, sample: words.slice(0, 40), where: named(el) };
    entry.count += 1;
    text.set(key, entry);
  }
  const radii = {};
  const controls = [];
  for (const el of document.body.querySelectorAll('*')) {
    if (el.closest('svg') || !seen(el)) continue;
    const s = getComputedStyle(el);
    const painted = s.backgroundColor !== 'rgba(0, 0, 0, 0)' || parseFloat(s.borderTopWidth) > 0;
    if (!painted) continue;
    const r = el.getBoundingClientRect();
    for (const corner of [s.borderTopLeftRadius, s.borderTopRightRadius, s.borderBottomLeftRadius, s.borderBottomRightRadius]) {
      const value = corner.endsWith('%') ? parseFloat(corner) / 100 * Math.min(r.width, r.height) : parseFloat(corner);
      const k = value >= Math.min(r.width, r.height) / 2 - 0.5 && value > 0 ? 'round' : String(px(value / scale));
      if (!(k in radii)) radii[k] = named(el);
    }
    if (el.matches('button, select, input:not([type=checkbox]):not([type=radio]):not([type=range]):not([type=file])')
        && words(el) && oneLine(el)) {
      controls.push({ where: named(el), name: (el.getAttribute('aria-label') || el.textContent || '').trim().slice(0, 30),
                      height: px(r.height / scale) });
    }
  }
  return { reading, styles: [...text.values()], radii, controls };
})()
""".replace("__CODE__", repr(CODE))


@dataclass(frozen=True)
class Verdict:
    screen: str
    styles: int
    faults: list[str]


def judge(screen: str, measured: dict) -> Verdict:
    roles = {**ROLES, (measured.get("reading") or 16, 400): "reading"}
    faults: list[str] = []
    styles = measured["styles"]
    if len(styles) > STYLE_BUDGET:
        listed = "; ".join(f"{s['size']:g}/{s['weight']} {s['family']} in {s['where']} ({s['sample']!r})"
                           for s in styles)
        faults.append(f"{screen} shows {len(styles)} type styles and the budget is {STYLE_BUDGET}: set each "
                      f"element in one of the six roles in src/styles.css (--type-meta, --type-support, --type-ui, "
                      f"--type-read, --type-title, with --weight-regular or --weight-strong): {listed}")
    for s in styles:
        if (s["size"], s["weight"]) not in roles:
            faults.append(f"{screen}: {s['size']:g}px at {s['weight']} in {s['where']} ({s['sample']!r}) is no type "
                          "role; use " + ", ".join(f"{name} {size}/{weight}" for (size, weight), name in ROLES.items()))
        if s["italic"]:
            faults.append(f"{screen}: {s['where']} ({s['sample']!r}) is italic; set it upright")
        if s["caps"] or s["tracking"] > 0:
            faults.append(f"{screen}: {s['where']} ({s['sample']!r}) is tracked or uppercase; "
                          "write it in sentence case with no letter-spacing")
        if "mono" in s["family"].lower() and not s["code"]:
            faults.append(f"{screen}: {s['where']} ({s['sample']!r}) is set in {s['family']}, which is only for "
                          "code; use var(--font-ui)")
    for radius, where in measured["radii"].items():
        if radius != "round" and float(radius) not in RADII:
            faults.append(f"{screen}: {where} has a {radius}px corner; use --radius-control (6px), "
                          "--radius-card (10px), or --radius-pill on the now pill and faces")
    for control in measured["controls"]:
        if control["height"] not in CONTROL_HEIGHTS:
            faults.append(f"{screen}: the control {control['where']} ({control['name']!r}) is "
                          f"{control['height']:g}px tall; use --control-h (32px) or --control-h-sm (28px)")
    return Verdict(screen, len(styles), faults)

LAYOUT = """
(() => {
  const seen = (el) => {
    if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    const r = el.getBoundingClientRect();
    return r.width >= 1 && r.height >= 1 && r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
  };
  const named = (el) => (el.className && typeof el.className === 'string' && el.className.split(' ')[0]) || el.tagName.toLowerCase();
  const boxes = [];
  const scrollers = [];
  for (const el of document.body.querySelectorAll('*')) {
    if (el.closest('svg, .ribbon, .home__run, .working, .topbar__day') || !seen(el)) continue;
    const r = el.getBoundingClientRect();
    boxes.push([named(el), Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)]);
    const s = getComputedStyle(el);
    if (/auto|scroll/.test(s.overflowY) && el.scrollHeight > el.clientHeight) {
      scrollers.push(`${named(el)} by ${el.scrollHeight - el.clientHeight}px`);
    }
  }
  const tops = new Map();
  for (const item of document.querySelectorAll('.picker > *')) {
    const top = Math.round(item.getBoundingClientRect().top);
    tops.set(top, (tops.get(top) || 0) + 1);
  }
  const rows = [...tops.entries()].sort((a, b) => a[0] - b[0]).map(([, count]) => count);
  const dialog = document.querySelector('.dialog--form, .dialog--settings');
  const rect = dialog?.getBoundingClientRect();
  const scrim = dialog?.parentElement;
  const horizontal = rect && {
    left: Math.round(rect.left),
    right: Math.round(innerWidth - rect.right),
    center: Math.round(rect.left + rect.width / 2),
    viewportCenter: Math.round(innerWidth / 2),
    scrimGutter: scrim.offsetWidth - scrim.clientWidth,
  };
  return { boxes, scrollers, rows, horizontal };
})()
"""

SETTINGS_CONTROLS = """
(() => {
  const edges = (selector) => {
    const rect = document.querySelector(selector).getBoundingClientRect();
    return [Math.round(rect.left), Math.round(rect.right)];
  };
  const gutter = (selector) => {
    const node = document.querySelector(selector);
    return node.offsetWidth - node.clientWidth - node.clientLeft -
      parseFloat(getComputedStyle(node).borderRightWidth);
  };
  return {
    general: edges('#settings-tab-general'),
    yourName: edges('#settings-pane input[placeholder="Unnamed"]'),
    gutters: [gutter('.settings__nav'), gutter('.settings__pane')],
  };
})()
"""


def moved(overlay: dict, classic: dict, limit: int = 6) -> list[str]:
    a, b = overlay["boxes"], classic["boxes"]
    if len(a) != len(b):
        return [f"{len(a)} elements are drawn with overlay scroll bars and {len(b)} with classic ones"]
    return [f"{one[0]} is {one[1:]} with overlay scroll bars and {two[1:]} with classic ones (x, y, width, height)"
            for one, two in zip(a, b) if one != two][:limit]
