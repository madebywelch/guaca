import { describe, expect, it } from "vitest";

import {
  anchorFor,
  describeTrigger,
  firstRunDelay,
  humanGap,
  type Moment,
  momentOf,
  parseTrigger,
  repeatLabel,
  routineTitle,
  secondsUntil,
  TRIGGER_CHOICES,
  toDateField,
  toTimeField,
} from "./routine";

/** A local wall-clock moment, so every assertion reads in the operator's time. */
function at(y: number, m: number, d: number, hour: number, minute = 0): number {
  return new Date(y, m - 1, d, hour, minute, 0, 0).getTime();
}

/** A moment with everything filled in; each test overrides what it is about. */
function moment(over: Partial<Moment> = {}): Moment {
  return { time: "09:00", weekday: 1, monthday: 1, date: "2025-06-10", ...over };
}

describe("trigger vocabulary", () => {
  it("offers a choice for every shape the backend can store on a clock", () => {
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
    // And no event trigger, because nothing delivers an event yet: offering one
    // would be a routine the operator can set and watch never fire.
    expect(TRIGGER_CHOICES.some((c) => c.spec.startsWith("event:"))).toBe(false);
  });

  it("reads every stored form, including the ones no choice offers", () => {
    expect(parseTrigger("weekdays")).toEqual({ kind: "calendar", repeat: "weekdays" });
    expect(parseTrigger("once")).toEqual({ kind: "once" });
    // An agent setting its own schedule works in seconds and picks whatever it
    // likes. The row still has to be legible in the operator's list.
    expect(parseTrigger("every:18000")).toEqual({ kind: "gap", secs: 18_000 });
    expect(repeatLabel("every:18000")).toBe("Every 5 hours");
    expect(repeatLabel("every:90")).toBe("Every 90 seconds");
  });

  it("reads a connector event as the two identifiers it is", () => {
    expect(parseTrigger("event:stripe/invoice.payment_failed")).toEqual({
      kind: "event",
      service: "stripe",
      topic: "invoice.payment_failed",
    });
    expect(repeatLabel("event:stripe/invoice.payment_failed")).toBe(
      "When Stripe reports invoice.payment_failed",
    );
    // Half of one is not one. The Rust parser refuses these, so drawing them
    // as though they worked would promise a routine that cannot exist.
    expect(parseTrigger("event:stripe").kind).toBe("unknown");
    expect(parseTrigger("event:/invoice.paid").kind).toBe("unknown");
  });

  it("draws a trigger from a build that knows more than this one", () => {
    // Forward-only migrations mean a newer build can write a value this one
    // has never heard of. Saying it is better than an empty row.
    expect(repeatLabel("fortnightly")).toBe("fortnightly");
    expect(parseTrigger("every:nonsense").kind).toBe("unknown");
    expect(parseTrigger("every:-1").kind).toBe("unknown");
  });

  it("asks for the part of the moment each trigger actually keeps", () => {
    // The complaint this answers: "every week at 09:00" landed on whichever day
    // the operator happened to be setting it up on, and nothing said so.
    expect(anchorFor("weekly")).toBe("weekday");
    expect(anchorFor("monthly")).toBe("monthday");
    expect(anchorFor("daily")).toBe("time");
    expect(anchorFor("weekdays")).toBe("time");
    // A one-off needs a date: a time alone can only mean the next 24 hours.
    expect(anchorFor("once")).toBe("date");
    // An hourly routine has no hour to name, and an event happens when it does.
    expect(anchorFor("every:3600")).toBe("none");
    expect(anchorFor("event:stripe/invoice.paid")).toBe("none");
  });

  it("says the day a repeat keeps, not just the hour", () => {
    const tuesday = at(2025, 6, 10, 9, 28);
    expect(describeTrigger("weekdays", tuesday)).toBe("Weekdays at 9:28 AM");
    expect(describeTrigger("weekly", tuesday)).toBe("Every Tuesday at 9:28 AM");
    expect(describeTrigger("monthly", tuesday)).toBe("Monthly on the 10th at 9:28 AM");
    expect(describeTrigger("every:3600", tuesday)).toBe("Every hour");
  });

  it("gives a one-off its date, because a time alone does not say when", () => {
    expect(describeTrigger("once", at(2025, 9, 25, 9, 0))).toBe("Once, on Sep 25 at 9:00 AM");
  });

  it("promises no moment for a trigger that holds none", () => {
    // The countdown has nothing to count to, and a date drawn there would be
    // one this side invented.
    expect(describeTrigger("event:stripe/invoice.payment_failed", null)).toBe(
      "When Stripe reports invoice.payment_failed",
    );
  });

  it("round-trips the fields the operator edits", () => {
    expect(toTimeField(at(2025, 6, 10, 9, 5))).toBe("09:05");
    expect(toTimeField(at(2025, 6, 10, 17, 30))).toBe("17:30");
    expect(toDateField(at(2025, 6, 10, 0, 0))).toBe("2025-06-10");
    expect(momentOf(at(2025, 6, 10, 14, 15))).toEqual({
      time: "14:15",
      // 2025-06-10 is a Tuesday, which is 2 the way `Date.getDay` counts.
      weekday: 2,
      monthday: 10,
      date: "2025-06-10",
    });
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

describe("firstRunDelay", () => {
  /** Tuesday 2025-06-10, midday. */
  const tuesday = at(2025, 6, 10, 12, 0);

  const landsOn = (delay: number | null) => new Date(tuesday + (delay ?? 0) * 1000);

  it("anchors a weekly repeat on the weekday that was picked", () => {
    // The weekday is not stored anywhere else: the first firing is the only
    // record of which day a weekly routine keeps.
    const thursday = firstRunDelay("weekly", moment({ weekday: 4, time: "09:00" }), tuesday);
    expect(landsOn(thursday)).toEqual(new Date(at(2025, 6, 12, 9, 0)));

    // Today still counts while its time is ahead, and is next week once it is
    // not: the same rule a time of day already follows.
    expect(
      landsOn(firstRunDelay("weekly", moment({ weekday: 2, time: "17:00" }), tuesday)),
    ).toEqual(new Date(at(2025, 6, 10, 17, 0)));
    expect(
      landsOn(firstRunDelay("weekly", moment({ weekday: 2, time: "09:00" }), tuesday)),
    ).toEqual(new Date(at(2025, 6, 17, 9, 0)));
  });

  it("anchors a monthly repeat on the day of the month that was picked", () => {
    expect(
      landsOn(firstRunDelay("monthly", moment({ monthday: 15, time: "08:00" }), tuesday)),
    ).toEqual(new Date(at(2025, 6, 15, 8, 0)));
    // A day already gone this month is next month's.
    expect(
      landsOn(firstRunDelay("monthly", moment({ monthday: 3, time: "08:00" }), tuesday)),
    ).toEqual(new Date(at(2025, 7, 3, 8, 0)));
  });

  it("takes a monthly 31st to a month that has one rather than clamping", () => {
    // Clamping to the end of a short month would anchor the routine on the
    // 30th, and every firing after it would keep that day: the walk backwards
    // down the calendar the backend is careful to avoid.
    const june = at(2025, 6, 10, 12, 0);
    const landed = new Date(
      june + firstRunDelay("monthly", moment({ monthday: 31 }), june)! * 1000,
    );
    expect(landed).toEqual(new Date(at(2025, 7, 31, 9, 0)));
  });

  it("takes a one-off to the date it was given, not to the next 24 hours", () => {
    const delay = firstRunDelay("once", moment({ date: "2025-09-25", time: "07:30" }), tuesday);
    expect(landsOn(delay)).toEqual(new Date(at(2025, 9, 25, 7, 30)));
  });

  it("refuses a moment that has already passed rather than firing at once", () => {
    // A negative delay reaches the backend as a routine due in the past, which
    // the scheduler fires on its next tick. Saying no is the honest answer.
    expect(
      firstRunDelay("once", moment({ date: "2020-01-01", time: "09:00" }), tuesday),
    ).toBeNull();
    expect(firstRunDelay("once", moment({ date: "nonsense" }), tuesday)).toBeNull();
    expect(firstRunDelay("weekly", moment({ time: "" }), tuesday)).toBeNull();
  });

  it("has nothing to state for a trigger with no moment", () => {
    expect(firstRunDelay("every:3600", moment(), tuesday)).toBeNull();
    expect(firstRunDelay("event:stripe/invoice.paid", moment(), tuesday)).toBeNull();
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
