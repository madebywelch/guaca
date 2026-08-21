import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  away,
  burst,
  classOf,
  type Moment,
  markQuiet,
  QUIET_MS,
  quiet,
  resetBurst,
  resetQuiet,
  shouldNotify,
} from "./notify";
import { NOTIFY_KINDS, type NotifyKind, type NotifyPrefs } from "./prefs";

/**
 * The gates a notification has to pass, read from the refusals inward.
 *
 * Every interesting assertion in this file is about silence. Raising a
 * notification is one call; the work is declining to, and each way of getting
 * that wrong costs the operator the whole feature, because nobody who turns
 * notifications off turns them back on again. A badge for a routine that fired
 * in the channel they were watching, a second badge for the same failure, a
 * weekend of overdue schedule announced at launch: any one of those and the
 * switch goes off for good.
 *
 * The judgement is in `Moment`. "Away" is graded, not binary, so the same three
 * booleans mean three different things depending on the kind, and the class
 * table is the only thing that says which. A kind that quietly changes class is
 * a permission request nobody is told about, or a settled run announced from a
 * channel that has never been opened.
 *
 * `away` itself is two conditions rather than one. A window sitting behind a
 * browser on the same screen is visible and unfocused, which is exactly the
 * case worth interrupting for and the one `document.hidden` misses.
 */

/** The master switch on and all four kinds wanted: prefs are never the refusal. */
function wanted(): NotifyPrefs {
  return {
    on: true,
    kinds: { approval: true, routine: true, settled: true, failed: true },
  };
}

/** Everything wanted but one, so that kind's own switch is the only refusal. */
function except(kind: NotifyKind): NotifyPrefs {
  const prefs = wanted();
  prefs.kinds[kind] = false;
  return prefs;
}

/**
 * The moment that lets every class through: away, looking at the channel it
 * concerns, past the quiet window. Each test overrides only what should refuse.
 */
function moment(over: Partial<Moment> = {}): Moment {
  return { away: true, onScreen: true, quiet: false, ...over };
}

/** A launch, in wall-clock milliseconds, so the quiet window has a real edge. */
const LAUNCH = 1_756_000_000_000;

beforeEach(() => {
  // Both windows are module state that outlives a render, and therefore a test.
  resetQuiet();
  resetBurst();
});

describe("the switches", () => {
  it("says nothing at all when the master switch is off, whatever the kinds ask for", () => {
    for (const kind of NOTIFY_KINDS) {
      expect(shouldNotify(kind, { ...wanted(), on: false }, moment())).toBe(false);
    }
  });

  it("refuses a kind the operator switched off while the master switch is still on", () => {
    // The dialog leaves the four kind switches enabled and readable with the
    // master on, so the kind's own answer has to be the one that counts.
    for (const kind of NOTIFY_KINDS) {
      expect(shouldNotify(kind, except(kind), moment())).toBe(false);
    }
  });

  it("holds one kind back without silencing the other three", () => {
    expect(shouldNotify("approval", except("routine"), moment())).toBe(true);
    expect(shouldNotify("routine", except("routine"), moment())).toBe(false);
  });
});

describe("the quiet window after a launch", () => {
  it("refuses every kind inside it, however much the moment favours them", () => {
    markQuiet(LAUNCH);
    // Away, on the channel, and wanted: the one moment where all three classes
    // fire. A routine whose slot passed over a weekend arrives overdue on the
    // first tick, and a weekend of schedule at once is why this gate outranks
    // the rest of them.
    for (const kind of NOTIFY_KINDS) {
      expect(shouldNotify(kind, wanted(), moment({ quiet: quiet(LAUNCH + QUIET_MS - 1) }))).toBe(
        false,
      );
    }
  });

  it("stops holding anything back the instant the window is up", () => {
    markQuiet(LAUNCH);
    expect(quiet(LAUNCH + QUIET_MS - 1)).toBe(true);
    expect(quiet(LAUNCH + QUIET_MS)).toBe(false);
    expect(shouldNotify("approval", wanted(), moment({ quiet: quiet(LAUNCH + QUIET_MS) }))).toBe(
      true,
    );
  });

  it("is not open before a launch has been marked", () => {
    // The mark lands when the first read of state does. Anything arriving
    // before that has to reach the operator, not fall into a window that was
    // opened by module load.
    expect(quiet(LAUNCH)).toBe(false);
  });
});

describe("the class of news a kind is", () => {
  it("makes a permission request the only thing that can break through", () => {
    expect(classOf("approval")).toBe("attention");
  });

  it("treats a schedule firing as ambient, because it was pointed somewhere else", () => {
    expect(classOf("routine")).toBe("ambient");
  });

  it("treats both ends of a run as completion, success or failure", () => {
    // A failure is still the end of something the operator started. It gets no
    // wider a hearing than a settle: a busy runtime fails runs in channels
    // nobody has opened.
    expect(classOf("settled")).toBe("completion");
    expect(classOf("failed")).toBe("completion");
  });
});

describe("attention", () => {
  it("stays quiet when the operator is watching the channel that is asking", () => {
    expect(shouldNotify("approval", wanted(), moment({ away: false, onScreen: true }))).toBe(false);
  });

  it("reaches an operator who is elsewhere, even from the channel on screen", () => {
    expect(shouldNotify("approval", wanted(), moment({ away: true, onScreen: true }))).toBe(true);
  });

  it("breaks through a focused window when the request is on a channel that is not on screen", () => {
    // The one rule that survives a window the operator is looking at. Nobody
    // finds a parked turn by noticing a row change colour three screens up the
    // rail, and the turn stays parked until it is answered.
    expect(shouldNotify("approval", wanted(), moment({ away: false, onScreen: false }))).toBe(true);
  });
});

describe("ambient", () => {
  it("stays quiet while the operator is at the window, on screen or not", () => {
    expect(shouldNotify("routine", wanted(), moment({ away: false, onScreen: true }))).toBe(false);
    expect(shouldNotify("routine", wanted(), moment({ away: false, onScreen: false }))).toBe(false);
  });

  it("reaches an operator who is away without asking which channel is on screen", () => {
    // A routine fires wherever it was pointed and `announcementFor` hands it no
    // channel, so `onScreen` is whatever the caller happened to compute. Read
    // it here and half of every schedule goes missing.
    expect(shouldNotify("routine", wanted(), moment({ away: true, onScreen: true }))).toBe(true);
    expect(shouldNotify("routine", wanted(), moment({ away: true, onScreen: false }))).toBe(true);
  });
});

describe("completion", () => {
  it("stays quiet about a run in a channel the operator was not looking at", () => {
    // The gate that keeps a busy runtime from announcing work nobody opened.
    for (const kind of ["settled", "failed"] as const) {
      expect(shouldNotify(kind, wanted(), moment({ away: true, onScreen: false }))).toBe(false);
    }
  });

  it("stays quiet while the operator is at the window watching it happen", () => {
    for (const kind of ["settled", "failed"] as const) {
      expect(shouldNotify(kind, wanted(), moment({ away: false, onScreen: true }))).toBe(false);
    }
  });

  it("reaches an operator who stepped away from the conversation they were waiting on", () => {
    for (const kind of ["settled", "failed"] as const) {
      expect(shouldNotify(kind, wanted(), moment({ away: true, onScreen: true }))).toBe(true);
    }
  });
});

describe("away", () => {
  const realHidden = Object.getOwnPropertyDescriptor(document, "hidden");

  /** Both are inherited getters in jsdom, so an own property shadows them. */
  function put(where: { hidden: boolean; focused?: boolean }): void {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => where.hidden });
    if (where.focused !== undefined) {
      document.hasFocus = vi.fn(() => where.focused === true);
    }
  }

  afterEach(() => {
    if (realHidden) Object.defineProperty(document, "hidden", realHidden);
    else Reflect.deleteProperty(document, "hidden");
    Reflect.deleteProperty(document, "hasFocus");
  });

  it("does not claim a webview without hasFocus is away", () => {
    // A host that does not implement it would otherwise throw here, on the
    // first event of the session, inside a subscription made once.
    Object.defineProperty(document, "hasFocus", { configurable: true, value: undefined });
    put({ hidden: false });
    expect(away()).toBe(false);
  });

  it("is false with the window visible and focused, which is the operator watching", () => {
    put({ hidden: false, focused: true });
    expect(away()).toBe(false);
  });

  it("is true for a window that is visible but not focused", () => {
    // The alt-tabbed case: a Guaca window uncovered on the same screen as the
    // browser being typed into. `document.hidden` never flips for it, and it is
    // the case the whole mechanism exists for.
    put({ hidden: false, focused: false });
    expect(away()).toBe(true);
  });

  it("is true for a minimised or fully covered window without consulting focus", () => {
    const hasFocus = vi.fn(() => true);
    put({ hidden: true });
    document.hasFocus = hasFocus;
    expect(away()).toBe(true);
    expect(hasFocus).not.toHaveBeenCalled();
  });
});

describe("the burst window", () => {
  it("says the same thing once, not twice, inside the same second", () => {
    expect(burst("failed:Cook", LAUNCH)).toBe(false);
    expect(burst("failed:Cook", LAUNCH + 200)).toBe(true);
  });

  it("does not let a repeat extend its own silence", () => {
    // The window runs from what was said, not from the last thing suppressed. A
    // stream retrying just inside the second would otherwise be muted forever.
    expect(burst("failed:Cook", LAUNCH)).toBe(false);
    expect(burst("failed:Cook", LAUNCH + 900)).toBe(true);
    expect(burst("failed:Cook", LAUNCH + 1_000)).toBe(false);
  });

  it("treats two agents failing at once as two things worth saying", () => {
    expect(burst("failed:Cook", LAUNCH)).toBe(false);
    expect(burst("failed:Scribe", LAUNCH)).toBe(false);
    expect(burst("approval:Cook", LAUNCH)).toBe(false);
  });

  it("forgets a key once its second is up, which is all that keeps the map bounded", () => {
    expect(burst("failed:Cook", LAUNCH)).toBe(false);
    // Chatter under other keys must not keep the first key alive, and must not
    // outlive its own second either.
    expect(burst("failed:Scribe", LAUNCH + 400)).toBe(false);
    expect(burst("failed:Cook", LAUNCH + 1_000)).toBe(false);
    expect(burst("failed:Scribe", LAUNCH + 1_500)).toBe(false);
    // The entry the prune replaced is the current one, so the reprieve is not
    // permanent: this is still the same thing said twice in a second.
    expect(burst("failed:Cook", LAUNCH + 1_500)).toBe(true);
  });
});
