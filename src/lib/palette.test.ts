import { describe, expect, it } from "vitest";

import { MAX_SCATTER_HUES, SCATTER_MARKS, SLOTS, seriesColor, seriesMark } from "./palette";

const SURFACE = { paper: "#ffffff", ink: "#171715" };

const CVD_TARGET = 8;
const NORMAL_FLOOR = 15;
const BAND: Record<"paper" | "ink", readonly [number, number]> = {
  paper: [0.43, 0.77],
  ink: [0.48, 0.67],
};
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

function neighbors(colors: string[]): [string, string][] {
  return colors.slice(0, -1).map((color, at) => [color, colors[at + 1] as string]);
}

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
    for (const [one, other] of neighbors([...colors])) {
      expect(
        distance(one, other),
        `${one} and ${other} are hard to tell apart in full color`,
      ).toBeGreaterThanOrEqual(NORMAL_FLOOR);
    }
  });

  it("keeps a scatter's first three apart from each other, in every pairing", () => {
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
    const pale = colors.filter((color) => contrast(color, ground) < 3);
    if (_surface === "ink") expect(pale).toEqual([]);
    else expect(pale.length, "paper leans on labels and the table twin").toBeLessThanOrEqual(3);
  });
});

describe("handing colors out", () => {
  it("gives a series the same color wherever its neighbors went", () => {
    expect(seriesColor(0)).toBe("var(--series-0)");
    expect(seriesColor(3)).toBe("var(--series-3)");
  });

  it("wraps rather than inventing a ninth hue", () => {
    expect(seriesColor(SLOTS.length)).toBe(seriesColor(0));
  });

  it("gives a scatter a shape as well as a hue", () => {
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
