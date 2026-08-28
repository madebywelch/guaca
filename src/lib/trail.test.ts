import { describe, expect, it } from "vitest";

import {
  callInFlight,
  foldTrail,
  hasDetail,
  readSpent,
  type Step,
  stepDiff,
  stepReason,
  tallyLabel,
  tallyTrail,
  tellsMore,
  trailStep,
} from "./trail";
import type { Part, ToolOutcome } from "./types";

type ToolCall = Extract<Part, { type: "toolCall" }>;

const ok = (summary: string): ToolOutcome => ({ status: "ok", summary });

function call(name: string, args: unknown, outcome: ToolOutcome = ok("done")): ToolCall {
  return { type: "toolCall", name, arguments: args, outcome };
}

/** A memory rewrite, which is the only call that carries what it overwrote. */
function rewrote(content: string, replaced: string): ToolCall {
  return { ...call("update_memory", { content }, ok("Memory saved.")), replaced };
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

  it("tells the two shells apart, because they run on two machines", () => {
    // An agent with a computer and a repository draws both in one turn, and the
    // chip is the only place an operator finds out which filesystem a command
    // touched.
    const [inRepo] = steps(
      call("shell", { command: "git status --short" }, ok("exit 0, 12 bytes out")),
    );
    expect(inRepo?.title).toBe("Ran a command in its repository");
    expect(inRepo?.target).toBe("git status --short");

    const [onSandbox] = steps(call("run_command", { command: "uname -a" }, ok("exit 0")));
    expect(onSandbox?.title).toBe("Ran a command");
  });

  it("names the brief a coding agent was started with", () => {
    const [step] = steps(call("code", { task: "fix the flaky test" }, ok("started work in guaca")));
    expect(step?.title).toBe("Started a coding agent");
    expect(step?.target).toBe("fix the flaky test");
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
      steps(call("update_memory", { content: "Smith handles verification." }, ok("Memory saved."))),
    );
    expect(hasDetail(groups[0]!)).toBe(true);
    expect(groups[0]?.steps[0]?.target).toBe("Smith handles verification.");
  });

  it("opens a memory that was cleared, which has no content and still lost one", () => {
    // Nothing in the arguments to draw, so the rule that reads them alone made
    // this the one memory write the operator could not open: the one where the
    // agent threw the whole page away.
    const groups = foldTrail(steps(rewrote("", "Smith handles verification.")));
    expect(groups[0]?.steps[0]?.target).toBeNull();
    expect(hasDetail(groups[0]!)).toBe(true);
  });
});

describe("what a rewrite changed", () => {
  it("compares against the version the runtime says it replaced", () => {
    const [step] = steps(rewrote("Smith verifies.\nJones signs off.", "Smith verifies."));
    expect(stepDiff(step!)?.map((line) => `${line.kind}:${line.text}`)).toEqual([
      "same:Smith verifies.",
      "added:Jones signs off.",
    ]);
  });

  it("reads an empty previous version as a first memory, not as a missing one", () => {
    // The difference is a falsy string. Read as nothing to compare against, an
    // agent's first memory draws as a page with no history rather than as a
    // page that is all new.
    const [step] = steps(rewrote("Smith verifies.", ""));
    expect(step?.replaced).toBe("");
    expect(stepDiff(step!)).toEqual([{ kind: "added", text: "Smith verifies." }]);
  });

  it("has nothing to compare where the runtime recorded nothing", () => {
    // Every write already in a channel, and every other tool there is.
    const [written] = steps(call("update_memory", { content: "Smith verifies." }));
    expect(stepDiff(written!)).toBeNull();

    const [ran] = steps(call("run_command", { command: "ls" }));
    expect(stepDiff(ran!)).toBeNull();
  });

  it("has nothing to draw where a rewrite cleared a memory that was empty", () => {
    // Both sides empty, so the diff is no lines at all and the panel it would
    // open is a control that opens onto nothing.
    const [step] = steps(rewrote("", ""));
    expect(stepDiff(step!)).toBeNull();
  });
});

describe("a call while it is still happening", () => {
  it("says what it is waiting on in the present tense", () => {
    // `describe` says what a call *was*, which is the wrong tense for the one
    // thing the operator is waiting on: a command still running is not a
    // command that ran.
    expect(callInFlight("run_command", { command: "npm test" })).toBe("Running a command");
    expect(callInFlight("update_memory", { content: "x" })).toBe("Updating its memory");
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
    // one does is how `update_memory` once drew as a message sent to nobody.
    expect(callInFlight("linear__create_issue", {})).toBe("Using linear__create_issue");
  });
});

describe("the memory tool under both of its names", () => {
  it("draws a transcript written before the rename", () => {
    // A tool call is stored as JSON on the message, so every turn recorded
    // while this tool was called `update_notes` still says so. A renderer that
    // knew only the new name would draw a year of history as "Used
    // update_notes", which is the same failure `Part::Approval` refuses to be
    // widened over: old rows cannot be migrated into a new spelling.
    const [old] = steps(call("update_notes", { content: "Smith verifies." }));
    const [current] = steps(call("update_memory", { content: "Smith verifies." }));
    expect(old!.title).toBe("Updated its memory");
    expect(current!.title).toBe(old!.title);
    expect(callInFlight("update_notes", { content: "x" })).toBe("Updating its memory");
  });
});

describe("a progress note", () => {
  it("says what it is rather than naming the tool", () => {
    const [step] = steps(call("note_progress", { note: "waiting on the legal read" }));
    expect(step!.title).toBe("Noted where its work stands");
    expect(step!.target).toBe("waiting on the legal read");
    expect(callInFlight("note_progress", { note: "x" })).toBe("Noting where its work stands");
  });

  it("adds nothing beside a chip that already said it", () => {
    // "Noted. You have 3 of 16 working notes" is a number the model needs and
    // the operator can already read off the panel that lists them. Noting is
    // meant to be cheap and frequent, so its chip has to stay one line.
    const [step] = steps(
      call("note_progress", { note: "waiting" }, ok("Noted. You have 3 of 16 working notes.")),
    );
    expect(tellsMore(step!)).toBe(false);
  });

  it("shows what went wrong when one could not be saved", () => {
    // A failure is never an echo: whatever broke is not something the title
    // could have said.
    const [step] = steps(
      call("note_progress", { note: "waiting" }, { status: "failed", error: "database is locked" }),
    );
    expect(tellsMore(step!)).toBe(true);
  });
});

describe("what the turn has done, on one line", () => {
  it("counts the calls that came back, and says nothing about their kind", () => {
    // The kinds are what the chips behind the count are for. A line naming the
    // first of several is a line that is wrong about the rest.
    const tally = tallyTrail(
      steps(
        call("browse", { action: "read" }),
        call("run_command", { command: "ls" }, ok("exit 0")),
      ),
    );
    expect(tally).toEqual({ done: 2, failed: 0, spent: [] });
    expect(tallyLabel(tally)).toBe("2 steps");
  });

  it("says how many went wrong, because that is the part worth interrupting for", () => {
    const tally = tallyTrail(
      steps(
        call("browse", { action: "read" }),
        call("run_command", { command: "boom" }, { status: "failed", error: "no machine" }),
        call(
          "browse",
          { action: "open", url: "https://example.com" },
          {
            status: "refused",
            reason: "not given a browser",
          },
        ),
      ),
    );
    expect(tally.failed).toBe(2);
    expect(tallyLabel(tally)).toBe("3 steps, 2 failed");
  });

  it("counts one call as a step rather than as steps", () => {
    expect(tallyLabel(tallyTrail(steps(call("directory", {}, ok("2 agent(s): Chef")))))).toBe(
      "1 step",
    );
  });

  it("names every credential the turn spent, once each", () => {
    // The operator's audit trail for their own tokens, which is the one part
    // of the trail that stays on the line while the turn runs. A name spent
    // twice is one credential, not two.
    const tally = tallyTrail(
      steps(
        call("run_command", { command: "one" }, ok("used Mistral ($MISTRAL_API_KEY) · exit 0")),
        call("run_command", { command: "two" }, ok("used Mistral ($MISTRAL_API_KEY) · exit 0")),
        call("run_command", { command: "three" }, ok("used Stripe ($STRIPE_KEY) · exit 0")),
      ),
    );
    expect(tally.spent).toEqual(["Mistral ($MISTRAL_API_KEY)", "Stripe ($STRIPE_KEY)"]);
  });
});

describe("a call that went wrong", () => {
  it("opens on its reason, where the reason is the whole of what happened", () => {
    // Clipped on the chip, because a refusal is written to be acted on and
    // runs to a paragraph: `U… a coding agent is already working in…` is a row
    // saying one character about which call it was.
    const groups = foldTrail(
      steps(
        call(
          "browse",
          { action: "read" },
          {
            status: "refused",
            reason:
              "a coding agent is already working in whizzworks-site, started by Content Marketer.",
          },
        ),
      ),
    );
    expect(hasDetail(groups[0]!)).toBe(true);
    expect(stepReason(groups[0]!.steps[0]!)).toContain("already working in whizzworks-site");
  });

  it("opens on what it acted on where there is one, not on the reason", () => {
    // Somebody opening a failed command came for the command.
    const [step] = steps(
      call("run_command", { command: "npm test" }, { status: "failed", error: "no machine" }),
    );
    expect(stepReason(step!)).toBeNull();
    expect(step?.target).toBe("npm test");
  });

  it("says nothing extra about a call that worked", () => {
    const [step] = steps(call("directory", {}, ok("2 agent(s): Chef, Scribe")));
    expect(stepReason(step!)).toBeNull();
  });
});
