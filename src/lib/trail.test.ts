import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  attempt,
  callInFlight,
  foldTrail,
  hasDetail,
  type LiveCall,
  machineInUse,
  readSpent,
  type Step,
  saysMore,
  stepDiff,
  stepReason,
  tallyLabel,
  tallyTrail,
  tellsMore,
  trailStep,
} from "./trail";
import type { Part, ToolOutcome } from "./types";

const read = (path: string) => readFileSync(join(process.cwd(), path), "utf8");

type ToolCall = Extract<Part, { type: "toolCall" }>;

const ok = (summary: string): ToolOutcome => ({ status: "ok", summary });

function call(name: string, args: unknown, outcome: ToolOutcome = ok("done")): ToolCall {
  return { type: "toolCall", name, arguments: args, outcome };
}

function rewrote(content: string, replaced: string): ToolCall {
  return { ...call("update_memory", { content }, ok("Memory saved.")), replaced };
}

function steps(...calls: ToolCall[]): Step[] {
  return calls.map((call, position) => trailStep(call, `m1:${position}`));
}

describe("what one call was", () => {
  it("names the site a browser was pointed at, not the tool", () => {
    const [step] = steps(call("browse", { action: "open", url: "https://www.cnn.com/world" }));
    expect(step?.title).toBe("Opened cnn.com");
    expect(step?.target).toBe("https://www.cnn.com/world");
  });

  it("names what a turn searched for and the site of a page it read", () => {
    const [search, page] = steps(
      call("web_search", { query: "lease or buy" }),
      call("read_page", { url: "https://www.edmunds.com/car-leasing/" }),
    );
    expect(search?.title).toBe("Searched the web for lease or buy");
    expect(search?.target).toBe("lease or buy");
    expect(page?.title).toBe("Read edmunds.com");
    expect(page?.where).toBe("edmunds.com");
    expect(callInFlight("read_page", { url: "https://www.edmunds.com/a" })).toBe(
      "Reading edmunds.com",
    );
    expect(callInFlight("web_search", { query: "x" })).toBe("Searching the web");
  });

  it("names the first sum a turn worked out, and says it is working one out while it runs", () => {
    const [one, several, bare] = steps(
      call("calculator", { expression: "500 * 36 + 3000" }),
      call("calculator", { expression: "rate = 6% / 12\npayment = 35000 * rate" }),
      call("calculator", {}),
    );
    expect(one?.title).toBe("Worked out 500 * 36 + 3000");
    expect(one?.target).toBe("500 * 36 + 3000");
    expect(several?.title).toBe("Worked out rate = 6% / 12");
    expect(bare?.title).toBe("Worked out a sum");
    expect(callInFlight("calculator", { expression: "1 + 1" })).toBe("Working out a sum");
  });

  it("shows a url that will not parse exactly as it was written", () => {
    const [step] = steps(call("browse", { action: "open", url: "cnn.com" }));
    expect(step?.title).toBe("Opened cnn.com");
  });

  it("names the skill a turn read or wrote, which is what the chip is opened for", () => {
    const [read, wrote, listed] = steps(
      call("skill", { action: "view", name: "release-notes" }),
      call("skill", { action: "write", name: "deploy", description: "When deploying" }),
      call("skill", { action: "list" }),
    );
    expect(read?.title).toBe("Read the release-notes skill");
    expect(wrote?.title).toBe("Wrote the deploy skill");
    expect(wrote?.target).toBe("When deploying");
    expect(listed?.title).toBe("Checked its skills");
    expect(callInFlight("skill", {})).toBe("Reading its skills");
  });

  it("names the notebook file a turn touched", () => {
    const [wrote, added, read] = steps(
      call("notebook", { action: "write", path: "leads/acme.md", content: "x" }),
      call("notebook", { action: "append", path: "log.md", content: "- tried it" }),
      call("notebook", { action: "read", path: "log.md" }),
    );
    expect(wrote?.title).toBe("Wrote leads/acme.md in its notebook");
    expect(added?.title).toBe("Added to log.md in its notebook");
    expect(added?.target).toBe("- tried it");
    expect(read?.title).toBe("Read log.md from its notebook");
    expect(callInFlight("notebook", {})).toBe("Working in its notebook");
  });

  it("keeps the command, which is the only interesting part of running one", () => {
    const [step] = steps(
      call("run_command", { command: "curl -s wttr.in" }, ok("exit 0, 8 bytes out")),
    );
    expect(step?.title).toBe("Ran a command");
    expect(step?.target).toBe("curl -s wttr.in");
    expect(step?.said).toBe("exit 0, 8 bytes out");
  });

  it("names how many errands were sent and opens onto their briefs and answers", () => {
    const [two] = steps(
      call(
        "errand",
        { briefs: ["Count the north office.", "Count the south office."] },
        ok("2 errands: 2 finished\n\n1. Its answer:\nNorth office: 12 staff."),
      ),
    );
    expect(two?.title).toBe("Sent 2 errands");
    expect(two?.target).toBe("Count the north office.\n\nCount the south office.");
    expect(tellsMore(two!), "what they came back with is behind the click").toBe(true);

    const [one] = steps(call("errand", { briefs: "Read the pricing page." }));
    expect(one?.title).toBe("Sent an errand");
    expect(one?.target).toBe("Read the pricing page.");
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
    const [step] = steps(
      call("attach_file", { files: ["/home/user/exec-brief.md"] }, ok("attached exec-brief.md")),
    );
    expect(step?.title).toBe("Attached exec-brief.md");
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

  it("says a refusal written for the model in words for the person, and keeps the model's", () => {
    const [step] = steps(
      call(
        "send_message",
        { text: "hello?" },
        {
          status: "refused",
          reason:
            "Refused: no agent named Zed exists. Call `directory` to see who is actually available.",
        },
      ),
    );
    expect(step?.said).toBe("Nothing was sent: no agent in this group is called Zed.");
    expect(step?.told).toContain("`directory`");
    expect(stepReason(step as Step)).toBe(step?.said);
  });

  it("leaves a refusal it cannot translate exactly as the runtime wrote it", () => {
    const said = "Refused: the archive you named is read-only for this session.";
    const [step] = steps(call("send_message", { text: "hi" }, { status: "refused", reason: said }));
    expect(step?.said).toBe(said);
    expect(step?.told).toBeNull();
  });
});

describe("a call that failed, said as an attempt", () => {
  it("does not put the successful past tense on work that did not happen", () => {
    const [step] = steps(
      call(
        "schedule",
        { action: "create", name: "Morning market note" },
        { status: "failed", error: "a routine with that name already exists" },
      ),
    );
    expect(step?.title).toBe("Tried to schedule Morning market note");
    expect(step?.failed).toBe(true);
    expect(step?.said).toContain("already exists");
  });

  it("keeps the title through the words in front of it", () => {
    expect(attempt("Ran a command")).toBe("Tried to run a command");
    expect(attempt("Read the page")).toBe("Tried to read the page");
    expect(attempt("Went back a page")).toBe("Tried to go back a page");
    expect(attempt("Sent an errand")).toBe("Tried to send an errand");
  });

  it("leaves a successful call in the past tense", () => {
    const [step] = steps(call("run_command", { command: "make" }, ok("exit 0")));
    expect(step?.title).toBe("Ran a command");
  });

  it("says a verb it does not know as a failure rather than rewriting it", () => {
    expect(attempt("Brewed the coffee")).toBe("Failed: Brewed the coffee");
  });
});

describe("credentials a command spent", () => {
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
  it("calls an agent's work on what it asked a question, live and folded", () => {
    expect(callInFlight("decision", {})).toBe("Working on a question");
    const groups = foldTrail(
      steps(call("decision", { action: "list" }), call("decision", { action: "list" })),
    );
    expect(groups.map((group) => group.label)).toEqual(["Worked on its questions 2 times"]);
  });

  it("keeps each ruled call visible with the rule as written", () => {
    const ruled = { ...call("run_command", { command: "ls" }), ruled: "Allow reports" };
    const groups = foldTrail(steps(call("run_command", {}), ruled, ruled, call("run_command", {})));
    expect(groups.map((group) => group.label)).toEqual([
      "Ran 2 commands",
      "Allowed by a rule · Allow reports",
      "Allowed by a rule · Allow reports",
    ]);
    expect(groups[1]?.steps[0]?.target).toBe("ls");
  });

  it("does not repeat the rule allowance beside its label", () => {
    const [step] = steps({
      ...call("request_permission", {}, ok("allowed by a rule")),
      ruled: "Allow reports",
    });
    expect(saysMore(step!)).toBe(false);
  });

  it("says what a single call was, in full", () => {
    const groups = foldTrail(steps(call("run_command", { command: "ls" }, ok("exit 0"))));
    expect(groups.map((group) => group.label)).toEqual(["Ran a command"]);
  });

  it("counts several of one kind instead of listing them", () => {
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

  it("counts the sums a turn worked out", () => {
    const groups = foldTrail(
      steps(
        call("calculator", { expression: "1 + 1" }),
        call("calculator", { expression: "2 * 3" }),
      ),
    );
    expect(groups[0]?.label).toBe("Worked out 2 sums");
  });

  it("gathers calls of one kind that a call of another came between", () => {
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
    expect(groups.map((group) => group.label)).toEqual([
      "Ran 2 commands",
      "Tried to run a command",
    ]);
    expect(groups[1]?.failed).toBe(true);
    expect(groups[1]?.steps[0]?.said).toBe("no machine");
  });
});

describe("whether a chip opens anything", () => {
  it("does not offer a control over a lookup with nothing behind it", () => {
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
    const [step] = steps(rewrote("Smith verifies.", ""));
    expect(step?.replaced).toBe("");
    expect(stepDiff(step!)).toEqual([{ kind: "added", text: "Smith verifies." }]);
  });

  it("has nothing to compare where the runtime recorded nothing", () => {
    const [written] = steps(call("update_memory", { content: "Smith verifies." }));
    expect(stepDiff(written!)).toBeNull();

    const [ran] = steps(call("run_command", { command: "ls" }));
    expect(stepDiff(ran!)).toBeNull();
  });

  it("has nothing to draw where a rewrite cleared a memory that was empty", () => {
    const [step] = steps(rewrote("", ""));
    expect(stepDiff(step!)).toBeNull();
  });
});

describe("a call while it is still happening", () => {
  it("says what it is waiting on in the present tense", () => {
    expect(callInFlight("run_command", { command: "npm test" })).toBe("Running a command");
  });

  it("says which machine a turn is on, off the call that has not come back", () => {
    const live = (name: string, done: Part | null = null) =>
      ({ callId: name, name, arguments: {}, done, startedAt: 0 }) as LiveCall;
    const finished = { type: "toolCall", name: "browse", arguments: {}, outcome: { kind: "ok" } };
    expect(machineInUse(undefined)).toBeNull();
    expect(machineInUse([live("send_message"), live("use_screen")])).toBe("computer");
    expect(machineInUse([live("run_command")])).toBe("computer");
    expect(machineInUse([live("open_on_desktop")])).toBe("computer");
    expect(machineInUse([live("browse")])).toBe("browser");
    expect(machineInUse([live("browse", finished as unknown as Part)])).toBeNull();
    expect(machineInUse([live("shell"), live("code")])).toBeNull();
    expect(callInFlight("update_memory", { content: "x" })).toBe("Updating its memory");
  });

  it("names the site a page is being opened at, because that is the wait", () => {
    expect(callInFlight("browse", { action: "open", url: "https://www.cnn.com/world" })).toBe(
      "Opening cnn.com",
    );
    expect(callInFlight("browse", { action: "click", id: 4 })).toBe("Working the browser");
  });

  it("names the server a plugin call is waiting on, and the tool it called", () => {
    expect(callInFlight("linear__create_issue", {})).toBe("Using Create issue on Linear");
    expect(callInFlight("run_code", { source: "print(1)" })).toBe("Using run_code");
  });
});

describe("a call to a plugin", () => {
  const google = (tool: string) => call(`google__${tool}`, {}, ok(`Google · ${tool}`));

  it("says which server the work went to, without the name a model calls it by", () => {
    const groups = foldTrail(steps(google("gmail_search")));
    expect(groups[0]?.label).toBe("Used Gmail search on Google");
    expect(groups[0]?.steps[0]?.where).toBe("Google");
  });

  it("keeps the server on the chip and off the calls under it", () => {
    const [step] = steps(google("gmail_search"));
    expect(step?.title).toBe("Used Gmail search");
    expect(step?.tool).toBe("google__gmail_search");
  });

  it("says nothing more beside a chip that already said it", () => {
    const [step] = steps(google("gmail_search"));
    expect(tellsMore(step as Step)).toBe(false);
  });

  it("says what went wrong when the server refused, which no title could have", () => {
    const failed = call("google__gmail_search", {}, { status: "failed", error: "not connected" });
    const [step] = steps(failed);
    expect(tellsMore(step as Step)).toBe(true);
    expect(foldTrail(steps(google("gmail_search"), failed)).map((group) => group.label)).toEqual([
      "Used Gmail search on Google",
      "Tried to use Gmail search on Google",
    ]);
  });

  it("calls a server the operator added what they called it", () => {
    const groups = foldTrail(
      steps(call("home_assistant__turn_on", {}, ok("home_assistant · turn_on"))),
    );
    expect(groups[0]?.label).toBe("Used Turn on on home_assistant");
  });

  it("gathers a turn's work at one server, the way it gathers a turn in a browser", () => {
    const groups = foldTrail(
      steps(google("drive_list_files"), google("drive_read_file"), google("gmail_search")),
    );
    expect(groups).toHaveLength(1);
    expect(groups[0]?.label).toBe("3 calls to Google");
    expect(hasDetail(groups[0]!)).toBe(true);
    expect(groups[0]?.steps.map((step) => step.title)).toEqual([
      "Used Drive list files",
      "Used Drive read file",
      "Used Gmail search",
    ]);
  });

  it("keeps two servers apart, because they are two places", () => {
    const groups = foldTrail(
      steps(google("gmail_search"), call("linear__create_issue", {}, ok("Linear · create_issue"))),
    );
    expect(groups.map((group) => group.label)).toEqual([
      "Used Gmail search on Google",
      "Used Create issue on Linear",
    ]);
  });

  it("leaves a tool with no server in front of it alone", () => {
    const [step] = steps(call("run_code", { source: "print(1)" }, ok("exit 0")));
    expect(step?.title).toBe("Used run_code");
    expect(step?.where).toBeNull();
  });
});

describe("the tools the runtime offers, in a person's words", () => {
  function said(name: string, args: unknown = {}): string {
    return steps(call(name, args))[0]!.title;
  }

  it("names the six that used to reach the row as their wire names", () => {
    expect(said("ask_operator", { question: "Ship it on Friday?" })).toBe("Asked you");
    expect(said("decision", { action: "request", question: "10 or 11?" })).toBe(
      "Left a question for you",
    );
    expect(said("decision", { action: "list" })).toBe("Checked its questions");
    expect(said("decision", { action: "complete", id: "d", outcome: "shipped" })).toBe(
      "Closed a question",
    );
    expect(said("decision", { action: "withdraw", id: "d", outcome: "not needed" })).toBe(
      "Withdrew a question",
    );
    expect(said("calendar", { action: "list" })).toBe("Checked the calendar");
    expect(
      said("calendar", { action: "add", title: "Board call", starts_at: "2026-09-14 15:00" }),
    ).toBe("Put Board call on the calendar");
    expect(said("calendar", { action: "update", starts_at: "2026-09-15 15:00" })).toBe(
      "Changed a date on the calendar",
    );
    expect(said("calendar", { action: "cancel", id: "12" })).toBe(
      "Took something off the calendar",
    );
    expect(said("settings", { action: "read" })).toBe("Read the settings");
    expect(said("settings", { action: "update", changes: { defaultModel: "x" } })).toBe(
      "Asked you to change a setting",
    );
    expect(said("read_file", { name: "brief.md" })).toBe("Read brief.md");
    expect(said("write_document", { name: "readiness.md" })).toBe("Wrote readiness.md");
  });

  it("has a phrase for every tool the runtime offers", () => {
    const source = read("src-tauri/src/llm/tools.rs");
    const names = [...source.matchAll(/^pub const [A-Z_]+: &str = "([a-z][a-z0-9_]*)";$/gm)].map(
      (match) => match[1]!,
    );
    expect(names.length).toBeGreaterThanOrEqual(20);
    for (const name of names) {
      const title = said(name);
      expect(title, name).not.toBe(`Used ${name}`);
      if (name.includes("_")) expect(title, name).not.toContain(name);
    }
  });

  it("says a plugin's own tool as words, and keeps the wire name on the row", () => {
    const [step] = steps(call("google__drive_read_file", {}, ok("Google · drive_read_file")));
    expect(step?.title).toBe("Used Drive read file");
    expect(step?.tool).toBe("google__drive_read_file");
  });
});

describe("the memory tool under both of its names", () => {
  it("draws a transcript written before the rename", () => {
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
    const [step] = steps(
      call("note_progress", { note: "waiting" }, ok("Noted. You have 3 of 16 working notes.")),
    );
    expect(tellsMore(step!)).toBe(false);
  });

  it("shows what went wrong when one could not be saved", () => {
    const [step] = steps(
      call("note_progress", { note: "waiting" }, { status: "failed", error: "database is locked" }),
    );
    expect(tellsMore(step!)).toBe(true);
  });
});

describe("an escalation", () => {
  it("says what it is, and is not called asking", () => {
    const [step] = steps(call("escalate", { summary: "The deploy needs a key only you have." }));
    expect(step!.title).toBe("Escalated to the operator");
    expect(step!.target).toBe("The deploy needs a key only you have.");
    expect(callInFlight("escalate", { summary: "x" })).toBe("Escalating to the operator");
  });

  it("adds nothing beside a chip that already said it", () => {
    const [step] = steps(
      call(
        "escalate",
        { summary: "The tooling is down." },
        ok("That is on the operator's desk now, and it stays there until they clear it."),
      ),
    );
    expect(tellsMore(step!)).toBe(false);
  });

  it("shows what went wrong when one could not be raised", () => {
    const [step] = steps(
      call(
        "escalate",
        { summary: "The tooling is down." },
        { status: "failed", error: "database is locked" },
      ),
    );
    expect(tellsMore(step!)).toBe(true);
  });
});

describe("what the turn has done, on one line", () => {
  it("counts the calls that came back, and says nothing about their kind", () => {
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
