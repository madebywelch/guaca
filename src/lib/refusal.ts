/**
 * What a refusal written for a model says to the person reading it.
 *
 * A call the runtime stopped is written for the agent that made it: it names
 * parameters in backticks, quotes the JSON key that was missing, and tells the
 * model what to send instead. That is exactly right where it is read — a model
 * handed a sentence it cannot parse retries into the same wall — and wrong
 * everywhere else, which is the trail, where the operator meets it at the
 * moment their agent misbehaved. A paragraph of parameter names there reads as
 * a stack trace in the one place that should read like a colleague.
 *
 * So this is a display layer and nothing else. The runtime's words do not
 * change: they are what the model is told, and `llm/tools.rs` and
 * `runtime/guard.rs` hold them to the way forward they have to offer. What
 * changes is which of the two sentences the row draws first, and the runtime's
 * own is kept beside it, one click away, next to the arguments the row already
 * shows.
 *
 * A refusal nobody has translated is drawn as it was written. That is the
 * honest fallback rather than a shortcoming: a build older than this one
 * refuses for reasons this file has never heard of, and a sentence invented for
 * one of those would describe the wrong failure, which is worse than a sentence
 * that is hard to read.
 *
 * The anchors below are the openings of the runtime's sentences, written out
 * rather than matched loosely, because these run to a paragraph and a match in
 * the middle would be a different failure. `refusal.test.ts` reads the Rust
 * sources and fails when one of them is reworded, so this table cannot silently
 * fall back to machine text.
 */

/** How one refusal reads to a person. */
export interface Rule {
  /**
   * The tool the call was to, or null for a refusal any call can hit.
   *
   * Needed because two tools refuse with the same opening: `run_command` and
   * `open_on_desktop` both say a `command` must be non-empty, and a command
   * that did not run is not the same news as a program that did not open.
   */
  tool: string | null;
  /** The part of the runtime's sentence that does not change from call to call. */
  opens: RegExp;
  /**
   * The runtime's own text, as Rust still writes it — interpolation and all.
   *
   * Carried so the coupling is visible: `refusal.test.ts` looks for every one of
   * these in `runtime/guard.rs`, `runtime/errand.rs`, `runtime/mod.rs` and
   * `llm/tools.rs`, so a refusal reworded on that side fails there rather than
   * quietly falling back to machine text in front of an operator.
   */
  mirrors: string;
  /** What it says to the operator. */
  says: (found: RegExpMatchArray) => string;
}

/**
 * The table, in the order it is read.
 *
 * Every sentence says what did not happen before it says why, because that is
 * the question somebody looking at a failed row has. None of them apologizes
 * and none of them tells the operator to do something the app has not offered,
 * which is the rule the whole app is written to (`deployment.rs` holds it for
 * the surfaces that can be built at): a refusal whose way forward does not
 * exist is worse than one that says nothing.
 */
export const RULES: Rule[] = [
  {
    tool: null,
    opens:
      /^Refused: the operator's rules never allow this \(([\s\S]*?)\), so nothing happened and they were not asked\./,
    mirrors:
      "Refused: the operator's rules never allow this ({what}), so nothing happened and they",
    says: (found) =>
      `Nothing happened: your rule (${found[1]}) refuses it, so it never reached you as a request.`,
  },
  // What the guard stopped, in `runtime/guard.rs`. These are the ones that read
  // most like a policy document and are most about the operator's own limits.
  {
    tool: null,
    opens: /^Refused: this conversation is (\d+) hops from the operator and the limit is (\d+)/,
    mirrors: "hops from the operator and the limit is {max}.",
    says: (found) =>
      `Nothing was delivered: the conversation was ${found[1]} relays from you and the limit is ${found[2]}.`,
  },
  {
    tool: null,
    opens: /^Refused: this run has used its full budget of (\d+) model calls/,
    mirrors: "this run has used its full budget of {max} model calls",
    says: (found) => `Nothing more was sent: the run used all ${found[1]} of its model calls.`,
  },
  {
    tool: null,
    opens:
      /^Refused: you have already sent (.+?) (\d+) messages in this run and the limit is (\d+)/,

    mirrors: "you have already sent {recipient} {sent} messages in this run and the",
    says: (found) =>
      `Nothing was sent: it had already messaged ${found[1]} ${found[2]} times this run, and the limit is ${found[3]}.`,
  },
  {
    tool: null,
    opens: /^Refused: you already sent (.+?) this exact message in this run/,
    mirrors: "you already sent {recipient} this exact message in this run",
    says: (found) => `Nothing was sent: ${found[1]} had already had this exact message this run.`,
  },
  {
    tool: null,
    opens: /^Refused: (\d+) recipients in one call exceeds the limit of (\d+)/,
    mirrors: "recipients in one call exceeds the limit of {max}",
    says: (found) =>
      `Nothing was sent: it addressed ${found[1]} agents in one call and the limit is ${found[2]}.`,
  },
  {
    tool: null,
    opens: /^Refused: you cannot send a message to yourself/,
    mirrors: "you cannot send a message to yourself",
    says: () => "Nothing was sent: it addressed itself.",
  },
  {
    tool: null,
    opens: /^Refused: no agent named (.+?) exists/,
    mirrors: "no agent named {recipient} exists",
    says: (found) => `Nothing was sent: no agent in this group is called ${found[1]}.`,
  },
  {
    tool: null,
    opens: /^Refused: (.+?) has been deleted and cannot receive messages/,
    mirrors: "has been deleted and cannot receive messages",
    says: (found) => `Nothing was sent: ${found[1]} has been deleted.`,
  },
  {
    tool: null,
    opens: /^Refused: you and (.+?) have both had your say/,
    mirrors: "you and {recipient} have both had your say",
    says: (found) =>
      `Nothing was sent: it and ${found[1]} had already finished their exchange, and this was a courtesy.`,
  },

  // What an agent was not given, in `runtime/mod.rs`. The runtime's version of
  // each says what the agent should do instead, which is why it is addressed to
  // them; the operator is the one who can change the answer, so theirs says so.
  {
    tool: null,
    opens: /^Refused: you have no computer/,
    mirrors: "you have no computer, so nothing ran and no machine was started",
    says: () => "Nothing ran: it has no computer. Giving it one is a decision only you can make.",
  },
  {
    tool: null,
    opens: /^Refused: a picture cannot reach the model you are running on/,
    mirrors: "a picture cannot reach the model you are running on",
    says: () =>
      "Nothing was done: it asked to look at its screen, and the model it runs on cannot see pictures.",
  },
  {
    tool: null,
    opens: /^Refused: you have no browser/,
    mirrors: "you have no browser, so nothing was opened and no page was read",
    says: () =>
      "Nothing was opened: it has no browser. Giving it one is a decision only you can make.",
  },
  {
    tool: null,
    opens: /^Refused: errands are not switched on for you/,
    mirrors: "errands are not switched on for you, so nothing was sent and nothing",
    says: () => "Nothing was sent: errands are off for this agent.",
  },

  // What an errand may not do, in `runtime/errand.rs`. One of these is a normal
  // thing to see: an errand is a brief with no memory of the conversation, so
  // the calls it cannot make are the calls that would need one.
  {
    tool: null,
    opens: /^Refused: you asked for (\d+) errands and at most (\d+) run at once/,
    mirrors: "errands and at most {} run at once, so none was",
    says: (found) =>
      `Nothing was sent out: it asked for ${found[1]} errands at once and the limit is ${found[2]}.`,
  },
  {
    tool: null,
    opens: /^Refused: an errand cannot message anyone/,
    mirrors: "an errand cannot message anyone, because the answer would arrive after",
    says: () => "Nothing was sent: an errand cannot message anyone.",
  },
  {
    tool: null,
    opens: /^Refused: an errand cannot rewrite (.+?)'s memory/,
    mirrors: "an errand cannot rewrite {name}'s memory",
    says: (found) => `Nothing changed: an errand cannot rewrite ${found[1]}'s memory.`,
  },
  {
    tool: null,
    opens: /^Refused: an errand cannot send errands of its own/,
    mirrors: "an errand cannot send errands of its own",
    says: () => "Nothing was sent: an errand cannot send errands of its own.",
  },

  // A call whose arguments were not enough to do anything with, in
  // `llm/tools.rs`. The model is told which parameter to send; the operator is
  // told that the call did nothing, which is the part they cannot see.
  {
    tool: "send_message",
    opens: /^send_message needs a non-empty `to` list/,
    mirrors: "send_message needs a non-empty `to` list of agent names",
    says: () => "Nothing was sent: the call named nobody to send to.",
  },
  {
    tool: "send_message",
    opens: /^send_message needs a non-empty `text`/,
    mirrors: "send_message needs a non-empty `text`",
    says: () => "Nothing was sent: the message was empty.",
  },
  {
    tool: "attach_file",
    opens: /^attach_file needs a non-empty `files` list/,
    mirrors: "attach_file needs a non-empty `files` list",
    says: () => "Nothing was attached: the call named no file.",
  },
  {
    tool: "errand",
    opens: /^errand needs at least one non-empty brief/,
    mirrors: "errand needs at least one non-empty brief in `briefs`",
    says: () => "Nothing was sent out: the call carried no brief.",
  },
  {
    tool: "update_memory",
    opens: /^update_memory needs a `content` string/,
    mirrors: "update_memory needs a `content` string",
    says: () => "Its memory was not changed: the call carried no text.",
  },
  {
    tool: "note_progress",
    opens: /^note_progress needs a non-empty `note` string/,
    mirrors: "note_progress needs a non-empty `note` string",
    says: () => "Nothing was noted: the note was empty.",
  },
  {
    tool: "run_command",
    opens: /^run_command needs a non-empty `command` string/,
    mirrors: "run_command needs a non-empty `command` string",
    says: () => "Nothing ran: the call had no command in it.",
  },
  {
    tool: "open_on_desktop",
    opens: /^open_on_desktop needs a non-empty `command` string/,
    mirrors: "open_on_desktop needs a non-empty `command` string",
    says: () => "Nothing opened on its screen: the call had no program in it.",
  },
  {
    tool: "schedule",
    opens: /^schedule needs /,
    mirrors: "schedule needs {needs}",
    says: () => "Nothing was scheduled: the call did not say enough about when it should happen.",
  },
  {
    tool: "calendar",
    opens: /^calendar needs /,
    mirrors: "calendar needs {needs}",
    says: () => "Nothing changed on the calendar: the call was missing part of the date.",
  },
  {
    tool: "artifact",
    opens: /^artifact needs /,
    mirrors: "artifact needs {needs}",
    says: () => "Nothing was kept: the call was missing part of the page.",
  },
  {
    tool: "notebook",
    opens: /^notebook needs /,
    mirrors: "notebook needs {needs}",
    says: () => "Nothing changed in its notebook: the call was missing part of it.",
  },
  {
    tool: "skill",
    opens: /^skill needs /,
    mirrors: "skill needs {needs}",
    says: () => "Nothing changed about its skills: the call was missing part of it.",
  },
  {
    tool: "create_agent",
    opens: /^create_agent needs /,
    mirrors: "create_agent needs {needs}",
    says: () => "No agent was added: the call was missing part of what one needs.",
  },

  // A call that could not be read at all. The name of the tool is on the chip
  // already, so the sentence is about the outcome rather than the vocabulary.
  {
    tool: null,
    opens: /^unknown tool /,
    mirrors: "unknown tool {name:?}.",
    says: () => "Nothing happened: it called a tool this build does not have.",
  },
  {
    tool: null,
    opens: /^arguments for (.+?) were not valid JSON/,
    mirrors: "arguments for {name} were not valid JSON: {detail}",
    says: () => "Nothing happened: the arguments to the call could not be read.",
  },
];

/**
 * The same refusal, in words for the person reading the transcript.
 *
 * Null when nothing here has met this refusal, which the caller draws exactly
 * as the runtime wrote it.
 */
export function plainRefusal(tool: string, said: string): string | null {
  for (const rule of RULES) {
    if (rule.tool !== null && rule.tool !== tool) continue;
    const found = rule.opens.exec(said);
    if (found) return rule.says(found);
  }
  return null;
}
