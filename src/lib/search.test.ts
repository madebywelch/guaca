import { describe, expect, it } from "vitest";

import { describeTrigger } from "./routine";
import { inScope, type SearchInput, score, searchResults, shortUrl } from "./search";
import type { AgentCard, Group, SearchHits } from "./types";

const NOW = 1_700_000_000_000;
const GROUP = "00000000-0000-4000-8000-000000000001";

function agent(name: string, extra: Partial<AgentCard> = {}): AgentCard {
  return {
    id: `id-${name}`,
    groupId: GROUP,
    computerId: null,
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "test/model",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 1,
    updatedAt: 1,
    ...extra,
  };
}

function group(name: string): Group {
  return {
    id: `group-${name}`,
    name,
    agentCount: 2,
    createdAt: 1,
    baseUrl: null,
    defaultModel: null,
    apiKeySet: false,
    apiKeyHint: "",
  };
}

const NOTHING: SearchHits = { messages: [], files: [], links: [], routines: [] };

function input(over: Partial<SearchInput> = {}): SearchInput {
  return {
    query: "",
    agents: [],
    groups: [],
    hits: NOTHING,
    lastActive: {},
    now: NOW,
    ...over,
  };
}

describe("score", () => {
  it("ranks by where the match falls, not by whether there is one", () => {
    // Somebody typing "man" wants Manager, not the message that says
    // "command". Both contain it; only one is what was meant.
    const name = score("Manager", "man");
    const inside = score("she ran the command herself", "man");
    expect(name).toBeGreaterThan(inside);
  });

  it("puts a whole name above a word inside a sentence", () => {
    expect(score("Manager", "manager")).toBeGreaterThan(score("ask the Manager first", "manager"));
  });

  it("finds a word that starts partway through", () => {
    expect(score("post the budget summary", "budget")).toBeGreaterThan(0);
    expect(score("post the budget summary", "budget")).toBeGreaterThan(
      score("unbudgeted spending", "budget"),
    );
  });

  it("is zero when there is no match at all", () => {
    expect(score("Manager", "chef")).toBe(0);
  });

  it("ignores case and surrounding space in the query", () => {
    expect(score("Manager", "  MANAGER  ")).toBe(score("Manager", "manager"));
  });
});

describe("searchResults", () => {
  it("puts every kind in one list, ranked together", () => {
    const hits: SearchHits = {
      messages: [
        {
          id: "m1",
          channelId: "id-Chef",
          from: { kind: "human" },
          to: { kind: "agent", id: "id-Chef" },
          excerpt: "the budget is signed off",
          createdAt: NOW - 1000,
        },
      ],
      files: [
        {
          file: { digest: "aaa", name: "budget.pdf", mime: "application/pdf", bytes: 2048 },
          messageId: "m2",
          channelId: "id-Chef",
          from: { kind: "human" },
          createdAt: NOW - 2000,
        },
      ],
      links: [
        {
          url: "https://example.com/budget",
          messageId: "m3",
          channelId: "id-Chef",
          createdAt: NOW - 3000,
        },
      ],
      routines: [
        {
          id: "r1",
          agentId: "id-Chef",
          name: "",
          what: "post the budget summary",
          trigger: "every:3600",
          active: true,
          nextRunAt: NOW + 1000,
          lastRunAt: null,
          createdAt: 1,
        },
      ],
    };

    const results = searchResults(
      input({ query: "budget", agents: [agent("Chef")], groups: [group("Budget")], hits }),
    );
    const kinds = new Set(results.map((r) => r.kind));
    // Six of the seven. No agent is called Budget, and "Budget settings" is,
    // so the action for the group is in here and the agent row is not.
    expect(kinds).toEqual(new Set(["messages", "files", "links", "routines", "groups", "actions"]));
  });

  it("ranks a name typed in full above the same word inside a message", () => {
    const hits: SearchHits = {
      ...NOTHING,
      messages: [
        {
          id: "m1",
          channelId: "id-Chef",
          from: { kind: "human" },
          to: { kind: "agent", id: "id-Chef" },
          // Newer than the agent has ever been active, so recency alone would
          // put this first. Scoring has to win that.
          excerpt: "ask the Manager about it",
          createdAt: NOW,
        },
      ],
    };
    const results = searchResults(
      input({ query: "manager", agents: [agent("Manager"), agent("Chef")], hits }),
    );
    expect(results[0]?.kind).toBe("agents");
    expect(results[0]?.title).toBe("Manager");
  });

  it("matches an agent on its skills and its prompt, not only its name", () => {
    const agents = [
      agent("Chef", { skills: ["pastry", "sourcing"] }),
      agent("Scribe", { systemPrompt: "You turn discussion into ordered notes." }),
    ];
    expect(searchResults(input({ query: "pastry", agents })).map((r) => r.title)).toContain("Chef");
    expect(searchResults(input({ query: "ordered notes", agents })).map((r) => r.title)).toContain(
      "Scribe",
    );
  });

  it("keeps a deleted agent out of the results but still uses its name", () => {
    // It is not somewhere you can go, and it is still who sent the message.
    const ghost = agent("Ghost", { lifecycle: "terminated" });
    const hits: SearchHits = {
      ...NOTHING,
      messages: [
        {
          id: "m1",
          channelId: "id-Ghost",
          from: { kind: "agent", id: "id-Ghost" },
          to: { kind: "human" },
          excerpt: "the last thing it said",
          createdAt: NOW,
        },
      ],
    };
    const results = searchResults(input({ query: "", agents: [ghost], hits }));

    expect(results.filter((r) => r.kind === "agents")).toEqual([]);
    expect(results.find((r) => r.kind === "messages")?.title).toBe("Ghost → You");
  });

  it("offers settings for every agent and group, plus the app's own", () => {
    const actions = inScope(
      searchResults(input({ agents: [agent("Chef")], groups: [group("Kitchen")] })),
      "actions",
    ).map((r) => r.title);

    expect(actions).toContain("Chef settings");
    expect(actions).toContain("Kitchen settings");
    expect(actions).toContain("App settings");
    expect(actions).toContain("New agent");
    expect(actions).toContain("New group");
  });

  it("opens an agent's channel, and its settings only from the action", () => {
    const results = searchResults(input({ query: "Chef", agents: [agent("Chef")] }));
    const row = results.find((r) => r.kind === "agents");
    const action = results.find((r) => r.kind === "actions");

    expect(row?.action).toEqual({ do: "openChannel", agentId: "id-Chef" });
    expect(action?.action).toEqual({ do: "editAgent", agentId: "id-Chef" });
  });

  it("has nothing that deletes, pauses or sends", () => {
    // A list you drive by typing and pressing Enter is the wrong place to keep
    // an action you cannot take back.
    const results = searchResults(input({ agents: [agent("Chef")], groups: [group("Kitchen")] }));
    const verbs = results.map((r) => r.action.do);
    expect(verbs.every((v) => !/delete|clear|pause|send|remove/i.test(v))).toBe(true);
  });

  it("shows the newest of everything before anybody types", () => {
    // The palette opens on an empty query, and an empty list there reads as a
    // workspace with nothing in it.
    const agents = [agent("Chef"), agent("Manager")];
    const results = searchResults(
      input({ query: "", agents, lastActive: { "id-Manager": NOW, "id-Chef": NOW - 5000 } }),
    );
    expect(inScope(results, "agents").map((r) => r.title)).toEqual(["Manager", "Chef"]);
  });

  it("keeps a store hit even when this side would score it zero", () => {
    // The store already decided it matched. Dropping it here would be the
    // search disagreeing with itself in front of the operator.
    const hits: SearchHits = {
      ...NOTHING,
      messages: [
        {
          id: "m1",
          channelId: "id-Chef",
          from: { kind: "human" },
          to: { kind: "agent", id: "id-Chef" },
          excerpt: "an excerpt that lost the match to its window",
          createdAt: NOW,
        },
      ],
    };
    const results = searchResults(input({ query: "elsewhere", agents: [agent("Chef")], hits }));
    expect(inScope(results, "messages")).toHaveLength(1);
  });

  it("titles and describes a routine the way the schedule panel does", () => {
    const hits: SearchHits = {
      ...NOTHING,
      routines: [
        {
          id: "r1",
          agentId: "id-Chef",
          name: "Watering",
          what: "water the plants",
          trigger: "every:7200",
          active: true,
          nextRunAt: NOW,
          lastRunAt: null,
          createdAt: 1,
        },
        {
          id: "r2",
          agentId: "id-Chef",
          name: "",
          what: "one off",
          trigger: "once",
          active: true,
          nextRunAt: NOW,
          lastRunAt: null,
          createdAt: 1,
        },
      ],
    };
    const rows = inScope(searchResults(input({ agents: [agent("Chef")], hits })), "routines");

    // A routine the operator named is titled by the name; one an agent set for
    // itself falls back to its instruction. Which field becomes the title is
    // this module's decision, so it is asserted literally.
    expect(rows.map((r) => r.title).sort()).toEqual(["Watering", "one off"]);

    // The wording is not: it comes from the same helpers the schedule panel
    // uses, and a palette that phrased a cadence its own way would be a second
    // place to fix when that phrasing changes.
    expect(rows.find((r) => r.title === "Watering")?.meta).toBe(describeTrigger("every:7200", NOW));
    expect(rows.find((r) => r.title === "one off")?.meta).toBe(describeTrigger("once", NOW));
    expect(rows[0]?.detail).toBe("Chef");

    // The schedule lives in the panel beside the conversation now, so a hit
    // opens the channel rather than the profile dialog it used to be in.
    expect(rows[0]?.action).toEqual({ do: "openChannel", agentId: "id-Chef" });
  });

  it("gives every result a key of its own", () => {
    // Two rows sharing a key make React reuse one of them, and the palette
    // draws a file where a message should be.
    const hits: SearchHits = {
      ...NOTHING,
      links: [
        { url: "https://a.example", messageId: "m1", channelId: "id-Chef", createdAt: NOW },
        { url: "https://b.example", messageId: "m1", channelId: "id-Chef", createdAt: NOW },
      ],
    };
    const results = searchResults(
      input({ agents: [agent("Chef")], groups: [group("Kitchen")], hits }),
    );
    expect(new Set(results.map((r) => r.key)).size).toBe(results.length);
  });
});

describe("inScope", () => {
  it("shows everything under all, and one kind under a tab", () => {
    const results = searchResults(input({ agents: [agent("Chef")], groups: [group("Kitchen")] }));
    expect(inScope(results, "all")).toHaveLength(results.length);
    expect(inScope(results, "groups").every((r) => r.kind === "groups")).toBe(true);
    expect(inScope(results, "files")).toEqual([]);
  });
});

describe("shortUrl", () => {
  it("drops what nobody is choosing between", () => {
    expect(shortUrl("https://www.example.com/report/")).toBe("example.com/report");
    expect(shortUrl("http://example.com")).toBe("example.com");
  });
});
