import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { ICON_NAMES, PATHS } from "./Icon";

const GRID = 24;
const INSET = 1;
const PUNCTUATION = new Set([".", ",", ";", ":", "!", "?", "@", "·", "…"]);
const wide = (char: string) => (char.codePointAt(0) ?? 0) > 0x7f;

function numbers(text: string): number[] {
  return [...text.matchAll(/-?(?:\d+\.?\d*|\.\d+)(?:e-?\d+)?/g)].map((m) => Number(m[0]));
}

function arcPoints(
  from: [number, number],
  [rx0 = 0, ry0 = 0, rotation = 0, large = 0, sweep = 0, x = 0, y = 0]: number[],
): [number, number][] {
  expect(rotation, "an arc on the grid is not rotated").toBe(0);
  const dx = (from[0] - x) / 2;
  const dy = (from[1] - y) / 2;
  const scale = Math.max(1, Math.sqrt((dx * dx) / (rx0 * rx0) + (dy * dy) / (ry0 * ry0)));
  const rx = rx0 * scale;
  const ry = ry0 * scale;
  const sign = large === sweep ? -1 : 1;
  const radicand =
    (rx * rx * ry * ry - rx * rx * dy * dy - ry * ry * dx * dx) /
    (rx * rx * dy * dy + ry * ry * dx * dx);
  const k = sign * Math.sqrt(Math.max(0, radicand));
  const cx = k * ((rx * dy) / ry) + (from[0] + x) / 2;
  const cy = k * ((-ry * dx) / rx) + (from[1] + y) / 2;
  const start = Math.atan2((from[1] - cy) / ry, (from[0] - cx) / rx);
  let span = Math.atan2((y - cy) / ry, (x - cx) / rx) - start;
  if (sweep && span < 0) span += 2 * Math.PI;
  if (!sweep && span > 0) span -= 2 * Math.PI;
  return Array.from({ length: 33 }, (_, i) => {
    const angle = start + (span * i) / 32;
    return [cx + rx * Math.cos(angle), cy + ry * Math.sin(angle)];
  });
}

function pointsOf(d: string): [number, number][] {
  const points: [number, number][] = [];
  let at: [number, number] = [0, 0];
  let open: [number, number] = [0, 0];
  for (const [, command = "", args = ""] of d.matchAll(
    /([MLHVCSQTAZmlhvcsqtaz])([^MLHVCSQTAZmlhvcsqtaz]*)/g,
  )) {
    const values = numbers(args);
    const relative = command === command.toLowerCase();
    const base = (): [number, number] => (relative ? at : [0, 0]);
    const upper = command.toUpperCase();
    const stride = { M: 2, L: 2, T: 2, H: 1, V: 1, C: 6, S: 4, Q: 4, A: 7, Z: 0 }[upper] ?? 0;
    if (upper === "Z") {
      at = open;
      continue;
    }
    for (let i = 0; i < values.length; i += stride) {
      const group = values.slice(i, i + stride);
      const [ox, oy] = base();
      if (upper === "H") at = [ox + (group[0] ?? 0), at[1]];
      else if (upper === "V") at = [at[0], (relative ? at[1] : 0) + (group[0] ?? 0)];
      else if (upper === "A") {
        const end: [number, number] = [ox + (group[5] ?? 0), oy + (group[6] ?? 0)];
        points.push(...arcPoints(at, [...group.slice(0, 5), ...end]));
        at = end;
      } else {
        for (let j = 0; j < group.length; j += 2) {
          points.push([ox + (group[j] ?? 0), oy + (group[j + 1] ?? 0)]);
        }
        at = points[points.length - 1] ?? at;
      }
      points.push(at);
      if (upper === "M" && i === 0) open = at;
    }
  }
  return points;
}

function sources(dir: string, extension: string): string[] {
  return readdirSync(join(process.cwd(), dir), { recursive: true })
    .map(String)
    .filter((file) => file.endsWith(extension) && !/\.test\.tsx?$/.test(file))
    .map((file) => join(dir, file));
}

function standIns(text: string): string[] {
  const glyph = /^[^\p{L}\p{N}\s]$/u;
  const found = [
    ...[...text.matchAll(/>\s*([^<>{}\s])\s*<\//g)].map((m) => m[1] ?? ""),
    ...[...text.matchAll(/["'`]([^"'`\s])["'`]/g)].map((m) => m[1] ?? "").filter(wide),
    ...[...text.matchAll(/(?:>\s*|["'`])([^\p{L}\p{N}\s<>{}"'`])\s+\p{L}/gu)]
      .map((m) => m[1] ?? "")
      .filter(wide),
  ];
  return found.filter((char) => glyph.test(char) && !PUNCTUATION.has(char));
}

describe("the icon set", () => {
  it("draws every name it declares, once", () => {
    expect(new Set(ICON_NAMES).size).toBe(ICON_NAMES.length);
    expect(Object.keys(PATHS).sort()).toEqual([...ICON_NAMES].sort());
  });

  it.each(ICON_NAMES)("keeps %s inside the 24 grid with room for its stroke", (name) => {
    const points = pointsOf(PATHS[name]);
    expect(points.length).toBeGreaterThan(1);
    const outside = points.filter(
      ([x, y]) =>
        x < INSET - 1e-6 || y < INSET - 1e-6 || x > GRID - INSET + 1e-6 || y > GRID - INSET + 1e-6,
    );
    expect(outside.map(([x, y]) => `${x.toFixed(2)},${y.toFixed(2)}`)).toEqual([]);
  });

  it("finds a glyph standing in for an icon", () => {
    expect(standIns('<button aria-label="Close">\n  ×\n</button>')).toEqual(["×"]);
    expect(standIns('{open ? "▾" : "▸"}')).toEqual(["▾", "▸"]);
    expect(standIns("<span>+</span>")).toEqual(["+"]);
    expect(standIns('<button>\n  ‹ All artifacts\n</button> {"‹ Back"}')).toEqual(["‹", "‹"]);
    expect(standIns('<b>@</b> to name an agent, "↑↓ to choose"')).toEqual([]);
  });

  it("is the only icon set: no component draws one with a character", () => {
    const found = sources("src", ".tsx").flatMap((file) =>
      standIns(readFileSync(join(process.cwd(), file), "utf8")).map((char) => `${file}: ${char}`),
    );
    expect(found).toEqual([]);
  });

  it("puts no glyph in a stylesheet's generated content", () => {
    const found = sources("src", ".css").flatMap((file) =>
      [...readFileSync(join(process.cwd(), file), "utf8").matchAll(/content:\s*([^;]+);/g)]
        .map((m) => m[1] ?? "")
        .filter((value) => [...value].some((char) => wide(char) && !PUNCTUATION.has(char)))
        .map((value) => `${file}: ${value}`),
    );
    expect(found).toEqual([]);
  });
});
