import { describe, expect, it } from "vitest";

import { anAgent, anApproval, anOutcome } from "../test-fixtures";
import {
  BAND_LABEL,
  layoutRibbon,
  type Ribbon,
  type RibbonFace,
  type RibbonInput,
  spread,
  xAt,
} from "./ribbon";
import type { AgentCard, Occasion, UpcomingRun } from "./types";

const at = (day: number, hour: number, minute = 0) =>
  new Date(2026, 10, day, hour, minute).getTime();
const NOW = at(16, 9, 12);
const SPAN = 0.05;

const AGENTS = ["dara", "marlow", "ines", "tomas", "noor", "kit", "wren", "pip"].map((id) =>
  anAgent(id, id === "tomas" ? { name: "Tomás" } : {}),
);

function run(over: Partial<UpcomingRun> & Pick<UpcomingRun, "agentId" | "name" | "nextRunAt">) {
  return { routineId: over.name, groupId: "g", ...over };
}

function occasion(over: Partial<Occasion>): Occasion {
  return {
    id: "plumber",
    groupId: "g",
    agentId: null,
    title: "Plumber",
    detail: "",
    place: "",
    startsAt: at(19, 8),
    minutes: 120,
    allDay: false,
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

function morning(over: Partial<RibbonInput> = {}): RibbonInput {
  return {
    now: NOW,
    agents: AGENTS,
    groups: [],
    activity: { marlow: { state: "thinking" } },
    outcomes: [
      anOutcome({
        agentId: "kit",
        startedAt: at(15, 22),
        finishedAt: at(15, 23),
        seenAt: at(15, 23, 40),
      }),
      anOutcome({ agentId: "ines", startedAt: at(16, 2, 40), finishedAt: at(16, 2, 56) }),
      anOutcome({ agentId: "tomas", startedAt: at(16, 4), finishedAt: at(16, 4, 10) }),
      anOutcome({ agentId: "noor", startedAt: at(16, 6, 20), finishedAt: at(16, 6, 30) }),
    ],
    approvals: [anApproval({ agentId: "dara", createdAt: at(16, 9, 5), lapsesAt: at(16, 9, 15) })],
    upcoming: [
      run({ agentId: "dara", name: "Page review", nextRunAt: at(16, 10) }),
      run({ agentId: "ines", name: "Type check", nextRunAt: at(17, 9) }),
      run({ agentId: "ines", name: "Release note", nextRunAt: at(20, 15) }),
    ],
    occasions: [occasion({})],
    faceSpan: SPAN,
    ...over,
  };
}

const face = (input: RibbonInput, id: string) =>
  layoutRibbon(input).faces.find((one) => one.agentId === id);

describe("an agent with nothing today", () => {
  it("has a face of its own, named and called idle", () => {
    const ribbon = layoutRibbon(morning());
    const idle = ribbon.faces.filter((one) => one.state === "idle");
    expect(idle.map((one) => one.agentId)).toEqual(["kit", "wren", "pip"]);
    expect(idle.map((one) => one.label)).toEqual(["Kit, idle", "Wren, idle", "Pip, idle"]);
    expect(ribbon).not.toHaveProperty("pile");
  });

  it("gives every agent exactly one face", () => {
    const ribbon = layoutRibbon(morning());
    expect(ribbon.faces.map((one) => one.agentId).sort()).toEqual(
      AGENTS.map((one) => one.id).sort(),
    );
  });

  it("is called paused, as the Groups sheet calls it, when it is paused", () => {
    const ribbon = layoutRibbon(morning({ activity: { kit: { state: "paused" } } }));
    expect(ribbon.faces.find((one) => one.agentId === "kit")?.label).toBe("Kit, paused");
  });

  it("sits at the left edge, before every face with a moment", () => {
    const ribbon = layoutRibbon(morning({ charSpan: 0.005 }));
    const byX = [...ribbon.faces].sort((a, b) => a.x - b.x).map((one) => one.state);
    expect(byX.slice(0, 3)).toEqual(["idle", "idle", "idle"]);
    expect(byX.slice(3)).not.toContain("idle");
    const first = [...ribbon.faces].sort((a, b) => a.x - b.x)[0];
    expect(first?.x).toBeCloseTo(SPAN / 2);
  });

  it("keeps the order of the Groups sheet: group by group, pinned first, then by hand", () => {
    const agents = [
      anAgent("ash", { groupId: "second", railOrder: 0 }),
      anAgent("bea", { groupId: "first", railOrder: 2 }),
      anAgent("cy", { groupId: "first", railOrder: 1 }),
      anAgent("dov", { groupId: "first", railOrder: 3, pinned: true }),
      anAgent("eli", { groupId: "gone", railOrder: 0 }),
    ];
    const ribbon = layoutRibbon(
      morning({ agents, groups: ["first", "second"], activity: {}, outcomes: [], approvals: [] }),
    );
    const order = [...ribbon.faces].sort((a, b) => a.x - b.x).map((one) => one.agentId);
    expect(order).toEqual(["dov", "cy", "bea", "ash", "eli"]);
  });

  it("holds its place when another agent's day moves around it", () => {
    const quiet = layoutRibbon(morning({ activity: {} }));
    const busy = layoutRibbon(morning({ activity: { kit: { state: "thinking" } } }));
    const idle = (ribbon: typeof quiet) =>
      [...ribbon.faces]
        .sort((a, b) => a.x - b.x)
        .filter((one) => one.state === "idle")
        .map((one) => one.agentId);
    expect(idle(quiet)).toEqual(["marlow", "kit", "wren", "pip"]);
    expect(idle(busy)).toEqual(["marlow", "wren", "pip"]);
  });
});

describe("where a face goes", () => {
  it("draws each agent once, however many things pull at it", () => {
    const input = morning({
      activity: { dara: { state: "thinking" }, marlow: { state: "thinking" } },
      outcomes: [...morning().outcomes, anOutcome({ agentId: "dara", finishedAt: at(16, 8) })],
    });
    const ribbon = layoutRibbon(input);
    const ids = ribbon.faces.map((one) => one.agentId);
    expect(ids.filter((id) => id === "dara")).toHaveLength(1);
    expect(ids).toHaveLength(AGENTS.length);
  });

  it("puts needing you before working, and working before an unseen run", () => {
    const outcomes = [anOutcome({ agentId: "dara", finishedAt: at(16, 8) })];
    const asking = face(morning({ activity: { dara: { state: "thinking" } }, outcomes }), "dara");
    expect(asking).toMatchObject({ state: "needsYou", at: at(16, 9, 5) });

    const working = face(
      morning({ approvals: [], activity: { dara: { state: "thinking" } }, outcomes }),
      "dara",
    );
    expect(working).toMatchObject({ state: "working", at: NOW });

    const finished = face(morning({ approvals: [], activity: {}, outcomes }), "dara");
    expect(finished).toMatchObject({ state: "finished", at: at(16, 8) });
  });

  it("keeps what has already happened left of now when the clock it is drawn with lags", () => {
    const asked = NOW + 30_000;
    const input = morning({
      approvals: [anApproval({ agentId: "dara", createdAt: asked })],
      outcomes: [...morning().outcomes, anOutcome({ agentId: "pip", finishedAt: asked })],
    });
    const ribbon = layoutRibbon(input);
    for (const id of ["dara", "pip"]) {
      const one = ribbon.faces.find((f) => f.agentId === id);
      expect(one, id).toBeDefined();
      expect(one!.x, `${id} is left of now`).toBeLessThan(ribbon.now.x);
    }
    const caughtUp = layoutRibbon({ ...input, now: NOW + 60_000 });
    expect(caughtUp.faces.find((f) => f.agentId === "dara")!.x).toBeLessThan(caughtUp.now.x);
  });

  it("places a run that finished after the clock last ticked, rather than calling it idle", () => {
    const finished = NOW + 30_000;
    const outcomes = [
      anOutcome({ agentId: "pip", startedAt: NOW + 5_000, finishedAt: finished, seenAt: finished }),
    ];
    const ribbon = layoutRibbon(morning({ outcomes }));
    expect(ribbon.faces.find((one) => one.agentId === "pip")).toMatchObject({
      state: "rest",
      at: finished,
    });
    expect(ribbon.now.at).toBeGreaterThanOrEqual(finished);
  });

  it("is pulled to the oldest question an agent has asked", () => {
    const approvals = [
      anApproval({ agentId: "dara", createdAt: at(16, 9, 5) }),
      anApproval({ agentId: "dara", createdAt: at(16, 8, 40) }),
    ];
    expect(face(morning({ approvals }), "dara")?.at).toBe(at(16, 8, 40));
  });

  it("marks a failed run apart from a finished one", () => {
    const outcomes = [anOutcome({ agentId: "ines", status: "failed", finishedAt: at(16, 3) })];
    expect(face(morning({ outcomes }), "ines")?.state).toBe("failed");
  });

  it("falls back to the last thing it did today, unmarked", () => {
    const outcomes = [
      anOutcome({
        agentId: "ines",
        startedAt: at(16, 7),
        finishedAt: at(16, 7, 30),
        seenAt: at(16, 8),
      }),
      anOutcome({
        agentId: "ines",
        startedAt: at(16, 5),
        finishedAt: at(16, 5, 30),
        seenAt: at(16, 8),
      }),
    ];
    expect(face(morning({ outcomes }), "ines")).toMatchObject({ state: "rest", at: at(16, 7, 30) });
  });

  it("is named with who, what and when", () => {
    const ribbon = layoutRibbon(morning());
    const label = (id: string) => ribbon.faces.find((one) => one.agentId === id)?.label;
    expect(label("dara")).toBe("Dara, needs you, asked 9:05 AM");
    expect(label("marlow")).toBe("Marlow, working now");
    expect(label("ines")).toBe("Ines, finished 2:56 AM");
    expect(label("tomas")).toBe("Tomás, finished 4:10 AM");
  });
});

describe("the band", () => {
  it("is absent when everything has been seen", () => {
    const outcomes = morning().outcomes.map((one) => ({ ...one, seenAt: at(16, 9) }));
    expect(layoutRibbon(morning({ outcomes })).band).toBeNull();
  });

  it("runs from the latest moment anything was seen", () => {
    const outcomes = [
      ...morning().outcomes,
      anOutcome({ agentId: "wren", finishedAt: at(15, 20), seenAt: at(15, 21) }),
    ];
    const band = layoutRibbon(morning({ outcomes })).band;
    expect(band).toMatchObject({ from: at(15, 23, 40), label: BAND_LABEL });
    expect(BAND_LABEL).toBe("Since you last looked");
  });

  it("runs from the oldest unseen run's start when nothing was ever seen", () => {
    const outcomes = morning().outcomes.filter((one) => one.seenAt === null);
    expect(layoutRibbon(morning({ outcomes })).band?.from).toBe(at(16, 2, 40));
  });

  it("starts left of now and inside the strip", () => {
    const ribbon = layoutRibbon(morning());
    expect(ribbon.band?.x).toBeGreaterThanOrEqual(0);
    expect(ribbon.band?.x).toBeLessThan(ribbon.now.x);
  });

  it("holds every unseen face as drawn, a crowd pushed left of its moment included", () => {
    const outcomes = [
      anOutcome({
        agentId: "kit",
        startedAt: at(16, 9),
        finishedAt: at(16, 9, 1),
        seenAt: at(16, 9, 10),
      }),
      ...["ines", "tomas", "noor"].map((agentId) =>
        anOutcome({ agentId, startedAt: at(16, 9, 10), finishedAt: at(16, 9, 11) }),
      ),
    ];
    const ribbon = layoutRibbon(morning({ outcomes, approvals: [], activity: {} }));
    const unseen = ribbon.faces.filter((one) => one.state === "finished");
    expect(unseen).toHaveLength(3);
    for (const one of unseen) {
      expect(one.x - SPAN / 2).toBeGreaterThanOrEqual((ribbon.band?.x ?? 1) - 1e-9);
    }
  });

  it("keeps the hours clear of its words where a crowd pulled it left", () => {
    const outcomes = [
      anOutcome({
        agentId: "kit",
        startedAt: at(16, 8),
        finishedAt: at(16, 8, 1),
        seenAt: at(16, 8, 50),
      }),
      ...["ines", "tomas", "noor", "wren", "pip"].map((agentId) =>
        anOutcome({ agentId, startedAt: at(16, 8, 55), finishedAt: at(16, 9) }),
      ),
    ];
    const ribbon = layoutRibbon(morning({ outcomes, approvals: [], activity: {}, faceSpan: 0.08 }));
    expect(ribbon.band?.label).toBe(BAND_LABEL);
    const from = ribbon.band?.x ?? 0;
    for (const tick of ribbon.ticks.filter((one) => one.kind === "hour")) {
      expect(tick.x < from - 0.04 || tick.x > from + 0.14).toBe(true);
    }
  });

  it("keeps its words clear of the now pill at every width", () => {
    let drawn = 0;
    for (const width of [480, 720, 1024, 1440]) {
      const charSpan = 8 / width;
      for (let minutes = 10; minutes <= 180; minutes += 5) {
        const seenAt = NOW - minutes * 60_000;
        const outcomes = [
          anOutcome({
            agentId: "kit",
            startedAt: seenAt - 120_000,
            finishedAt: seenAt - 60_000,
            seenAt,
          }),
          anOutcome({ agentId: "ines", startedAt: NOW - 120_000, finishedAt: NOW - 60_000 }),
        ];
        const ribbon = layoutRibbon(
          morning({ outcomes, approvals: [], activity: {}, faceSpan: 52 / width, charSpan }),
        );
        if (!ribbon.band?.label) continue;
        drawn += 1;
        const wordsEnd = ribbon.band.x + ribbon.band.label.length * charSpan;
        const pillStart = ribbon.now.x - (ribbon.now.label.length / 2) * charSpan;
        expect(wordsEnd).toBeLessThanOrEqual(pillStart);
      }
    }
    expect(drawn).toBeGreaterThan(0);
  });

  it("leaves its words off when it is too narrow to hold them clear of the now pill", () => {
    const outcomes = [
      anOutcome({
        agentId: "kit",
        startedAt: at(16, 9),
        finishedAt: at(16, 9, 1),
        seenAt: at(16, 9, 10),
      }),
      anOutcome({ agentId: "ines", startedAt: at(16, 9, 10), finishedAt: at(16, 9, 11) }),
    ];
    const band = layoutRibbon(morning({ outcomes, approvals: [], activity: {} })).band;
    expect(band).not.toBeNull();
    expect(band?.label).toBeNull();
  });
});

describe("where a request stops waiting", () => {
  it("has no point when the request waits for good", () => {
    const approvals = [anApproval({ agentId: "dara", createdAt: at(16, 9, 5) })];
    expect(layoutRibbon(morning({ approvals })).lapses).toEqual([]);
  });

  it("has no point once the moment it lapses has passed", () => {
    const approvals = [anApproval({ agentId: "dara", createdAt: at(16, 7), lapsesAt: at(16, 8) })];
    expect(layoutRibbon(morning({ approvals })).lapses).toEqual([]);
  });

  it("is a point at the moment it lapses, named for whose it is", () => {
    const ribbon = layoutRibbon(morning());
    expect(ribbon.lapses).toHaveLength(1);
    expect(ribbon.lapses[0]).toMatchObject({
      agentId: "dara",
      at: at(16, 9, 15),
      label: "Dara stops waiting",
    });
    expect(ribbon.lapses[0]?.x).toBeCloseTo(xAt(ribbon.scale, at(16, 9, 15)));
  });
});

describe("crowding", () => {
  const NAMES = ["Researcher", "Critic", "Manager", "Scribe", "Planner", "Archivist"];
  const NAMES_12 = [
    ...NAMES,
    "Translator",
    "Navigator",
    "Editor",
    "Auditor",
    "Reviewer",
    "Builder",
  ];
  const NAMES_9 = NAMES_12.slice(0, 9);
  const FOUR = ["Chief of Staff", "Market Researcher", "Financial Analyst", "Travel Planner"];
  const NINE = [
    ...FOUR,
    "Market Analyst",
    "Head of Sales",
    "Research Librarian",
    "Customer Support Lead",
    "Executive Assistant",
  ];

  function crowd(names: string[], width: number) {
    const agents = names.map((name) => anAgent(name.toLowerCase(), { name }));
    const outcomes = agents.map((agent, i) =>
      anOutcome({
        agentId: agent.id,
        startedAt: NOW - 600_000,
        finishedAt: NOW - (i % 6) * 60_000,
      }),
    );
    const faceSpan = 52 / width;
    const charSpan = 8 / width;
    const input = morning({
      agents,
      outcomes,
      approvals: [],
      activity: {},
      upcoming: [],
      occasions: [],
      faceSpan,
      charSpan,
    });
    return { ribbon: layoutRibbon(input), agents, faceSpan, charSpan };
  }

  const drawnHalf = (face: RibbonFace, faceSpan: number, charSpan: number) =>
    face.room === null ? Math.max(faceSpan, face.name.length * charSpan) / 2 : face.room / 2;

  function shown(ribbon: Ribbon, faceSpan: number, charSpan: number) {
    return [
      ...ribbon.faces.map((face) => ({ x: face.x, half: drawnHalf(face, faceSpan, charSpan) })),
      ...ribbon.counts.map((one) => ({
        x: one.x,
        half: Math.max(faceSpan, one.text.length * charSpan) / 2,
      })),
    ].sort((a, b) => a.x - b.x);
  }

  function apart(ribbon: Ribbon, faceSpan: number, charSpan: number) {
    const drawn = shown(ribbon, faceSpan, charSpan);
    for (let i = 1; i < drawn.length; i++) {
      const a = drawn[i - 1] as (typeof drawn)[number];
      const b = drawn[i] as (typeof drawn)[number];
      expect(b.x - a.x).toBeGreaterThanOrEqual(a.half + b.half - 1e-9);
    }
    for (const one of drawn) {
      expect(one.x - one.half).toBeGreaterThanOrEqual(-1e-9);
      expect(one.x + one.half).toBeLessThanOrEqual(ribbon.now.x + 1e-9);
    }
  }

  function everyoneOnce(ribbon: Ribbon, agents: AgentCard[]) {
    const counted = ribbon.counts.flatMap((one) => one.agentIds);
    expect([...ribbon.faces.map((face) => face.agentId), ...counted].sort()).toEqual(
      agents.map((agent) => agent.id).sort(),
    );
  }

  for (const [names, width] of [
    [NAMES, 1280],
    [NAMES, 900],
    [NAMES_9, 1280],
    [NAMES_12, 1280],
  ] as const) {
    it(`draws all ${names.length} names whole and lets no two touch at ${width} wide`, () => {
      const { ribbon, agents, faceSpan, charSpan } = crowd([...names], width);
      everyoneOnce(ribbon, agents);
      expect(ribbon.counts).toEqual([]);
      for (const face of ribbon.faces) {
        expect(face.room).toBeNull();
        expect(face.name).toBe(agents.find((agent) => agent.id === face.agentId)?.name);
      }
      apart(ribbon, faceSpan, charSpan);
    });
  }

  for (const [names, width] of [
    [NAMES, 375],
    [NAMES_9, 900],
    [NAMES_12, 900],
    [NAMES_12, 375],
  ] as const) {
    it(`counts every one of ${names.length} agents once and lets nothing touch at ${width} wide`, () => {
      const { ribbon, agents, faceSpan, charSpan } = crowd([...names], width);
      everyoneOnce(ribbon, agents);
      apart(ribbon, faceSpan, charSpan);
    });
  }

  it("keeps four agents' whole names at 1280 and at 900 wide", () => {
    for (const width of [1280, 900]) {
      const { ribbon } = crowd(FOUR, width);
      expect(ribbon.faces.map((face) => face.name).sort()).toEqual([...FOUR].sort());
      expect(ribbon.faces.every((face) => face.room === null)).toBe(true);
    }
  });

  it("drops a name to the one word that tells it apart before cutting any", () => {
    const { ribbon, faceSpan, charSpan } = crowd(NINE, 900);
    expect(ribbon.counts).toEqual([]);
    expect(ribbon.faces.every((face) => face.room === null)).toBe(true);
    const names = ribbon.faces.map((face) => face.name);
    expect(new Set(names).size).toBe(NINE.length);
    expect(names).toContain("Chief");
    expect(names).toContain("Researcher");
    expect(names).toContain("Market Analyst");
    expect(names.some((name) => NINE.includes(name))).toBe(true);
    apart(ribbon, faceSpan, charSpan);
  });

  it("cuts a name to the room it has only once every name is as short as its words go", () => {
    const { ribbon, faceSpan, charSpan } = crowd(NAMES, 480);
    const cut = ribbon.faces.filter((face) => face.room !== null);
    expect(cut.length).toBeGreaterThan(0);
    for (const face of cut) expect(face.room).toBeLessThanOrEqual(faceSpan + 1e-9);
    const sorted = [...ribbon.faces].sort((a, b) => a.x - b.x);
    for (let i = 1; i < sorted.length; i++) {
      const gap = (sorted[i] as RibbonFace).x - (sorted[i - 1] as RibbonFace).x;
      for (const face of [sorted[i - 1], sorted[i]] as RibbonFace[]) {
        if (face.room !== null) expect(face.room).toBeLessThanOrEqual(gap - charSpan + 1e-9);
      }
    }
  });

  it("folds a crowd too large for its stretch into a count rather than stacking faces", () => {
    const { ribbon, agents } = crowd(NAMES_12, 375);
    expect(ribbon.counts.map((one) => one.text)).toEqual([
      `+${agents.length - ribbon.faces.length} more`,
    ]);
    const xs = ribbon.faces.map((face) => face.x).sort((a, b) => a - b);
    for (let i = 1; i < xs.length; i++) {
      expect((xs[i] as number) - (xs[i - 1] as number)).toBeGreaterThanOrEqual(52 / 375 - 1e-9);
    }
  });

  function live(kind: "working" | "needsYou", count: number, width: number) {
    const agents = NAMES_12.slice(0, count).map((name) => anAgent(name.toLowerCase(), { name }));
    const faceSpan = 52 / width;
    const charSpan = 8 / width;
    const input = morning({
      agents,
      outcomes: [],
      activity:
        kind === "working"
          ? Object.fromEntries(agents.map((agent) => [agent.id, { state: "thinking" as const }]))
          : {},
      approvals:
        kind === "needsYou"
          ? agents.map((agent, i) =>
              anApproval({
                id: `ask-${agent.id}`,
                agentId: agent.id,
                createdAt: NOW - (i + 1) * 60_000,
              }),
            )
          : [],
      upcoming: [],
      occasions: [],
      faceSpan,
      charSpan,
    });
    return { ribbon: layoutRibbon(input), agents, faceSpan, charSpan };
  }

  for (const [kind, count] of [
    ["working", 5],
    ["working", 8],
    ["needsYou", 8],
  ] as const) {
    for (const width of [1280, 900, 375]) {
      it(`never lets ${count} ${kind} faces or their names touch at ${width} wide`, () => {
        const { ribbon, agents, faceSpan, charSpan } = live(kind, count, width);
        const drawn = shown(ribbon, faceSpan, charSpan);
        for (let i = 1; i < drawn.length; i++) {
          const a = drawn[i - 1] as (typeof drawn)[number];
          const b = drawn[i] as (typeof drawn)[number];
          expect(b.x - a.x).toBeGreaterThanOrEqual(a.half + b.half + charSpan - 1e-9);
        }
        everyoneOnce(ribbon, agents);
        for (const one of ribbon.counts) expect(one.kind).toBe(kind === "working" ? "more" : kind);
        if (kind === "needsYou") {
          for (const face of ribbon.faces) expect(face.room).toBeNull();
        }
      });
    }
  }

  it("says how many more there are, or how many more need you, by name, in one counted item", () => {
    const working = live("working", 8, 375).ribbon.counts[0];
    expect(working?.text).toMatch(/^\+\d+ more$/);
    expect(working?.label.startsWith(`${working?.text}: `)).toBe(true);
    const asking = live("needsYou", 8, 375).ribbon.counts[0];
    expect(asking?.text).toMatch(/^\+\d+ needs? you$/);
  });

  it("keeps a resting face beside an unseen failure when the strip is crowded", () => {
    const agents = NAMES.map((name) => anAgent(name.toLowerCase(), { name }));
    const outcomes = [
      anOutcome({
        agentId: "researcher",
        status: "failed",
        startedAt: at(16, 2, 55),
        finishedAt: at(16, 3),
      }),
      anOutcome({
        agentId: "critic",
        startedAt: at(16, 7, 50),
        finishedAt: at(16, 8),
        seenAt: at(16, 8, 1),
      }),
      ...["manager", "scribe", "planner", "archivist"].map((agentId, i) =>
        anOutcome({ agentId, startedAt: NOW - 600_000, finishedAt: NOW - (i + 1) * 60_000 }),
      ),
    ];
    const width = 1000;
    const ribbon = layoutRibbon(
      morning({
        agents,
        outcomes,
        approvals: [],
        activity: {},
        upcoming: [],
        occasions: [],
        faceSpan: 52 / width,
        charSpan: 8 / width,
      }),
    );
    expect(ribbon.faces.find((face) => face.agentId === "critic")?.state).toBe("rest");
    expect(ribbon.faces.find((face) => face.agentId === "researcher")?.state).toBe("failed");
    expect(ribbon.faces).toHaveLength(agents.length);
  });

  it("pushes faces at one moment a face apart", () => {
    const outcomes = ["ines", "tomas", "noor"].map((agentId) =>
      anOutcome({ agentId, startedAt: at(16, 4), finishedAt: at(16, 4, 10) }),
    );
    const xs = layoutRibbon(morning({ outcomes }))
      .faces.map((one) => one.x)
      .sort((a, b) => a - b);
    for (let i = 1; i < xs.length; i++) {
      expect((xs[i] as number) - (xs[i - 1] as number)).toBeGreaterThanOrEqual(SPAN - 1e-9);
    }
  });

  it("centers a crowd on its moment rather than pushing it all one way", () => {
    expect(spread([0.5, 0.5, 0.5], 0.1).map((x) => Number(x.toFixed(6)))).toEqual([0.4, 0.5, 0.6]);
  });

  it("leaves faces that are already clear where they are", () => {
    expect(spread([0.1, 0.3, 0.6], 0.1)).toEqual([0.1, 0.3, 0.6]);
  });

  it("keeps a crowd on the strip at its edges", () => {
    const xs = spread([0, 0, 0], 0.1);
    expect(xs[0]).toBeCloseTo(0.05);
    expect(xs[2]).toBeCloseTo(0.25);
  });
});

describe("what a face's place says", () => {
  const WIDTH = 1280;
  function crowdedMorning() {
    const agents = [
      ...AGENTS,
      ...["Researcher", "Critic", "Manager", "Scribe"].map((name) =>
        anAgent(name.toLowerCase(), { name }),
      ),
    ];
    const outcomes = agents
      .filter((agent) => !["dara", "marlow", "kit", "wren", "pip"].includes(agent.id))
      .map((agent, i) =>
        anOutcome({ agentId: agent.id, startedAt: NOW - 900_000, finishedAt: NOW - i * 60_000 }),
      );
    return morning({
      agents,
      outcomes,
      approvals: [anApproval({ agentId: "dara", createdAt: at(16, 7, 50) })],
      faceSpan: 52 / WIDTH,
      charSpan: 8 / WIDTH,
    });
  }

  it("never draws an hour label beside a face that happened at another hour", () => {
    let checked = 0;
    for (const input of [
      morning({ faceSpan: 52 / WIDTH, charSpan: 8 / WIDTH }),
      crowdedMorning(),
    ]) {
      const ribbon = layoutRibbon(input);
      const hours = ribbon.ticks.filter((tick) => tick.kind === "hour");
      for (const one of ribbon.faces.filter((face) => face.state !== "idle")) {
        for (const tick of hours) {
          checked += 1;
          expect(one.x < tick.x, `${one.label} against ${tick.label}`).toBe(one.at < tick.at);
        }
      }
    }
    expect(checked).toBeGreaterThan(0);
  });

  it("puts agents with nothing today in a tray left of the hours, where no hour is labeled", () => {
    const input = crowdedMorning();
    const ribbon = layoutRibbon(input);
    const idle = ribbon.faces.filter((face) => face.state === "idle");
    expect(idle.map((face) => face.agentId)).toEqual(["kit", "wren", "pip"]);
    expect(ribbon.axis).toBeGreaterThan(0);
    for (const face of idle) {
      expect(face.x + Math.max(52, face.name.length * 8) / WIDTH / 2).toBeLessThanOrEqual(
        ribbon.axis,
      );
    }
    for (const face of ribbon.faces.filter((one) => one.state !== "idle")) {
      expect(face.x).toBeGreaterThan(ribbon.axis);
    }
    for (const tick of ribbon.ticks) expect(tick.x).toBeGreaterThanOrEqual(ribbon.axis);
    expect(ribbon.band?.x ?? ribbon.axis).toBeGreaterThanOrEqual(ribbon.axis);
  });

  it("has no tray and starts the hours at the left edge when every agent did something today", () => {
    const outcomes = AGENTS.map((agent, i) =>
      anOutcome({ agentId: agent.id, startedAt: at(16, 1 + i), finishedAt: at(16, 1 + i, 10) }),
    );
    expect(layoutRibbon(morning({ outcomes })).axis).toBe(0);
  });

  it("gives up hours ahead before it shortens a name, and keeps the coming days on the strip", () => {
    const roomy = layoutRibbon(morning({ faceSpan: 52 / WIDTH, charSpan: 8 / WIDTH }));
    const crowded = layoutRibbon(crowdedMorning());
    expect(crowded.now.x).toBeGreaterThan(roomy.now.x);
    expect(crowded.faces.every((face) => face.room === null)).toBe(true);
    const days = crowded.ticks.filter((tick) => tick.kind === "day");
    expect(days).toHaveLength(4);
    for (const day of days) expect(day.x).toBeGreaterThan(crowded.now.x);
  });
});

describe("what folds when the strip is full", () => {
  function mixed(width: number) {
    const agents = [
      "Idle One",
      "Idle Two",
      "Resting",
      "Finisher",
      "Worker",
      "Failing",
      "First Asker",
      "Second Asker",
    ].map((name) => anAgent(name.toLowerCase().replace(" ", "-"), { name }));
    const input = morning({
      agents,
      activity: { worker: { state: "thinking" } },
      outcomes: [
        anOutcome({ agentId: "resting", finishedAt: at(16, 8), seenAt: at(16, 8, 5) }),
        anOutcome({ agentId: "finisher", finishedAt: at(16, 8, 30) }),
        anOutcome({ agentId: "failing", status: "failed", finishedAt: at(16, 8, 40) }),
      ],
      approvals: [
        anApproval({ id: "a1", agentId: "first-asker", createdAt: at(16, 9) }),
        anApproval({ id: "a2", agentId: "second-asker", createdAt: at(16, 9, 5) }),
      ],
      upcoming: [],
      occasions: [],
      faceSpan: 52 / width,
      charSpan: 8 / width,
    });
    return layoutRibbon(input);
  }

  it("folds idle agents first, then what is resting, finished, working and failed, and questions last", () => {
    const order = ["idle", "rest", "finished", "working", "failed", "needsYou"];
    let folded = 0;
    for (const width of [900, 640, 480, 375, 320, 240]) {
      const ribbon = mixed(width);
      const more = new Set(ribbon.counts.find((one) => one.kind === "more")?.agentIds ?? []);
      folded = Math.max(folded, more.size);
      const shown = ribbon.faces.map((face) => order.indexOf(face.state));
      const lowestShown = Math.min(...shown);
      for (const id of more) {
        const state = mixed(2000).faces.find((face) => face.agentId === id)?.state ?? "idle";
        expect(order.indexOf(state), `${id} folded at ${width}`).toBeLessThanOrEqual(lowestShown);
      }
    }
    expect(folded).toBeGreaterThan(0);
  });

  it("never folds a question while an agent without one keeps a face, at any width", () => {
    for (let width = 1280; width >= 200; width -= 20) {
      const ribbon = mixed(width);
      const asking = ribbon.counts.find((one) => one.kind === "needsYou");
      if (!asking) continue;
      expect(
        ribbon.faces.every((face) => face.state === "needsYou"),
        `at ${width}`,
      ).toBe(true);
    }
    expect(mixed(900).faces.filter((face) => face.state === "needsYou")).toHaveLength(2);
    expect(mixed(375).faces.filter((face) => face.state === "needsYou")).toHaveLength(2);
  });
});

describe("the scale", () => {
  it("never runs backwards, and spans the strip from now's left to the week's end", () => {
    const ribbon = layoutRibbon(morning());
    let last = -1;
    for (let t = at(15, 0); t <= at(21, 0); t += 10 * 60_000) {
      const x = xAt(ribbon.scale, t);
      expect(x).toBeGreaterThanOrEqual(last);
      expect(x).toBeGreaterThanOrEqual(0);
      expect(x).toBeLessThanOrEqual(1);
      last = x;
    }
    expect(ribbon.now.x).toBeGreaterThan(0);
    expect(ribbon.now.x).toBeLessThan(1);
  });

  it("is wider per hour near now than in the night before it", () => {
    const { scale } = layoutRibbon(morning());
    const near = xAt(scale, at(16, 10)) - xAt(scale, at(16, 9));
    const night = xAt(scale, at(16, 4)) - xAt(scale, at(16, 3));
    expect(near).toBeGreaterThan(night * 2);
  });

  it("gives each coming day through Friday a column, labeled", () => {
    const days = layoutRibbon(morning()).ticks.filter((tick) => tick.kind === "day");
    expect(days.map((tick) => tick.label)).toEqual(["Tue", "Wed", "Thu", "Fri"]);
  });

  it("labels hours clear of the now pill and of each other", () => {
    const ribbon = layoutRibbon(morning());
    const hours = ribbon.ticks.filter((tick) => tick.kind === "hour");
    expect(hours.length).toBeGreaterThan(0);
    for (const tick of hours) expect(Math.abs(tick.x - ribbon.now.x)).toBeGreaterThan(0.04);
    expect(ribbon.now.label).toBe("9:12");
  });
});

describe("what is scheduled", () => {
  it("is named with what and when, day included", () => {
    const labels = layoutRibbon(morning()).scheduled.map((one) => one.label);
    expect(labels).toEqual([
      "Page review, today 10:00 AM",
      "Type check, Tue 9:00 AM",
      "Plumber, Thu 8 to 10 AM",
      "Release note, Fri 3:00 PM",
    ]);
  });

  it("shows the owner for a routine and a glyph for the calendar", () => {
    const items = layoutRibbon(morning()).scheduled;
    expect(items.find((one) => one.what === "Type check")?.agentId).toBe("ines");
    expect(items.find((one) => one.what === "Plumber")).toMatchObject({
      agentId: null,
      when: "8 to 10 AM",
    });
  });

  it("leaves out what falls after the week the strip shows", () => {
    const upcoming = [run({ agentId: "ines", name: "Monthly", nextRunAt: at(23, 9) })];
    expect(layoutRibbon(morning({ upcoming, occasions: [] })).scheduled).toEqual([]);
  });

  it("is one item when the same routine is due for several agents at once", () => {
    const upcoming = ["dara", "kit", "wren"].map((agentId) =>
      run({ agentId, routineId: `review-${agentId}`, name: "Page review", nextRunAt: at(16, 10) }),
    );
    const scheduled = layoutRibbon(morning({ upcoming, occasions: [] })).scheduled;
    expect(scheduled.map((one) => one.label)).toEqual(["Page review for 3 agents, today 10:00 AM"]);
  });

  it("counts agents, not routines, when one agent has the same routine twice", () => {
    const upcoming = ["first", "second"].map((routineId) =>
      run({ agentId: "dara", routineId, name: "Page review", nextRunAt: at(16, 10) }),
    );
    const scheduled = layoutRibbon(morning({ upcoming, occasions: [] })).scheduled;
    expect(scheduled.map((one) => one.label)).toEqual(["Page review, today 10:00 AM"]);
  });

  it("gives way to the faces on a strip too narrow for both", () => {
    const ribbon = layoutRibbon(morning({ faceSpan: 0.2 }));
    const drawn = ribbon.faces.map((one) => one.agentId);
    const counted = ribbon.counts.flatMap((one) => one.agentIds);
    expect([...drawn, ...counted].sort()).toEqual(AGENTS.map((one) => one.id).sort());
    expect(drawn).toContain("dara");
    expect(drawn).toContain("marlow");
    const xs = ribbon.faces.map((one) => one.x).sort((a, b) => a - b);
    expect(new Set(xs).size).toBe(xs.length);
    for (const x of xs) expect(x).toBeGreaterThanOrEqual(0);
    expect(ribbon.scheduled).toEqual([]);
  });

  it("shares the row with the faces and is never nearer to one than a face", () => {
    const ribbon = layoutRibbon(morning());
    const xs = [...ribbon.faces, ...ribbon.scheduled].map((one) => one.x).sort((a, b) => a - b);
    for (let i = 1; i < xs.length; i++) {
      expect((xs[i] as number) - (xs[i - 1] as number)).toBeGreaterThanOrEqual(0.05 - 1e-9);
    }
  });

  it("keeps its words only where there is room for them beside it", () => {
    const upcoming = [0, 5, 10].map((minute) =>
      run({ agentId: "ines", name: `Check ${minute}`, nextRunAt: at(18, 9, minute) }),
    );
    const scheduled = layoutRibbon(morning({ upcoming, occasions: [] })).scheduled;
    const middle = scheduled.find((one) => one.what === "Check 5");
    expect(middle?.words).toBe(false);
    expect(middle?.label).toBe("Check 5, Wed 9:05 AM");
    const alone = layoutRibbon(morning()).scheduled.find((one) => one.what === "Type check");
    expect(alone?.words).toBe(true);
  });

  it("names a routine saved without a name rather than drawing it blank", () => {
    const upcoming = [run({ agentId: "ines", name: "  ", nextRunAt: at(17, 9) })];
    const [item] = layoutRibbon(morning({ upcoming, occasions: [] })).scheduled;
    expect(item?.what).toBe("Routine");
    expect(item?.label).toBe("Routine, Tue 9:00 AM");
  });

  it("is never a face: an agent's routine leaves its face where it was", () => {
    const ribbon = layoutRibbon(morning());
    expect(ribbon.faces.filter((one) => one.agentId === "ines")).toHaveLength(1);
  });
});

describe("labels", () => {
  const CHAR = 0.01;
  const half = (text: string) => Math.max(SPAN, text.length * CHAR) / 2;

  it("keeps every name and every coming item off the now line, and every face left of it", () => {
    const input = morning({ charSpan: CHAR });
    const ribbon = layoutRibbon(input);
    const name = (id: string) => AGENTS.find((one) => one.id === id)?.name ?? "";
    for (const one of ribbon.faces) {
      expect(Math.abs(one.x - ribbon.now.x)).toBeGreaterThanOrEqual(half(name(one.agentId)));
    }
    for (const one of ribbon.scheduled) {
      const text = one.what.length > one.when.length ? one.what : one.when;
      expect(Math.abs(one.x - ribbon.now.x)).toBeGreaterThanOrEqual(half(text));
    }
    for (const one of ribbon.faces) expect(one.x).toBeLessThan(ribbon.now.x);
    expect(face(input, "marlow")?.x).toBeGreaterThan(face(input, "dara")?.x ?? 1);
  });

  it("keeps a label near either end of the strip inside it", () => {
    const upcoming = [
      run({ agentId: "tomas", name: "Friday release note", nextRunAt: at(20, 15) }),
    ];
    const outcomes = [
      anOutcome({
        agentId: "tomas",
        startedAt: at(16, 0, 5),
        finishedAt: at(16, 0, 10),
      }),
    ];
    const agents = [anAgent("tomas", { name: "Tomás Villanueva-Ortega" })];
    const ribbon = layoutRibbon(
      morning({
        agents,
        upcoming,
        outcomes,
        approvals: [],
        activity: {},
        occasions: [],
        charSpan: CHAR,
      }),
    );
    const note = ribbon.scheduled[0];
    expect(note?.words).toBe(true);
    expect(note?.align).toBe("end");
    const tomas = ribbon.faces[0];
    expect((tomas?.x ?? 0) - half("Tomás Villanueva-Ortega")).toBeGreaterThanOrEqual(-1e-9);
    expect((note?.x ?? 1) + SPAN / 2).toBeLessThanOrEqual(1);
    expect((tomas?.x ?? 0) - SPAN / 2).toBeGreaterThanOrEqual(0);
  });

  it("centers a label that fits under its mark", () => {
    const ribbon = layoutRibbon(morning({ charSpan: CHAR }));
    expect(ribbon.faces.find((one) => one.agentId === "tomas")?.align).toBe("center");
  });
});

describe("a quiet afternoon", () => {
  const quiet = morning({
    now: at(16, 14, 40),
    activity: {},
    approvals: [],
    outcomes: [
      anOutcome({
        agentId: "dara",
        startedAt: at(16, 9, 50),
        finishedAt: at(16, 10),
        seenAt: at(16, 14),
      }),
      anOutcome({
        agentId: "marlow",
        startedAt: at(16, 11),
        finishedAt: at(16, 11, 32),
        seenAt: at(16, 14),
      }),
    ],
  });

  it("rests faces at their last event today, unmarked", () => {
    const ribbon = layoutRibbon(quiet);
    const resting = ribbon.faces.filter((one) => one.state !== "idle");
    expect(resting.map((one) => [one.agentId, one.state, one.at])).toEqual([
      ["dara", "rest", at(16, 10)],
      ["marlow", "rest", at(16, 11, 32)],
    ]);
    expect(resting[0]?.label).toBe("Dara, last active 10:00 AM");
  });

  it("has no band and draws everyone else idle at the left edge, in the sheet's order", () => {
    const ribbon = layoutRibbon(quiet);
    expect(ribbon.band).toBeNull();
    const idle = [...ribbon.faces]
      .sort((a, b) => a.x - b.x)
      .filter((one) => one.state === "idle")
      .map((one) => one.agentId);
    expect(idle).toEqual(["ines", "tomas", "noor", "kit", "wren", "pip"]);
  });
});
