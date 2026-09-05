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
 *
 * The closed sets below are the one thing that reads like an exception and is
 * not. Nothing here says 13px is the right size for a second line. It says a
 * size is *named*, so that changing your mind means editing one token and
 * seeing every rule that shares the decision move with it. Spelled at the
 * point of use, a decision is invisible: this file reached 41 font sizes, 38
 * of them inside ten pixels, with a considered color system sitting at the top
 * of it the whole time. Nobody chose that and no review would have caught it.
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

describe("avatar hit targets", () => {
  it.each(["picker__item", "orb", "agent-row"])(
    "keeps animated artwork out of hit testing inside %s",
    (control) => {
      const button = document.createElement("button");
      button.className = control;
      const avatar = document.createElement("span");
      avatar.className = "avatar";
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      svg.classList.add("avatar__body");
      const layer = document.createElementNS(svg.namespaceURI, "use");
      svg.append(layer);
      avatar.append(svg);
      button.append(avatar);
      document.body.append(button);

      // WebKit can lose the click when the referenced outline is repainted
      // between press and release. Neither the SVG nor its layers is a target;
      // the stable wrapper still receives input and bubbles it to the button.
      expect(getComputedStyle(svg).pointerEvents).toBe("none");
      expect(getComputedStyle(layer).pointerEvents).toBe("none");
      expect(getComputedStyle(avatar).pointerEvents).toBe("auto");
      expect(getComputedStyle(button).pointerEvents).toBe("auto");
      button.remove();
    },
  );
});

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

describe("the columns that are not the page", () => {
  /**
   * The rail, the crews' column and the inspector follow the surface, and every
   * value they are drawn from has to be declared for both of them.
   *
   * They used to be ink whichever surface the operator picked, which meant they
   * pinned their own `--flesh`, `--flesh-soft` and `--alarm` against a reading
   * column that could go dark underneath them. That is gone: the left edge of
   * the app and the right edge of it are now the same ground, a shade off the
   * page rather than opposite it. What is left to keep true is the other half
   * of the same trap. A `--rail-*` name declared for paper and forgotten in the
   * ink block is a column that stays off-white in a dark room, and no DOM
   * assertion sees a color.
   *
   * Checked in the source rather than through the cascade, and that is not
   * laziness: jsdom resolves a custom property declared on an element but does
   * not inherit one down a subtree, so reading a token off a child of a column
   * reports nothing at all whether it is declared or not. The declaration is
   * the thing that has to be there, so the declaration is what is read.
   */
  function declared(selector: string): Set<string> {
    const block = css.match(
      new RegExp(`${selector.replace(/[[\]"=]/g, "\\$&")} \\{[\\s\\S]*?\\n\\}`),
    );
    if (!block) throw new Error(`no ${selector} block`);
    return new Set([...block[0].matchAll(/^\s{2}(--[a-z-]+):/gm)].map((found) => found[1]!));
  }

  /** What a block declares a token as, trimmed. */
  function declaredAs(selector: string, token: string): string {
    const block = css.match(
      new RegExp(`${selector.replace(/[[\]"=]/g, "\\$&")} \\{[\\s\\S]*?\\n\\}`),
    );
    return block?.[0].match(new RegExp(`^\\s{2}${token}: (.+);$`, "m"))?.[1]?.trim() ?? "";
  }

  const PAPER = declared(":root");
  const INK = declared(':root[data-surface="dark"]');

  /** Every rule in the file, as its selector and its body. */
  const RULES = [...css.matchAll(/^([^\n@}][^{\n]*)\{\n([\s\S]*?)^\}/gm)];

  it("found both surface blocks", () => {
    // Every assertion below is vacuous if these regexes stop matching.
    expect(PAPER.has("--rail-ground")).toBe(true);
    expect(INK.has("--ground")).toBe(true);
    expect(RULES.length).toBeGreaterThan(50);
  });

  /**
   * The column values that are colors. A width is geometry and has no surface:
   * `--grail-w` is the same four rem in a dark room. Told apart by the value
   * rather than by a list, so a color added to the family tomorrow is covered
   * without anybody remembering to add it here.
   */
  const COLORS = [...PAPER].filter(
    (token) =>
      /^--(?:rail|grail)-/.test(token) && /^#[0-9a-f]{3,8}$/i.test(declaredAs(":root", token)),
  );

  it.each(COLORS)("moves %s with the surface", (token) => {
    expect(INK.has(token)).toBe(true);
  });

  /**
   * Nothing in either navigation column pins an accent any more.
   *
   * The pins were right while the columns were ink under both surfaces and are
   * exactly wrong now: the app has one amber, it means "answer me", and a
   * column holding a second value for it is a column where the same state is a
   * different color on the wrong side of a border.
   */
  it.each([".rail", ".grail"])("pins no accent of its own (%s)", (scope) => {
    for (const token of declared(scope)) {
      expect(
        /^--(?:flesh|pit|mint|alarm)/.test(token),
        `${scope} pins ${token}, which the reading column owns`,
      ).toBe(false);
    }
  });

  /**
   * A column's recessed surface is recessed from the column.
   *
   * `--sunken` is a hair off white, which is a field on paper and nothing at
   * all on an off-white panel. It is remapped once, on the three elements every
   * rule in those columns descends from, so a row added to the inspector
   * tomorrow is recessed from what it is actually drawn on. This asserts the
   * remap exists and covers all three, because losing one of them is a panel
   * whose fields quietly stop having edges.
   */
  /**
   * How visible a color is against another one, as WCAG reads it.
   *
   * Here rather than through the cascade for the reason everything else in this
   * block is: jsdom does not inherit a custom property down a subtree, so a
   * token read off a child of a column comes back empty whether or not it was
   * ever declared. Both values are hex in the two `:root` blocks, which is what
   * makes them arithmetic at all.
   */
  function contrast(one: string, other: string): number {
    const light = (hex: string) => {
      const packed = Number.parseInt(hex.slice(1), 16);
      const channel = (at: number) => {
        const part = ((packed >> at) & 255) / 255;
        return part <= 0.03928 ? part / 12.92 : ((part + 0.055) / 1.055) ** 2.4;
      };
      return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0);
    };
    const [high, low] = [light(one), light(other)].sort((a, b) => b - a) as [number, number];
    return (high + 0.05) / (low + 0.05);
  }

  /**
   * The mark saying which crew the rail is inside has to be visible on the
   * column it is drawn on, on both surfaces.
   *
   * It was `--flesh-soft` for as long as the column existed, which is the
   * accent's *ground* rather than an ink: #fdeed9 over a #eceae2 column and
   * #33240e over a #0b0b0a one, 1.06 to 1 and 1.31 to 1. So the one permanent
   * statement in the app about where the operator is standing was drawn in a
   * color neither surface shows, on the column whose whole argument is that
   * "which crew am I in" has an answer in a fixed place. Nothing caught it: the
   * bar was in the document with the right class on it the entire time, and no
   * DOM assertion sees a color.
   *
   * Three to one is what WCAG 1.4.11 asks of a mark that is not text, and a
   * bar this thin has nothing else to lean on. The band behind it is
   * deliberately not held to the same floor: it is a wash under the whole row,
   * which is what `.agent-row` does one column over for the open channel, and a
   * wash that met a text ratio would be a block of accent with faces on it.
   */
  it("marks the crew the rail is inside in something the column shows", () => {
    const rule = css.match(/\.orb\[aria-current="true"\]::before \{[\s\S]*?\n\}/)?.[0];
    expect(rule, "no mark for the crew the rail is inside").toBeTruthy();

    const token = rule!.match(/background: var\((--[a-z-]+)\);/)?.[1];
    expect(token, `the mark is not spelled from a token: ${rule}`).toBeTruthy();

    for (const surface of [":root", ':root[data-surface="dark"]']) {
      const ink = declaredAs(surface, token!);
      const ground = declaredAs(surface, "--grail-ground");
      expect(/^#[0-9a-f]{6}$/i.test(ink) && /^#[0-9a-f]{6}$/i.test(ground)).toBe(true);
      expect(
        contrast(ink, ground),
        `${token} on --grail-ground in ${surface} is ${contrast(ink, ground).toFixed(2)} to 1`,
      ).toBeGreaterThanOrEqual(3);
    }
  });

  it("recesses from the column rather than from the page", () => {
    // Matched against the source rather than against `RULES`, whose selector
    // group stops at a newline: this rule names its three scopes one per line.
    const remap = css.match(/([^{}]*)\{\s*--sunken: var\(--rail-sunken\);\s*\}/);
    expect(remap, "no rule remaps --sunken onto --rail-sunken").toBeTruthy();
    for (const scope of [".rail,", ".grail,", ".inspector"]) {
      expect(remap![1]!).toContain(scope);
    }
    expect(PAPER.has("--rail-sunken")).toBe(true);
  });
});

describe("the navigation columns", () => {
  /**
   * A right-click on either of them has to leave nothing highlighted behind.
   * WebKit selects the word under the pointer before it dispatches
   * `contextmenu`, for the menu it is about to draw, so the row that answers
   * with a menu of its own cannot undo that by preventing a default which has
   * already happened. Neither column is selectable instead, and the assertion
   * is here rather than in a DOM test because jsdom has no native selection to
   * make: the declaration is the whole of the fix, so the declaration is what
   * is read.
   */
  it.each(["rail", "grail"])("select nothing under a right-click (.%s)", (column) => {
    expect(getComputedStyle(nest(column)).userSelect).toBe("none");
  });
});

describe("what a crew has spent", () => {
  /**
   * The card is held open by the heading underneath it.
   *
   * It opens downward over the first rows of the crew, and those rows are drop
   * targets and buttons. Taking the pointer, it would take it off the heading
   * that is the only reason it is open, close itself, hand the pointer back and
   * open again: a panel flickering under a hand that has not moved. jsdom lays
   * nothing out and stacks nothing, so no DOM assertion sees any of it.
   */
  it("lets the pointer through to the heading and the rows it covers", () => {
    expect(getComputedStyle(nest("spend")).pointerEvents).toBe("none");
  });

  /**
   * And it is laid over the rail rather than opening a space in it.
   *
   * The whole argument for moving the numbers off the heading was that the rail
   * is fifteen rem wide and the crew's name was paying for them. A card that
   * reflowed the column on hover would spend that width back, on a surface that
   * appears and disappears under the pointer.
   */
  it("takes no width from the column it hangs off", () => {
    expect(getComputedStyle(nest("spend")).position).toBe("fixed");
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

describe("the trail above the composer", () => {
  /**
   * Which of the two things on a chip gives up room, and which keeps it.
   *
   * Flex shrinks in proportion to what each item asked for, so a refused call
   * whose reason runs to a paragraph asked for twenty times what its label did
   * and took the row on the way to being clipped itself: the chip drew
   * `U… a coding agent is already working in whizzworks-site, started by…`,
   * which is one character about which call it was. No DOM assertion sees it,
   * because jsdom lays nothing out and both nodes are there either way.
   *
   * A weighting rather than a refusal is what this replaced, and it is the
   * near-miss worth naming: at a hundred to one the label still gave up its
   * last character, because proportional is proportional however lopsided.
   */
  it("cuts what came back before it cuts the label", () => {
    const chip = getComputedStyle(nest("trail__chip"));
    const label = getComputedStyle(nest("trail__chip", "trail__label"));

    // The answer beside it takes the shrinking, on the default every flex item
    // has and this one gives up.
    expect(label.flexShrink).toBe("0");
    // And what will not fit either way is cut by the chip rather than drawn
    // across the message beside it.
    expect(chip.overflow).toBe("hidden");
  });

  /**
   * The working and the turn's calls are two disclosures sharing one slot.
   *
   * Stacked, they were the transcript giving up twice the height for a
   * question asked once, and a composer that moved twice. The number is a
   * decision and not asserted; that the two agree on it is the rule, because
   * they are one place on screen that draws two things.
   */
  it("bounds both of the line's panels the same", () => {
    const thought = getComputedStyle(nest("thought"));
    const steps = getComputedStyle(nest("steps"));

    expect(steps.maxHeight).toBe(thought.maxHeight);
    expect(steps.margin).toBe(thought.margin);
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

describe("a table in a message", () => {
  /**
   * A cell breaks at a word, and the body it sits in breaks anywhere.
   *
   * They look like the same decision and are opposite ones. Prose in a chat
   * bubble has to break a URL mid-token or it pushes the reading column
   * sideways, which is what `anywhere` buys. In a table it also drops every
   * cell's min-content width to a single character, and auto layout hands a
   * squeezed column exactly its min-content: a `Result` column beside a wide
   * one came out one letter wide, spelling `Re/sul/t` and `Qu/eu/ed` down
   * three lines each. The two values are indistinguishable on a paragraph,
   * which is why this is asserted rather than left to the eye.
   */
  function cell(tag: "th" | "td"): HTMLElement {
    const md = document.createElement("div");
    md.className = "md";
    const table = document.createElement("table");
    const row = document.createElement("tr");
    const at = document.createElement(tag);
    row.append(at);
    table.append(row);
    md.append(table);
    document.body.append(md);
    return at;
  }

  it.each(["th", "td"] as const)("sizes a %s column to a word, not a letter", (tag) => {
    expect(getComputedStyle(cell(tag)).overflowWrap).toBe("break-word");
  });

  it("still breaks anywhere in the prose around it", () => {
    expect(getComputedStyle(nest("md")).overflowWrap).toBe("anywhere");
  });
});

describe("the composer's mention layer", () => {
  /**
   * The pill is painted on a copy of the draft, under the operator's own text.
   *
   * Which only works while the copy wraps and spaces its characters exactly as
   * the textarea does. One extra pixel of padding on either and every pill in
   * the box sits beside the name it belongs to rather than behind it, on a
   * surface where nothing renders in review and nothing lays out in a test.
   * So the properties that decide where a glyph lands are read back off both
   * elements and compared to each other.
   */
  const PLACES_A_GLYPH = [
    "fontSize",
    "fontFamily",
    "fontWeight",
    "lineHeight",
    "letterSpacing",
    "wordSpacing",
    "paddingTop",
    "paddingRight",
    "paddingBottom",
    "paddingLeft",
    "borderTopWidth",
    "borderLeftWidth",
    "whiteSpace",
    "overflowWrap",
  ] as const;

  function drawn(): { mirror: CSSStyleDeclaration; input: CSSStyleDeclaration } {
    const field = document.createElement("div");
    field.className = "composer__field";
    const mirror = document.createElement("div");
    mirror.className = "composer__mirror";
    const input = document.createElement("textarea");
    input.className = "composer__input";
    field.append(mirror, input);
    document.body.append(field);
    return { mirror: getComputedStyle(mirror), input: getComputedStyle(input) };
  }

  it("wraps the copy exactly as the box wraps the original", () => {
    const { mirror, input } = drawn();

    // Vacuous if the rule stopped reaching either element. Read off a property
    // that is a keyword rather than a length: jsdom substitutes no custom
    // property, so everything on a scale reads back as the empty string here
    // and would pass this on both elements while proving nothing. The check
    // below is what holds the tokenized half.
    expect(mirror.whiteSpace).toBeTruthy();
    expect(mirror.overflowWrap).toBeTruthy();

    for (const property of PLACES_A_GLYPH) {
      expect(mirror[property], `${property} differs from the box's`).toBe(input[property]);
    }
  });

  /**
   * The same invariant, read off the rules instead of off the cascade.
   *
   * The comparison above is the original check and still catches a UA default
   * coming apart on the two keywords. It stopped being able to catch a *value*
   * the moment the composer moved onto the scale, because jsdom resolves no
   * `var()`: every tokenized property reads back as the empty string on both
   * elements and compares equal to it. Its own vacuity guard said so, which is
   * the guard working rather than the rule breaking.
   *
   * What survives is the structure the equality was standing in for. Every
   * property that decides where a glyph lands is declared once, in the rule the
   * two elements share, and neither of them says one of those again underneath
   * it. A value cannot drift between two elements that never had two values.
   */
  it("declares what places a glyph once, in the rule they share", () => {
    const MOVES_A_GLYPH =
      /^(font|line-height|letter-spacing|word-spacing|padding|border(?!-radius)|white-space|overflow-wrap|text-indent|tab-size)/;
    const shared = declarations()
      .filter((d) => d.selector === ".composer__mirror, .composer__input")
      .filter((d) => MOVES_A_GLYPH.test(d.property));

    // Vacuous if the shared rule was split or renamed.
    expect(shared.map((d) => d.property)).toContain("padding");
    expect(shared.length).toBeGreaterThan(6);

    for (const selector of [".composer__mirror", ".composer__input"]) {
      const own = declarations()
        .filter((d) => d.selector === selector)
        .filter((d) => MOVES_A_GLYPH.test(d.property));
      expect(
        own.map((d) => `${selector} { ${d.property}: ${d.value} }`),
        `${selector} moves its own characters and leaves the other's where they were`,
      ).toEqual([]);
    }
  });

  it("keeps the copy out of the flow and out of the way", () => {
    const { mirror } = drawn();

    // The textarea is what gives the row its height and grows as the draft
    // does. A copy in the flow would double both.
    expect(mirror.position).toBe("absolute");
    // And it is under the box: a click that landed on it would take the caret.
    expect(mirror.pointerEvents).toBe("none");
  });

  /**
   * The chip is one class, drawn in a sent message and under the composer, and
   * the second of those is why it may not change a metric. Padding, a weight or
   * a letter-spacing on it moves the characters in the copy and leaves the ones
   * in the textarea where they were, which is the same drift by another route.
   * The room around a name is a spread shadow, which takes up none.
   */
  it("draws the chip without moving a character", () => {
    const rule = css.match(/^\.mention \{\n([\s\S]*?)^\}/m);
    expect(rule).toBeTruthy();

    const declared = [...rule![1]!.matchAll(/^\s{2}([a-z-]+):/gm)].map((found) => found[1]!);
    const moves = declared.filter((property) =>
      /^(padding|margin|border(?!-radius)|font|letter-spacing|word-spacing|display|line-height|vertical-align)/.test(
        property,
      ),
    );
    expect(moves, `.mention declares ${moves.join(", ")}, which moves the text under it`).toEqual(
      [],
    );
  });
});

/**
 * Every declaration in the file, with the rule it belongs to.
 *
 * Comments come out first, so a value named in prose is not read as one that
 * was written. The two `:root` blocks are where the tokens are defined and are
 * the one place a literal is the point, so everything above the first ordinary
 * rule is skipped.
 */
function declarations(): { selector: string; property: string; value: string }[] {
  const body = css.slice(css.indexOf("\n* {")).replace(/\/\*[\s\S]*?\*\//g, "");
  const found: { selector: string; property: string; value: string }[] = [];
  const stack: string[] = [];
  let token = "";
  for (const ch of body) {
    if (ch === "{") {
      stack.push(token.trim().replace(/\s+/g, " "));
      token = "";
    } else if (ch === "}") {
      const selector = stack.join(" ") || "?";
      for (const declaration of token.matchAll(/([a-z-]+):\s*([^;]+);/g)) {
        const [, property = "", value = ""] = declaration;
        found.push({ selector, property, value: value.trim().replace(/\s+/g, " ") });
      }
      stack.pop();
      token = "";
    } else {
      token += ch;
    }
  }
  return found;
}

/**
 * The closed sets.
 *
 * A design system nobody can bypass is a build gate; one nobody checks is a
 * comment. Each rule below says which token family a property has to be spelled
 * from, and names the exceptions with the reason they are exceptions, because
 * an unexplained hole in a gate is how the gate stops meaning anything.
 */
describe("every length is named, not spelled", () => {
  /**
   * Whether a value is spelled out of a family rather than written down.
   *
   * Two questions, and both have to hold: every token it names is from the
   * right family, and every literal left over is one this property is allowed
   * to contain. A multiplier inside `calc()` is arithmetic rather than a
   * length, which is how a token is spent as a pull-back; `0` is the absence
   * of a length and `auto` is a question for the layout, so neither is a
   * decision any scale could hold.
   */
  const named = (value: string, family: RegExp, also = /^$/) => {
    const tokens = [...value.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1] ?? "");
    if (!tokens.every((token) => family.test(token))) return false;
    return value
      .replace(/[*/]\s*-?[0-9.]+/g, " ")
      .replace(/var\(--[a-z0-9-]+(,[^)]*)?\)/g, " ")
      .replace(/calc\(|min\(|max\(|[()+]|,/g, " ")
      .split(/\s+/)
      .filter(Boolean)
      .every((part) => part === "0" || part === "auto" || also.test(part));
  };

  it.each([
    // Sizes come from the type scale. `em` is the one unit that is a ratio
    // rather than a size, which is what inline code inside a paragraph wants
    // and the only thing it is used for; `--ui-scale` is the root anchor every
    // rem in the file is measured against, so it cannot be a rem itself.
    ["font-size", /^--type-/, /^[0-9.]+em$|--ui-scale/, /^$/],
    // `inherit` is the composer's mention layer saying out loud what a textarea
    // does not inherit, so the copy under it cannot come apart from the box on
    // a UA default. It is a value taken from somewhere else rather than one
    // this rule picked, which is the one thing no scale can hold.
    ["letter-spacing", /^--track-/, /^inherit$/, /^$/],
    // `0` is the inline-descender reset a few SVG wrappers want. It is a
    // layout fix rather than leading, and there is no leading it could mean.
    ["line-height", /^--(lead-|badge)/, /^0$/, /^$/],
    // `50%` is a circle and `1px` is a hairline softening on a chart mark:
    // neither is on a radius scale, and quantizing them would round a 3px bar
    // into a lozenge.
    ["border-radius", /^--radius/, /^$/, /^(50%|1px)$/],
  ])("%s", (property, family, exempt, also) => {
    const bad = declarations()
      .filter((d) => d.property === property)
      .filter((d) => !exempt.test(d.value) && !named(d.value, family, also));
    expect(bad.map((d) => `${d.selector} { ${d.property}: ${d.value} }`)).toEqual([]);
  });

  it.each(["padding", "margin", "gap", "row-gap", "column-gap"])(
    "%s and its long-hands",
    (base) => {
      const bad = declarations()
        .filter((d) => d.property === base || d.property.startsWith(`${base}-`))
        // `1px` is a hairline: the gap between two bars of a sparkline, and the
        // one-pixel pull-back that hides a visually-hidden box. `12vh` holds the
        // palette below the title bar and is a fraction of the window rather
        // than a step on any scale.
        .filter((d) => !named(d.value, /^--(space-|column-|dot|flow-)/, /^(-?1px|12vh)$/));
      expect(bad.map((d) => `${d.selector} { ${d.property}: ${d.value} }`)).toEqual([]);
    },
  );

  /**
   * Motion is where cohesion is felt rather than read, so it is held hardest.
   *
   * Three curves, and each one says which register the motion is in: a surface
   * arriving, a character reacting, something that loops or travels. A fourth
   * has to argue it is a register rather than a preference.
   */
  it("transitions run on a tempo and a named curve", () => {
    const bad = declarations()
      .filter((d) => d.property === "transition")
      .filter((d) => d.value !== "none")
      .filter((d) => !/^(?:[a-z-]+ var\(--tempo-[a-z]+\) var\(--ease\)(?:, )?)+$/.test(d.value));
    expect(bad.map((d) => `${d.selector} { transition: ${d.value} }`)).toEqual([]);
  });

  /**
   * The long-hands too, or the rule only covers the spelling it expected.
   *
   * A `transition-duration: 90ms` sat in the rail for as long as the shorthand
   * rule did, because nothing was reading that property name. The reduced-motion
   * block is the exemption and is not a hole: `0.001ms !important` over every
   * element in the document is the standard way to turn motion off, and it has
   * to beat every tempo in the file to work.
   */
  it.each(["transition-duration", "animation-duration"])("%s", (property) => {
    const bad = declarations()
      .filter((d) => d.property === property)
      .filter((d) => !d.value.startsWith("0.001ms"))
      .filter((d) => !named(d.value, /^--tempo-/));
    expect(bad.map((d) => `${d.selector} { ${property}: ${d.value} }`)).toEqual([]);
  });

  it("animations run on a named curve", () => {
    // Duration is deliberately free here. An ambient loop is timed against the
    // other loops beside it by eye, and a shared tempo would sync a breath, a
    // blink and a glance into one metronome.
    const bad = declarations()
      .filter((d) => d.property === "animation")
      .filter((d) => d.value !== "none")
      .filter((d) => !/var\(--ease(-spring|-loop)?\)|steps\(|linear/.test(d.value));
    expect(bad.map((d) => `${d.selector} { animation: ${d.value} }`)).toEqual([]);
  });

  /**
   * A shadow is one of three elevations, a ring, or a glow.
   *
   * The px ones this replaced are why the rule mentions units at all: in an app
   * where every other length is a rem, a `24px` blur is a blur that ignores the
   * operator's interface scale.
   *
   * `--stage-lift` is the fourth and only lives on one surface, the full-window
   * machine viewer, which is ink whichever surface the operator picked and so
   * cannot spend a lift resolved against the reading column. It is named beside
   * the three other values that surface pins, which is what makes it a decision
   * rather than a shadow somebody typed.
   */
  it("shadows are a lift, a ring or a glow", () => {
    const bad = declarations()
      .filter((d) => d.property === "box-shadow")
      .filter((d) => d.value !== "none")
      .filter((d) => !d.value.includes("var(--lift-"))
      .filter((d) => d.value !== "var(--stage-lift)")
      .filter((d) => !d.value.includes("inset"))
      // A glow sits on the thing that casts it: no offset, so nothing about it
      // is claiming height off the page.
      .filter((d) => !/^0 0 /.test(d.value));
    expect(bad.map((d) => `${d.selector} { box-shadow: ${d.value} }`)).toEqual([]);
  });
});
