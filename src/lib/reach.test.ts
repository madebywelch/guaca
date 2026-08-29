import { describe, expect, it } from "vitest";

import { type Reach, reaches } from "./reach";

/** A collapsed column: the band is 8px wide, the zone reaches 20px in. */
const SHUT: Reach = { reach: { top: 36, right: 20 }, column: { right: 8 } };

/** The same column out: 64px of it, and the zone unchanged. */
const OUT: Reach = { reach: { top: 36, right: 20 }, column: { right: 64 } };

describe("the crews' column", () => {
  it("comes out when the pointer reaches the edge of the window", () => {
    expect(reaches(false, { x: 6, y: 400 }, SHUT)).toBe(true);
  });

  it("stays in for a pointer crossing the middle of the app", () => {
    expect(reaches(false, { x: 700, y: 400 }, SHUT)).toBe(false);
  });

  it("leaves the window's own buttons alone", () => {
    // macOS floats close, minimize and zoom over the top left corner, which is
    // this column. Opening on the way to them would put the crews under the
    // pointer at the moment it was aimed at the button that closes the app.
    expect(reaches(false, { x: 6, y: 18 }, SHUT)).toBe(false);
  });

  it("stays out while the pointer is anywhere on it", () => {
    expect(reaches(true, { x: 40, y: 400 }, OUT)).toBe(true);
  });

  it("stays out just past it, rather than closing at the pixel that opened it", () => {
    // The gap between the two thresholds. Without it a hand resting on the
    // boundary flickers the column: it closes, which puts the pointer back
    // inside the zone, which opens it.
    expect(reaches(true, { x: 70, y: 400 }, OUT)).toBe(true);
  });

  it("goes back in once the pointer has left it by the same distance again", () => {
    expect(reaches(true, { x: 90, y: 400 }, OUT)).toBe(false);
  });

  it("comes out for a pointer coming back at it from the right", () => {
    expect(reaches(false, { x: 3, y: 500 }, SHUT)).toBe(true);
  });
});
