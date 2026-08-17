import { describe, expect, it } from "vitest";

import {
  describeTrigger,
  gapSeconds,
  humanGap,
  isTimed,
  repeatLabel,
  routineTitle,
  secondsUntil,
  TRIGGER_CHOICES,
  toTimeField,
} from "./routine";

/** A local wall-clock moment, so every assertion reads in the operator's time. */
function at(y: number, m: number, d: number, hour: number, minute = 0): number {
  return new Date(y, m - 1, d, hour, minute, 0, 0).getTime();
}

describe("trigger vocabulary", () => {
  it("offers a choice for every shape the backend can store", () => {
    // The picker is the only place these are written down on this side. A
    // choice whose spec the Rust parser refuses is a routine that saves and
    // then never fires.
    expect(TRIGGER_CHOICES.map((c) => c.spec)).toEqual([
      "every:3600",
      "daily",
      "weekdays",
      "weekly",
      "monthly",
      "once",
    ]);
  });

  it("reads back a gap no choice offers", () => {
    // An agent setting its own schedule works in seconds and picks whatever it
    // likes. The row still has to be legible in the operator's list.
    expect(repeatLabel("every:18000")).toBe("Every 5 hours");
    expect(repeatLabel("every:3600")).toBe("Every hour");
    expect(repeatLabel("every:90")).toBe("Every 90 seconds");
    expect(gapSeconds("every:18000")).toBe(18_000);
    expect(gapSeconds("weekdays")).toBeNull();
  });

  it("draws a trigger from a build that knows more than this one", () => {
    // Forward-only migrations mean a newer build can write a value this one
    // has never heard of. Saying it is better than an empty row.
    expect(repeatLabel("event:linear.assigned")).toBe("event:linear.assigned");
    expect(gapSeconds("every:nonsense")).toBeNull();
    expect(gapSeconds("every:-1")).toBeNull();
  });

  it("says the time of day only for the triggers that have one", () => {
    // An hourly routine has no hour to name, and claiming one would be a lie
    // that changes every time it runs.
    expect(isTimed("every:3600")).toBe(false);
    expect(isTimed("weekdays")).toBe(true);

    const morning = at(2025, 6, 10, 9, 28);
    expect(describeTrigger("weekdays", morning)).toBe("Weekdays at 9:28 AM");
    expect(describeTrigger("every:3600", morning)).toBe("Every hour");
  });

  it("gives a one-off its date, because a time alone does not say when", () => {
    expect(describeTrigger("once", at(2025, 9, 25, 9, 0))).toBe("Once, on Sep 25 at 9:00 AM");
  });

  it("round-trips a time of day through the field the operator edits", () => {
    expect(toTimeField(at(2025, 6, 10, 9, 5))).toBe("09:05");
    expect(toTimeField(at(2025, 6, 10, 17, 30))).toBe("17:30");
    expect(toTimeField(at(2025, 6, 10, 0, 0))).toBe("00:00");
  });

  it("counts to the next occurrence of a time, not to one already gone", () => {
    const noon = at(2025, 6, 10, 12, 0);
    expect(secondsUntil("14:30", noon)).toBe(2.5 * 3600);
    // Nine has been and gone, so it is tomorrow's nine.
    expect(secondsUntil("09:00", noon)).toBe(21 * 3600);
    // And the current minute is behind by the time anyone clicks save.
    expect(secondsUntil("12:00", noon)).toBe(24 * 3600);
  });

  it("refuses a time it cannot read rather than scheduling something else", () => {
    // A null reaches the backend as "no delay", which is a deliberate meaning.
    // Turning garbage into midnight would silently schedule the wrong thing.
    expect(secondsUntil("")).toBeNull();
    expect(secondsUntil("25:00")).toBeNull();
    expect(secondsUntil("09:70")).toBeNull();
    expect(secondsUntil("half nine")).toBeNull();
  });

  it("says a gap in the units it was set in", () => {
    expect(humanGap(60)).toBe("minute");
    expect(humanGap(300)).toBe("5 minutes");
    expect(humanGap(3600)).toBe("hour");
    expect(humanGap(86_400)).toBe("day");
    expect(humanGap(90)).toBe("90 seconds");
  });
});

describe("routineTitle", () => {
  const routine = (name: string, what: string) => ({ name, what });

  it("is the name whenever there is one", () => {
    expect(routineTitle(routine("Tue-Thu social posts", "a long instruction"))).toBe(
      "Tue-Thu social posts",
    );
    expect(routineTitle(routine("   ", "check the listings"))).toBe("check the listings");
  });

  it("cuts a long instruction down to something that fits a row", () => {
    // The complaint this answers: an agent's own routine is written to be acted
    // on with no other context, so it runs to several sentences, and the whole
    // thing was drawn as the title. One routine filled the panel.
    const long = routine(
      "",
      "Publish on the day only. America/New_York. Manager already cleared this set, so check the feed first and post one copy.",
    );
    const title = routineTitle(long);
    expect(title).toBe("Publish on the day only");
    expect(title.length).toBeLessThanOrEqual(45);
  });

  it("never cuts a word in half", () => {
    // A title ending mid-word reads as corruption rather than as truncation.
    const title = routineTitle(
      routine("", "Check every incoming application against the eligibility criteria we agreed"),
    );
    expect(title.endsWith("…")).toBe(true);
    expect(title.replace("…", "").trimEnd().split(" ").pop()).not.toBe("appl");
    expect(/\S…$/.test(title)).toBe(true);
  });

  it("collapses the whitespace a multi-line instruction carries", () => {
    expect(routineTitle(routine("", "check\n  the   listings"))).toBe("check the listings");
  });

  it("still says something for a routine with nothing in it", () => {
    expect(routineTitle(routine("", "   "))).toBe("Untitled routine");
  });
});
