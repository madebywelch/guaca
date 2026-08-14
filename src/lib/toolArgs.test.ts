import { describe, expect, it } from "vitest";

import { sendBody, sendRecipients } from "./toolArgs";

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
