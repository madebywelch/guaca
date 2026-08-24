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
 * Only invariants that survive a redesign belong in this file. A color, a
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

describe("the dark columns", () => {
  /**
   * The rail and the group column are dark under both surfaces, so a token the
   * reading column redefines for dark paper repaints them unless they pin it.
   * `--flesh` on dark paper is a bright leaf green meant to sit on a near-black
   * page; landing it on the rail turns Guaca's one accent into a highlighter,
   * and no DOM assertion notices, which is the whole of the note about it in
   * `docs/WORKSPACE.md`.
   *
   * Checked in the source rather than through the cascade, and that is not
   * laziness: jsdom resolves a custom property declared on an element but does
   * not inherit one down a subtree, so reading `--alarm` off a child of the
   * column reports nothing at all whether it is pinned or not. The declaration
   * is the thing that has to be there, so the declaration is what is read.
   *
   * Only the two families that are drawn nowhere else are covered. A rule under
   * `.agent-row` reaching for an unpinned token is the same defect and is not
   * caught here: that family predates this check and pinning is a property of a
   * column, so the map below has to say which column a class is inside.
   */
  const SURFACE_VARYING = new Set(
    [
      ...(css.match(/:root\[data-surface="dark"\] \{[\s\S]*?\n\}/)?.[0] ?? "").matchAll(
        /^\s{2}(--[a-z-]+):/gm,
      ),
    ].map((found) => found[1]!),
  );

  /** Every rule in the file, as its selector and its body. */
  const RULES = [...css.matchAll(/^([^\n@}][^{\n]*)\{\n([\s\S]*?)^\}/gm)];

  /**
   * Selectors naming one of these class families.
   *
   * The suffixes are part of the match on purpose: `.orb__waiting` is drawn
   * inside the group column exactly as `.orb` is, and a pattern that stopped at
   * the base class would skip every element that carries the color.
   */
  function family(...names: string[]): RegExp {
    return new RegExp(`(?:^|[\\s,>])\\.(?:${names.join("|")})(?:__|--)?[\\w-]*`);
  }

  /** What a scope's own base rule declares. */
  function pinnedOn(scope: string): Set<string> {
    const base = RULES.find((rule) => rule[1]!.trim() === scope);
    if (!base) throw new Error(`no base rule for ${scope}`);
    return new Set([...base[2]!.matchAll(/^\s{2}(--[a-z-]+):/gm)].map((found) => found[1]!));
  }

  it("found the tokens a surface moves", () => {
    // Every assertion below is vacuous if this regex stops matching.
    expect(SURFACE_VARYING.has("--flesh")).toBe(true);
    expect(SURFACE_VARYING.has("--alarm")).toBe(true);
    expect(RULES.length).toBeGreaterThan(50);
  });

  it.each([
    [".grail", family("grail", "orb")],
    [".rail", family("rail")],
  ])("pin every accent %s draws with", (scope, family) => {
    const pinned = pinnedOn(scope);

    for (const [, selector, body] of RULES) {
      if (!family.test(selector!)) continue;
      for (const [, token] of body!.matchAll(/var\((--[a-z-]+)/g)) {
        if (!SURFACE_VARYING.has(token!)) continue;
        expect(
          pinned.has(token!),
          `${selector!.trim()} draws with ${token}, which ${scope} does not pin`,
        ).toBe(true);
      }
    }
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

describe("a message's clock", () => {
  /**
   * The clock is a lane, not something laid over the corner.
   *
   * It used to be absolutely positioned in the top right of the row, which is
   * also where the first line of a full-measure paragraph ends: on any window
   * narrow enough that the words reached the edge, hovering a message put the
   * time over the last word of its own first line. A reserve in the row's
   * padding does not fix it, because the width being reserved for is the
   * operator's locale rather than this file's, so the check is that the clock
   * and the words are in different columns and neither is out of flow.
   */
  it.each([
    ["an agent's", undefined],
    ["the operator's", "true"],
  ])("is beside %s words rather than over them", (_whose, operator) => {
    const msg = document.createElement("article");
    msg.className = "msg";
    if (operator) msg.dataset.operator = operator;
    const body = document.createElement("div");
    body.className = "msg__body";
    const at = document.createElement("time");
    at.className = "msg__at";
    msg.append(body, at);
    document.body.append(msg);

    // Read back as declared: jsdom does not default a property nobody set, so
    // in the flow is the empty string rather than `static`.
    expect(getComputedStyle(at).position).not.toBe("absolute");
    expect(getComputedStyle(at).gridColumn).not.toBe(getComputedStyle(body).gridColumn);
  });
});
