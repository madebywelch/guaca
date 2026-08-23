import { describe, expect, it } from "vitest";

import {
  callInFlight,
  foldTrail,
  hasDetail,
  type LiveCall,
  liveStep,
  readSpent,
  type Step,
  tellsMore,
  trailStep,
} from "./trail";
import type { Part, ToolOutcome } from "./types";

type ToolCall = Extract<Part, { type: "toolCall" }>;

const ok = (summary: string): ToolOutcome => ({ status: "ok", summary });

function call(name: string, args: unknown, outcome: ToolOutcome = ok("done")): ToolCall {
  return { type: "toolCall", name, arguments: args, outcome };
}

function steps(...calls: ToolCall[]): Step[] {
  return calls.map((call, position) => trailStep(call, `m1:${position}`));
}

describe("what one call was", () => {
  it("names the site a browser was pointed at, not the tool", () => {
    // `used browse` is the name of a function in a file the operator does not
    // have. Where it went is the thing they opened the channel to find out.
    const [step] = steps(call("browse", { action: "open", url: "https://www.cnn.com/world" }));
    expect(step?.title).toBe("Opened cnn.com");
    expect(step?.target).toBe("https://www.cnn.com/world");
  });

  it("shows a url that will not parse exactly as it was written", () => {
    // An agent that browsed to `cnn.com` browsed to cnn.com. Reporting nothing
    // because `new URL` threw describes a different session.
    const [step] = steps(call("browse", { action: "open", url: "cnn.com" }));
    expect(step?.title).toBe("Opened cnn.com");
  });

  it("keeps the command, which is the only interesting part of running one", () => {
    const [step] = steps(
      call("run_command", { command: "curl -s wttr.in" }, ok("exit 0, 8 bytes out")),
    );
    expect(step?.title).toBe("Ran a command");
    expect(step?.target).toBe("curl -s wttr.in");
    expect(step?.said).toBe("exit 0, 8 bytes out");
  });

  it("reads a screen action as the place it happened", () => {
    const [step] = steps(call("use_screen", { action: "click", x: 412, y: 96 }));
    expect(step?.title).toBe("Clicked on its screen");
    expect(step?.target).toBe("412, 96");
  });

  it("names a tool it has never heard of rather than guessing", () => {
    const [step] = steps(call("run_code", { source: "print(1)" }, ok("exit 0")));
    expect(step?.title).toBe("Used run_code");
    expect(step?.target).toBeNull();
  });

  it("names the document that was attached, since the operator can see the card", () => {
    // The file itself is drawn under the message. The chip's job is to say
    // which call put it there, and the path it came from is on a machine the
    // operator has never seen.
    const [step] = steps(
      call("attach_file", { files: ["/home/user/exec-brief.md"] }, ok("attached exec-brief.md")),
    );
    expect(step?.title).toBe("Attached exec-brief.md");
    // Nothing behind the chip: the summary is the title again, and the document
    // is already on screen.
    expect(step?.target).toBeNull();
    expect(tellsMore(step as Step)).toBe(false);
  });

  it("says what could not be attached, because that is not on screen anywhere", () => {
    const [step] = steps(
      call(
        "attach_file",
        { files: ["/home/user/brief.md"] },
        { status: "refused", reason: "brief.md was not attached: there is no file at it." },
      ),
    );
    expect(step?.failed).toBe(true);
    // A failure is never an echo. Whatever went wrong is not something the
    // title could have said.
    expect(tellsMore(step as Step)).toBe(true);
    expect(step?.said).toContain("was not attached");
  });

  it("carries the reason a call was refused, not the fact that it happened", () => {
    const [step] = steps(
      call(
        "send_message",
        { text: "hello?" },
        { status: "refused", reason: "Refused: name a recipient." },
      ),
    );
    expect(step?.failed).toBe(true);
    expect(step?.said).toBe("Refused: name a recipient.");
  });
});

describe("credentials a command spent", () => {
  // Mirrors `credentials_named_in` in `runtime/mod.rs`. If that wording changes
  // this test is where it is noticed, rather than a transcript quietly losing
  // the operator's audit trail for their own tokens.
  it("reads the names the runtime wrote, and leaves the rest of the summary", () => {
    const read = readSpent("used Mistral ($MISTRAL_API_KEY) · exit 0, 812 bytes out");
    expect(read.spent).toEqual(["Mistral ($MISTRAL_API_KEY)"]);
    expect(read.rest).toBe("exit 0, 812 bytes out");
  });

  it("reads several", () => {
    const read = readSpent("used Mistral ($A), GitHub ($B) · exit 0, 4 bytes out");
    expect(read.spent).toEqual(["Mistral ($A)", "GitHub ($B)"]);
  });

  it("leaves an ordinary summary alone", () => {
    expect(readSpent("exit 0, 812 bytes out")).toEqual({
      spent: [],
      rest: "exit 0, 812 bytes out",
    });
  });

  it("never lets one be folded into a count", () => {
    // Two commands would ordinarily read as "Ran 2 commands". The one that
    // spent a token keeps its own chip, because that is the row the operator
    // audits their own credentials from.
    const groups = foldTrail(
      steps(
        call("run_command", { command: "ls" }, ok("exit 0, 4 bytes out")),
        call("run_command", { command: "curl $X" }, ok("used Stripe ($X) · exit 0, 9 bytes out")),
      ),
    );
    expect(groups).toHaveLength(2);
    expect(groups[1]?.spent).toEqual(["Stripe ($X)"]);
  });
});

describe("folding a turn's work", () => {
  it("says what a single call was, in full", () => {
    const groups = foldTrail(steps(call("run_command", { command: "ls" }, ok("exit 0"))));
    expect(groups.map((group) => group.label)).toEqual(["Ran a command"]);
  });

  it("counts several of one kind instead of listing them", () => {
    // The row this replaced: twenty-four lines of `Chef used browse` between
    // the operator's question and the answer to it.
    const groups = foldTrail(
      steps(
        call("run_command", { command: "ls" }, ok("exit 0")),
        call("run_command", { command: "pwd" }, ok("exit 0")),
        call("run_command", { command: "whoami" }, ok("exit 0")),
      ),
    );
    expect(groups).toHaveLength(1);
    expect(groups[0]?.label).toBe("Ran 3 commands");
    expect(groups[0]?.steps).toHaveLength(3);
  });

  it("names the site when a browsing turn only visited one", () => {
    // A count that names nothing hides what the operator opened the channel to
    // find out, which is the same reason a burst draws one chip per peer.
    const groups = foldTrail(
      steps(
        call("browse", { action: "open", url: "https://cnn.com" }),
        call("browse", { action: "read" }),
        call("browse", { action: "click", id: 12 }),
      ),
    );
    expect(groups[0]?.label).toBe("3 steps on cnn.com");
  });

  it("names the site from the step and not from the words on the chip", () => {
    // The label used to be recovered by slicing the title back apart, so
    // rewording "Opened cnn.com" silently cost the group its name.
    const [step] = steps(call("browse", { action: "open", url: "https://cnn.com/world" }));
    expect(step?.where).toBe("cnn.com");
    expect(steps(call("browse", { action: "read" }))[0]?.where).toBeNull();
  });

  it("does not name a site when the turn moved between several", () => {
    const groups = foldTrail(
      steps(
        call("browse", { action: "open", url: "https://cnn.com" }),
        call("browse", { action: "open", url: "https://bbc.co.uk" }),
      ),
    );
    expect(groups[0]?.label).toBe("2 steps in the browser");
  });

  it("gathers calls of one kind that a call of another came between", () => {
    // What the turn did, rather than the order a model happened to interleave
    // its tools in, which is not something anybody asked about.
    const groups = foldTrail(
      steps(
        call("browse", { action: "read" }),
        call("run_command", { command: "ls" }, ok("exit 0")),
        call("browse", { action: "read" }),
      ),
    );
    expect(groups.map((group) => group.label)).toEqual(["2 steps in the browser", "Ran a command"]);
  });

  it("keeps a failure out of the count, in the place it happened", () => {
    const groups = foldTrail(
      steps(
        call("run_command", { command: "ls" }, ok("exit 0")),
        call("run_command", { command: "boom" }, { status: "failed", error: "no machine" }),
        call("run_command", { command: "pwd" }, ok("exit 0")),
      ),
    );
    expect(groups.map((group) => group.label)).toEqual(["Ran 2 commands", "Ran a command"]);
    expect(groups[1]?.failed).toBe(true);
    expect(groups[1]?.steps[0]?.said).toBe("no machine");
  });
});

describe("whether a chip opens anything", () => {
  it("does not offer a control over a lookup with nothing behind it", () => {
    // A button that opens nothing is one the operator stops trusting the rest
    // of.
    const groups = foldTrail(steps(call("directory", {}, ok("2 agent(s): Chef, Scribe"))));
    expect(hasDetail(groups[0]!)).toBe(false);
  });

  it("opens a command, because the command is not on the chip", () => {
    const groups = foldTrail(steps(call("run_command", { command: "ls" }, ok("exit 0"))));
    expect(hasDetail(groups[0]!)).toBe(true);
  });

  it("opens what an agent wrote to its own memory", () => {
    const groups = foldTrail(
      steps(call("update_notes", { content: "Smith handles verification." }, ok("Memory saved."))),
    );
    expect(hasDetail(groups[0]!)).toBe(true);
    expect(groups[0]?.steps[0]?.target).toBe("Smith handles verification.");
  });
});

describe("a call while it is still happening", () => {
  const live = (name: string, args: unknown): LiveCall => ({
    callId: "call_1",
    name,
    arguments: args,
    outcome: null,
    startedAt: 0,
  });

  it("draws the chip the message will draw, from the same rules", () => {
    // The point of the live trail: what the operator watches accumulate during
    // a turn is the same chip the transcript holds afterwards. Two sets of
    // rules would be two things to be wrong about one call.
    const held = live("browse", { action: "open", url: "https://www.cnn.com/world" });
    const drawn = liveStep(held, ok("read cnn.com"), "live:0");
    const recorded = trailStep(
      { type: "toolCall", name: held.name, arguments: held.arguments, outcome: ok("read cnn.com") },
      "live:0",
    );
    expect(drawn).toEqual(recorded);
  });

  it("says what it is waiting on in the present tense", () => {
    // `describe` says what a call *was*, which is the wrong tense for the one
    // thing the operator is waiting on: a command still running is not a
    // command that ran.
    expect(callInFlight("run_command", { command: "npm test" })).toBe("Running a command");
    expect(callInFlight("update_notes", { content: "x" })).toBe("Updating its memory");
  });

  it("names the site a page is being opened at, because that is the wait", () => {
    expect(callInFlight("browse", { action: "open", url: "https://www.cnn.com/world" })).toBe(
      "Opening cnn.com",
    );
    // Anything else on that browser is the browser, which is what is being
    // waited on and all the operator needs to know about it.
    expect(callInFlight("browse", { action: "click", id: 4 })).toBe("Working the browser");
  });

  it("names a tool this build has never heard of after the tool", () => {
    // A crew's plugin tools are named by that crew's servers. Guessing at what
    // one does is how `update_notes` once drew as a message sent to nobody.
    expect(callInFlight("linear__create_issue", {})).toBe("Using linear__create_issue");
  });
});
