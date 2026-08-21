/**
 * What the stylesheet has to keep being true.
 *
 * Every other suite here renders components into jsdom, which does no layout,
 * so a rule that lays a surface out wrongly passes all of them. Two defects in
 * the file reading view were exactly that shape: it clipped the document it
 * existed to show and could not be scrolled, and it opened at the width of an
 * ordinary dialog. Neither is visible in a DOM assertion and neither is
 * visible in review, so both are asserted here against the cascade itself.
 *
 * Only invariants that survive a redesign belong in this file. A colour, a
 * spacing or a font size is a decision, not a rule, and locking one down here
 * would make changing your mind a test failure.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { beforeAll, describe, expect, it } from "vitest";

// From the project root rather than from this module: the jsdom environment
// rewrites `import.meta.url` to an `http:` URL, which `readFileSync` refuses.
const css = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

/** The app's own stylesheet, in the document, so the cascade is the real one. */
beforeAll(() => {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
});

/** Builds a nesting and hands back the innermost node. */
function nest(...classes: string[]): HTMLElement {
  let at: HTMLElement = document.body;
  for (const className of classes) {
    const node = document.createElement("div");
    node.className = className;
    at.append(node);
    at = node;
  }
  return at;
}

describe("the file reading view", () => {
  // The card under a message clips and fades its preview on purpose. The full
  // view reuses the same classes with the bounds taken off, and `overflow` is
  // the one that has to come off with them: a clipping flex item has an
  // automatic minimum size of zero, so a document left clipping shrinks to
  // whatever room is going and swallows the rest of itself. Nothing then
  // overflows the body, so nothing scrolls, and a brief opened for reading
  // stops at the height of the window with no scrollbar and no way down.
  it.each([
    ["a document", "file__doc"],
    ["a log", "file__text"],
  ])("neither clips nor caps %s", (_what, className) => {
    const shown = getComputedStyle(nest("file-view__body", className));

    expect(shown.overflow).toBe("visible");
    expect(shown.maxHeight).toBe("none");
  });

  it("scrolls in the body, which is the one thing in it that scrolls", () => {
    expect(getComputedStyle(nest("file-view__body")).overflow).toBe("auto");
  });
});

describe("dialog modifiers", () => {
  /**
   * A modifier declared above `.dialog` silently loses to it.
   *
   * Both selectors carry one class, so the base rule wins every property they
   * share on source order alone, and the full file view came out at the
   * ordinary 38rem for that reason. It cannot be asserted through
   * `getComputedStyle`: jsdom's CSS parser drops any declaration whose value
   * is a `min()`, which is how every dialog width in this file is written, so
   * the width never reaches the cascade to be read back. The source order is
   * what is checkable, and it is also the actual trap.
   */
  it("out-specify the base rule, or are declared after it", () => {
    const base = css.search(/^\.dialog \{$/m);
    expect(base).toBeGreaterThan(-1);

    // The selector is the whole match rather than a group, so it is a string
    // under `noUncheckedIndexedAccess` without a cast to say so.
    const modifiers = [...css.matchAll(/^(?:\.dialog)?\.dialog--[a-z-]+(?= \{$)/gm)];
    expect(modifiers.length).toBeGreaterThan(0);

    for (const found of modifiers) {
      const selector = found[0];
      const wins = selector.startsWith(".dialog.") || found.index > base;
      expect(wins, `${selector} is declared above .dialog and loses to it`).toBe(true);
    }
  });
});
