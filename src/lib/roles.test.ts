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

  // A name is a word, not a sentence, so the second half of it carries as much
  // as the first. "Counsel" alone is the whole answer.
  it("reads a name that is two words", () => {
    expect(roleFor(agent({ name: "Legal Counsel" }))?.id).toBe("legal");
    expect(roleFor(agent({ name: "Ada the Accountant" }))?.id).toBe("finance");
  });

  // Whole words. A role picked out of the middle of another word is the failure
  // mode of every substring matcher, and the words here are short enough for it:
  // "law" is inside "flawless", "dev" inside "device", "api" inside "rapid".
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

  // The case the whole thing exists for: the operator wrote a paragraph about
  // the job and never wrote the word in the name.
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

  // Prose mentions things in passing. One word of it is not a role, or every
  // agent whose instructions say "keep a record of the invoice" is an accountant.
  it("wants two words of prose, not one", () => {
    expect(roleFor(agent({ instructions: "File the invoice." }))).toBeUndefined();
    expect(roleFor(agent({ instructions: "File the invoice and update the budget." }))?.id).toBe(
      "finance",
    );
  });

  // The common answer. Most agents are a Manager, a Router or an Inbox, and
  // OpenRouter has no category for any of them.
  it.each([
    ["Manager", "You coordinate the other agents. Delegate rather than doing work yourself."],
    ["Inbox", "Read what arrives and pass it to whoever should see it."],
    ["Scout", ""],
    ["", ""],
  ])("says nothing about %s", (name, instructions) => {
    expect(roleFor(agent({ name, instructions }))).toBeUndefined();
  });

  // Evidence pointing two ways is not a coin to toss. An agent that is equally
  // legal and financial genuinely has no single best model, and the operator
  // reads a confident wrong answer as a reason to distrust the right ones.
  it("says nothing when two use cases are level", () => {
    expect(roleFor(agent({ name: "Legal Finance" }))).toBeUndefined();
    expect(roleFor(agent({ skills: ["contracts", "invoicing"] }))).toBeUndefined();
  });

  it("breaks the tie when one side has more of the evidence", () => {
    const found = roleFor(agent({ name: "Legal Finance", skills: ["litigation"] }));
    expect(found?.id).toBe("legal");
  });

  // OpenRouter ranks no models for sales, and an agent called Sales is the
  // second thing anybody builds here. Marketing is the nearest thing that
  // exists; nothing at all reads as broken.
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

  // The label only ever appears in "this reads as ___ work", so an empty one
  // renders a sentence with a hole in it.
  it("gives every use case something to be called in a sentence", () => {
    for (const role of ROLES) expect(role.label.trim().length).toBeGreaterThan(0);
  });

  // Every use case has to be reachable, or it is a category the app can never
  // suggest and a list entry nothing reads.
  it.each(ROLES.map((role) => [role.id, role.label]))("can reach %s", (id, label) => {
    // The label is not always the word: "academic" for academia, "SEO" for
    // marketing/seo. Both are in their own vocabulary, so both find themselves.
    expect(roleFor(agent({ name: label }))?.id ?? roleFor(agent({ name: id }))?.id).toBe(id);
  });
});
