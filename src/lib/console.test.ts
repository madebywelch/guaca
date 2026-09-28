import { describe, expect, it } from "vitest";

import { epitaph, heard, resize, stateWord } from "./console";

describe("what the host says on a console socket", () => {
  // The literals `server/console.rs` asserts it sends. A rename on either
  // side is a terminal that never says its shell ended.
  it("reads an exit, with the code or without one", () => {
    expect(heard('{"type":"exit","code":7}')).toEqual({ type: "exit", code: 7 });
    expect(heard('{"type":"exit","code":null}')).toEqual({ type: "exit", code: null });
  });

  it("reads a refusal", () => {
    expect(heard('{"type":"refused","message":"no"}')).toEqual({ type: "refused", message: "no" });
  });

  it("drops what this build has no word for, rather than ending the shell", () => {
    expect(heard('{"type":"bell"}')).toBeNull();
    expect(heard("not json")).toBeNull();
    expect(heard("null")).toBeNull();
  });

  it("does not take a code that is not a number for one", () => {
    expect(heard('{"type":"exit","code":"7"}')).toEqual({ type: "exit", code: null });
  });
});

describe("what the page says", () => {
  it("sends a resize as the host reads one", () => {
    expect(JSON.parse(resize(120, 40))).toEqual({ type: "resize", cols: 120, rows: 40 });
  });
});

describe("how the shell ending is said", () => {
  it("says nothing while it runs", () => {
    expect(epitaph({ kind: "opening" })).toBeNull();
    expect(epitaph({ kind: "open" })).toBeNull();
  });

  it("tells an exit from a stop, and says how to get another", () => {
    expect(epitaph({ kind: "exited", code: 0 })).toBe(
      "The shell exited. Open the terminal again for a new one.",
    );
    expect(epitaph({ kind: "exited", code: 7 })).toMatch(/^The shell exited with code 7\./);
    expect(epitaph({ kind: "exited", code: null })).toMatch(/^The shell was stopped\./);
  });

  it("says the host's refusal as a sentence", () => {
    expect(
      epitaph({
        kind: "refused",
        message:
          "Engineer has no terminal. Give it one from the Terminal section of its panel first",
      }),
    ).toBe("Engineer has no terminal. Give it one from the Terminal section of its panel first.");
    expect(epitaph({ kind: "refused", message: "that agent is gone." })).toBe(
      "That agent is gone.",
    );
  });

  it("says a dropped connection ended the shell, since the host ends it", () => {
    expect(epitaph({ kind: "dropped" })).toMatch(/dropped, which ended this shell/);
  });

  it("names three states in the bar", () => {
    expect(stateWord({ kind: "opening" })).toBe("starting");
    expect(stateWord({ kind: "open" })).toBe("running");
    expect(stateWord({ kind: "refused", message: "" })).toBe("ended");
  });
});
