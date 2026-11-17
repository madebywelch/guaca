import { describe, expect, it } from "vitest";

import { type Evidence, ROLES, roleFor } from "./roles";

function agent(over: Partial<Evidence> = {}): Evidence {
  return { name: "", skills: [], instructions: "", ...over };
}

describe("what an agent reads as", () => {
  it("takes the name when the name says it", () => {
    expect(roleFor(agent({ name: "Legal" }))?.id).toBe("legal");
    expect(roleFor(agent({ name: "Marketing" }))?.id).toBe("marketing");
    expect(roleFor(agent({ name: "Translator" }))?.id).toBe("translation");
  });

  it("reads a name that is two words", () => {
    expect(roleFor(agent({ name: "Legal Counsel" }))?.id).toBe("legal");
    expect(roleFor(agent({ name: "Ada the Accountant" }))?.id).toBe("finance");
  });

  it.each([
    ["a flawless outcome", "law"],
    ["order a new device", "dev"],
    ["rapid replies", "api"],
    ["mow the lawn", "law"],
    ["a companionable tone", "companion"],
  ])("does not find a role inside another word: %s", (instructions) => {
    expect(roleFor(agent({ instructions }))).toBeUndefined();
  });

  it("takes the skills when the name says nothing", () => {
    const found = roleFor(agent({ name: "Ada", skills: ["contract review", "compliance"] }));
    expect(found?.id).toBe("legal");
  });

  it("falls back to the instructions", () => {
    const found = roleFor(
      agent({
        name: "Ada",
        instructions:
          "Draft the campaign copy for each launch and keep the brand voice consistent.",
      }),
    );
    expect(found?.id).toBe("marketing");
  });

  it("wants two words of prose, not one", () => {
    expect(roleFor(agent({ instructions: "File the invoice." }))).toBeUndefined();
    expect(roleFor(agent({ instructions: "File the invoice and update the budget." }))?.id).toBe(
      "finance",
    );
  });

  it.each([
    ["Manager", "You coordinate the other agents. Delegate rather than doing work yourself."],
    ["Inbox", "Read what arrives and pass it to whoever should see it."],
    ["Scout", ""],
    ["", ""],
  ])("says nothing about %s", (name, instructions) => {
    expect(roleFor(agent({ name, instructions }))).toBeUndefined();
  });

  it("says nothing when two use cases are level", () => {
    expect(roleFor(agent({ name: "Legal Finance" }))).toBeUndefined();
    expect(roleFor(agent({ skills: ["contracts", "invoicing"] }))).toBeUndefined();
  });

  it("breaks the tie when one side has more of the evidence", () => {
    const found = roleFor(agent({ name: "Legal Finance", skills: ["litigation"] }));
    expect(found?.id).toBe("legal");
  });

  it("sends sales to marketing, which is where the models are", () => {
    expect(roleFor(agent({ name: "Sales" }))?.id).toBe("marketing");
    expect(roleFor(agent({ name: "Ada", skills: ["prospecting", "cold email"] }))?.id).toBe(
      "marketing",
    );
  });

  it("tells SEO apart from the rest of marketing", () => {
    expect(roleFor(agent({ name: "Ada", skills: ["keyword research", "backlinks"] }))?.id).toBe(
      "marketing/seo",
    );
  });

  it("reads punctuation and case as nothing", () => {
    expect(roleFor(agent({ name: "LEGAL" }))?.id).toBe("legal");
    expect(roleFor(agent({ skills: ["contracts,", "  NDAs  "] }))?.id).toBe("legal");
  });
});

describe("the twelve", () => {
  it("names each use case once", () => {
    expect(new Set(ROLES.map((role) => role.id)).size).toBe(ROLES.length);
  });

  it("gives every use case something to be called in a sentence", () => {
    for (const role of ROLES) expect(role.label.trim().length).toBeGreaterThan(0);
  });

  it.each(ROLES.map((role) => [role.id, role.label]))("can reach %s", (id, label) => {
    expect(roleFor(agent({ name: label }))?.id ?? roleFor(agent({ name: id }))?.id).toBe(id);
  });
});
