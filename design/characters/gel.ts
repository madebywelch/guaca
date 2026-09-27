/**
 * B. Gel. The body is simulated, not a function of time.
 *
 * Thirty-two point masses are held to the mood's rest shape by springs (shape
 * matching, translation only, so a creature never rolls over), with a pressure
 * term that keeps the area and a floor they stand on. A mood is a rest shape
 * and a tension; changing mood changes the rest shape and the body flows into
 * it. Everything that reads as weight (overshoot, settle, squash on landing,
 * a wobble that dies) falls out of that rather than being keyframed.
 *
 * The look is spent the way `form.ts` spends it, as a pear pulled after the
 * eyes, but on the goal rather than on the drawing, so the mass arrives a
 * fraction late and overshoots a little. That lag is the thing CHARACTERS.md
 * removed; it is back here at a tenth of the size, because a body that answers
 * on the same frame has no mass.
 */

import { AIM } from "../../src/avatars/eyes";
import { grip, outline } from "../../src/avatars/form";
import { markFor, type Mood } from "../../src/avatars/moods";
import { SILHOUETTES, type Silhouette } from "../../src/avatars/silhouette";
import {
  blinkAt,
  C,
  clamp,
  type Design,
  el,
  follow,
  type GazeSpec,
  hint,
  lerp,
  mix,
  type Point,
  poly,
  R,
  REACH,
  rnd,
  saccade,
  smooth,
  TAU,
  uid,
} from "./shared";

const N = 32;

/* --- the five rest shapes -------------------------------------------------- */

/** A gel cannot hold a corner. The square is a pillow and the octagon a gumdrop. */
type Rest = "ball" | "pillow" | "gumdrop" | "drop" | "cloud";

const FROM: Record<Silhouette, Rest> = {
  circle: "ball",
  square: "pillow",
  octagon: "gumdrop",
  drop: "drop",
  cloud: "cloud",
};

const RAW: Record<Rest, (a: number) => number> = {
  ball: () => 1,
  pillow: (a) => (Math.abs(Math.cos(a)) ** 3.4 + Math.abs(Math.sin(a)) ** 3.4) ** (-1 / 3.4),
  /* A dome on a flat base: a superellipse that is round on top and squared off
     underneath. */
  gumdrop: (a) => {
    const n = Math.sin(a) > 0 ? 4.2 : 2;
    const ax = 1.08;
    const ay = Math.sin(a) > 0 ? 0.78 : 1.02;
    const c = Math.abs(Math.cos(a)) / ax;
    const s = Math.abs(Math.sin(a)) / ay;
    return (c ** n + s ** n) ** (-1 / n);
  },
  drop: SILHOUETTES.drop,
  cloud: SILHOUETTES.cloud,
};

function area(shape: (a: number) => number): number {
  let sum = 0;
  const steps = 1024;
  for (let i = 0; i < steps; i++) sum += 0.5 * shape((i / steps) * TAU) ** 2 * (TAU / steps);
  return sum;
}

const SCALE = Object.fromEntries(
  (Object.keys(RAW) as Rest[]).map((k) => [k, Math.sqrt(Math.PI / area(RAW[k]))]),
) as Record<Rest, number>;

/* --- moods ------------------------------------------------------------------ */

/** A bead eye, in bead radii. The bead is the whole eye, dark, with a light in it. */
interface Bead {
  /** Upper cut: 0 clear, 1 to the middle. The brow, cut across the bead. */
  cut: number;
  /** How far the upper cut arches in its middle. Negative sags: a sleeping eye. */
  arch: number;
  /** Degrees. Positive drops the inner end. */
  tilt: number;
  /** Lower cut: 0 clear, 1 to the middle, bowed up by `smile`. */
  low: number;
  smile: number;
  /** Height and width of the bead. */
  h: number;
  w: number;
  /** Catch-light size. A second, lower light is a wet eye. */
  glint: number;
  wet: number;
  /** The creature's right cut lower than its left. */
  skew: number;
}

interface Gel {
  /** Spring toward the rest shape, 1/s². High is tense. */
  k: number;
  /** Fraction of critical damping. Low wobbles. */
  zeta: number;
  aspect: [number, number];
  /** Degrees the rest shape leans: a head tilt. */
  tilt: number;
  /** Rest area against the resting body. Under 1 is deflated. */
  volume: number;
  gravity: number;
  /** A random shove on every point, units/s². */
  tremble?: number;
  /** A squeeze on a beat. */
  pulse?: { hz: number; amp: number };
  /** An effort upward that fails, on a beat. */
  heave?: { hz: number; amp: number };
  breath?: { hz: number; amp: number };
  /** What entering this mood does to the top of the body, units/s. Negative is up. */
  enter?: number;
  bead: Bead;
  gaze?: GazeSpec;
  blink?: boolean | "slow";
}

const EYE: Bead = { cut: 0, arch: 0, tilt: 0, low: 0, smile: 0, h: 1, w: 1, glint: 1, wet: 0, skew: 0 };
const BASE: Gel = { k: 420, zeta: 0.42, aspect: [1, 1], tilt: 0, volume: 1, gravity: 170, bead: EYE };

const GELS: Record<Mood, Gel> = {
  idle: { ...BASE, gaze: { range: 0.4, hz: 0.32, cross: 0.24, far: 0.3 } },
  listening: {
    ...BASE,
    k: 460,
    aspect: [0.97, 1.04],
    bead: { ...EYE, h: 1.08, w: 1.04, glint: 1.25 },
    gaze: { range: 0.05, hz: 0.5, cross: 0.3 },
  },
  thinking: {
    ...BASE,
    k: 340,
    tilt: -5,
    bead: { ...EYE, cut: 0.28, tilt: -8, skew: 0.3, h: 0.96 },
    gaze: { range: 0.2, hz: 0.6, cross: 0.22, bias: [-0.2, -0.2], far: 0.25 },
  },
  working: {
    ...BASE,
    k: 520,
    zeta: 0.3,
    pulse: { hz: 1.1, amp: 0.07 },
    bead: { ...EYE, cut: 0.42, tilt: 6, low: 0.1, glint: 0.8 },
    gaze: { range: 0.14, hz: 2.2, cross: 0.1, bias: [0, 0.1] },
  },
  frustrated: {
    ...BASE,
    k: 900,
    zeta: 0.22,
    aspect: [1.05, 0.95],
    tremble: 900,
    enter: 40,
    bead: { ...EYE, cut: 0.5, tilt: 28, low: 0.16, glint: 0.55 },
    gaze: { range: 0.34, hz: 1.4, cross: 0.1, far: 0.22, near: 0.12 },
    blink: "slow",
  },
  blocked: {
    ...BASE,
    k: 380,
    tilt: 6,
    bead: { ...EYE, cut: 0.12, tilt: -12, skew: 0.32, glint: 1.1 },
    blink: "slow",
    gaze: {
      cross: 0.34,
      script: [
        [0, 0.02, 1.7],
        [0.24, -0.25, 1.2],
        [0.28, -0.3, 0.9],
        [0, 0.02, 1.5],
      ],
    },
  },
  pleased: {
    ...BASE,
    k: 380,
    zeta: 0.25,
    aspect: [1.04, 0.97],
    enter: -70,
    bead: { ...EYE, low: 0.9, smile: 1.2, cut: 0.05, glint: 0 },
    blink: false,
    gaze: { range: 0.08, hz: 0.3, cross: 0.4, bias: [0, -0.05] },
  },
  paused: {
    ...BASE,
    k: 120,
    zeta: 0.9,
    aspect: [1.07, 0.93],
    volume: 0.9,
    gravity: 360,
    breath: { hz: 0.13, amp: 0.035 },
    bead: { ...EYE, cut: 1, arch: -0.55, low: 1, smile: -0.6, glint: 0 },
    blink: false,
  },
  stuck: {
    ...BASE,
    k: 170,
    zeta: 0.35,
    aspect: [1.08, 0.92],
    volume: 0.94,
    gravity: 300,
    heave: { hz: 0.28, amp: 190 },
    bead: { ...EYE, cut: 0.3, tilt: -26, h: 1.05, glint: 1.35, wet: 1 },
    blink: "slow",
    gaze: { range: 0.14, hz: 1.2, cross: 0.1, bias: [0, 0.08] },
  },
  surprised: {
    ...BASE,
    k: 600,
    zeta: 0.2,
    aspect: [0.94, 1.08],
    volume: 1.05,
    enter: -120,
    bead: { ...EYE, h: 1.28, w: 1.12, glint: 1.4 },
    blink: false,
    gaze: { range: 0.015, hz: 3, cross: 0.05 },
  },
};

function mixBead(a: Bead, b: Bead, u: number): Bead {
  const out = { ...a };
  for (const k of Object.keys(a) as (keyof Bead)[]) out[k] = lerp(a[k], b[k], u);
  return out;
}

/* --- drawing a bead -------------------------------------------------------- */

const ARC = 24;

/** The dark of one eye: an ellipse between two cuts, never thinner than a line. */
function beadPath(x: number, y: number, rx: number, ry: number, b: Bead, side: number, line: number) {
  const slope = Math.tan((b.tilt * Math.PI) / 180);
  const across = (lx: number) => (side === 0 ? rx * 0.5 - Math.abs(lx) : -side * lx);
  const yU = -ry + ry * b.cut;
  const yL = ry - ry * b.low;
  const top: Point[] = [];
  const bottom: Point[] = [];
  for (let i = 0; i <= ARC; i++) {
    const lx = -rx + (2 * rx * i) / ARC;
    const h = ry * Math.sqrt(Math.max(0, 1 - (lx / rx) ** 2));
    const bow = 1 - (lx / rx) ** 2;
    let t = Math.max(-h, yU + slope * across(lx) - b.arch * ry * bow);
    let d = Math.min(h, yL - b.smile * ry * bow * 0.9);
    if (d - t < line) {
      const mid = (t + d) / 2;
      const room = Math.min(line, 2 * h);
      t = mid - room / 2;
      d = mid + room / 2;
    }
    top.push([x + lx, y + t]);
    bottom.push([x + lx, y + d]);
  }
  return { d: poly([...top, ...bottom.reverse()]), open: clamp(1 - b.cut - b.low * 0.8) };
}

/* --- the creature -------------------------------------------------------------- */

export const gel: Design = {
  key: "gel",
  name: "B. Gel",
  create(svg, who, color, seed) {
    const id = uid("gel");
    const rest = FROM[who.form];
    const shapeAt = (a: number) => {
      let r = RAW[rest](a) * SCALE[rest];
      for (const lobe of who.sig) r += lobe.amp * Math.sin(lobe.k * a + lobe.phase);
      return r;
    };
    const dirs: Point[] = [];
    const base: Point[] = [];
    for (let i = 0; i < N; i++) {
      const a = (i / N) * TAU;
      dirs.push([Math.cos(a), Math.sin(a)]);
      const r = shapeAt(a) * R;
      base.push([Math.cos(a) * r * who.ax, Math.sin(a) * r * who.ay]);
    }
    const bottomOf = Math.max(...base.map((q) => q[1]));
    const middle = base.reduce((sum, q) => sum + q[1], 0) / N;
    const center = base.reduce((sum, q) => sum + q[0], 0) / N;
    const floor = C + bottomOf;
    const area0 = polyArea(base);

    const p = base.map(([qx, qy]) => [C + qx, C + qy] as Point);
    const v = base.map(() => [0, 0] as Point);

    const defs = el("defs", {}, svg);
    const grad = el("radialGradient", { id: `${id}-g`, cx: 0.36, cy: 0.3, r: 0.85, fx: 0.32, fy: 0.24 }, defs);
    el("stop", { offset: 0, "stop-color": mix(color, "#ffffff", 0.38) }, grad);
    el("stop", { offset: 0.5, "stop-color": color }, grad);
    el("stop", { offset: 1, "stop-color": mix(color, "#1a1a18", 0.34) }, grad);
    const spec = el("radialGradient", { id: `${id}-s` }, defs);
    el("stop", { offset: 0, "stop-color": "#fff", "stop-opacity": 0.85 }, spec);
    el("stop", { offset: 1, "stop-color": "#fff", "stop-opacity": 0 }, spec);
    const clipNode = el("clipPath", { id: `${id}-c` }, defs);
    const clipPath = el("path", {}, clipNode);

    const shadow = el("ellipse", { fill: "#1a1a18", class: "gel__shadow" }, svg);
    const tint = el("g", { class: "gel__tint" }, svg);
    const body = el("path", { fill: `url(#${id}-g)` }, tint);
    const lit = el("g", { "clip-path": `url(#${id}-c)` }, tint);
    const rim = el("path", { fill: "none", stroke: mix(color, "#ffffff", 0.55), "stroke-opacity": 0.55 }, lit);
    const shine = el("ellipse", { fill: `url(#${id}-s)` }, lit);
    const beads = (who.eye.one ? [0] : [0, 1]).map(() => ({
      dark: el("path", { fill: "#141614" }, tint),
      glint: el("circle", { fill: "#fff" }, tint),
      wet: el("circle", { fill: "#fff", opacity: 0.85 }, tint),
    }));
    const mark = el("g", { class: "avatar__mark" }, svg);

    const look: Point = [0, 0];
    const lookVel: Point = [0, 0];
    let marked: Mood | null = null;
    let entered: Mood | null = null;
    let sendKick = false;
    let receiveKick = false;
    let lastGesture = -99;
    const jitterSeed = rnd(seed.length * 7.3);

    const goals = (g: Gel, t: number, pull: Point): Point[] => {
      const th = (g.tilt * Math.PI) / 180;
      const cos = Math.cos(th);
      const sin = Math.sin(th);
      const pulse = g.pulse ? pulseAt(t, g.pulse.hz) * g.pulse.amp : 0;
      const breath = g.breath ? Math.sin(t * g.breath.hz * TAU) * g.breath.amp : 0;
      const sx = g.aspect[0] * (1 + pulse * 0.8 + breath * 0.5);
      const sy = g.aspect[1] * (1 - pulse + breath);
      const vol = Math.sqrt(g.volume);
      const reach = Math.hypot(pull[0], pull[1]);
      const held = reach > 0.004 ? grip(reach) : 0;
      const ux = reach > 0.004 ? pull[0] / reach : 1;
      const uy = reach > 0.004 ? pull[1] / reach : 0;
      const out = base.map(([qx, qy]): Point => {
        let x = qx * sx * vol;
        let y = qy * sy * vol;
        /* Tilted about the base, not the middle, so a head tilt keeps its feet. */
        const by = y - bottomOf;
        const rx = x * cos - by * sin;
        const ry = x * sin + by * cos + bottomOf;
        x = rx;
        y = ry;
        if (held > 0) {
          /* The pear from `form.ts`: the front drawn out and narrowed, the
             back left round. Without the narrowing a pulled gel is a point. */
          const along = (x * ux + y * uy) / R;
          const side = (-x * uy + y * ux) / R;
          const s = smooth((along + 0.15) / 1.15);
          const u2 = along + held * 1.1 * s * s;
          const v2 = side * (1 - held * 1.1 * s * s);
          x = (u2 * ux - v2 * uy) * R;
          y = (u2 * uy + v2 * ux) * R;
        }
        return [x, y];
      });
      /* Goals are offsets from the body's own centroid, so they have to sum
         to nothing: a pull that moved their mean would carry the whole body
         off the floor after the look instead of stretching it. */
      let mx = 0;
      let my = 0;
      for (const [x, y] of out) {
        mx += x;
        my += y;
      }
      mx /= N;
      my /= N;
      return out.map(([x, y]) => [x - mx, y - my] as Point);
    };

    return {
      paint(f) {
        if (marked !== f.mood) {
          marked = f.mood;
          mark.innerHTML = markFor(f.mood);
          svg.classList.toggle("is-dim", f.mood === "paused");
        }
        const g = GELS[f.mood];
        const from = GELS[f.from];
        const u = f.live ? smooth(f.since / 0.35) : 1;

        let target: Point;
        if (f.look) target = [0, f.look === "up" ? -AIM.up : AIM.down];
        else if (!f.live) target = [0, 0];
        else if (f.point) target = f.point;
        else {
          const a = saccade(f.t, from.gaze);
          const b = saccade(f.t, g.gaze);
          target = [lerp(a.at[0], b.at[0], u), lerp(a.at[1], b.at[1], u)];
        }
        if (f.live) follow(look, lookVel, target, f.dt, 0.1);
        else {
          look[0] = target[0];
          look[1] = target[1];
        }

        const goal = goals(g, f.t, look);
        if (!f.live) {
          for (let i = 0; i < N; i++) {
            const q = goal[i] as Point;
            (p[i] as Point)[0] = C + center + q[0];
            (p[i] as Point)[1] = Math.min(floor, C + middle + q[1]);
          }
        } else {
          /* What a mood does on the way in, and what a gesture does: velocity,
             given once, and the springs do the rest. */
          /* A beat late, so a message landing squashes first and the
             surprise stretches after it: anticipation, then the take. */
          if (entered !== f.mood && f.since > 0.09) {
            entered = f.mood;
            if (g.enter) kick(p, v, [0, g.enter], 1.4);
          }
          if (f.gesture && f.age < 0.05 && lastGesture !== f.now - f.age) {
            lastGesture = f.now - f.age;
            sendKick = f.gesture === "send";
            receiveKick = f.gesture === "receive";
          }
          if (receiveKick) {
            receiveKick = false;
            const from = f.look === "down" ? -1 : 1;
            kick(p, v, [0, 230 * from], 1.6);
          }
          if (sendKick && f.age > 0.12) {
            sendKick = false;
            const toward = f.look === "up" ? -1 : 1;
            kick(p, v, [0, 120 * toward], 1.2);
          }
          const wind = f.gesture === "send" && f.age < 0.12 ? 0.9 : 1;
          const steps = 4;
          const h = f.dt / steps;
          const w = Math.sqrt(g.k);
          const damp = 2 * w * g.zeta;
          for (let s = 0; s < steps; s++) {
            let cx = 0;
            let cy = 0;
            for (const [x, y] of p) {
              cx += x;
              cy += y;
            }
            cx /= N;
            cy /= N;
            const now = polyArea(p.map(([x, y]) => [x - cx, y - cy] as Point));
            const pressure = ((area0 * g.volume - now) / area0) * 1400;
            const heave = g.heave ? Math.max(0, Math.sin(f.t * g.heave.hz * TAU)) ** 4 * g.heave.amp : 0;
            for (let i = 0; i < N; i++) {
              const pt = p[i] as Point;
              const vel = v[i] as Point;
              const q = goal[i] as Point;
              const gx = cx + q[0];
              const gy = cy + q[1] * wind;
              const prev = p[(i - 1 + N) % N] as Point;
              const next = p[(i + 1) % N] as Point;
              const tx = next[0] - prev[0];
              const ty = next[1] - prev[1];
              const tl = Math.hypot(tx, ty) || 1;
              const nx = ty / tl;
              const ny = -tx / tl;
              let ax = g.k * (gx - pt[0]) - damp * vel[0] + pressure * nx;
              let ay = g.k * (gy - pt[1]) - damp * vel[1] + pressure * ny + g.gravity;
              const d = dirs[i] as Point;
              if (heave && d[1] < 0) ay -= heave * -d[1];
              if (g.tremble) {
                /* A shiver of the whole mass in its two lowest modes, not
                   noise per point: noise per point is a crumpled bag. */
                const th = (i / N) * TAU;
                const n =
                  Math.sin(f.t * 83 + jitterSeed * 40) * Math.cos(2 * th) +
                  0.6 * Math.sin(f.t * 67 + 1.3) * Math.cos(3 * th + 1);
                ax += g.tremble * n * d[0];
                ay += g.tremble * n * d[1];
              }
              vel[0] += ax * h;
              vel[1] += ay * h;
            }
            for (let i = 0; i < N; i++) {
              const pt = p[i] as Point;
              const vel = v[i] as Point;
              pt[0] += vel[0] * h;
              pt[1] += vel[1] * h;
              if (pt[1] > floor) {
                pt[1] = floor;
                if (vel[1] > 0) vel[1] = 0;
                vel[0] *= 1 - Math.min(1, 4 * h);
              }
              const dx = pt[0] - C;
              const dy = pt[1] - C;
              const far = Math.hypot(dx, dy);
              if (far > REACH - 1.5) {
                const k = (REACH - 1.5) / far;
                pt[0] = C + dx * k;
                pt[1] = C + dy * k;
                vel[0] *= 0.5;
                vel[1] *= 0.5;
              }
            }
          }
        }

        const d = outline(p);
        body.setAttribute("d", d);
        clipPath.setAttribute("d", d);
        rim.setAttribute("d", d);
        rim.setAttribute("stroke-width", (1.6 * hint(f.px, 44, 1.4)).toFixed(2));

        let minX = 99;
        let maxX = -99;
        let minY = 99;
        let maxY = -99;
        let footL = 99;
        let footR = -99;
        for (const [x, y] of p) {
          minX = Math.min(minX, x);
          maxX = Math.max(maxX, x);
          minY = Math.min(minY, y);
          maxY = Math.max(maxY, y);
          if (y > floor - 2.5) {
            footL = Math.min(footL, x);
            footR = Math.max(footR, x);
          }
        }
        const bw = maxX - minX;
        const bh = maxY - minY;
        shine.setAttribute("cx", (minX + bw * 0.3).toFixed(2));
        shine.setAttribute("cy", (minY + bh * 0.2).toFixed(2));
        shine.setAttribute("rx", (bw * 0.2).toFixed(2));
        shine.setAttribute("ry", (bh * 0.12).toFixed(2));
        const lift = clamp((floor - maxY) / 4);
        const foot = footR > footL ? footR - footL : bw * 0.4;
        shadow.setAttribute("cx", ((footL + footR) / 2 || C).toFixed(2));
        shadow.setAttribute("cy", (floor + 1.4).toFixed(2));
        shadow.setAttribute("rx", (foot * 0.5 + 5).toFixed(2));
        shadow.setAttribute("ry", "1.9");
        shadow.setAttribute("opacity", (0.2 * (1 - lift * 0.6)).toFixed(3));

        /* The face rides the top of the body: wherever the top half has been
           pushed from its rest, the eyes go with it. */
        let cx = 0;
        let cy = 0;
        for (const [x, y] of p) {
          cx += x;
          cy += y;
        }
        cx /= N;
        cy /= N;
        let tx = 0;
        let ty = 0;
        let count = 0;
        for (let i = 0; i < N; i++) {
          const q = goal[i] as Point;
          if (q[1] >= 0) continue;
          const pt = p[i] as Point;
          tx += pt[0] - (cx + q[0]);
          ty += pt[1] - (cy + q[1]);
          count += 1;
        }
        tx /= count || 1;
        ty /= count || 1;
        const seen = Math.hypot(look[0], look[1]);
        const lead = seen > 0.004 ? (grip(seen) / seen) * 0.8 : 0;
        const ex = cx - center + (who.eye.x ?? 0) + tx * 0.8 + look[0] * R * (0.55 + lead);
        const ey = cy - middle + (who.eye.y ?? 0) - 1 + ty * 0.8 + look[1] * R * 0.45;

        let bead = u >= 1 ? g.bead : mixBead(from.bead, g.bead, u);
        const watch = u > 0.5 ? g : from;
        let blink = 0;
        if (f.live && watch.blink !== false) blink = blinkAt(f.t * (watch.blink === "slow" ? 0.5 : 0.85));
        if (f.look === "down") bead = { ...bead, cut: bead.cut + 0.3 };
        bead = {
          ...bead,
          cut: bead.cut + (1.02 - bead.cut) * blink * 0.5,
          low: bead.low + (1.02 - bead.low) * blink * 0.5,
        };

        const r = who.eye.r * (who.eye.one ? 1.75 : 1.22);
        const spread = who.eye.one ? 0 : who.eye.spread * (1 - Math.abs(look[0]) * 0.4);
        const line = 0.7 * hint(f.px, 44, 2);
        beads.forEach((parts, i) => {
          const side = who.eye.one ? 0 : i === 0 ? -1 : 1;
          const mine = side > 0 ? { ...bead, cut: bead.cut + bead.skew } : bead;
          const size = 1 + side * look[0] * 0.25;
          const bx = ex + side * spread;
          const rx = r * mine.w * size;
          const ry = r * mine.h * size;
          const drawn = beadPath(bx, ey, rx, ry, mine, side, line);
          parts.dark.setAttribute("d", drawn.d);
          /* The light stays with the light. When the eye turns, the glint
             slides the other way across it, which is what reads as a turn. */
          const gr = r * 0.34 * mine.glint * drawn.open;
          parts.glint.setAttribute("cx", (bx - rx * 0.32 - look[0] * rx * 0.5).toFixed(2));
          parts.glint.setAttribute("cy", (ey - ry * 0.34 - look[1] * ry * 0.4 + ry * mine.cut * 0.5).toFixed(2));
          parts.glint.setAttribute("r", Math.max(0, gr).toFixed(2));
          const wr = r * 0.2 * mine.wet * drawn.open;
          parts.wet.setAttribute("cx", (bx + rx * 0.36).toFixed(2));
          parts.wet.setAttribute("cy", (ey + ry * 0.42).toFixed(2));
          parts.wet.setAttribute("r", Math.max(0, wr).toFixed(2));
        });
      },
    };
  },
};

/** Velocity added to the top of the body, most at the crown. */
function kick(p: Point[], v: Point[], dv: Point, power: number) {
  let top = 99;
  let bottom = -99;
  for (const [, y] of p) {
    top = Math.min(top, y);
    bottom = Math.max(bottom, y);
  }
  for (let i = 0; i < p.length; i++) {
    const k = clamp(((bottom - (p[i] as Point)[1]) / (bottom - top || 1)) ** power);
    (v[i] as Point)[0] += dv[0] * k;
    (v[i] as Point)[1] += dv[1] * k;
  }
}

function pulseAt(t: number, hz: number): number {
  const u = (t * hz) % 1;
  return u < 0.3 ? Math.sin((u / 0.3) * Math.PI) : -0.34 * Math.sin(((u - 0.3) / 0.7) * Math.PI);
}

function polyArea(pts: Point[]): number {
  let a = 0;
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i] as Point;
    const q = pts[(i + 1) % pts.length] as Point;
    a += p[0] * q[1] - q[0] * p[1];
  }
  return Math.abs(a) / 2;
}
