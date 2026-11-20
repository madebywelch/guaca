import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { inflateSync } from "node:zlib";

import { beforeAll, describe, expect, it } from "vitest";

import { AVATAR_PX } from "./avatars/frame";
import { ROOT_PX } from "./lib/appearance";
import { contrast } from "./lib/color";

const css = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");
const signalCss = readFileSync(join(process.cwd(), "src/signal-layout.css"), "utf8");
const composerCss = readFileSync(join(process.cwd(), "src/components/Composer.css"), "utf8");

beforeAll(() => {
  const style = document.createElement("style");
  style.textContent = `${css}\n${signalCss}\n${composerCss}`;
  document.head.append(style);
});

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

describe("avatar sizes", () => {
  it.each(Object.entries(AVATAR_PX))(
    "draws %s at the %dpx its lines are hinted for",
    (size, px) => {
      const rule = css.match(
        new RegExp(`\\.avatar--${size} \\{\\s*width: ([\\d.]+)rem;\\s*height: ([\\d.]+)rem;`),
      );
      expect(rule, `.avatar--${size}`).not.toBeNull();
      expect(Number(rule?.[1]) * ROOT_PX).toBe(px);
      expect(Number(rule?.[2]) * ROOT_PX).toBe(px);
    },
  );
});

describe("the type roles", () => {
  const sizes = Object.fromEntries(
    [...css.matchAll(/^\s*--type-([a-z]+): ([\d.]+)rem;$/gm)].map(([, role, rem]) => [
      role,
      Number(rem) * ROOT_PX,
    ]),
  );
  const weights = Object.fromEntries(
    [...css.matchAll(/^\s*--weight-([a-z]+): (\d+);$/gm)].map(([, name, value]) => [
      name,
      Number(value),
    ]),
  );
  const rules = () => {
    const by = new Map<number, Record<string, string>>();
    for (const d of declarations()) by.set(d.rule, { ...by.get(d.rule), [d.property]: d.value });
    return [...by.values()];
  };
  const shown = (rule: Record<string, string>) =>
    Object.entries(rule)
      .map(([property, value]) => `${property}: ${value}`)
      .join("; ");

  it("are five sizes and two weights, and nothing else", () => {
    expect(sizes).toEqual({ meta: 12, support: 13, ui: 15, read: 16, title: 20 });
    expect(weights).toEqual({ regular: 400, strong: 600 });
  });

  it("set every weight from a weight token", () => {
    const bad = declarations()
      .filter((d) => d.property === "font-weight")
      .filter((d) => !/^var\(--weight-(regular|strong)\)$/.test(d.value));
    expect(bad.map((d) => `${d.selector} { font-weight: ${d.value} }`)).toEqual([]);
  });

  it("pair a size only with the weights its role allows", () => {
    const bad = rules().filter(
      (rule) =>
        (/--type-(meta|support|read)\)/.test(rule["font-size"] ?? "") &&
          rule["font-weight"] === "var(--weight-strong)") ||
        (rule["font-size"] === "var(--type-title)" &&
          rule["font-weight"] !== "var(--weight-strong)"),
    );
    expect(bad.map(shown)).toEqual([]);
  });

  it("track only a title", () => {
    const bad = rules().filter(
      (rule) =>
        rule["letter-spacing"] !== undefined &&
        !/^(inherit|0)$/.test(rule["letter-spacing"]) &&
        !(
          rule["letter-spacing"] === "var(--track-title)" &&
          rule["font-size"] === "var(--type-title)"
        ),
    );
    expect(bad.map(shown)).toEqual([]);
  });

  it("set no capitals and no italics", () => {
    const bad = declarations().filter(
      (d) =>
        (d.property === "text-transform" && /uppercase|capitalize/.test(d.value)) ||
        (d.property === "font-style" && /italic|oblique/.test(d.value)) ||
        (d.property === "font-variant-caps" && d.value !== "normal"),
    );
    expect(bad.map((d) => `${d.selector} { ${d.property}: ${d.value} }`)).toEqual([]);
  });

  it("set code, and only code, in the mono face", () => {
    const mono = declarations().filter((d) => d.value.includes("var(--font-mono)"));
    expect(mono.map((d) => d.selector)).toEqual([
      ":is(code, pre, .diff, .file__text, .connector-failure__detail > pre, .textarea--code, .webhook, .devicecode__code, .skill__body)",
    ]);
  });

  it("are set in the stylesheet, never by a component", () => {
    const dir = join(process.cwd(), "src");
    const code = readdirSync(dir, { recursive: true, encoding: "utf8" })
      .filter((file) => /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file))
      .filter((file) =>
        /\b(fontSize|fontWeight|fontFamily|fontStyle|letterSpacing|textTransform)\s*:/.test(
          readFileSync(join(dir, file), "utf8"),
        ),
      );
    expect(code).toEqual([]);
  });
});

describe("the control family", () => {
  const px = (pattern: RegExp) =>
    Object.fromEntries(
      [...css.matchAll(pattern)].map(([, name, rem]) => [name, Number(rem) * ROOT_PX]),
    );

  it("has two heights, 32px and 28px compact", () => {
    expect(px(/^\s*--(control-h(?:-sm)?): ([\d.]+)rem;$/gm)).toEqual({
      "control-h": 32,
      "control-h-sm": 28,
    });
  });

  it("has a 6px corner for controls and a 10px corner for cards", () => {
    expect(px(/^\s*--(radius-[a-z]+): ([\d.]+)rem;$/gm)).toEqual({
      "radius-control": 6,
      "radius-card": 10,
    });
  });

  it("draws a disclosure as a compact control with its own triangle, not the browser's", () => {
    const rule = (selector: string) =>
      Object.fromEntries(
        declarations()
          .filter((d) => d.selector === selector)
          .map((d) => [d.property, d.value]),
      );
    expect(rule(".disclosure > summary")).toMatchObject({
      "min-height": "var(--control-h-sm)",
      "border-radius": "var(--radius-control)",
      "list-style": "none",
    });
    expect(rule(".disclosure > summary::-webkit-details-marker")).toMatchObject({
      display: "none",
    });
    expect(rule(".disclosure > summary:hover")).toMatchObject({ "text-decoration": "none" });
    expect(rule(".disclosure[open] > summary > .icon").transform).toBeTruthy();
  });

  it("gives a button a pressed, a disabled and a pending state as well as a hover", () => {
    const selectors = declarations().map((d) => d.selector);
    for (const state of [":hover", ":active", ":disabled", '[aria-busy="true"]']) {
      expect(
        selectors.some((selector) => selector.startsWith(".btn") && selector.includes(state)),
        state,
      ).toBe(true);
    }
  });
});

describe("avatar hit targets", () => {
  it.each(["picker__item", "orb", "agent-row", "roster__agent"])(
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

      expect(getComputedStyle(svg).pointerEvents).toBe("none");
      expect(getComputedStyle(layer).pointerEvents).toBe("none");
      expect(getComputedStyle(avatar).pointerEvents).toBe("auto");
      expect(getComputedStyle(button).pointerEvents).toBe("auto");
      button.remove();
    },
  );
});

describe("the file reading view", () => {
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
  function declared(selector: string): Set<string> {
    const block = css.match(
      new RegExp(`${selector.replace(/[[\]"=]/g, "\\$&")} \\{[\\s\\S]*?\\n\\}`),
    );
    if (!block) throw new Error(`no ${selector} block`);
    return new Set([...block[0].matchAll(/^\s{2}(--[a-z-]+):/gm)].map((found) => found[1]!));
  }

  function declaredAs(selector: string, token: string): string {
    const block = css.match(
      new RegExp(`${selector.replace(/[[\]"=]/g, "\\$&")} \\{[\\s\\S]*?\\n\\}`),
    );
    return block?.[0].match(new RegExp(`^\\s{2}${token}: (.+);$`, "m"))?.[1]?.trim() ?? "";
  }

  const PAPER = declared(":root");
  const INK = declared(':root[data-surface="dark"]');

  const RULES = [...css.matchAll(/^([^\n@}][^{\n]*)\{\n([\s\S]*?)^\}/gm)];

  it("found both surface blocks", () => {
    expect(PAPER.has("--rail-ground")).toBe(true);
    expect(INK.has("--ground")).toBe(true);
    expect(RULES.length).toBeGreaterThan(50);
  });

  const COLORS = [...PAPER].filter(
    (token) =>
      /^--(?:rail|grail)-/.test(token) && /^#[0-9a-f]{3,8}$/i.test(declaredAs(":root", token)),
  );

  it.each(COLORS)("moves %s with the surface", (token) => {
    expect(INK.has(token)).toBe(true);
  });

  it.each([".rail", ".grail"])("pins no accent of its own (%s)", (scope) => {
    for (const token of declared(scope)) {
      expect(
        /^--(?:attention|system|ok|danger)/.test(token),
        `${scope} pins ${token}, which the reading column owns`,
      ).toBe(false);
    }
  });

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

  it("recesses from the column and the agent panel rather than from the page", () => {
    const remap = css.match(/([^{}]*)\{\s*--sunken: var\(--rail-sunken\);\s*\}/);
    expect(remap, "no rule remaps --sunken onto --rail-sunken").toBeTruthy();
    for (const scope of [".rail,", ".grail,", ".agent-panel,", ".menubar"]) {
      expect(remap![1]!).toContain(scope);
    }
    expect(PAPER.has("--rail-sunken")).toBe(true);
  });
});

describe("the navigation columns", () => {
  it.each(["rail", "grail", "menubar"])("select nothing under a right-click (.%s)", (column) => {
    expect(getComputedStyle(nest(column)).userSelect).toBe("none");
  });
});

describe("the window's columns", () => {
  it.each(["rail__list", "grail__list", "pane__scroll", "agent-panel"])(
    "scrolls .%s on one axis",
    (column) => {
      const shown = getComputedStyle(nest(column));

      expect(shown.overflowY).toBe("auto");
      expect(shown.overflowX).toBe("hidden");
    },
  );

  it("keeps the window still while the roster scrolls separately and Settings hides the conversation", () => {
    const app = nest("app");
    app.setAttribute("data-roster", "");
    const main = document.createElement("main");
    const view = document.createElement("div");
    const roster = document.createElement("nav");
    const rows = document.createElement("div");
    view.className = "app__view";
    roster.className = "roster";
    rows.className = "roster__scroll";
    roster.append(rows);
    app.append(roster);
    main.append(view);
    app.append(main);
    expect(getComputedStyle(main).overflow).toBe("hidden");
    expect(getComputedStyle(rows).overflow).toBe("auto");
    expect(getComputedStyle(view).display).toBe("contents");
    view.hidden = true;
    expect(getComputedStyle(view).display).toBe("none");
  });

  it("breaks a path in the agent panel rather than cutting it off", () => {
    expect(getComputedStyle(nest("agent-panel")).overflowWrap).toBe("break-word");
  });
});

describe("a banner", () => {
  it("breaks its words anywhere, which is what lets them shrink", () => {
    expect(getComputedStyle(nest("banner")).overflowWrap).toBe("anywhere");
  });

  it("never shrinks its button", () => {
    expect(getComputedStyle(nest("banner", "btn")).flexShrink).toBe("0");
  });
});

describe("the actions under a webhook's curl line", () => {
  it("are a row of their own, not words in the hint", () => {
    expect(getComputedStyle(nest("webhook__actions")).display).toBe("flex");
  });
});

describe("what a crew has spent", () => {
  it("lets the pointer through to the heading and the rows it covers", () => {
    expect(getComputedStyle(nest("spend")).pointerEvents).toBe("none");
  });

  it("takes no width from the column it hangs off", () => {
    expect(getComputedStyle(nest("spend")).position).toBe("fixed");
  });
});

describe("a group's tag", () => {
  it("lets the pointer through to the circle and the pane under it", () => {
    expect(getComputedStyle(nest("orb", "orb__tag")).pointerEvents).toBe("none");
  });
});

describe("the rail's corner", () => {
  it("is the strip the window's buttons already leave, with no row under it", () => {
    const strip = getComputedStyle(nest("rail__brand"));
    const [top, , bottom = top] = strip.padding.split(" ");
    expect([top, bottom]).toEqual(["0", "0"]);
    expect(strip.minHeight).toBe(getComputedStyle(nest("grail__reach")).top);
  });

  it("keeps the folded name in the document", () => {
    const name = getComputedStyle(nest("brand", "brand__name"));
    expect(name.display).not.toBe("none");
    expect(name.visibility).not.toBe("hidden");
  });
});

describe("dialog modifiers", () => {
  it("out-specify the base rule, or are declared after it", () => {
    const base = css.search(/^\.dialog \{$/m);
    expect(base).toBeGreaterThan(-1);

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
  it("cuts what came back before it cuts the label", () => {
    const chip = getComputedStyle(nest("trail__chip"));
    const label = getComputedStyle(nest("trail__chip", "trail__label"));

    expect(label.flexShrink).toBe("0");
    expect(chip.overflow).toBe("hidden");
  });

  it("bounds both of the line's panels the same", () => {
    const thought = getComputedStyle(nest("thought"));
    const steps = getComputedStyle(nest("steps"));

    expect(steps.maxHeight).toBe(thought.maxHeight);
    expect(steps.margin).toBe(thought.margin);
  });
});

describe("a turn on the page", () => {
  const rule = (selector: string) =>
    css.match(
      new RegExp(`^${selector.replace(/[.[\]"=]/g, "\\$&")} \\{\\n([\\s\\S]*?)^\\}`, "m"),
    )?.[1] ?? "";

  it("keeps no time gutter: a turn is one column, with its clock on the head line", () => {
    expect(rule(".msg")).not.toMatch(/grid-template-columns/);
    expect(rule(".msg__head")).toMatch(/grid-template-columns: var\(--column-face\) auto auto/);
  });

  it("starts an agent's words on the name's edge, and the person's inside a bubble on the same edge", () => {
    expect(rule(".msg__body")).toMatch(/margin-left: var\(--column-face\)/);
    expect(css).toMatch(/--column-bubble: calc\(var\(--column-face\) - var\(--space-5\)\);/);
    expect(rule(".msg__bubble")).toMatch(/margin-left: var\(--column-bubble\)/);
    expect(rule(".msg__bubble")).toMatch(/padding: var\(--space-4\) var\(--space-5\)/);
    expect(rule(".msg__bubble")).toMatch(/background: var\(--wash\)/);
    expect(css).toMatch(/--page: var\(--sunken\);/);
  });

  it("fades a turn scrolled under the tabs over the same length the first turn starts below them", () => {
    expect(rule(".pane__scroll")).toMatch(/padding: var\(--space-6\) /);
    expect(rule(".pane__scroll")).toMatch(/scroll-padding-top: var\(--space-6\);/);
    expect(rule(".pane__scroll")).toMatch(
      /mask-image: linear-gradient\(to bottom, transparent, #000 var\(--space-6\)\);/,
    );
  });

  it("draws no vertical rule beside a turn or an aside", () => {
    for (const selector of [".msg", ".msg__body", ".msg__bubble", ".aside .md", ".footnote"]) {
      expect(rule(selector)).not.toMatch(/border-left/);
    }
  });
});

describe("a table in a message", () => {
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

    expect(mirror.whiteSpace).toBeTruthy();
    expect(mirror.overflowWrap).toBeTruthy();

    for (const property of PLACES_A_GLYPH) {
      expect(mirror[property], `${property} differs from the box's`).toBe(input[property]);
    }
  });

  it("declares what places a glyph once, in the rule they share", () => {
    const MOVES_A_GLYPH =
      /^(font|line-height|letter-spacing|word-spacing|padding|border(?!-radius)|white-space|overflow-wrap|text-indent|tab-size)/;
    const shared = declarations()
      .filter((d) => d.selector === ".composer__mirror, .composer__input")
      .filter((d) => MOVES_A_GLYPH.test(d.property));

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

    expect(mirror.position).toBe("absolute");
    expect(mirror.pointerEvents).toBe("none");
  });

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

describe("the agent's tabs", () => {
  it("gives the caption's room up before any tab's label", () => {
    const at = (selector: string) =>
      Object.fromEntries(
        declarations()
          .filter((d) => d.selector === selector)
          .map((d) => [d.property, d.value]),
      );
    const tabs = at(".agent-tabs__list");
    const caption = at(".agent-tabs .pane__meta");
    expect(tabs.flex, "the tabs keep their width").toBe("none");
    expect(caption.flex?.split(" ")[1], "the caption shrinks").not.toBe("0");
    expect(caption["min-width"], "the caption can shrink below its text").toBe("0");
    expect(caption["text-overflow"]).toBe("ellipsis");
  });
});

function declarations(): { selector: string; property: string; value: string; rule: number }[] {
  const body = `${css.slice(css.indexOf("\n* {"))}\n${signalCss}\n${composerCss}`.replace(
    /\/\*[\s\S]*?\*\//g,
    "",
  );
  const found: { selector: string; property: string; value: string; rule: number }[] = [];
  let rule = 0;
  const stack: string[] = [];
  let token = "";
  for (const ch of body) {
    if (ch === "{") {
      stack.push(token.trim().replace(/\s+/g, " "));
      token = "";
    } else if (ch === "}") {
      const selector = stack.join(" ") || "?";
      rule += 1;
      for (const declaration of token.matchAll(/([a-z-]+):\s*([^;]+);/g)) {
        const [, property = "", value = ""] = declaration;
        found.push({ selector, property, value: value.trim().replace(/\s+/g, " "), rule });
      }
      stack.pop();
      token = "";
    } else {
      token += ch;
    }
  }
  return found;
}

describe("every length is named, not spelled", () => {
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
    ["font-size", /^--type-/, /--ui-scale/, /^$/],
    ["letter-spacing", /^--track-/, /^inherit$/, /^$/],
    ["line-height", /^--(lead-|badge)/, /^0$/, /^$/],
    ["border-radius", /^--radius-(control|card|pill)$/, /^$/, /^50%$/],
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
        .filter(
          (d) =>
            !named(
              d.value,
              /^--(space-|column-|dot|flow-|scrollbar-size$|settings-(inline-end|gutter)$)/,
              /^(0px|-?1px|12vh)$/,
            ),
        );
      expect(bad.map((d) => `${d.selector} { ${d.property}: ${d.value} }`)).toEqual([]);
    },
  );

  it("transitions run on a tempo and a named curve", () => {
    const bad = declarations()
      .filter((d) => d.property === "transition")
      .filter((d) => d.value !== "none")
      .filter((d) => !/^(?:[a-z-]+ var\(--tempo-[a-z]+\) var\(--ease\)(?:, )?)+$/.test(d.value));
    expect(bad.map((d) => `${d.selector} { transition: ${d.value} }`)).toEqual([]);
  });

  it.each(["transition-duration", "animation-duration"])("%s", (property) => {
    const bad = declarations()
      .filter((d) => d.property === property)
      .filter((d) => !d.value.startsWith("0.001ms"))
      .filter((d) => !named(d.value, /^--tempo-/));
    expect(bad.map((d) => `${d.selector} { ${property}: ${d.value} }`)).toEqual([]);
  });

  it("animations run on a named curve", () => {
    const bad = declarations()
      .filter((d) => d.property === "animation")
      .filter((d) => d.value !== "none")
      .filter((d) => !/var\(--ease(-spring|-loop)?\)|steps\(|linear/.test(d.value));
    expect(bad.map((d) => `${d.selector} { animation: ${d.value} }`)).toEqual([]);
  });

  it("shadows are a lift, a ring or a glow", () => {
    const bad = declarations()
      .filter((d) => d.property === "box-shadow")
      .filter((d) => d.value !== "none")
      .filter((d) => !d.value.includes("var(--lift-"))
      .filter((d) => d.value !== "var(--stage-lift)")
      .filter((d) => !d.value.includes("inset"))
      .filter((d) => !/^0 0 /.test(d.value));
    expect(bad.map((d) => `${d.selector} { box-shadow: ${d.value} }`)).toEqual([]);
  });
});

describe("scroll bars", () => {
  const ruled = (selector: string) =>
    declarations().filter((d) =>
      d.selector
        .split(",")
        .map((one) => one.trim())
        .includes(selector),
    );

  it("reserve their room in every region that scrolls, so a bar never takes it from the content", () => {
    const hidden = new Set(
      declarations()
        .filter((d) => d.property === "scrollbar-width" && d.value === "none")
        .flatMap((d) => d.selector.split(",").map((one) => one.trim())),
    );
    const reserved = new Set(
      declarations()
        .filter((d) => d.property === "scrollbar-gutter" && d.value === "stable")
        .flatMap((d) => d.selector.split(",").map((one) => one.trim())),
    );
    const scrolling = declarations()
      .filter((d) => !d.selector.startsWith("@"))
      .filter(
        (d) =>
          (d.property === "overflow-y" && /^(auto|scroll)$/.test(d.value)) ||
          (d.property === "overflow" && /^(auto|scroll)\b/.test(d.value)),
      )
      .flatMap((d) => d.selector.split(",").map((one) => one.trim()));
    expect(scrolling.length).toBeGreaterThan(20);
    expect(
      scrolling.filter((selector) => !hidden.has(selector) && !reserved.has(selector)),
    ).toEqual([]);
  });

  it("are one thin bar drawn from the tokens, with a hover and a drag state", () => {
    expect(ruled("::-webkit-scrollbar").find((d) => d.property === "width")?.value).toBe(
      "var(--scrollbar-size)",
    );
    expect(ruled("::-webkit-scrollbar").find((d) => d.property === "height")?.value).toBe(
      "var(--scrollbar-size)",
    );
    const thumb = (selector: string) =>
      ruled(selector).find((d) => d.property === "background" || d.property === "background-color")
        ?.value;
    expect(thumb("::-webkit-scrollbar-thumb")).toBe("var(--scrollbar-thumb) content-box");
    expect(thumb("::-webkit-scrollbar-thumb:hover")).toBe("var(--scrollbar-thumb-hover)");
    expect(thumb("::-webkit-scrollbar-thumb:active")).toBe("var(--scrollbar-thumb-drag)");
    expect(css).toMatch(
      /@supports not selector\(::-webkit-scrollbar\) \{\n {2}\* \{\n {4}scrollbar-width: thin;\n {4}scrollbar-color: var\(--scrollbar-thumb\) transparent;/,
    );
    const sizes = declarations().filter(
      (d) => /^(width|height)$/.test(d.property) && d.selector.includes("scrollbar"),
    );
    expect(sizes.every((d) => d.value === "var(--scrollbar-size)")).toBe(true);
  });
});

describe("the menu bar panel", () => {
  it("keeps the agent row's keyboard ring inside the scrolling edge", () => {
    const offset = declarations().find(
      (d) =>
        d.selector === ".menubar__agents .agent-row:focus-visible" &&
        d.property === "outline-offset",
    );
    expect(offset?.value).toBe("calc(var(--focus-outline) * -1)");
  });

  it("draws no focus ring until a key is pressed", () => {
    const rule = declarations().find(
      (d) =>
        d.selector === ":root:not([data-keyboard]) .menubar :focus-visible" &&
        d.property === "outline",
    );
    expect(rule?.value).toBe("none");
  });
});

describe("every token a rule spends exists", () => {
  it("names a property this file declares or a component sets", () => {
    const rules = `${css}\n${signalCss}\n${composerCss}`.replace(/\/\*[\s\S]*?\*\//g, "");
    const declared = new Set([...rules.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
    const root = join(process.cwd(), "src");
    const code = readdirSync(root, { recursive: true, encoding: "utf8" })
      .filter((file) => /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file))
      .map((file) => readFileSync(join(root, file), "utf8"))
      .join("\n");
    const spent = new Set([...rules.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1] ?? ""));
    const nowhere = [...spent].filter(
      (token) => !declared.has(token) && !new RegExp(`["'\`]${token}["'\`]`).test(code),
    );
    expect(nowhere).toEqual([]);
  });
});

describe("what the attention color is spent on", () => {
  it("draws no focus ring, focused field or drop target in it", () => {
    const bad = declarations()
      .filter((d) => /:focus|data-over|data-dragging/.test(d.selector))
      .filter((d) => d.value.includes("var(--attention"));
    expect(bad.map((d) => `${d.selector} { ${d.property}: ${d.value} }`)).toEqual([]);
  });

  it("draws no primary control or on-state in it", () => {
    const bad = declarations()
      .filter((d) =>
        /\.btn--primary|\.composer__send|aria-checked="true"|aria-pressed="true"/.test(d.selector),
      )
      .filter((d) => d.value.includes("var(--attention"));
    expect(bad.map((d) => `${d.selector} { ${d.property}: ${d.value} }`)).toEqual([]);
  });

  it("sets the command menu's rows in the regular weight, with their state on the same line", () => {
    const rule = (selector: string, property: string) =>
      declarations().find((d) => d.selector === selector && d.property === property)?.value;
    expect(rule(".palette--menu .palette__title", "font-weight")).toBe("var(--weight-regular)");
    expect(rule(".palette--menu .palette__text", "flex-direction")).toBe("row");
    expect(rule(".palette__who", "font-weight")).toBe("var(--weight-strong)");
  });

  it("spends the system color on focus and drop targets only, never on a fill or an edge", () => {
    const bad = declarations()
      .filter((d) => /^(background|border|outline|box-shadow)/.test(d.property))
      .filter((d) => /var\(--(system|focus)\b/.test(d.value))
      .filter((d) => !/:focus|data-over|data-dragging|data-lands/.test(d.selector));
    expect(bad.map((d) => `${d.selector} { ${d.property}: ${d.value} }`)).toEqual([]);
  });

  it("fills a primary control in ink, at rest and under the pointer", () => {
    const fills = declarations()
      .filter((d) => /\.btn--primary(?![\w-])/.test(d.selector) && d.property === "background")
      .map((d) => d.value);
    expect(fills.length).toBeGreaterThan(0);
    for (const fill of fills)
      expect(fill).toMatch(/^(var|color-mix\(in oklab, var)\(--(text|action-fill)\)/);
    const signal = declarations().filter(
      (d) => d.selector === ".btn--primary:hover:not(:disabled)" && d.property === "background",
    );
    expect(new Set(signal.map((d) => d.value)).size).toBe(signal.length);
    expect(signal.at(-1)?.value).not.toBe("var(--action-fill)");
  });

  it("sets text on a fill in that fill's own text color", () => {
    const rules = new Map<string, Map<string, string>>();
    for (const d of declarations()) {
      const rule = rules.get(d.selector) ?? new Map<string, string>();
      rule.set(d.property, d.value);
      rules.set(d.selector, rule);
    }
    const bad: string[] = [];
    for (const [selector, rule] of rules) {
      const fill = rule.get("background")?.match(/^var\(--([a-z]+)-fill\)$/)?.[1];
      const color = rule.get("color");
      if (fill && color && color !== `var(--on-${fill}-fill)`)
        bad.push(`${selector} { color: ${color} }`);
    }
    expect(bad).toEqual([]);
  });

  it.each([".input, .textarea, .select", ".choice"])(
    "borders %s in a control's edge",
    (selector) => {
      const border = declarations().find(
        (d) => d.selector.endsWith(selector) && d.property === "border",
      );
      expect(border?.value).toBe("1px solid var(--edge-control)");
    },
  );

  it("bounds the composer on every side", () => {
    const rules = declarations();
    const last = (selector: string, property: string) =>
      rules.filter((d) => d.selector === selector && d.property === property).at(-1)?.value;
    expect(last(".composer", "border")).toBe("1px solid var(--edge-control)");
    expect(last(".composer", "border-radius")).toBe("var(--radius-card)");
    expect(last(".composer:focus-within", "border-color")).toBe("var(--focus)");
    expect(
      last(
        ".composer__send, .composer__send:disabled, .composer__send:hover:not(:disabled)",
        "background",
      ),
    ).toBe("var(--action-fill)");
    expect(
      last(
        ".composer__send, .composer__send:disabled, .composer__send:hover:not(:disabled)",
        "color",
      ),
    ).toBe("var(--on-action-fill)");
    expect(last(".composer__recipient option, .composer__recipient optgroup", "color")).toBe(
      "var(--text)",
    );
    expect(last(".composer__recipient option, .composer__recipient optgroup", "background")).toBe(
      "var(--raised)",
    );
  });
});

describe("native settings controls show that they are interactive", () => {
  it.each(["file", "checkbox", "radio"])("gives an enabled %s input a pointer", (type) => {
    const input = document.createElement("input");
    input.type = type;
    document.body.append(input);
    expect(getComputedStyle(input).cursor).toBe("pointer");
    input.disabled = true;
    expect(getComputedStyle(input).cursor).not.toBe("pointer");
  });

  it.each(["select", "summary"])("gives %s a pointer", (tag) => {
    const control = document.createElement(tag);
    document.body.append(control);
    expect(getComputedStyle(control).cursor).toBe("pointer");
  });

  it("draws the file picker button with the same border and surface as other buttons", () => {
    const shared = declarations().filter(
      (d) => d.selector === '.btn, input[type="file"]::file-selector-button',
    );
    expect(shared.map((d) => d.property)).toEqual(
      expect.arrayContaining([
        "border",
        "background",
        "color",
        "min-height",
        "padding",
        "border-radius",
      ]),
    );
    expect(
      declarations().some(
        (d) =>
          d.selector.includes('input[type="file"]:not(:disabled)::file-selector-button:hover') &&
          d.property === "background",
      ),
    ).toBe(true);
  });
});

describe("a host and this app on different builds", () => {
  it("marks the rows that name them the way it marks the status bar", () => {
    const facts = document.createElement("dl");
    facts.className = "host-update-facts";
    const plain = facts.appendChild(document.createElement("dd"));
    const marked = facts.appendChild(document.createElement("dd"));
    marked.setAttribute("data-drift", "");
    const label = nest("statusbar__host");
    label.setAttribute("data-drift", "");
    document.body.append(facts);
    expect(getComputedStyle(marked).color).toBe(getComputedStyle(label).color);
    expect(getComputedStyle(marked).color).not.toBe(getComputedStyle(plain).color);
  });
});

describe("the new window's surfaces", () => {
  function ruleOf(source: string, selector: string): string {
    const bare = source.replace(/\/\*[\s\S]*?\*\//g, "");
    return [...bare.matchAll(/(^|\n)([^{}@\n][^{}]*)\{([^{}]*)\}/g)]
      .filter((match) =>
        (match[2] ?? "")
          .split(",")
          .map((one) => one.trim())
          .includes(selector),
      )
      .map((match) => match[3] ?? "")
      .join("\n");
  }

  it("keeps roster faces and names at full strength even for idle agents", () => {
    const bare = css.replace(/\/\*[\s\S]*?\*\//g, "");
    const fading = [...bare.matchAll(/(^|\n)([^{}@\n][^{}]*)\{([^{}]*)\}/g)]
      .filter((match) => /(^|[;{\s])(opacity|filter)\s*:/.test(match[3] ?? ""))
      .flatMap((match) => (match[2] ?? "").split(",").map((one) => one.trim()))
      .filter((selector) => /\.roster\b/.test(selector));
    expect(fading).toEqual([]);
  });

  it("brings the Groups sheet in on the one arrival, with no keyframe of its own", () => {
    expect(ruleOf(css, ".sheet")).not.toContain("sheet-in");
    const arrival = [...css.matchAll(/([^{}]*)\{[^{}]*animation: pop var/g)].map((m) => m[1] ?? "");
    expect(
      arrival.some((selectors) =>
        selectors
          .split(",")
          .map((s) => s.trim())
          .includes(".sheet"),
      ),
    ).toBe(true);
    expect(css).not.toMatch(/@keyframes sheet-in/);
  });

  it("gives the page's tabs, the top bar's buttons and the field's send a finger's room", () => {
    const opener = "@media (max-width: 48rem), (hover: none) and (pointer: coarse) {";
    const blocks = css.split(opener).slice(1).map((block) => block.slice(0, block.indexOf("\n}\n")));
    expect(blocks.length).toBeGreaterThan(0);
    for (const selector of [
      ".agent-tabs__tab",
      ".topbar__button",
      ".composer__send",
      ".desk__open",
      ".composer__attach",
    ]) {
      const sized = blocks.flatMap((block) => [...block.matchAll(/([^{}]*)\{([^{}]*)\}/g)]).some(
        (m) =>
          (m[1] ?? "")
            .split(",")
            .map((s) => s.trim())
            .includes(selector) &&
          (m[2] ?? "").includes("min-height: var(--control-touch)") &&
          (m[2] ?? "").includes("min-width: var(--control-touch)"),
      );
      expect(sized, `${selector} is not sized for touch`).toBe(true);
    }
  });

  it("spends one named column width on the agent panel and a separate one on the roster", () => {
    expect(ruleOf(css, ".agent-panel")).toContain("var(--column-w)");
    expect(ruleOf(css, ".agent-panel")).not.toMatch(/\d+rem/);
    expect(ruleOf(css, ".app[data-roster]")).toContain("var(--roster-w)");
    expect(ruleOf(css, ".app[data-roster]")).toContain("var(--roster-w-compact)");
    expect(ruleOf(css, ".app[data-roster]")).not.toMatch(/\d+rem/);
  });
});

describe("the waiting count", () => {
  const tray = readFileSync(join(process.cwd(), "scripts/make-tray.py"), "utf8");
  const trayInk = (name: string) => {
    const rgb = new RegExp(`^${name} = \\(0x(\\w\\w), 0x(\\w\\w), 0x(\\w\\w)\\)$`, "m").exec(tray);
    expect(rgb, name).not.toBeNull();
    return `#${rgb?.slice(1).join("")}`.toLowerCase();
  };
  const tokens = (name: string) =>
    [...css.matchAll(new RegExp(`--${name}: (#\\w{6});`, "g"))].map((m) => m[1]?.toLowerCase());
  it("draws the menu bar glyph in the attention fill, edged in the ink drawn on that fill", () => {
    for (const [name, token] of [
      ["ATTENTION", "attention-fill"],
      ["ATTENTION_EDGE", "on-attention-fill"],
    ] as const) {
      const values = tokens(token);
      expect(values.length, token).toBeGreaterThan(0);
      expect(values).toEqual(values.map(() => trayInk(name)));
    }
  });

  it.each([
    ["a white bar", "#ffffff"],
    ["a light gray bar", "#ececec"],
    ["a light bar tinted by a blue wallpaper", "#486b94"],
    ["a dark bar", "#1e1e1e"],
    ["a black bar", "#000000"],
  ])("keeps the waiting glyph at 3 to 1 or more on %s", (_, bar) => {
    const fill = trayInk("ATTENTION");
    const edge = trayInk("ATTENTION_EDGE");
    expect(Math.max(contrast(fill, bar), contrast(edge, bar))).toBeGreaterThanOrEqual(3);
    expect(contrast(fill, edge)).toBeGreaterThanOrEqual(9);
  });

  it("rings the waiting glyph's outline in the edge ink and fills its middle with amber", () => {
    const png = readFileSync(join(process.cwd(), "src-tauri/icons/tray-attention.png"));
    const size = png.readUInt32BE(16);
    const idat: Buffer[] = [];
    for (let at = 8; at < png.length; ) {
      const length = png.readUInt32BE(at);
      if (png.toString("ascii", at + 4, at + 8) === "IDAT")
        idat.push(png.subarray(at + 8, at + 8 + length));
      at += length + 12;
    }
    const raw = inflateSync(Buffer.concat(idat));
    const pixel = (x: number, y: number) => {
      const at = y * (size * 4 + 1) + 1 + x * 4;
      return { hex: `#${raw.subarray(at, at + 3).toString("hex")}`, alpha: raw[at + 3] ?? 0 };
    };
    const edge = trayInk("ATTENTION_EDGE");
    const fill = trayInk("ATTENTION");
    let rows = 0;
    let filled = 0;
    for (let y = 0; y < size; y++) {
      expect(raw[y * (size * 4 + 1)], `row ${y} filter`).toBe(0);
      const solid = [...Array(size).keys()].filter((x) => pixel(x, y).alpha === 255);
      if (solid.length < 4) continue;
      rows++;
      expect(pixel(solid[0] ?? 0, y).hex, `row ${y}`).toBe(edge);
      expect(pixel(solid.at(-1) ?? 0, y).hex, `row ${y}`).toBe(edge);
      if (solid.some((x) => pixel(x, y).hex === fill)) filled++;
    }
    expect(rows).toBeGreaterThan(size / 2);
    expect(filled).toBeGreaterThan(rows / 2);
  });

  it.each([
    ".orb__waiting",
    ".roster__questions",
    ".statusbar__count",
    ".for-you h2 span",
    ".menubar__decisions",
  ])(
    "draws %s in amber, which means needs you, and never in red, which means failed",
    (selector) => {
      const own = declarations().filter((d) => d.selector === selector);
      const ink = own.filter((d) => d.property === "color");
      expect(ink.length, selector).toBeGreaterThan(0);
      for (const d of ink) expect(d.value, selector).toMatch(/^var\(--(on-)?attention(-fill)?\)$/);
      expect(own.filter((d) => d.value.includes("danger"))).toEqual([]);
    },
  );

  it("keeps the counts beside it that wait on nobody out of amber", () => {
    const counts = declarations().filter(
      (d) => d.selector === ".roster__questions:disabled" && d.property === "color",
    );
    expect(counts.map((d) => d.value)).toEqual(["var(--muted)"]);
  });

  it("edges a stuck agent's card in amber, since it waits on a person rather than failing", () => {
    const edge = declarations().find(
      (d) => d.selector === '.desk__card[data-kind="stuck"]' && d.property === "border-left",
    );
    expect(edge?.value).toBe("2px solid var(--attention)");
  });
});
