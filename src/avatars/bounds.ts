/**
 * Bounding box of an SVG path.
 *
 * Exists so "every character fills the same optical box" is a number a test can
 * check rather than a claim in a comment. Handles the subset the catalog draws
 * with: moves, lines, cubics and their smooth and shorthand forms. Arcs are not
 * supported, so silhouettes are written as cubics.
 */

export interface Bounds {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

const NUMBER = /-?\d*\.?\d+(?:e[-+]?\d+)?/gi;

function args(chunk: string): number[] {
  return (chunk.match(NUMBER) ?? []).map(Number);
}

/** Extrema of one cubic axis, clamped to the segment. */
function cubicExtrema(p0: number, p1: number, p2: number, p3: number): number[] {
  const a = -p0 + 3 * p1 - 3 * p2 + p3;
  const b = 2 * (p0 - 2 * p1 + p2);
  const c = p1 - p0;
  const at = (t: number) => {
    const u = 1 - t;
    return u * u * u * p0 + 3 * u * u * t * p1 + 3 * u * t * t * p2 + t * t * t * p3;
  };
  const ts: number[] = [];
  if (Math.abs(a) < 1e-9) {
    if (Math.abs(b) > 1e-9) ts.push(-c / b);
  } else {
    const disc = b * b - 4 * a * c;
    if (disc >= 0) {
      const root = Math.sqrt(disc);
      ts.push((-b + root) / (2 * a), (-b - root) / (2 * a));
    }
  }
  return [p0, p3, ...ts.filter((t) => t > 0 && t < 1).map(at)];
}

export function pathBounds(d: string): Bounds {
  let x0 = Number.POSITIVE_INFINITY;
  let y0 = Number.POSITIVE_INFINITY;
  let x1 = Number.NEGATIVE_INFINITY;
  let y1 = Number.NEGATIVE_INFINITY;
  const see = (x: number, y: number) => {
    if (x < x0) x0 = x;
    if (y < y0) y0 = y;
    if (x > x1) x1 = x;
    if (y > y1) y1 = y;
  };

  let x = 0;
  let y = 0;
  let startX = 0;
  let startY = 0;
  // Reflection point for S/s. Resets to the current point after a non-cubic.
  let prevCx = 0;
  let prevCy = 0;
  let wasCubic = false;

  const curve = (c1x: number, c1y: number, c2x: number, c2y: number, ex: number, ey: number) => {
    const xs = cubicExtrema(x, c1x, c2x, ex);
    const ys = cubicExtrema(y, c1y, c2y, ey);
    see(Math.min(...xs), Math.min(...ys));
    see(Math.max(...xs), Math.max(...ys));
    prevCx = c2x;
    prevCy = c2y;
    x = ex;
    y = ey;
    wasCubic = true;
  };

  for (const [, letter, rest] of d.matchAll(/([MmLlHhVvCcSsZz])([^MmLlHhVvCcSsZz]*)/g)) {
    const n = args(rest ?? "");
    switch (letter) {
      case "M":
      case "m": {
        for (let i = 0; i + 1 < n.length; i += 2) {
          const rel = letter === "m";
          x = rel ? x + n[i]! : n[i]!;
          y = rel ? y + n[i + 1]! : n[i + 1]!;
          if (i === 0) {
            startX = x;
            startY = y;
          }
          see(x, y);
        }
        wasCubic = false;
        break;
      }
      case "L":
      case "l": {
        for (let i = 0; i + 1 < n.length; i += 2) {
          x = letter === "l" ? x + n[i]! : n[i]!;
          y = letter === "l" ? y + n[i + 1]! : n[i + 1]!;
          see(x, y);
        }
        wasCubic = false;
        break;
      }
      case "H":
      case "h": {
        for (const v of n) {
          x = letter === "h" ? x + v : v;
          see(x, y);
        }
        wasCubic = false;
        break;
      }
      case "V":
      case "v": {
        for (const v of n) {
          y = letter === "v" ? y + v : v;
          see(x, y);
        }
        wasCubic = false;
        break;
      }
      case "C":
      case "c": {
        const rel = letter === "c";
        for (let i = 0; i + 5 < n.length; i += 6) {
          curve(
            rel ? x + n[i]! : n[i]!,
            rel ? y + n[i + 1]! : n[i + 1]!,
            rel ? x + n[i + 2]! : n[i + 2]!,
            rel ? y + n[i + 3]! : n[i + 3]!,
            rel ? x + n[i + 4]! : n[i + 4]!,
            rel ? y + n[i + 5]! : n[i + 5]!,
          );
        }
        break;
      }
      case "S":
      case "s": {
        const rel = letter === "s";
        for (let i = 0; i + 3 < n.length; i += 4) {
          const c1x = wasCubic ? 2 * x - prevCx : x;
          const c1y = wasCubic ? 2 * y - prevCy : y;
          curve(
            c1x,
            c1y,
            rel ? x + n[i]! : n[i]!,
            rel ? y + n[i + 1]! : n[i + 1]!,
            rel ? x + n[i + 2]! : n[i + 2]!,
            rel ? y + n[i + 3]! : n[i + 3]!,
          );
        }
        break;
      }
      default: {
        x = startX;
        y = startY;
        wasCubic = false;
      }
    }
  }

  return { x0, y0, x1, y1 };
}
