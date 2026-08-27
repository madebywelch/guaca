import { describe, expect, it } from "vitest";

import { isFault } from "./chart";
import { fenceLanguage, fenceText, looksComplete, readFigure } from "./figure";

const SPEC = '{"type":"bar","labels":["a"],"series":[{"data":[1]}]}';

describe("what a fence turns out to be", () => {
  it("draws the three tags that mean a figure", () => {
    for (const tag of ["chart", "graph", "plot"]) {
      expect(readFigure(tag, SPEC).kind).toBe("chart");
    }
  });

  it("leaves every other language as source", () => {
    // The rule that keeps ```python from turning into something nobody asked
    // for. A fence is text unless it was tagged as one of the few that are not.
    for (const tag of ["", "python", "rust", "json", "sql", "ts"]) {
      expect(readFigure(tag, SPEC).kind).toBe("source");
    }
  });

  it("waits for a spec that is still arriving instead of failing it", () => {
    // A reply lands a token at a time, so a chart spends most of its life on
    // screen as half an object. Called an error, that is a red box under every
    // figure for a second, which teaches an operator the feature is broken.
    expect(readFigure("chart", '{"type":"ba').kind).toBe("pending");
    expect(readFigure("chart", '{"type":"bar","series":[{"data":[1,2').kind).toBe("pending");
  });

  it("does call it wrong once every brace has closed", () => {
    const figure = readFigure("chart", "{ nope: }");
    if (figure.kind !== "chart") throw new Error("expected a chart");
    expect(isFault(figure.read)).toBe(true);
  });

  it("keeps the source beside a refusal", () => {
    // An operator looking at a figure that did not draw needs to see what was
    // asked for, and the agent needs to be told what to change. A box saying
    // "invalid chart" is neither of those.
    const figure = readFigure("chart", '{"type":"sunburst","series":[{"data":[1]}]}');
    if (figure.kind !== "chart") throw new Error("expected a chart");
    expect(figure.source).toContain("sunburst");
    expect(isFault(figure.read) && figure.read.why).toContain("bar, line");
  });
});

describe("an html fence", () => {
  const page = `<!doctype html><html><body><h1>Hello</h1><p>Some content here.</p></body></html>`;

  it("is a page when it is long enough to be worth a frame", () => {
    expect(readFigure("html", page).kind).toBe("html");
    expect(readFigure("artifact", page).kind).toBe("html");
  });

  it("is source when it is a snippet somebody wants to read", () => {
    // Framing `<div>hi</div>` on its own origin is a whole renderer and a
    // network round trip spent on what a code block already showed.
    expect(readFigure("html", "<div>hi</div>").kind).toBe("source");
  });

  it("is source when it has no markup in it at all", () => {
    expect(readFigure("html", "just some prose, mistagged, and reasonably long").kind).toBe(
      "source",
    );
  });
});

describe("a fence in a reply that is still being written", () => {
  const PAGE = "<!doctype html><html><body><h1>Plan</h1><p>Long enough to frame.</p></body></html>";

  it("holds a page until the reply settles", () => {
    // A page is drawn by registering a document and pointing a frame at the
    // address that comes back, so drawing one per token is a reload per token:
    // a round trip each, an entry each in a store that holds two dozen, and a
    // frame that throws away whatever the operator did in it every sixteen
    // milliseconds.
    expect(readFigure("html", PAGE, true).kind).toBe("pending");
    expect(readFigure("artifact", PAGE, true).kind).toBe("pending");
    expect(readFigure("html", PAGE, false).kind).toBe("html");
  });

  it("says so the moment the fence opens, rather than flashing a code block", () => {
    expect(readFigure("html", "", true).kind).toBe("pending");
    expect(readFigure("html", "<!doc", true).kind).toBe("pending");
  });

  it("leaves a chart assembling itself, which is what it is for", () => {
    // Pure function to coordinates, redrawn for free. This is the one figure
    // where watching it arrive is the feature rather than the cost.
    expect(readFigure("chart", SPEC, true).kind).toBe("chart");
    expect(readFigure("chart", '{"type":"ba', true).kind).toBe("pending");
  });

  it("leaves every other fence exactly as it was", () => {
    expect(readFigure("python", "print(1)", true).kind).toBe("source");
  });
});

describe("telling a finished document from one still arriving", () => {
  it("counts braces and brackets", () => {
    expect(looksComplete("{}")).toBe(true);
    expect(looksComplete("{ ")).toBe(false);
    expect(looksComplete('{"a":[1,2]}')).toBe(true);
    expect(looksComplete('{"a":[1,2]')).toBe(false);
  });

  it("does not count a brace inside a string", () => {
    // A label of "}" is a perfectly ordinary category name, and reading it as
    // structure ends the document early and draws half a chart.
    expect(looksComplete('{"label":"}"')).toBe(false);
    expect(looksComplete('{"label":"}"}')).toBe(true);
  });

  it("does not let an escaped quote end a string", () => {
    expect(looksComplete('{"label":"\\""}')).toBe(true);
    expect(looksComplete('{"label":"\\"')).toBe(false);
  });

  it("does not call an empty string finished", () => {
    expect(looksComplete("")).toBe(false);
    expect(looksComplete("   ")).toBe(false);
  });
});

describe("getting at a fence", () => {
  it("finds the language react-markdown leaves on the class", () => {
    expect(fenceLanguage("language-chart")).toBe("chart");
    expect(fenceLanguage("hljs language-html other")).toBe("html");
    expect(fenceLanguage(undefined)).toBe("");
    expect(fenceLanguage("no-language-here")).toBe("");
  });

  it("flattens the nodes a fence's text arrives as", () => {
    expect(fenceText(["a", ["b", "c"]])).toBe("abc");
    expect(fenceText(null)).toBe("");
  });
});
