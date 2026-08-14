import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const openExternal = vi.fn<(url: string) => Promise<void>>(async () => {});
vi.mock("../lib/ipc", () => ({ openExternal: (url: string) => openExternal(url) }));

const { Markdown } = await import("./Markdown");

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
