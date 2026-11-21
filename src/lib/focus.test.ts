import { afterEach, describe, expect, it } from "vitest";
import { focusQuestion } from "./focus";

afterEach(() => {
  document.body.replaceChildren();
});

function draw(html: string) {
  const root = document.createElement("div");
  root.innerHTML = html;
  document.body.append(root);
  return root;
}

describe("focusing a waiting question where it is drawn", () => {
  it("reports that it found nothing when the question is not drawn", () => {
    draw('<div data-question="other"><button>Yes</button></div>');
    expect(focusQuestion("asked")).toBe(false);
    expect(document.activeElement).toBe(document.body);
  });

  it("skips a copy inside a hidden branch", () => {
    draw('<li hidden><div data-question="asked" tabindex="-1"><button>Yes</button></div></li>');
    expect(focusQuestion("asked")).toBe(false);
  });

  it("skips a card whose answers are all disabled", () => {
    draw('<div data-question="asked"><button disabled>Yes</button></div>');
    expect(focusQuestion("asked")).toBe(false);
  });

  it("focuses the map's question itself, where a number answers it", () => {
    const root = draw('<div data-question="asked" tabindex="-1"><button>Yes</button></div>');
    expect(focusQuestion("asked")).toBe(true);
    expect(document.activeElement).toBe(root.querySelector("[data-question]"));
  });

  it("focuses a card's first answer", () => {
    const root = draw(
      '<div data-question="asked"><button disabled>Wait</button><button>Yes</button></div>',
    );
    expect(focusQuestion("asked", root)).toBe(true);
    expect(document.activeElement?.textContent).toBe("Yes");
  });
});
