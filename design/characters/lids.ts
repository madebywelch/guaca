/**
 * A. Lids. The same paper body, with an eye that has a pupil and two lids.
 *
 * The pupil takes the glance and the lids take the emotion, which is how an
 * animator divides a face. The body, the silhouettes and the moods' shapes are
 * the app's own (`bodyPoints` and `MOODS[m].shape`), so the only thing this
 * direction changes is the eye, and what the eye lets the body stop doing.
 */

import { AIM } from "../../src/avatars/eyes";
import { blend, bodyPoints, grip, outline } from "../../src/avatars/form";
import { MOODS, markFor, type Mood } from "../../src/avatars/moods";
import { paper } from "./baseline";
import {
  blinkAt,
  bump,
  C,
  clamp,
  type Design,
  el,
  follow,
  type GazeSpec,
  hint,
  INK,
  lerp,
  type Point,
  poly,
  R,
  rnd,
  saccade,
  smooth,
  uid,
} from "./shared";

/** One eye's lids, in eye radii unless said otherwise. */
interface Lid {
  /** Upper lid: 0 clear of the eye, 1 shut, negative retracted. */
  open: number;
  /** Degrees. Positive drops the inner end, which is cross; negative lifts it, which is worry. */
  tilt: number;
  /** How far the upper lid arches in its middle. */
  arch: number;
  /** Lower lid: 0 at rest, 1 up to where a closed eye meets. */
  low: number;
  /** How far the lower lid bows up in its middle. Cheeks pushing: a smile. */
  smile: number;
  /** Pupil radius. Small is shock, large is interest. */
  pupil: number;
  /** The eye's own scale. */
  size: number;
  /** The creature's right upper lid lower than its left, in `open`. */
  skew: number;
}

interface Face {
  lid: Lid;
  gaze?: GazeSpec;
  blink?: boolean | "slow";
  jitter?: number;
  /** How the lids change as the look goes up, blended by how far up. */
  squint?: Partial<Lid>;
}

const REST: Lid = { open: 0.16, tilt: 0, arch: 0.3, low: 0.05, smile: 0, pupil: 0.5, size: 1, skew: 0 };

/* Working reads. Short steps to the right along a line, one long return. */
const READING: [number, number, number][] = [
  [-0.2, 0.12, 0.42],
  [-0.08, 0.12, 0.3],
  [0.04, 0.12, 0.34],
  [0.16, 0.13, 0.4],
  [-0.2, 0.16, 0.46],
  [-0.06, 0.16, 0.28],
  [0.08, 0.16, 0.36],
  [0.18, 0.17, 0.52],
];

const FACES: Record<Mood, Face> = {
  idle: {
    lid: REST,
    gaze: { range: 0.4, hz: 0.32, cross: 0.22, far: 0.3 },
  },
  listening: {
    lid: { ...REST, open: -0.04, tilt: -4, arch: 0.4, low: 0, pupil: 0.6, size: 1.06 },
    gaze: { range: 0.05, hz: 0.5, cross: 0.28 },
  },
  thinking: {
    lid: { ...REST, open: 0.3, tilt: -6, arch: 0.18, low: 0.16, pupil: 0.46, skew: 0.3 },
    gaze: { range: 0.2, hz: 0.55, cross: 0.2, bias: [-0.22, -0.2], far: 0.25 },
  },
  working: {
    lid: { ...REST, open: 0.42, tilt: 0, arch: 0.1, low: 0.26, pupil: 0.46 },
    gaze: { cross: 0.07, script: READING },
  },
  frustrated: {
    lid: { ...REST, open: 0.4, tilt: 30, arch: -0.06, low: 0.32, smile: -0.12, pupil: 0.36, size: 0.97 },
    gaze: { range: 0.34, hz: 1.3, cross: 0.09, far: 0.25, near: 0.12 },
    blink: "slow",
    jitter: 0.05,
  },
  blocked: {
    lid: { ...REST, open: 0.06, tilt: -10, arch: 0.3, low: 0.04, pupil: 0.5, size: 1.03 },
    blink: "slow",
    squint: { open: 0.32, tilt: 22, skew: 0.34, low: 0.18 },
    gaze: {
      cross: 0.3,
      script: [
        [0, 0.02, 1.7],
        [0.24, -0.26, 1.2],
        [0.28, -0.3, 0.9],
        [0, 0.02, 1.5],
      ],
    },
  },
  pleased: {
    lid: { ...REST, open: 0.24, tilt: -3, arch: 0.26, low: 0.86, smile: 1.05, pupil: 0.52, size: 1.04 },
    blink: "slow",
    gaze: { range: 0.08, hz: 0.3, cross: 0.4, bias: [0, -0.06] },
  },
  paused: {
    lid: { ...REST, open: 1, low: 0.4, smile: -0.32, pupil: 0.5 },
    blink: false,
  },
  stuck: {
    lid: { ...REST, open: 0.26, tilt: -24, arch: 0.12, low: 0.12, smile: -0.08, pupil: 0.4, skew: 0.12 },
    blink: "slow",
    jitter: 0.02,
    gaze: { range: 0.16, hz: 1.2, cross: 0.08, bias: [0, 0.08] },
  },
  surprised: {
    lid: { ...REST, open: -0.12, tilt: -3, arch: 0.5, low: -0.08, pupil: 0.27, size: 1.2 },
    blink: false,
    gaze: { range: 0.015, hz: 3, cross: 0.05 },
  },
};

function mixLid(a: Lid, b: Lid, u: number): Lid {
  const out = { ...a };
  for (const k of Object.keys(a) as (keyof Lid)[]) out[k] = lerp(a[k], b[k], u);
  return out;
}

/** Where the lid lines cross the eye, sampled across it. */
const SAMPLES = 25;

interface EyeParts {
  opening: SVGPathElement;
  lashClip: SVGEllipseElement;
  sclera: SVGPathElement;
  pupil: SVGCircleElement;
  shade: SVGPathElement;
  lash: SVGPathElement;
  under: SVGPathElement;
}

function eyeParts(svg: SVGSVGElement, defs: SVGDefsElement): EyeParts {
  const id = uid("lid");
  const clipNode = el("clipPath", { id: `${id}-in` }, defs);
  const opening = el("path", {}, clipNode);
  const lashNode = el("clipPath", { id: `${id}-lash` }, defs);
  const lashClip = el("ellipse", {}, lashNode);
  const sclera = el("path", { fill: "#fbf8f1" }, svg);
  const inside = el("g", { "clip-path": `url(#${id}-in)` }, svg);
  const pupil = el("circle", { fill: INK }, inside);
  const shade = el("path", { fill: INK, opacity: 0.16 }, inside);
  const edge = el(
    "g",
    {
      "clip-path": `url(#${id}-lash)`,
      fill: "none",
      stroke: INK,
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
    },
    svg,
  );
  const lash = el("path", {}, edge);
  const under = el("path", {}, edge);
  return { opening, lashClip, sclera, pupil, shade, lash, under };
}

/**
 * One eye, drawn. `side` is -1 for the eye on the viewer's left and 0 for a
 * single eye. The inner end of each lid is the one toward the other eye, which
 * is what a tilt drops.
 *
 * What is drawn is the opening, not the lids: the white between the two lid
 * lines, cut to the eyeball. Drawing lids in the body's color over a white
 * eyeball leaves a hairline of white round a shut eye wherever the two edges
 * antialias, and the body is already there to be the lid.
 */
function drawEye(
  p: EyeParts,
  x: number,
  y: number,
  E: number,
  side: number,
  lid: Lid,
  look: Point,
  px: number,
) {
  const rx = E * lid.size;
  const ry = E * lid.size;
  const shut = 0.22 * ry;
  const L = rx * 1.3;
  const slope = Math.tan((lid.tilt * Math.PI) / 180);
  const lowSlope = slope * 0.25;
  /* A single eye has no inner end, so its middle is what a tilt drops. */
  const across = (lx: number) => (side === 0 ? L * 0.5 - Math.abs(lx) : -side * lx);
  const bow = (lx: number) => 1 - (lx / L) ** 2;
  /* The upper lid closes onto the lower one, wherever that is, so a shut eye
     is one line and never a line with white under it. */
  const lowerAt = (lx: number) =>
    ry * 1.02 - (ry * 1.02 - shut) * lid.low + lowSlope * across(lx) - lid.smile * ry * bow(lx);
  const upperAt = (lx: number) =>
    Math.min(
      lerp(-ry * 1.04 - lid.arch * ry * bow(lx), lowerAt(lx), lid.open) + slope * across(lx),
      lowerAt(lx),
    );

  const top: Point[] = [];
  const bottom: Point[] = [];
  const lashLine: Point[] = [];
  const lowLine: Point[] = [];
  for (let i = 0; i < SAMPLES; i++) {
    const s = i / (SAMPLES - 1);
    const ex = -rx + 2 * rx * s;
    const h = ry * Math.sqrt(Math.max(0, 1 - (ex / rx) ** 2));
    let t = Math.max(-h, upperAt(ex));
    let d = Math.min(h, lowerAt(ex));
    if (t > d) {
      const mid = clamp((t + d) / 2, -h, h);
      t = mid;
      d = mid;
    }
    top.push([x + ex, y + t]);
    bottom.push([x + ex, y + d]);
    const lx = -L + 2 * L * s;
    lashLine.push([x + lx, y + upperAt(lx)]);
    lowLine.push([x + lx, y + lowerAt(lx)]);
  }
  const opening = poly([...top, ...bottom.reverse()]);
  p.opening.setAttribute("d", opening);
  p.sclera.setAttribute("d", opening);
  p.lashClip.setAttribute("cx", x.toFixed(2));
  p.lashClip.setAttribute("cy", y.toFixed(2));
  p.lashClip.setAttribute("rx", (rx * 1.14).toFixed(2));
  p.lashClip.setAttribute("ry", (ry * 1.14).toFixed(2));

  /* The pupil, held inside the white by the same ellipse it is drawn in. */
  const P = lid.pupil * E;
  const roomX = rx - P * 0.8;
  const roomY = ry - P * 0.8;
  let ox = (look[0] / 0.42) * roomX;
  let oy = (look[1] / 0.42) * roomY;
  const out = Math.hypot(ox / Math.max(roomX, 0.01), oy / Math.max(roomY, 0.01));
  if (out > 1) {
    ox /= out;
    oy /= out;
  }
  p.pupil.setAttribute("cx", (x + ox).toFixed(2));
  p.pupil.setAttribute("cy", (y + oy).toFixed(2));
  p.pupil.setAttribute("r", P.toFixed(2));

  /* The lid's shadow on the eyeball: a band under the lid line, faint. */
  p.shade.setAttribute(
    "d",
    poly([...lashLine.map(([a, b]) => [a, b - ry] as Point), ...lashLine.map(([a, b]) => [a, b + ry * 0.22] as Point).reverse()]),
  );

  const closed = clamp((lid.open - 0.55) / 0.45);
  const w = 0.6 * hint(px, 44, 2.2) * (1 + 0.9 * closed);
  p.lash.setAttribute("d", poly(lashLine, false));
  p.lash.setAttribute("stroke-width", w.toFixed(2));
  p.under.setAttribute("d", poly(lowLine, false));
  p.under.setAttribute("stroke-width", (w * (0.5 + 0.5 * clamp((lid.low - 0.4) * 2))).toFixed(2));
  p.under.setAttribute("opacity", clamp((lid.low - 0.35) * 2).toFixed(2));
}

export const lids: Design = {
  key: "lids",
  name: "A. Lids",
  create(svg, who) {
    const shape = paper(svg, uid("lidbody"));
    const defs = el("defs", {}, svg);
    const one = Boolean(who.eye.one);
    const E = one ? who.eye.r * 1.95 : who.eye.r * 1.5;
    const spread = one ? 0 : Math.max(who.eye.spread * 1.04, E * 1.3);
    const eyes = one ? [eyeParts(svg, defs)] : [eyeParts(svg, defs), eyeParts(svg, defs)];
    const mark = el("g", { class: "avatar__mark" }, svg);
    const look: Point = [0, 0];
    const vel: Point = [0, 0];
    let marked: Mood | null = null;
    let lastLook: string | null = null;
    let lookBlink = -99;

    return {
      paint(f) {
        if (marked !== f.mood) {
          marked = f.mood;
          mark.innerHTML = markFor(f.mood);
          svg.classList.toggle("is-dim", Boolean(MOODS[f.mood].dim));
        }
        const u = f.live ? smooth(f.since / 0.5) : 1;
        const to = FACES[f.mood];
        const from = FACES[f.from];

        /* Where it looks, and whether this is a jump big enough to blink into. */
        let target: Point;
        let jumpBlink = 0;
        const lookKey = f.look ?? (f.point ? "you" : "mood");
        if (lookKey !== lastLook) {
          if (lastLook !== null && f.live) lookBlink = f.now;
          lastLook = lookKey;
        }
        if (f.look) target = [0, f.look === "up" ? -AIM.up * 1.15 : AIM.down];
        else if (!f.live) target = [0, 0];
        else if (f.point) target = f.point;
        else {
          const a = saccade(f.t, from.gaze);
          const b = saccade(f.t, to.gaze);
          target = [lerp(a.at[0], b.at[0], u), lerp(a.at[1], b.at[1], u)];
          const g = u > 0.5 ? b : a;
          /* Lids lead a large saccade by a few frames, as they do in a face. */
          if (g.size > 0.2 && rnd(g.slot + 0.19) < 0.7) jumpBlink = bump(g.jump + 0.05, 0.17);
        }
        if (f.live) follow(look, vel, target, f.dt, 0.09);
        else {
          look[0] = target[0];
          look[1] = target[1];
        }

        /* The body is today's body, answering most of the look. The pupil is
           what took the rest. */
        const bodyLook: Point = [look[0] * 0.8, look[1] * 0.8];
        if (f.gesture && f.live && f.age < 0.9) {
          const wave = f.gesture === "send" ? Math.sin(f.age * 9.4) : Math.cos(f.age * 13.8);
          const away = f.look === "down" ? -1 : 1;
          bodyLook[1] += away * 0.28 * wave * Math.exp(-f.age / 0.29);
        }
        const next = bodyPoints(who, MOODS[f.mood].shape, f.t, bodyLook).pts;
        const pts = u >= 1 ? next : blend(bodyPoints(who, MOODS[f.from].shape, f.t, bodyLook).pts, next, u);
        shape.setAttribute("d", outline(pts));

        let lid = u >= 1 ? to.lid : mixLid(from.lid, to.lid, u);
        const face = u > 0.5 ? to : from;

        const up = clamp(-look[1] / 0.28);
        const down = clamp(look[1] / 0.28);
        if (face.squint && up > 0) {
          const s = face.squint;
          lid = {
            ...lid,
            open: lid.open + up * (s.open ?? 0),
            tilt: lid.tilt + up * (s.tilt ?? 0),
            skew: lid.skew + up * (s.skew ?? 0),
            low: lid.low + up * (s.low ?? 0),
          };
        }
        /* The upper lid rides the pupil down. This, not the stroke, is what
           makes a downward look read as looking down. */
        lid = { ...lid, open: lid.open + down * 0.32 - up * 0.1, low: lid.low + up * 0.06 };
        if (f.point) lid = { ...lid, pupil: lid.pupil * 1.1 };

        let blink = 0;
        if (f.live && face.blink !== false) {
          blink = Math.max(blinkAt(f.t * (face.blink === "slow" ? 0.5 : 0.85)), jumpBlink);
        }
        if (f.live) blink = Math.max(blink, bump(f.now - lookBlink, 0.18));

        const seen = Math.hypot(bodyLook[0], bodyLook[1]);
        const lead = seen > 0.004 ? (grip(seen) / seen) * 0.9 : 0;
        const jit = f.live && face.jitter ? face.jitter : 0;
        const x = C + (who.eye.x ?? 0) + look[0] * R * 0.42 + bodyLook[0] * R * lead + Math.sin(f.t * 41) * jit * E;
        const y = C + (who.eye.y ?? 0) + look[1] * R * 0.3 + Math.sin(f.t * 33 + 1.7) * jit * E * 0.7;
        const sep = spread * (1 - Math.abs(look[0]) * 0.35);
        const turn = look[0] * 0.25;

        eyes.forEach((parts, i) => {
          const side = one ? 0 : i === 0 ? -1 : 1;
          const mine = side > 0 ? lid.skew : 0;
          const own = { ...lid, open: lid.open + mine };
          own.open = own.open + (1 - own.open) * blink;
          own.low = own.low + Math.max(0, 0.4 - own.low) * blink;
          drawEye(parts, x + side * sep, y, E * (1 + side * turn), side, own, look, f.px);
        });
      },
    };
  },
};
