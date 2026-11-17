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
    for (const tag of ["", "python", "rust", "json", "sql", "ts"]) {
      expect(readFigure(tag, SPEC).kind).toBe("source");
    }
  });

  it("waits for a spec that is still arriving instead of failing it", () => {
    expect(readFigure("chart", '{"type":"ba').kind).toBe("pending");
    expect(readFigure("chart", '{"type":"bar","series":[{"data":[1,2').kind).toBe("pending");
  });

  it("does call it wrong once every brace has closed", () => {
    const figure = readFigure("chart", "{ nope: }");
    if (figure.kind !== "chart") throw new Error("expected a chart");
    expect(isFault(figure.read)).toBe(true);
  });

  it("keeps the source beside a refusal", () => {
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
    expect(readFigure("html", PAGE, true).kind).toBe("pending");
    expect(readFigure("artifact", PAGE, true).kind).toBe("pending");
    expect(readFigure("html", PAGE, false).kind).toBe("html");
  });

  it("says so the moment the fence opens, rather than flashing a code block", () => {
    expect(readFigure("html", "", true).kind).toBe("pending");
    expect(readFigure("html", "<!doc", true).kind).toBe("pending");
  });

  it("leaves a chart assembling itself, which is what it is for", () => {
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
