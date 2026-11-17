export interface Seat {
  x: number;
  y: number;
  size: number;
  tilt: number;
}

export const SEATS = 6;

const RINGS: Record<number, { radius: number; size: number; from: number }> = {
  1: { radius: 0, size: 0.7, from: 0 },
  2: { radius: 0.2, size: 0.55, from: 180 },
  3: { radius: 0.235, size: 0.48, from: -90 },
  4: { radius: 0.265, size: 0.42, from: -90 },
  5: { radius: 0.285, size: 0.38, from: -90 },
  6: { radius: 0.3, size: 0.35, from: -90 },
};

const LEAN = 6;

function hash(seed: string): number {
  let at = 2166136261;
  for (let i = 0; i < seed.length; i++) {
    at = Math.imul(at ^ seed.charCodeAt(i), 16777619);
  }
  at = Math.imul(at ^ (at >>> 15), 2246822507);
  return ((at ^ (at >>> 13)) >>> 0) / 2 ** 32;
}

function round(value: number): number {
  return Math.round(value * 1e4) / 1e4;
}

export function cluster<T extends { id: string }>(
  members: T[],
): { seats: (Seat & { of: T })[]; rest: number } {
  const seated = members.slice(0, SEATS);
  const ring = RINGS[seated.length];
  if (!ring) return { seats: [], rest: 0 };

  const step = 360 / seated.length;
  const seats = seated.map((of, i) => {
    const angle = ((ring.from + i * step) * Math.PI) / 180;
    return {
      of,
      x: round(0.5 + ring.radius * Math.cos(angle)),
      y: round(0.5 + ring.radius * Math.sin(angle)),
      size: ring.size,
      tilt: seated.length === 1 ? 0 : Math.round((hash(of.id) - 0.5) * 2 * LEAN * 10) / 10,
    };
  });

  return { seats, rest: members.length - seated.length };
}
