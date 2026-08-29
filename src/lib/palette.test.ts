/**
 * The chart palette's gates, recomputed rather than remembered.
 *
 * Every number in `palette.ts`'s doc comment is worked out here from the hexes
 * themselves, so the comment cannot drift away from the values it describes and
 * a hex changed by eye fails the suite. That is the whole point of this file:
 * colorblind separation is not something anybody can check by looking, and a
 * palette is exactly the kind of thing somebody adjusts because a screenshot
 * looked slightly off.
 *
 * The math is the standard one for this: sRGB to linear, Machado, Oliveira &
 * Fernandes (2009) at full severity for the two red-green colorblindnesses,
 * then Euclidean distance in OKLab ×100. The simulation model is part of the
 * standard rather than an implementation detail, because the thresholds below
 * are calibrated against it and a different model moves every borderline pair.
 */

import { describe, expect, it } from "vitest";

import { MAX_SCATTER_HUES, SCATTER_MARKS, SLOTS, seriesColor, seriesMark } from "./palette";

/** The surface a figure card is drawn on, which is `--raised` in both. */
const SURFACE = { paper: "#ffffff", ink: "#171715" };

/** Neighbors must clear this under simulated colorblindness. */
const CVD_TARGET = 8;
/** And this under ordinary color vision, which is a separate failure. */
const NORMAL_FLOOR = 15;
/** OKLCH lightness a hue has to sit inside, per surface. */
const BAND: Record<"paper" | "ink", readonly [number, number]> = {
  paper: [0.43, 0.77],
  ink: [0.48, 0.67],
};
/** Below this a hue reads as gray and stops telling series apart at all. */
const CHROMA_FLOOR = 0.1;

const MACHADO = {
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
} as const;

type Vision = keyof typeof MACHADO;

function linear(hex: string): [number, number, number] {
  const channels = [0, 2, 4].map((at) => Number.parseInt(hex.slice(at + 1, at + 3), 16) / 255);
  return channels.map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)) as [
    number,
    number,
    number,
  ];
}

function oklab([r, g, b]: [number, number, number]): [number, number, number] {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function simulate(hex: string, vision: Vision): [number, number, number] {
  const [r, g, b] = linear(hex);
  const m = MACHADO[vision];
  const clamp = (c: number) => Math.max(0, Math.min(1, c));
  return [
    clamp(m[0][0] * r + m[0][1] * g + m[0][2] * b),
    clamp(m[1][0] * r + m[1][1] * g + m[1][2] * b),
    clamp(m[2][0] * r + m[2][1] * g + m[2][2] * b),
  ];
}

/** Perceived distance between two colors, optionally through one vision. */
function distance(one: string, other: string, vision?: Vision): number {
  const a = oklab(vision ? simulate(one, vision) : linear(one));
  const b = oklab(vision ? simulate(other, vision) : linear(other));
  return 100 * Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

function chroma(hex: string): number {
  const [, a, b] = oklab(linear(hex));
  return Math.hypot(a, b);
}

function lightness(hex: string): number {
  return oklab(linear(hex))[0];
}

function contrast(one: string, other: string): number {
  const luminance = (hex: string) => {
    const [r, g, b] = linear(hex);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [high, low] = [luminance(one), luminance(other)].sort((a, b) => b - a) as [number, number];
  return (high + 0.05) / (low + 0.05);
}

/** Every pair of slots that can touch in a stack, a group or a line chart. */
function neighbors(colors: string[]): [string, string][] {
  return colors.slice(0, -1).map((color, at) => [color, colors[at + 1] as string]);
}

/** Every pair, period, which is what a scatter has. */
function everyPair(colors: string[]): [string, string][] {
  return colors.flatMap((color, at) => colors.slice(at + 1).map((other) => [color, other])) as [
    string,
    string,
  ][];
}

const surfaces = [
  ["paper", SLOTS.map((slot) => slot.paper), SURFACE.paper, BAND.paper],
  ["ink", SLOTS.map((slot) => slot.ink), SURFACE.ink, BAND.ink],
] as const;

describe.each(surfaces)("the chart palette on %s", (_surface, colors, ground, band) => {
  it("keeps neighboring slots apart for a red-green colorblind reader", () => {
    // The gate the order exists to pass. Neighbors are what touch in a stack
    // and cross in a line chart, so this is the pair list that decides whether
    // a chart is readable rather than merely colorful.
    for (const vision of ["protan", "deutan"] as const) {
      for (const [one, other] of neighbors([...colors])) {
        expect(
          distance(one, other, vision),
          `${one} and ${other} collapse under ${vision}`,
        ).toBeGreaterThanOrEqual(CVD_TARGET);
      }
    }
  });

  it("keeps them apart for everybody else too", () => {
    // A separate failure from the one above, and not excused by it: a pair can
    // be safe under simulation and still be two colors an ordinary reader has
    // to squint at.
    for (const [one, other] of neighbors([...colors])) {
      expect(
        distance(one, other),
        `${one} and ${other} are hard to tell apart in full color`,
      ).toBeGreaterThanOrEqual(NORMAL_FLOOR);
    }
  });

  it("keeps a scatter's first three apart from each other, in every pairing", () => {
    // Scatter has no neighbors: any dot can land beside any other, so the
    // adjacent test above says nothing about it. This is the harder gate, and
    // it is why `MAX_SCATTER_HUES` is three.
    const three = [...colors].slice(0, MAX_SCATTER_HUES);
    for (const vision of ["protan", "deutan"] as const) {
      for (const [one, other] of everyPair(three)) {
        expect(distance(one, other, vision)).toBeGreaterThanOrEqual(CVD_TARGET);
      }
    }
    for (const [one, other] of everyPair(three)) {
      expect(distance(one, other)).toBeGreaterThanOrEqual(NORMAL_FLOOR);
    }
  });

  it("sits inside the lightness band this surface has room for", () => {
    const [floor, ceiling] = band;
    for (const color of colors) {
      expect(lightness(color), `${color} is off ${_surface}'s band`).toBeGreaterThanOrEqual(floor);
      expect(lightness(color), `${color} is off ${_surface}'s band`).toBeLessThanOrEqual(ceiling);
    }
  });

  it("stays colorful enough to carry identity", () => {
    for (const color of colors) {
      expect(chroma(color), `${color} reads as gray`).toBeGreaterThanOrEqual(CHROMA_FLOOR);
    }
  });

  it("says which hues need the numbers written next to them", () => {
    // Contrast under 3:1 is allowed here and is not a license to ship a chart
    // that cannot be read: every figure carries direct labels and a table of
    // its own numbers, which is what makes a pale fill legible. This asserts
    // the relief is owed, so removing the table twin is not a silent change.
    const pale = colors.filter((color) => contrast(color, ground) < 3);
    if (_surface === "ink") expect(pale).toEqual([]);
    else expect(pale.length, "paper leans on labels and the table twin").toBeLessThanOrEqual(3);
  });
});

describe("handing colors out", () => {
  it("gives a series the same color wherever its neighbors went", () => {
    // Color follows the series, never its rank. An operator who has learned
    // that revenue is green is misled by a legend that repaints what is left
    // when something is switched off.
    expect(seriesColor(0)).toBe("var(--series-0)");
    expect(seriesColor(3)).toBe("var(--series-3)");
  });

  it("wraps rather than inventing a ninth hue", () => {
    // A generated ninth is indistinguishable from one of the eight under
    // colorblindness. Wrapping is at least honest about repeating.
    expect(seriesColor(SLOTS.length)).toBe(seriesColor(0));
  });

  it("gives a scatter a shape as well as a hue", () => {
    // The second channel, and the reason a scatter is not capped at three
    // series. Shape survives every colorblindness, grayscale print and a
    // screenshot; hue survives none of them.
    expect(seriesMark(0)).toBe(SCATTER_MARKS[0]);
    expect(seriesMark(MAX_SCATTER_HUES)).not.toBe(seriesMark(0));
  });
});

describe("the palette itself", () => {
  it("opens on green, because the app does", () => {
    expect(SLOTS[0]?.hue).toBe("green");
  });

  it("has eight, which is the ceiling and not a coincidence", () => {
    expect(SLOTS).toHaveLength(8);
    expect(new Set(SLOTS.map((slot) => slot.hue)).size).toBe(8);
  });
});
