export interface Rule {
  tool: string | null;
  opens: RegExp;
  mirrors: string;
  says: (found: RegExpMatchArray) => string;
}

export const RULES: Rule[] = [
  {
    tool: null,
    opens:
      /^Refused: the operator's rules never allow this \(([\s\S]*?)\), so nothing happened and they were not asked\./,
    mirrors:
      "Refused: the operator's rules never allow this ({what}), so nothing happened and they",
    says: (found) =>
      `Nothing happened: your rule (${found[1]}) refuses it, so it never reached you as a question.`,
  },
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

export function plainRefusal(tool: string, said: string): string | null {
  for (const rule of RULES) {
    if (rule.tool !== null && rule.tool !== tool) continue;
    const found = rule.opens.exec(said);
    if (found) return rule.says(found);
  }
  return null;
}
