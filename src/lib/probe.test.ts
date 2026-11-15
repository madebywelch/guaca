import { afterEach, describe, expect, it, vi } from "vitest";

import { findAll, nameOf, roleOf, run } from "./probe";

function page(html: string): Document {
  document.body.innerHTML = html;
  return document;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("roles and names, as a screen reader would read them", () => {
  it("takes an explicit role over the element's own", () => {
    const doc = page(`<div role="switch tab" aria-label="Failures">On</div><button>Save</button>`);
    expect(roleOf(doc.querySelector("div") as Element)).toBe("switch");
    expect(roleOf(doc.querySelector("button") as Element)).toBe("button");
  });

  it("names a field by its label, then its placeholder", () => {
    const doc = page(
      `<label for="u">Address</label><input id="u" /><input placeholder="Token" /><input type="hidden" />`,
    );
    expect(findAll(doc, "textbox").map(nameOf)).toEqual(["Address", "Token"]);
  });

  it("prefers aria-labelledby, then aria-label, then the text inside", () => {
    const doc = page(
      `<span id="t">Open Guaca</span><button aria-labelledby="t">x</button><button aria-label="App settings"><span>Settings</span></button><a href="#">  For   you </a>`,
    );
    expect(findAll(doc, "button").map(nameOf)).toEqual(["Open Guaca", "App settings"]);
    expect(findAll(doc, "link").map(nameOf)).toEqual(["For you"]);
  });

  it("leaves out what a person cannot see", () => {
    const doc = page(
      `<button>Shown</button><button hidden>Hidden</button><div aria-hidden="true"><button>Behind</button></div><div style="display:none"><button>Gone</button></div>`,
    );
    expect(findAll(doc, "button").map(nameOf)).toEqual(["Shown"]);
  });
});

describe("a call answered", () => {
  it("presses the one button with that name", () => {
    const doc = page(`<button>Send a test notification</button><button>Other</button>`);
    const pressed = vi.fn();
    doc.querySelector("button")?.addEventListener("click", pressed);
    expect(run({ do: "click", role: "button", name: "Send a test notification" }, doc)).toEqual({
      ok: true,
      value: null,
    });
    expect(pressed).toHaveBeenCalledOnce();
  });

  it("says what is shown when nothing matches, and how many when several do", () => {
    const doc = page(`<button>Save</button><button>Save</button><button>Close</button>`);
    expect(run({ do: "click", role: "button", name: "Send" }, doc)).toEqual({
      ok: false,
      error: 'no button named "Send" is shown; shown: "Save", "Save", "Close"',
    });
    expect(run({ do: "click", role: "button", name: "Save" }, doc)).toEqual({
      ok: false,
      error: '2 of the buttons shown are named "Save"',
    });
  });

  it("refuses a disabled control, as a pointer would", () => {
    const doc = page(`<button disabled>Connect</button>`);
    const answer = run({ do: "click", role: "button", name: "Connect" }, doc);
    expect(answer).toEqual({ ok: false, error: '"Connect" is disabled' });
  });

  it("types through the value setter, so a controlled field hears it", () => {
    const doc = page(`<label>Address <input /></label>`);
    const input = doc.querySelector("input") as HTMLInputElement;
    const heard: string[] = [];
    input.addEventListener("input", () => heard.push(input.value));
    expect(
      run({ do: "type", role: "textbox", name: "Address", text: "http://127.0.0.1:9" }, doc),
    ).toEqual({
      ok: true,
      value: null,
    });
    expect(heard).toEqual(["http://127.0.0.1:9"]);
    expect(doc.activeElement).toBe(input);
  });

  it("reads the text of one region, or of the page", () => {
    const doc = page(
      `<section role="region" aria-label="Panel"><h2>3 working</h2><p>Manager</p></section><p>Elsewhere</p>`,
    );
    expect(run({ do: "text", role: "region", name: "Panel" }, doc)).toEqual({
      ok: true,
      value: "3 workingManager",
    });
    const all = run({ do: "text" }, doc);
    expect(all.ok && String(all.value)).toContain("Elsewhere");
  });

  it("counts what it finds without pressing anything", () => {
    const doc = page(`<div role="tab">General</div><div role="tab">Notifications</div>`);
    expect(run({ do: "find", role: "tab" }, doc)).toEqual({
      ok: true,
      value: { count: 2, names: ["General", "Notifications"] },
    });
  });

  it("answers a call it does not know instead of throwing", () => {
    const answer = run({ do: "format" } as never, page(""));
    expect(answer.ok).toBe(false);
  });
});
