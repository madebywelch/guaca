import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const openExternal = vi.fn<(url: string) => Promise<void>>(async () => {});
vi.mock("../lib/ipc", () => ({ openExternal: (url: string) => openExternal(url) }));

const { Markdown, Roster } = await import("./Markdown");

describe("Markdown", () => {
  it("renders the formatting models actually emit", () => {
    const { container } = render(
      <Markdown>{"**bold** and `code`\n\n- one\n- two\n\n# Heading"}</Markdown>,
    );
    expect(container.querySelector("strong")?.textContent).toBe("bold");
    expect(container.querySelector("code")?.textContent).toBe("code");
    expect(container.querySelectorAll("li")).toHaveLength(2);
    expect(container.querySelector("h1")?.textContent).toBe("Heading");
  });

  it("renders GitHub tables and strikethrough", () => {
    const { container } = render(
      <Markdown>{"| a | b |\n| - | - |\n| 1 | 2 |\n\n~~gone~~"}</Markdown>,
    );
    expect(container.querySelector("table")).toBeTruthy();
    expect(container.querySelector("del")?.textContent).toBe("gone");
  });

  it("wraps tables so a wide one scrolls instead of stretching the pane", () => {
    const { container } = render(<Markdown>{"| a | b |\n| - | - |\n| 1 | 2 |"}</Markdown>);
    expect(container.querySelector(".md__scroll table")).toBeTruthy();
  });

  it("never renders embedded HTML", () => {
    // Message bodies come from a model, and on the peer path from another
    // agent's model. Raw HTML would make that the least trustworthy input in
    // the app into markup.
    const { container } = render(
      <Markdown>{'<img src=x onerror="alert(1)"><script>alert(2)</script>**safe**'}</Markdown>,
    );
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("strong")?.textContent).toBe("safe");
  });

  it("opens links in the system browser rather than navigating the app", () => {
    render(<Markdown>{"[docs](https://example.com/x)"}</Markdown>);
    screen.getByText("docs").click();
    expect(openExternal).toHaveBeenCalledWith("https://example.com/x");
  });

  it("renders a code fence as a scrollable block", () => {
    const { container } = render(<Markdown>{"```rust\nfn main() {}\n```"}</Markdown>);
    expect(container.querySelector(".md__pre code")?.textContent).toContain("fn main()");
  });

  it("handles the partial markdown a stream produces", () => {
    // Mid-stream text routinely has an unclosed fence or emphasis. It must
    // still render rather than throwing and blanking the message.
    expect(() => render(<Markdown>{"here is a list\n\n- one\n- tw"}</Markdown>)).not.toThrow();
    expect(() => render(<Markdown>{"```rust\nfn main("}</Markdown>)).not.toThrow();
    expect(() => render(<Markdown>{"**unclosed"}</Markdown>)).not.toThrow();
  });

  it("renders an empty body without crashing", () => {
    expect(() => render(<Markdown>{""}</Markdown>)).not.toThrow();
  });
});

describe("a callout in a body", () => {
  /** The markup a marker actually produces, which is what the CSS hangs off. */
  function drawn(body: string) {
    return render(<Markdown>{body}</Markdown>).container;
  }

  it("draws the box the operator is meant to land on first", () => {
    const container = drawn("> [!IMPORTANT]\n> Rotate the staging key before Friday.");

    const box = container.querySelector(".callout--asks");
    expect(box).toBeTruthy();
    expect(box?.querySelector(".callout__label")?.textContent).toBe("Needs you");
    expect(box?.textContent).toContain("Rotate the staging key before Friday.");
  });

  it("draws the quiet box for a marker that needs nobody", () => {
    const container = drawn("> [!NOTE]\n> The rebuild took nine minutes.");

    expect(container.querySelector(".callout--aside")).toBeTruthy();
    expect(container.querySelector(".callout__label")?.textContent).toBe("Note");
  });

  it("stops being a quote, so no rule written for one reaches it", () => {
    // `.md blockquote` is a rule down the left and a muted color, which is the
    // opposite of a box. The element is what decides, not a class beside it.
    const container = drawn("> [!WARNING]\n> This deletes the production bucket.");

    expect(container.querySelector("blockquote")).toBeNull();
    expect(container.querySelector("div.callout")).toBeTruthy();
  });

  it("holds the prose a box is worth having: a list, a link, a name", () => {
    // The whole reason this is a quote and not a fence. A fence holds text.
    const container = render(
      <Roster.Provider value={["Chef"]}>
        <Markdown>
          {"> [!IMPORTANT]\n> Two things:\n>\n> - ask @Chef\n> - read [the note](https://x.test)"}
        </Markdown>
      </Roster.Provider>,
    ).container;

    expect(container.querySelectorAll(".callout li")).toHaveLength(2);
    expect(container.querySelector(".callout .mention")?.textContent).toBe("@Chef");
    expect(container.querySelector(".callout a")?.textContent).toBe("the note");
  });

  it("leaves no blank line where the marker was", () => {
    // The marker owns its line, so stripping it leaves an empty paragraph
    // behind: a gap between the label and the first thing said.
    const container = drawn("> [!NOTE]\n> One line.");

    const paragraphs = [...container.querySelectorAll(".callout p")];
    expect(paragraphs.map((p) => p.textContent)).toEqual(["Note", "One line."]);
  });

  it("leaves a quote a quote when the marker is not one this app draws", () => {
    const container = drawn("> [!DANGER]\n> Mind the step.");

    expect(container.querySelector(".callout")).toBeNull();
    expect(container.querySelector("blockquote")?.textContent).toContain("[!DANGER]");
  });

  it("draws a plain quote as a quote", () => {
    const container = drawn("> They said it shipped.");

    expect(container.querySelector("blockquote")?.textContent).toContain("They said it shipped.");
    expect(container.querySelector(".callout")).toBeNull();
  });

  it("survives the half-written marker a stream produces", () => {
    // Mid-reply the marker is `[!IMPO`, which is a quote, and a token later it
    // is a box. Neither may throw and blank the message.
    for (const body of ["> [!IMPO", "> [!IMPORTANT]", "> [!IMPORTANT]\n> "]) {
      expect(() => drawn(body)).not.toThrow();
    }
    expect(drawn("> [!IMPORTANT]").querySelector(".callout--asks")).toBeTruthy();
  });
});

describe("a mention in a body", () => {
  const NAMES = ["Critic", "Head Chef"];

  /** A body drawn with a roster behind it, which is what the app provides. */
  function drawn(body: string) {
    return render(
      <Roster.Provider value={NAMES}>
        <Markdown>{body}</Markdown>
      </Roster.Provider>,
    ).container;
  }

  it("is drawn as one thing, wherever in the document it lands", () => {
    // A tree walk rather than a pass over the prose: a mention turns up inside
    // a bold run, a heading, a list item and a table cell, and the tree is the
    // one place all four are the same node.
    const container = drawn("# @Critic\n\n- **@Head Chef** cooks\n\n| who |\n| - |\n| @Critic |");
    const chips = [...container.querySelectorAll(".mention")];

    expect(chips.map((chip) => chip.getAttribute("data-mention"))).toEqual([
      "Critic",
      "Head Chef",
      "Critic",
    ]);
    expect(container.querySelector("h1 .mention")).toBeTruthy();
    expect(container.querySelector("strong .mention")).toBeTruthy();
    expect(container.querySelector("td .mention")).toBeTruthy();
  });

  it("names nobody the roster does not have", () => {
    // Which is most of them. A model writes `@` in front of a flag, a handle
    // and an email address, and a chip on one of those says this app knows who
    // that is.
    const container = drawn("try @lunch, or mail bob@example.com, or @Critical thinking");
    expect(container.querySelectorAll(".mention")).toHaveLength(0);
  });

  it("leaves code alone, where an @ is a decorator", () => {
    const container = drawn("`@Critic` and\n\n```py\n@Critic\ndef go(): ...\n```");
    expect(container.querySelectorAll(".mention")).toHaveLength(0);
  });

  it("leaves a link's own words alone", () => {
    // A chip inside an anchor is two things claiming one click, and an
    // autolinked address is the one place an @ is certainly not a mention.
    const container = drawn("[@Critic](https://example.com) and bob@example.com");
    expect(container.querySelectorAll(".mention")).toHaveLength(0);
    expect(container.querySelectorAll("a")).toHaveLength(2);
  });

  it("draws the prose the model wrote when no roster was provided", () => {
    // The default, and every surface that has not opted in gets it: nothing
    // resolves, so nothing is claimed.
    const { container } = render(<Markdown>{"ask @Critic"}</Markdown>);
    expect(container.querySelectorAll(".mention")).toHaveLength(0);
    expect(container.textContent).toBe("ask @Critic");
  });

  it("keeps the words either side of it", () => {
    const container = drawn("ask @Critic to review");
    expect(container.textContent).toBe("ask @Critic to review");
  });
});
