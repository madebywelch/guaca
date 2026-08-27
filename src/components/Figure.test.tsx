import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const frameArtifact = vi.fn(async () => ({ port: 9999, id: "a".repeat(64) }));
const sendMessage = vi.fn(async (_agentId: string, _text: string) => "run-1");
vi.mock("../lib/ipc", () => ({
  api: {
    frameArtifact: () => frameArtifact(),
    sendMessage: (agentId: string, text: string) => sendMessage(agentId, text),
  },
  openExternal: vi.fn(),
}));

const { Markdown } = await import("./Markdown");
const { Answering, answerMessage } = await import("./HtmlArtifact");

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
    // One color, and the title above already says what is plotted. A box with
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

describe("a page that hands a value back", () => {
  const PAGE =
    "<!doctype html><html><body><h1>Plan</h1><p>Something worth framing.</p></body></html>";
  const TO = { id: "agent-1", name: "Analyst" };

  beforeEach(() => {
    sendMessage.mockClear();
  });

  /** Renders the page in a channel and answers from inside the frame. */
  const answering = async (to: typeof TO | null = TO) => {
    const view = render(
      <Answering.Provider value={to}>
        <Markdown>{fence("html", PAGE)}</Markdown>
      </Answering.Provider>,
    );
    const frame = await waitFor(() => {
      const found = view.container.querySelector("iframe");
      if (!found) throw new Error("no frame yet");
      return found as HTMLIFrameElement;
    });
    const say = (value: unknown) =>
      act(() => {
        // Exactly what the bridge in `artifact.rs` posts, and delivered from
        // the frame's own window, which is the only thing the parent trusts.
        window.dispatchEvent(
          new MessageEvent("message", {
            source: frame.contentWindow,
            data: { guaca: "artifact-answer", value },
          }),
        );
      });
    return { ...view, say };
  };

  it("shows what was handed back and sends nothing on its own", async () => {
    // The whole safety argument. A transcript re-frames a page whenever it
    // draws one, so a page that could send by itself would send again every
    // time it was scrolled past, and each send is a turn nobody asked for.
    const { say } = await answering();
    say('{"plan":"pro","seats":12}');

    expect(screen.getByText('{"plan":"pro","seats":12}')).toBeTruthy();
    expect(sendMessage).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Send to Analyst" })).toBeTruthy();
  });

  it("sends it when the operator presses the button, and says so", async () => {
    const { say } = await answering();
    say('{"plan":"pro"}');

    await act(async () => {
      screen.getByRole("button", { name: "Send to Analyst" }).click();
    });

    expect(sendMessage).toHaveBeenCalledWith("agent-1", answerMessage('{"plan":"pro"}'));
    expect(screen.getByText("Sent to Analyst.")).toBeTruthy();
  });

  it("says why when the send is refused, and keeps the value on screen", async () => {
    sendMessage.mockRejectedValueOnce(new Error("this agent has been deleted"));
    const { say } = await answering();
    say('{"plan":"pro"}');

    await act(async () => {
      screen.getByRole("button", { name: "Send to Analyst" }).click();
    });

    expect(screen.getByText("this agent has been deleted")).toBeTruthy();
  });

  it("keeps only the latest, because a page answering on every drag is right", async () => {
    const { say } = await answering();
    say('{"at":1}');
    say('{"at":2}');

    expect(screen.queryByText('{"at":1}')).toBeNull();
    expect(screen.getByText('{"at":2}')).toBeTruthy();
  });

  it("refuses a document dressed as an answer, and names the cap", async () => {
    const { say } = await answering();
    say(JSON.stringify({ everything: "x".repeat(5000) }));

    expect(screen.queryByRole("button", { name: "Send to Analyst" })).toBeNull();
    expect(screen.getByText(/most Guaca will carry/)).toBeTruthy();
  });

  it("ignores anything that is not the string the bridge posts", async () => {
    const { say } = await answering();
    say({ plan: "pro" });

    expect(screen.queryByText(/answer ready/)).toBeNull();
  });

  it("draws nothing where there is nobody to answer", async () => {
    // A search hit, a pair's thread, a document preview. A Send button there
    // is a control that cannot say who it sends to.
    const { say } = await answering(null);
    say('{"plan":"pro"}');

    expect(screen.queryByText(/answer ready/)).toBeNull();
    expect(sendMessage).not.toHaveBeenCalled();
  });

  it("fences the value clear of its own backticks", () => {
    // A page about code hands back a snippet, and a fixed three would end the
    // block in the middle of the value.
    const message = answerMessage('{"snippet":"```js"}');
    expect(message).toContain("````json");
    expect(message.endsWith("````")).toBe(true);
    // The plain case stays the plain case.
    expect(answerMessage('{"plan":"pro"}')).toContain("```json");
  });
});

/**
 * The seam between the script `artifact.rs` prepends and the parent that reads
 * it, checked by running the real one.
 *
 * Nothing else in the build can see this break. The Rust suite asserts what the
 * bridge's text contains, the suite above asserts what the renderer accepts, and
 * the two agree because the same literal is written in both places: renaming the
 * message in Rust fails one test, and updating that test makes the build green
 * with a page that can no longer answer. Same failure `ipc.contract.test.ts`
 * exists for, so the same answer: read both sources and compare them.
 */
describe("the bridge the page is served with", () => {
  /** The bridge's JavaScript, out of the Rust constant that carries it. */
  const bridge = () => {
    const rust = readFileSync(resolve(__dirname, "../../src-tauri/src/artifact.rs"), "utf8");
    const held = rust.match(/const BRIDGE: &str = r#"([\s\S]*?)"#;/);
    if (!held) throw new Error("could not find BRIDGE in artifact.rs");
    const script = held[1]?.match(/<script>([\s\S]*)<\/script>/);
    if (!script) throw new Error("BRIDGE is not a script");
    return script[1] as string;
  };

  /** Runs it the way the page does, and collects what it posts to its parent. */
  const running = () => {
    const posted: unknown[] = [];
    const page: Record<string, unknown> = {};
    new Function("window", "document", "addEventListener", "parent", bridge())(page, {}, () => {}, {
      postMessage: (said: unknown) => posted.push(said),
    });
    return { posted, guaca: page.guaca as { answer: (value: unknown) => boolean } };
  };

  it("defines the call the prompt tells an agent to make", () => {
    expect(typeof running().guaca?.answer).toBe("function");
  });

  it("posts what the renderer is listening for, and the renderer draws it", async () => {
    const { posted, guaca } = running();
    expect(guaca.answer({ plan: "pro", seats: 12 })).toBe(true);
    expect(posted).toEqual([{ guaca: "artifact-answer", value: '{"plan":"pro","seats":12}' }]);

    // The same object, straight from the bridge into the component that reads
    // it. Neither side is retyped here, which is the whole point.
    const view = render(
      <Answering.Provider value={{ id: "agent-1", name: "Analyst" }}>
        <Markdown>
          {fence(
            "html",
            "<!doctype html><html><body><h1>Plan</h1><p>Worth framing.</p></body></html>",
          )}
        </Markdown>
      </Answering.Provider>,
    );
    const frame = await waitFor(() => {
      const found = view.container.querySelector("iframe");
      if (!found) throw new Error("no frame yet");
      return found as HTMLIFrameElement;
    });
    act(() => {
      window.dispatchEvent(
        new MessageEvent("message", { source: frame.contentWindow, data: posted[0] }),
      );
    });

    expect(screen.getByText('{"plan":"pro","seats":12}')).toBeTruthy();
  });

  it("fails in the page, where the page can be told, rather than in the app", () => {
    // A value that will not survive `JSON.stringify`. Arriving as something the
    // app has to decide about is strictly worse than the page getting a false.
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    const { posted, guaca } = running();

    expect(guaca.answer(cyclic)).toBe(false);
    expect(guaca.answer(() => {})).toBe(false);
    expect(guaca.answer(undefined)).toBe(false);
    expect(posted).toEqual([]);
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
