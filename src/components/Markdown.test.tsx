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
    expect(() => render(<Markdown>{"here is a list\n\n- one\n- tw"}</Markdown>)).not.toThrow();
    expect(() => render(<Markdown>{"```rust\nfn main("}</Markdown>)).not.toThrow();
    expect(() => render(<Markdown>{"**unclosed"}</Markdown>)).not.toThrow();
  });

  it("renders an empty body without crashing", () => {
    expect(() => render(<Markdown>{""}</Markdown>)).not.toThrow();
  });
});

describe("a callout in a body", () => {
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
    const container = drawn("> [!WARNING]\n> This deletes the production bucket.");

    expect(container.querySelector("blockquote")).toBeNull();
    expect(container.querySelector("div.callout")).toBeTruthy();
  });

  it("holds the prose a box is worth having: a list, a link, a name", () => {
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
    for (const body of ["> [!IMPO", "> [!IMPORTANT]", "> [!IMPORTANT]\n> "]) {
      expect(() => drawn(body)).not.toThrow();
    }
    expect(drawn("> [!IMPORTANT]").querySelector(".callout--asks")).toBeTruthy();
  });
});

describe("a mention in a body", () => {
  const NAMES = ["Critic", "Head Chef"];

  function drawn(body: string) {
    return render(
      <Roster.Provider value={NAMES}>
        <Markdown>{body}</Markdown>
      </Roster.Provider>,
    ).container;
  }

  it("is drawn as one thing, wherever in the document it lands", () => {
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
    const container = drawn("try @lunch, or mail bob@example.com, or @Critical thinking");
    expect(container.querySelectorAll(".mention")).toHaveLength(0);
  });

  it("leaves code alone, where an @ is a decorator", () => {
    const container = drawn("`@Critic` and\n\n```py\n@Critic\ndef go(): ...\n```");
    expect(container.querySelectorAll(".mention")).toHaveLength(0);
  });

  it("leaves a link's own words alone", () => {
    const container = drawn("[@Critic](https://example.com) and bob@example.com");
    expect(container.querySelectorAll(".mention")).toHaveLength(0);
    expect(container.querySelectorAll("a")).toHaveLength(2);
  });

  it("draws the prose the model wrote when no roster was provided", () => {
    const { container } = render(<Markdown>{"ask @Critic"}</Markdown>);
    expect(container.querySelectorAll(".mention")).toHaveLength(0);
    expect(container.textContent).toBe("ask @Critic");
  });

  it("keeps the words either side of it", () => {
    const container = drawn("ask @Critic to review");
    expect(container.textContent).toBe("ask @Critic to review");
  });
});
