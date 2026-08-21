import { describe, expect, it } from "vitest";

import { attachedNames, sendBody, sendRecipients } from "./toolArgs";

describe("sendRecipients", () => {
  it("reads the specified array shape", () => {
    expect(sendRecipients({ to: ["Chef", "Host"] })).toEqual(["Chef", "Host"]);
  });

  it("splits a comma separated string", () => {
    expect(sendRecipients({ to: "Chef, Host ,Scribe" })).toEqual(["Chef", "Host", "Scribe"]);
  });

  it("unwraps recipient objects", () => {
    expect(sendRecipients({ to: [{ name: "Chef" }, { agent: "Host" }] })).toEqual(["Chef", "Host"]);
  });

  it("accepts the singular agent alias", () => {
    expect(sendRecipients({ agent: "Chef" })).toEqual(["Chef"]);
  });

  it("returns nothing for shapes it cannot read", () => {
    // Arguments come from a model, so this has to survive anything.
    for (const junk of [null, undefined, 42, "plain", ["x"], { to: 7 }, { to: [1, 2] }]) {
      expect(sendRecipients(junk)).toEqual([]);
    }
  });
});

describe("sendBody", () => {
  it("reads text, and the message alias", () => {
    expect(sendBody({ text: "hello" })).toBe("hello");
    expect(sendBody({ message: "hello" })).toBe("hello");
  });

  it("prefers text over the alias", () => {
    expect(sendBody({ text: "real", message: "alias" })).toBe("real");
  });

  it("returns an empty string when there is nothing to show", () => {
    for (const junk of [null, undefined, 42, "plain", { text: 7 }]) {
      expect(sendBody(junk)).toBe("");
    }
  });
});

describe("attachedNames", () => {
  it("reads the specified shape, and the aliases a model reaches for", () => {
    // The same set `tools.rs` accepts, because both sides read the same call.
    expect(attachedNames({ files: ["brief.md"] })).toEqual(["brief.md"]);
    expect(attachedNames({ attachments: ["brief.md"] })).toEqual(["brief.md"]);
    expect(attachedNames({ paths: ["brief.md"] })).toEqual(["brief.md"]);
    expect(attachedNames({ path: "brief.md" })).toEqual(["brief.md"]);
    expect(attachedNames({ file: "brief.md" })).toEqual(["brief.md"]);
    expect(attachedNames({ files: [{ path: "brief.md" }] })).toEqual(["brief.md"]);
  });

  it("keeps the file and drops the machine it was on", () => {
    // What the model passes is a path on its own computer, and the directory is
    // the one fact about it the operator has no use for: they cannot reach that
    // disk, which is why the file was attached instead of named.
    expect(attachedNames({ files: ["/home/user/work/exec-brief.md"] })).toEqual(["exec-brief.md"]);
    expect(attachedNames({ files: ["C:\\Users\\bob\\notes.txt"] })).toEqual(["notes.txt"]);
    // A name that was never a path is already the answer.
    expect(attachedNames({ files: ["brief.md"] })).toEqual(["brief.md"]);
  });

  it("returns nothing for shapes it cannot read", () => {
    for (const junk of [null, undefined, 42, "plain", { files: 7 }, { files: [1, 2] }]) {
      expect(attachedNames(junk)).toEqual([]);
    }
  });
});
