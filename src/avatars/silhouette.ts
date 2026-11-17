export type Silhouette = "circle" | "octagon" | "square" | "drop" | "cloud";

export const CREST = 1.06;

const TAU = Math.PI * 2;

function ball(a: number, cx: number, cy: number, r: number): number {
  const dx = Math.cos(a);
  const dy = Math.sin(a);
  const along = cx * dx + cy * dy;
  const off = cx * dy - cy * dx;
  const under = r * r - off * off;
  return under <= 0 ? 0 : Math.max(0, along + Math.sqrt(under));
}

function fromUp(a: number): number {
  let w = a + Math.PI / 2;
  while (w > Math.PI) w -= TAU;
  while (w <= -Math.PI) w += TAU;
  return w;
}

function drop(aspect: number): (a: number) => number {
  const b = 1;
  const k = (aspect - 1) * b;
  const h = k + b;
  const sin = b / (h + k);
  const cone = Math.asin(sin);
  const side = Math.sqrt((h + k) * (h + k) - b * b);
  const seam = Math.atan2(side * sin, h - side * Math.cos(cone));
  return (a) => {
    const w = fromUp(a);
    if (Math.abs(w) <= seam) return (h * sin) / Math.sin(Math.abs(w) + cone);
    return -k * Math.cos(w) + Math.sqrt(b * b - k * k * Math.sin(w) * Math.sin(w));
  };
}

function cloud(puffs: [number, number, number][], floor: number): (a: number) => number {
  return (a) => {
    let r = 0;
    for (const [cx, cy, br] of puffs) r = Math.max(r, ball(a, cx, cy, br));
    const down = Math.sin(a);
    return down > 1e-6 ? Math.min(r, floor / down) : r;
  };
}

function squeeze(shape: (a: number) => number, kx: number, ky: number): (a: number) => number {
  return (a) => {
    const ux = Math.cos(a) / kx;
    const uy = Math.sin(a) / ky;
    return shape(Math.atan2(uy, ux)) / Math.hypot(ux, uy);
  };
}

const RAW: Record<Silhouette, (a: number) => number> = {
  circle: () => 1,

  octagon: (a) => {
    const seg = Math.PI / 4;
    return Math.cos(seg / 2) / Math.cos(a - Math.round(a / seg) * seg);
  },

  square: (a) => {
    const n = 6;
    return (Math.abs(Math.cos(a)) ** n + Math.abs(Math.sin(a)) ** n) ** (-1 / n);
  },

  drop: drop(1.24),

  cloud: squeeze(
    cloud(
      [
        [0, -0.17, 0.46],
        [-0.48, 0.13, 0.42],
        [0.48, 0.13, 0.42],
        [0, 0.21, 0.6],
      ],
      0.63,
    ),
    0.94,
    1.064,
  ),
};

const STEPS = 2048;

function encloses(shape: (a: number) => number): number {
  let sum = 0;
  for (let i = 0; i < STEPS; i++) {
    const r = shape((i / STEPS) * TAU);
    sum += 0.5 * r * r * (TAU / STEPS);
  }
  return sum;
}

function crestOf(shape: (a: number) => number): number {
  let most = 0;
  for (let i = 0; i < STEPS; i++) most = Math.max(most, shape((i / STEPS) * TAU));
  return most;
}

function sizeOf(shape: (a: number) => number): number {
  const even = Math.sqrt(Math.PI / encloses(shape));
  return Math.min(even, CREST / crestOf(shape));
}

const SIZE = Object.fromEntries(
  Object.entries(RAW).map(([key, shape]) => [key, sizeOf(shape)]),
) as Record<Silhouette, number>;

export const SILHOUETTES = Object.fromEntries(
  Object.entries(RAW).map(([key, shape]) => {
    const scale = SIZE[key as Silhouette];
    return [key, (a: number) => shape(a) * scale];
  }),
) as Record<Silhouette, (a: number) => number>;
