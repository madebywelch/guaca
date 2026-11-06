/**
 * The controls the second polish audit measured under 24×24, and the glass
 * around them.
 *
 * jsdom lays nothing out, so the sizes here are the ones the audit measured in
 * the running app at 1440×900 (AUDIT-2, item 20) and the arithmetic is over the
 * rules that widen them. What this holds is the widening: a pseudo-element that
 * is clear glass around the words, because a padding or a height on the control
 * itself would move every line of the page the control sits on.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { ROOT_PX } from "./appearance";

const css = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

/** A length token from the sheet, in pixels. */
function px(name: string): number {
  const rem = css.match(new RegExp(`${name}: ([\\d.]+)rem;`))?.[1];
  if (!rem) throw new Error(`styles.css declares no ${name}`);
  return Number(rem) * ROOT_PX;
}

/** One rule's declarations, from its selector to its closing brace. */
function rule(selector: string): string {
  const at = css.indexOf(selector);
  expect(at, `${selector} is a rule`).toBeGreaterThan(-1);
  const end = css.indexOf("}", at);
  return css.slice(at, end);
}

/** How far the hit area reaches past the drawn box, each way, in pixels. */
function reach(selector: string): number {
  const drawn = rule(`${selector}::after`);
  const inset = drawn.match(/inset: calc\(var\((--space-\d+)\) \* -1\) 0;/);
  expect(inset, `${selector}'s hit area is a transparent inset`).not.toBeNull();
  // A hit area that painted anything would be a change to how the row looks.
  expect(drawn, `${selector}'s hit area paints nothing`).not.toMatch(
    /\b(background|border|outline|box-shadow):/,
  );
  return px(inset![1]!);
}

describe("the controls the audit measured under 24 by 24", () => {
  // Item 20 of the second audit, measured in the running app. `drawn` is what
  // the control is; `reach` is the glass put around it.
  const measured = [
    { what: "a footnote's tool call", drawn: 16, selector: ".footnote__call--open" },
    { what: "Add a header", drawn: 17, selector: ".toolset__more" },
    { what: "the rail's four places", drawn: 21, selector: ".rail__place" },
  ];

  for (const one of measured) {
    it(`gives ${one.what} a target of at least 24px`, () => {
      expect(one.drawn + 2 * reach(one.selector)).toBeGreaterThanOrEqual(24);
    });
  }

  it("spends the design system's own steps rather than a number nobody declared", () => {
    // Every reach is a multiple of a spacing token, so changing the scale moves
    // the target with it instead of leaving a pixel count behind.
    expect(css).toMatch(/\.rail__place::after \{[^}]*inset: calc\(var\(--space-1\) \* -1\) 0;/);
    expect(css).toMatch(/\.toolset__more::after \{[^}]*inset: calc\(var\(--space-2\) \* -1\) 0;/);
  });
});
