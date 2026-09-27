/**
 * Color arithmetic, in the model the chart palette is gated with.
 *
 * sRGB to linear, OKLab and OKLCH, WCAG 2 contrast, and Machado, Oliveira &
 * Fernandes (2009) at full severity for the two red-green colorblindnesses,
 * with distance as OKLab ×100. The simulation model is part of the standard
 * rather than an implementation detail: every threshold a theme is held to is
 * calibrated against it, and a different model moves every borderline pair.
 */

export type Vec3 = [number, number, number];
export type Vision = "normal" | "protan" | "deutan";

const MACHADO: Record<Exclude<Vision, "normal">, readonly Vec3[]> = {
  protan: [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
  ],
  deutan: [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
  ],
};

const toLinear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toGamma = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
const clamp = (c: number) => Math.max(0, Math.min(1, c));

/** `#rrggbb` to linear light. */
export function linear(color: string): Vec3 {
  return [1, 3, 5].map((at) =>
    toLinear(Number.parseInt(color.slice(at, at + 2), 16) / 255),
  ) as Vec3;
}

export function oklab([r, g, b]: Vec3): Vec3 {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function fromOklab([L, a, b]: Vec3): Vec3 {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

function lch(L: number, C: number, h: number): Vec3 {
  const angle = (h * Math.PI) / 180;
  return fromOklab([L, C * Math.cos(angle), C * Math.sin(angle)]);
}

const inGamut = (rgb: Vec3) => rgb.every((c) => c >= -1e-4 && c <= 1 + 1e-4);

/**
 * OKLCH to `#rrggbb`, giving up chroma until the color fits in sRGB.
 *
 * Chroma rather than lightness or hue, because lightness is what every ratio
 * below was solved for and hue is what a person chose: a pale amber that has
 * to lose something loses saturation and stays the amber it was asked to be.
 */
export function hex(L: number, C: number, h: number): string {
  let chroma = C;
  if (!inGamut(lch(L, C, h))) {
    let lo = 0;
    let hi = C;
    for (let i = 0; i < 24; i++) {
      const mid = (lo + hi) / 2;
      if (inGamut(lch(L, mid, h))) lo = mid;
      else hi = mid;
    }
    chroma = lo;
  }
  return `#${lch(L, chroma, h)
    .map((c) =>
      Math.round(clamp(toGamma(clamp(c))) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

/** WCAG 2 relative luminance. */
export function luminance(color: string): number {
  const [r, g, b] = linear(color);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2 contrast ratio, 1 to 21. */
export function contrast(a: string, b: string): number {
  const x = luminance(a);
  const y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

function simulate(color: string, vision: Vision): Vec3 {
  const rgb = linear(color);
  if (vision === "normal") return rgb;
  return MACHADO[vision].map(([r, g, b]) => clamp(r * rgb[0] + g * rgb[1] + b * rgb[2])) as Vec3;
}

/** OKLab distance ×100, as seen with the vision given. */
export function distance(a: string, b: string, vision: Vision = "normal"): number {
  const p = oklab(simulate(a, vision));
  const q = oklab(simulate(b, vision));
  return 100 * Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
}

/** The worse of the two red-green simulations. */
export function colorblindDistance(a: string, b: string): number {
  return Math.min(distance(a, b, "protan"), distance(a, b, "deutan"));
}

/**
 * The color at this hue and chroma that clears `ratio` against every ground
 * given, at the lightness closest to `prefer` that does.
 *
 * On a light surface it solves downward toward ink, and on a dark one upward
 * toward paper. `prefer` is where the color looks like itself; the ratio is
 * the floor it is not allowed under.
 */
export function solve(
  C: number,
  h: number,
  grounds: readonly string[],
  ratio: number,
  prefer: number,
  dark: boolean,
): string {
  const clears = (L: number) => grounds.every((g) => contrast(hex(L, C, h), g) >= ratio);
  if (clears(prefer)) return hex(prefer, C, h);
  let lo = dark ? prefer : 0;
  let hi = dark ? 1 : prefer;
  for (let i = 0; i < 30; i++) {
    const mid = (lo + hi) / 2;
    if (dark === clears(mid)) hi = mid;
    else lo = mid;
  }
  return hex(dark ? hi : lo, C, h);
}
