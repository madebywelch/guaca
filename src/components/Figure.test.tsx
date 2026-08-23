import { render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const frameArtifact = vi.fn(async () => ({ port: 9999, id: "a".repeat(64) }));
vi.mock("../lib/ipc", () => ({
  api: { frameArtifact: () => frameArtifact() },
  openExternal: vi.fn(),
}));

const { Markdown } = await import("./Markdown");

const fence = (language: string, body: unknown) =>
  `\`\`\`${language}\n${typeof body === "string" ? body : JSON.stringify(body)}\n\`\`\``;

const BARS = JSON.stringify({
  type: "bar",
  title: "Revenue by quarter",
  prefix: "$",
  labels: ["Q1", "Q2"],
  series: [{ name: "2026", data: [12, 18] }],
});

describe("a chart in a message", () => {
  it("draws instead of printing the JSON", () => {
    const { container } = render(<Markdown>{fence("chart", BARS)}</Markdown>);
    expect(container.querySelector("svg.chart__svg")).toBeTruthy();
    expect(container.querySelector(".chart__bar")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Revenue by quarter" })).toBeTruthy();
  });

  it("says what it is, for anyone who cannot see it", () => {
    // The drawing itself is one image with one sentence on it. The numbers are
    // in the table underneath, which is where a reader is sent rather than
    // being read a hundred coordinates.
    render(<Markdown>{fence("chart", BARS)}</Markdown>);
    const drawing = screen.getByRole("img");
    expect(drawing.getAttribute("aria-label")).toContain("Revenue by quarter");
    expect(drawing.getAttribute("aria-label")).toContain("table below");
  });

  it("carries every value as text as well as as a shape", () => {
    // A value behind a hover is a value some readers can never reach. This is
    // the channel that makes the drawing an enhancement rather than the only
    // way to find out what the numbers were.
    const { container } = render(<Markdown>{fence("chart", BARS)}</Markdown>);
    const table = container.querySelector(".chart__table table");
    if (!table) throw new Error("a chart shipped without its figures");
    expect(within(table as HTMLElement).getByText("$12")).toBeTruthy();
    expect(within(table as HTMLElement).getByText("$18")).toBeTruthy();
  });

  it("writes the values on a single series, where they are the point", () => {
    const { container } = render(<Markdown>{fence("chart", BARS)}</Markdown>);
    const labels = [...container.querySelectorAll(".chart__value")].map((at) => at.textContent);
    expect(labels).toEqual(["$12", "$18"]);
  });

  it("offers a legend past one series and none at one", () => {
    // One colour, and the title above already says what is plotted. A box with
    // a single swatch in it restates the title and costs a line.
    const { container } = render(<Markdown>{fence("chart", BARS)}</Markdown>);
    expect(container.querySelector(".chart__legend")).toBeNull();

    const two = render(
      <Markdown>
        {fence(
          "chart",
          JSON.stringify({
            type: "bar",
            labels: ["Q1"],
            series: [
              { name: "2025", data: [1] },
              { name: "2026", data: [2] },
            ],
          }),
        )}
      </Markdown>,
    );
    expect(two.container.querySelector(".chart__legend")).toBeTruthy();
    expect(screen.getByRole("button", { name: "2025", pressed: true })).toBeTruthy();
  });

  it("makes a pie's legend a key rather than a set of switches", () => {
    // Its slices are shares of one whole, so switching one off would leave the
    // others claiming percentages of a total that has not changed. A button
    // that does nothing when pressed is worse than a label.
    const { container } = render(
      <Markdown>
        {fence("chart", {
          type: "pie",
          labels: ["a", "b"],
          series: [{ data: [3, 1] }],
        } as never)}
      </Markdown>,
    );
    expect(container.querySelector(".chart__legend")).toBeTruthy();
    expect(container.querySelector(".chart__legend button")).toBeNull();
    expect(container.querySelectorAll(".chart__legend-item[data-static]")).toHaveLength(2);
  });

  it("keeps the source one press away", () => {
    // A figure drawn from a model's own JSON is a claim about numbers, and an
    // operator checking the chart against what was written has nowhere else to
    // look. It is also the only thing that can be copied out.
    render(<Markdown>{fence("chart", BARS)}</Markdown>);
    expect(screen.getByRole("button", { name: "Source" })).toBeTruthy();
  });
});

describe("a chart that will not draw", () => {
  it("shows what was asked for and why it was refused", () => {
    const { container } = render(
      <Markdown>{fence("chart", '{"type": "sunburst", "series": [{"data": [1]}]}')}</Markdown>,
    );
    expect(container.querySelector(".figure__fault")?.textContent).toContain("not a chart type");
    // The spec itself stays on screen: the operator needs to see the request,
    // and the agent needs to be told what to change.
    expect(container.querySelector(".figure__source")?.textContent).toContain("sunburst");
  });

  it("waits quietly while one is still arriving", () => {
    // Mid-stream a chart is half an object. Called an error, that is a red box
    // under every figure for a second, which teaches an operator that the
    // feature is broken.
    const { container } = render(<Markdown>{'```chart\n{"type": "ba'}</Markdown>);
    expect(container.querySelector(".figure--waiting")).toBeTruthy();
    expect(container.querySelector(".figure__fault")).toBeNull();
  });
});

describe("a page in a message", () => {
  const PAGE =
    "<!doctype html><html><body><h1>Plan</h1><p>Something worth framing.</p></body></html>";

  it("mounts exactly one frame, however many places it could go", async () => {
    // The same element written into both the inline card and the full view is
    // two of them: React draws it once per position in the tree, so a second
    // renderer would start loading the same page and the height message would
    // arrive from the wrong window.
    const { container } = render(<Markdown>{fence("html", PAGE)}</Markdown>);
    await waitFor(() => expect(container.querySelector("iframe")).toBeTruthy());
    expect(container.querySelectorAll("iframe")).toHaveLength(1);
  });

  it("is framed on an origin of its own, with scripts and nothing else", async () => {
    // Never `allow-same-origin` beside `allow-scripts`: together they let the
    // page take its own sandbox off and reload without one.
    const { container } = render(<Markdown>{fence("html", PAGE)}</Markdown>);
    await waitFor(() => {
      const frame = container.querySelector("iframe");
      if (!frame) throw new Error("no frame yet");
      expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
      expect(frame.getAttribute("src")).toBe(`http://127.0.0.1:9999/${"a".repeat(64)}`);
    });
  });

  it("never becomes part of this document", async () => {
    // The markup goes over IPC and comes back as an address. Nothing in the
    // fence is ever parsed into the app's own tree.
    const { container } = render(<Markdown>{fence("html", PAGE)}</Markdown>);
    await waitFor(() => expect(container.querySelector("iframe")).toBeTruthy());
    expect(container.querySelector("h1")).toBeNull();
  });

  it("leaves markup in the prose alone, as it always did", () => {
    const { container } = render(
      <Markdown>{'<img src=x onerror="alert(1)"><script>alert(2)</script>**safe**'}</Markdown>,
    );
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("iframe")).toBeNull();
    expect(container.querySelector("strong")?.textContent).toBe("safe");
  });

  it("leaves a snippet as a code block", () => {
    // Framing `<div>hi</div>` on its own origin is a renderer and a round trip
    // spent on what a code block already showed.
    const { container } = render(<Markdown>{fence("html", "<div>hi</div>")}</Markdown>);
    expect(container.querySelector(".md__pre")).toBeTruthy();
    expect(container.querySelector("iframe")).toBeNull();
  });
});

describe("every other fence", () => {
  it("stays source", () => {
    const { container } = render(<Markdown>{fence("python", "print(1)")}</Markdown>);
    expect(container.querySelector(".md__pre code")?.textContent).toContain("print(1)");
    expect(container.querySelector(".figure")).toBeNull();
  });

  it("including one holding JSON that happens to be a chart spec", () => {
    // A model showing an operator what a spec looks like is showing them text.
    const { container } = render(<Markdown>{fence("json", BARS)}</Markdown>);
    expect(container.querySelector("svg.chart__svg")).toBeNull();
    expect(container.querySelector(".md__pre")).toBeTruthy();
  });
});
