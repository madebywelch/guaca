import { SILHOUETTES, type Silhouette } from "./silhouette";

export const FORM = {
  box: 64,
  center: 32,
  radius: 20,
  reach: 30,
  samples: 32,
} as const;

const TAU = Math.PI * 2;

export interface Lobe {
  k: number;
  amp: number;
  phase: number;
}

export interface Ripple extends Omit<Lobe, "phase"> {
  spd: number;
}

export interface Press {
  th: number;
  w: number;
  amp: number;
  spin?: number;
  beat?: number;
}

export interface Lump {
  key: string;
  label: string;
  form: Silhouette;
  ax: number;
  ay: number;
  sig: Lobe[];
  eye: {
    spread: number;
    r: number;
    x?: number;
    y?: number;
    one?: boolean;
  };
}

export interface Shape {
  aspect?: [number, number];
  knead?: { amp: number; hz: number; sharp?: boolean };
  wob?: Ripple[];
  press?: Press[];
  sag?: number;
  spread?: number;
  rise?: number;
  heave?: { amp: number; hz: number };
}

export type Point = [number, number];

export function pulse(t: number, hz: number, sharp?: boolean): number {
  const u = (t * hz) % 1;
  if (!sharp) return Math.sin(u * TAU);
  return u < 0.3 ? Math.sin((u / 0.3) * Math.PI) : -0.34 * Math.sin(((u - 0.3) / 0.7) * Math.PI);
}

export const PULL = {
  quiet: 0.06,
  wide: 0.26,
  hold: 0.38,
  stretch: 1.2,
  taper: 1.1,
  back: 0.15,
  lead: 0.6,
  lean: 0.1,
  shear: 0.2,
  pivot: 0.45,
  crane: 0.1,
};

function smooth(u: number): number {
  const e = Math.max(0, Math.min(1, u));
  return e * e * e * (e * (e * 6 - 15) + 10);
}

export function grip(reach: number): number {
  const held = Math.min(reach, PULL.hold);
  return held * smooth((held - PULL.quiet) / (PULL.wide - PULL.quiet));
}

export function bodyPoints(lump: Lump, shape: Shape, t: number, gaze?: Point) {
  const seen = Math.hypot(gaze ? gaze[0] : 0, gaze ? gaze[1] : 0);
  const reach = seen > 0.004 ? grip(seen) : 0;
  const gx = gaze && seen > 0.004 ? (gaze[0] / seen) * reach : 0;
  const gy = gaze && seen > 0.004 ? (gaze[1] / seen) * reach : 0;
  const R = FORM.radius;

  const cx = FORM.center + gx * R * PULL.lean;
  const cy = FORM.center + (shape.rise ?? 0) + gy * R * PULL.lean;

  const knead = shape.knead ? pulse(t, shape.knead.hz, shape.knead.sharp) * shape.knead.amp : 0;
  const crane = reach * PULL.crane;
  const ax = lump.ax * (shape.aspect ? shape.aspect[0] : 1) * (1 + knead * 0.9) * (1 - crane * 0.5);
  const ay = lump.ay * (shape.aspect ? shape.aspect[1] : 1) * (1 - knead) * (1 + crane);

  const heave = shape.heave
    ? Math.max(0, Math.sin(t * shape.heave.hz * TAU)) ** 3 * shape.heave.amp
    : 0;

  const ux = reach > 0.004 ? gx / reach : 1;
  const uy = reach > 0.004 ? gy / reach : 0;
  const silhouette = SILHOUETTES[lump.form];
  const walk = (swell: number): Point[] => {
    const pts: Point[] = [];
    for (let i = 0; i < FORM.samples; i++) {
      const a = (i / FORM.samples) * TAU;
      let rr = silhouette(a);
      for (const lobe of lump.sig) rr += lobe.amp * Math.sin(lobe.k * a + lobe.phase);
      if (shape.wob) for (const w of shape.wob) rr += w.amp * Math.sin(w.k * a + w.spd * t);
      if (shape.press) {
        for (const p of shape.press) {
          const amp = p.beat ? p.amp * (0.55 + 0.45 * Math.sin(t * p.beat)) : p.amp;
          rr += press(a, p.th + (p.spin ? p.spin * t : 0), p.w, amp);
        }
      }
      rr += heave * Math.max(0, -Math.sin(a));

      let x = cx + Math.cos(a) * R * rr * ax;
      let y = cy + Math.sin(a) * R * rr * (ay + heave * 0.5);

      if (reach > 0.004) {
        const dx = (x - cx) / R;
        const dy = (y - cy) / R;
        const u = dx * ux + dy * uy;
        const v = -dx * uy + dy * ux;
        const s = smooth((u + PULL.back) / (1 + PULL.back));
        const u2 = u + swell * s * s;
        const v2 = v * (1 - reach * PULL.taper * s * s);
        x = cx + (u2 * ux - v2 * uy) * R;
        y = cy + (u2 * uy + v2 * ux) * R;
      }

      x += (cy + PULL.pivot * R - y) * gx * PULL.shear;

      if (shape.sag) {
        const below = Math.max(0, (y - cy) / R);
        y += shape.sag * below * below;
        x = cx + (x - cx) * (1 + (shape.spread ?? 0) * below);
      }
      pts.push([x, y]);
    }
    return pts;
  };

  let swell = reach * PULL.stretch;
  let pts = walk(swell);
  if (swell > 0) {
    let rest = Number.NaN;
    for (let pass = 0; pass < 3; pass++) {
      const far = furthest(pts);
      if (far <= FORM.reach) break;
      if (Number.isNaN(rest)) rest = furthest(walk(0));
      swell = rest >= FORM.reach ? 0 : swell * ((FORM.reach - rest) / (far - rest)) * 0.98;
      pts = walk(swell);
    }
  }
  return { pts, knead };
}

function furthest(pts: Point[]): number {
  let most = 0;
  for (const [x, y] of pts) most = Math.max(most, Math.hypot(x - FORM.center, y - FORM.center));
  return most;
}

export function press(a: number, th: number, w: number, amp: number): number {
  let d = a - th * TAU;
  while (d > Math.PI) d -= TAU;
  while (d < -Math.PI) d += TAU;
  return amp * Math.exp(-(d * d) / (w * w));
}

export function blend(a: Point[], b: Point[], u: number): Point[] {
  const out: Point[] = [];
  for (let i = 0; i < a.length; i++) {
    const p = a[i] as Point;
    const q = b[i] as Point;
    out.push([p[0] + (q[0] - p[0]) * u, p[1] + (q[1] - p[1]) * u]);
  }
  return out;
}

export function outline(pts: Point[]): string {
  const first = pts[0] as Point;
  const d = [`M${first[0].toFixed(2)} ${first[1].toFixed(2)}`];
  for (let i = 0; i < pts.length; i++) {
    const p0 = pts[(i - 1 + pts.length) % pts.length] as Point;
    const p1 = pts[i] as Point;
    const p2 = pts[(i + 1) % pts.length] as Point;
    const p3 = pts[(i + 2) % pts.length] as Point;
    const c1x = p1[0] + (p2[0] - p0[0]) / 6;
    const c1y = p1[1] + (p2[1] - p0[1]) / 6;
    const c2x = p2[0] - (p3[0] - p1[0]) / 6;
    const c2y = p2[1] - (p3[1] - p1[1]) / 6;
    d.push(
      `C${c1x.toFixed(2)} ${c1y.toFixed(2)} ${c2x.toFixed(2)} ${c2y.toFixed(2)} ${p2[0].toFixed(2)} ${p2[1].toFixed(2)}`,
    );
  }
  return d.join("");
}
