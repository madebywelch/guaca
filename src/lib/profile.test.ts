import { describe, expect, it } from "vitest";

import { modelName, runsOn } from "./profile";
import type { AgentCard, Group, Settings } from "./types";

const agent = (over: Partial<AgentCard> = {}): AgentCard =>
  ({
    id: "a1",
    groupId: "g1",
    name: "Cook",
    model: "",
    subscriptionModel: "",
    ...over,
  }) as AgentCard;

const group = (over: Partial<Group["inference"]> = {}, name = "Kitchen"): Group =>
  ({
    id: "g1",
    name,
    inference: {
      provider: null,
      baseUrl: null,
      defaultModel: null,
      subscriptionModel: null,
      ...over,
    },
  }) as Group;

const settings = (over: Partial<Settings> = {}): Settings =>
  ({
    provider: "compatible",
    defaultModel: "openai/gpt-5",
    subscriptionModel: "gpt-5.6-luna",
    ...over,
  }) as Settings;

describe("runsOn", () => {
  it("names the agent's own model over everything", () => {
    expect(runsOn(agent({ model: "anthropic/claude-sonnet-4.5" }), group(), settings())).toEqual({
      crew: "Kitchen",
      model: "anthropic/claude-sonnet-4.5",
      inherited: false,
    });
  });

  it("falls back to the crew, then the app, and says it did", () => {
    expect(runsOn(agent(), group({ defaultModel: "crew/model" }), settings()).model).toBe(
      "crew/model",
    );
    expect(runsOn(agent(), group(), settings())).toEqual({
      crew: "Kitchen",
      model: "openai/gpt-5",
      inherited: true,
    });
  });

  it("reads the subscription field when ChatGPT pays, never the key's model", () => {
    const card = agent({ model: "openai/gpt-5", subscriptionModel: "" });
    expect(runsOn(card, group({ provider: "chatgpt" }), settings()).model).toBe("gpt-5.6-luna");
    expect(
      runsOn(
        agent({ subscriptionModel: "gpt-5.4-mini" }),
        group({ provider: "chatgpt" }),
        settings(),
      ),
    ).toMatchObject({ model: "gpt-5.4-mini", inherited: false });
  });

  it("names a Claude subscription rather than a model, because it chooses none", () => {
    expect(runsOn(agent({ model: "x" }), group({ provider: "claude" }), settings())).toEqual({
      crew: "Kitchen",
      model: "Claude subscription",
      inherited: true,
    });
  });

  it("says when nothing anywhere names a model", () => {
    expect(runsOn(agent(), group(), settings({ defaultModel: "" }))).toMatchObject({
      model: "No model set",
      inherited: true,
    });
  });

  it("stands an agent with no crew on its own", () => {
    expect(runsOn(agent(), undefined, null).crew).toBe("Independent agent");
  });
});

describe("modelName", () => {
  it("keeps an id it cannot read a name from, rather than showing nothing", () => {
    expect(modelName("vendor/")).toBe("vendor/");
    expect(modelName("  ")).toBe("");
  });

  it("drops the vendor and a variant suffix, and spells the words a person reads", () => {
    expect(modelName("anthropic/claude-sonnet-4.5")).toBe("Claude Sonnet 4.5");
    expect(modelName("openai/gpt-4o-mini")).toBe("GPT-4o Mini");
    expect(modelName("gpt-5.6-luna")).toBe("GPT-5.6 Luna");
    expect(modelName("meta-llama/llama-3.1-70b-instruct:free")).toBe("Llama 3.1 70B Instruct");
    expect(modelName("openai/o4-mini")).toBe("o4 Mini");
    expect(modelName("verify/stand-in")).toBe("Stand In");
    expect(modelName("claude-sonnet-4-5-20250929")).toBe("Claude Sonnet 4.5");
    expect(modelName("claude-3-5-haiku")).toBe("Claude 3.5 Haiku");
    expect(modelName("qwen-2-72b")).toBe("Qwen 2 72B");
  });
});
