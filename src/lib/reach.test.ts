import { describe, expect, it } from "vitest";

import { type Reach, reaches } from "./reach";

const SHUT: Reach = { reach: { top: 36, right: 20 }, column: { right: 8 } };

const OUT: Reach = { reach: { top: 36, right: 20 }, column: { right: 64 } };

describe("the crews' column", () => {
  it("comes out when the pointer reaches the edge of the window", () => {
    expect(reaches(false, { x: 6, y: 400 }, SHUT, false)).toBe(true);
  });

  it("stays in for a pointer crossing the middle of the app", () => {
    expect(reaches(false, { x: 700, y: 400 }, SHUT, false)).toBe(false);
  });

  it("leaves the window's own buttons alone", () => {
    expect(reaches(false, { x: 6, y: 18 }, SHUT, false)).toBe(false);
  });

  it("stays out while the pointer is anywhere on it", () => {
    expect(reaches(true, { x: 40, y: 400 }, OUT, false)).toBe(true);
  });

  it("stays out just past it, rather than closing at the pixel that opened it", () => {
    expect(reaches(true, { x: 70, y: 400 }, OUT, false)).toBe(true);
  });

  it("goes back in once the pointer has left it by the same distance again", () => {
    expect(reaches(true, { x: 90, y: 400 }, OUT, false)).toBe(false);
  });

  it("comes out for a pointer coming back at it from the right", () => {
    expect(reaches(false, { x: 3, y: 500 }, SHUT, false)).toBe(true);
  });
});

describe("a row in hand", () => {
  it("does not bring the column out from across the app", () => {
    expect(reaches(false, { x: 700, y: 400 }, SHUT, true)).toBe(false);
  });

  it("still lets the pointer reach for it", () => {
    expect(reaches(false, { x: 6, y: 400 }, SHUT, true)).toBe(true);
  });

  it("holds out a column already reached for, wherever the hand goes next", () => {
    expect(reaches(true, { x: 900, y: 400 }, OUT, true)).toBe(true);
  });

  it("lets it close again once the row is dropped", () => {
    expect(reaches(true, { x: 900, y: 400 }, OUT, false)).toBe(false);
  });
});
