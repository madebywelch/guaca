import { describe, expect, it } from "vitest";

import { anAgent, anApproval, anOutcome } from "../test-fixtures";
import { BAND_LABEL, layoutRibbon, type RibbonInput, spread, xAt } from "./ribbon";
import type { Occasion, UpcomingRun } from "./types";

/** Monday, November 16, 2026, local time, so the suite holds in every zone. */
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

/** The approved mock's morning: away overnight, three runs unseen, one question. */
function morning(over: Partial<RibbonInput> = {}): RibbonInput {
  return {
    now: NOW,
    agents: AGENTS,
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

describe("the pile", () => {
  it("takes an agent with nothing live, unseen or from today", () => {
    // Kit's last run was yesterday and was seen, so nothing pulls it onto the strip.
    const ribbon = layoutRibbon(morning());
    expect(ribbon.pile).toEqual(["kit", "wren", "pip"]);
    expect(ribbon.faces.map((one) => one.agentId)).not.toContain("kit");
  });

  it("is empty when every agent has somewhere to be", () => {
    const ribbon = layoutRibbon(morning({ agents: AGENTS.slice(0, 5) }));
    expect(ribbon.pile).toEqual([]);
  });
});

describe("where a face goes", () => {
  it("draws each agent once, however many things pull at it", () => {
    // Dara is asking, working and has an unseen run all at once.
    const input = morning({
      activity: { dara: { state: "thinking" }, marlow: { state: "thinking" } },
      outcomes: [...morning().outcomes, anOutcome({ agentId: "dara", finishedAt: at(16, 8) })],
    });
    const ribbon = layoutRibbon(input);
    const ids = [...ribbon.faces.map((one) => one.agentId), ...ribbon.pile];
    expect(ids.filter((id) => id === "dara")).toHaveLength(1);
    expect(new Set(ids).size).toBe(AGENTS.length);
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
});

describe("where a request stops waiting", () => {
  it("has no point when the request waits for good", () => {
    const approvals = [anApproval({ agentId: "dara", createdAt: at(16, 9, 5) })];
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
    // Nine copies side by side pushed the last of them days past the moment
    // all nine were due at.
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

  it("is never a face: an agent's routine leaves its face where it was", () => {
    const ribbon = layoutRibbon(morning());
    expect(ribbon.faces.filter((one) => one.agentId === "ines")).toHaveLength(1);
  });
});

describe("a quiet afternoon", () => {
  // Everything seen and nobody working: the faces rest where they last did
  // anything today, with no marks and no band.
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
    expect(ribbon.faces.map((one) => [one.agentId, one.state, one.at])).toEqual([
      ["dara", "rest", at(16, 10)],
      ["marlow", "rest", at(16, 11, 32)],
    ]);
    expect(ribbon.faces[0]?.label).toBe("Dara, last active 10:00 AM");
  });

  it("has no band and piles everyone else", () => {
    const ribbon = layoutRibbon(quiet);
    expect(ribbon.band).toBeNull();
    expect(ribbon.pile).toEqual(["ines", "tomas", "noor", "kit", "wren", "pip"]);
  });
});
