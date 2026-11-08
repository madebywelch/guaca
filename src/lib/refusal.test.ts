import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { plainRefusal, RULES } from "./refusal";

const read = (path: string) => readFileSync(join(process.cwd(), path), "utf8");

/**
 * The runtime's own refusals, as they arrive in a trail.
 *
 * Written out rather than imported: they are what `llm/tools.rs` and
 * `runtime/guard.rs` interpolate into, and the whole point of this file is that
 * the two sides agree. A sample that no longer matches is a mapping that has
 * stopped working, so each case asserts both halves — that the sentence is
 * translated, and that it is the sentence the runtime still writes.
 */
const SAMPLES: { tool: string; said: string; says: string }[] = [
  {
    tool: "request_permission",
    said: "Refused: the operator's rules never allow this (Never email example.com), so nothing happened and they were not asked. Do not look for another way round it.",
    says: "Nothing happened: your rule (Never email example.com) refuses it, so it never reached you as a request.",
  },
  // The guard, from `runtime/guard.rs`.
  {
    tool: "send_message",
    said: "Refused: this conversation is 8 hops from the operator and the limit is 8. Do not relay further. Summarize what you have and reply to whoever asked you.",
    says: "Nothing was delivered: the conversation was 8 relays from you and the limit is 8.",
  },
  {
    tool: "send_message",
    said: "Refused: this run has used its full budget of 40 model calls (40 used). No further messages can be sent. Reply to the operator with what you have.",
    says: "Nothing more was sent: the run used all 40 of its model calls.",
  },
  {
    tool: "send_message",
    said: "Refused: you have already sent Chef 3 messages in this run and the limit is 3. Stop messaging Chef and continue without them.",
    says: "Nothing was sent: it had already messaged Chef 3 times this run, and the limit is 3.",
  },
  {
    tool: "send_message",
    said: "Refused: you already sent Chef this exact message in this run. Repeating it will not produce a different reply. Move on.",
    says: "Nothing was sent: Chef had already had this exact message this run.",
  },
  {
    tool: "send_message",
    said: "Refused: 9 recipients in one call exceeds the limit of 8. Send to at most 8 at a time.",
    says: "Nothing was sent: it addressed 9 agents in one call and the limit is 8.",
  },
  {
    tool: "send_message",
    said: "Refused: you cannot send a message to yourself. Think it through instead.",
    says: "Nothing was sent: it addressed itself.",
  },
  {
    tool: "send_message",
    said: "Refused: no agent named Zed exists. Call `directory` to see who is actually available.",
    says: "Nothing was sent: no agent in this group is called Zed.",
  },
  {
    tool: "send_message",
    said: "Refused: Scribe has been deleted and cannot receive messages.",
    says: "Nothing was sent: Scribe has been deleted.",
  },
  {
    tool: "send_message",
    said: "Refused: you and Chef have both had your say in this run, neither of you is waiting on the other, and this message was sent as a courtesy.",
    says: "Nothing was sent: it and Chef had already finished their exchange, and this was a courtesy.",
  },

  // What an agent was not given, from `runtime/mod.rs`.
  {
    tool: "run_command",
    said: "Refused: you have no computer, so nothing ran and no machine was started. Nothing is broken and there is nothing to retry: you have not been given one, and only the operator can give you one.",
    says: "Nothing ran: it has no computer. Giving it one is a decision only you can make.",
  },
  {
    tool: "use_screen",
    said: "Refused: a picture cannot reach the model you are running on, so looking at your screen would hand back nothing and nothing was done.",
    says: "Nothing was done: it asked to look at its screen, and the model it runs on cannot see pictures.",
  },
  {
    tool: "browse",
    said: "Refused: you have no browser, so nothing was opened and no page was read. Nothing is broken and there is nothing to retry: you have not been given one, and only the operator can give you one.",
    says: "Nothing was opened: it has no browser. Giving it one is a decision only you can make.",
  },
  {
    tool: "errand",
    said: "Refused: errands are not switched on for you, so nothing was sent and nothing was spent. Only the operator can switch them on.",
    says: "Nothing was sent: errands are off for this agent.",
  },

  // What an errand may not do, from `runtime/errand.rs`.
  {
    tool: "errand",
    said: "Refused: you asked for 9 errands and at most 4 run at once, so none was sent and nothing was spent.",
    says: "Nothing was sent out: it asked for 9 errands at once and the limit is 4.",
  },
  {
    tool: "send_message",
    said: "Refused: an errand cannot message anyone, because the answer would arrive after you are gone.",
    says: "Nothing was sent: an errand cannot message anyone.",
  },
  {
    tool: "update_memory",
    said: "Refused: an errand cannot rewrite Chef's memory, because other errands may be writing it at the same moment.",
    says: "Nothing changed: an errand cannot rewrite Chef's memory.",
  },
  {
    tool: "errand",
    said: "Refused: an errand cannot send errands of its own. Do this part yourself.",
    says: "Nothing was sent: an errand cannot send errands of its own.",
  },

  // A call whose arguments were not enough, from `llm/tools.rs`.
  {
    tool: "send_message",
    said: "send_message needs a non-empty `to` list of agent names",
    says: "Nothing was sent: the call named nobody to send to.",
  },
  {
    tool: "send_message",
    said: "send_message needs a non-empty `text`",
    says: "Nothing was sent: the message was empty.",
  },
  {
    tool: "attach_file",
    said: "attach_file needs a non-empty `files` list",
    says: "Nothing was attached: the call named no file.",
  },
  {
    tool: "errand",
    said: "errand needs at least one non-empty brief in `briefs`",
    says: "Nothing was sent out: the call carried no brief.",
  },
  {
    tool: "update_memory",
    said: "update_memory needs a `content` string",
    says: "Its memory was not changed: the call carried no text.",
  },
  {
    tool: "note_progress",
    said: "note_progress needs a non-empty `note` string",
    says: "Nothing was noted: the note was empty.",
  },
  {
    tool: "run_command",
    said: "run_command needs a non-empty `command` string",
    says: "Nothing ran: the call had no command in it.",
  },
  {
    tool: "open_on_desktop",
    said: "open_on_desktop needs a non-empty `command` string",
    says: "Nothing opened on its screen: the call had no program in it.",
  },
  {
    tool: "schedule",
    said: "schedule needs `repeat` or `every_secs` to keep doing it, or `in_secs` to do it once",
    says: "Nothing was scheduled: the call did not say enough about when it should happen.",
  },
  {
    tool: "calendar",
    said: "calendar needs a `title` and a `starts_at`",
    says: "Nothing changed on the calendar: the call was missing part of the date.",
  },
  {
    tool: "artifact",
    said: "artifact needs a `title` and a `page`",
    says: "Nothing was kept: the call was missing part of the page.",
  },
  {
    tool: "notebook",
    said: "notebook needs a `path`",
    says: "Nothing changed in its notebook: the call was missing part of it.",
  },
  {
    tool: "skill",
    said: "skill needs a `name` and a `content`",
    says: "Nothing changed about its skills: the call was missing part of it.",
  },
  {
    tool: "create_agent",
    said: "create_agent needs a `name`",
    says: "No agent was added: the call was missing part of what one needs.",
  },
  {
    tool: "summon_kraken",
    said: 'unknown tool "summon_kraken". Available tools: directory, send_message, read_file.',
    says: "Nothing happened: it called a tool this build does not have.",
  },
  {
    tool: "browse",
    said: "arguments for browse were not valid JSON: expected value at line 1 column 1",
    says: "Nothing happened: the arguments to the call could not be read.",
  },
];

describe("a refusal the runtime wrote for a model", () => {
  it.each(SAMPLES)("says what happened to the person reading it", (sample) => {
    expect(plainRefusal(sample.tool, sample.said)).toBe(sample.says);
  });

  it("says nothing about the vocabulary the model needed", () => {
    // The point of the exercise: no backticks, no parameter name, no JSON key
    // in the sentence an operator reads. The runtime's own version is one click
    // away for the ones who want it.
    for (const sample of SAMPLES) {
      const said = plainRefusal(sample.tool, sample.said)!;
      expect(said, sample.said).not.toContain("`");
      expect(said, sample.said).not.toMatch(/every_secs|in_secs|_secs|`to`|`text`|_memory/);
      // And it opens with what did not happen, which is the question somebody
      // looking at a failed row has.
      expect(said, sample.said).toMatch(/^(Nothing|Its|No agent) /);
    }
  });

  it("does not translate one tool's refusal into another's", () => {
    // `run_command` and `open_on_desktop` refuse with the same opening and are
    // different news: nothing left the machine, and the screen stayed empty.
    const command = plainRefusal("run_command", "run_command needs a non-empty `command` string");
    const desktop = plainRefusal(
      "open_on_desktop",
      "open_on_desktop needs a non-empty `command` string",
    );
    expect(command).toBe("Nothing ran: the call had no command in it.");
    expect(desktop).toBe("Nothing opened on its screen: the call had no program in it.");
  });

  it("leaves a refusal it has never seen exactly as it was written", () => {
    // A build newer than this one, or a refusal this table has not met. Drawing
    // the original is the honest answer: a sentence invented for it would
    // describe the wrong failure, which is worse than one that is hard to read.
    const unknown = "Refused: the archive you named is read-only for this session.";
    expect(plainRefusal("archive", unknown)).toBeNull();
    // And a summary is not a refusal at all.
    expect(plainRefusal("browse", "read in the browser")).toBeNull();
  });
});

describe("the table against the runtime it translates", () => {
  // The coupling is the one thing worth a test of its own: this file is a
  // second copy of a decision `llm/tools.rs` and `runtime/guard.rs` already
  // made, and a reword on that side has to fail here rather than silently put
  // machine text in front of an operator again.
  const sources = [
    "src-tauri/src/runtime/guard.rs",
    "src-tauri/src/runtime/mod.rs",
    "src-tauri/src/runtime/errand.rs",
    "src-tauri/src/llm/tools.rs",
  ]
    .map(read)
    .join("\n");

  it("still mirrors a sentence the runtime writes", () => {
    const missing = RULES.filter((rule) => !sources.includes(rule.mirrors));
    expect(missing.map((rule) => rule.mirrors)).toEqual([]);
  });
});
