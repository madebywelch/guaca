/**
 * What an agent did on its own turn, arranged the way it is read.
 *
 * A turn's tool calls arrive as one envelope, and a turn may make two dozen of
 * them: the round limit is 24 and a browsing turn legitimately spends most of
 * it. Drawn a line each, that is two dozen rows of `Chef used browse` between
 * the operator's question and the answer to it, which is the same burial that
 * peer traffic was collapsed to fix. So this does to a turn's own work what
 * `transcript.ts` does to its peer traffic: several calls of one kind become
 * one chip saying which kind and how many, and the calls themselves are one
 * click away.
 *
 * Two things never fold, for the same reason a refusal never joins a burst.
 * A call the runtime refused or that failed is the row's whole point. And a
 * command that spent one of the operator's credentials is their audit trail
 * for their own tokens, which is not a thing to put behind a click.
 *
 * A single call still says exactly what it was. `Opened cnn.com` and
 * `Ran a command` are what the operator wanted to know; `used browse` is the
 * name of a function in a file they do not have.
 */

import { asRecord, sendRecipients } from "./toolArgs";
import type { Part, ToolOutcome } from "./types";

type ToolCall = Extract<Part, { type: "toolCall" }>;
type Args = Record<string, unknown>;

/** One tool call, as the row draws it. */
export interface Step {
  key: string;
  tool: string;
  /** What this one call was, in words. */
  title: string;
  /**
   * What it acted on, drawn as machine text: a command, a URL, the memory it
   * wrote. Null where there is nothing behind the title, which is also what
   * makes a chip unclickable.
   */
  target: string | null;
  /**
   * The place this call happened, where it has one a group can be named after.
   * A browsing turn that stayed on one site is a turn that can say which site.
   */
  where: string | null;
  /** What came back, in the runtime's own words. */
  said: string;
  /** True when the runtime refused the call or it failed outright. */
  failed: boolean;
  /** Credentials this call spent, by name. Never a value. */
  spent: string[];
}

/** A run of steps the row draws as one chip. */
export interface TrailGroup {
  key: string;
  label: string;
  steps: Step[];
  failed: boolean;
  spent: string[];
}

/**
 * Credentials a command spent, read back off the summary the runtime wrote.
 *
 * Mirrors `credentials_named_in` in `runtime/mod.rs`, which prefixes a
 * `run_command` summary with `used Mistral ($MISTRAL_API_KEY) · `. The two have
 * to agree, and the test holds this side to that exact wording. If the prefix
 * ever stops matching, the names stay legible inside the summary and lose only
 * their own place on the collapsed row.
 */
const SPENT = /^used (.+?) · /;

export function readSpent(summary: string): { spent: string[]; rest: string } {
  const found = SPENT.exec(summary);
  if (!found) return { spent: [], rest: summary };
  return {
    spent: found[1]!.split(", ").filter(Boolean),
    rest: summary.slice(found[0].length),
  };
}

/** What came back, whatever became of the call. */
function outcomeText(outcome: ToolOutcome): string {
  switch (outcome.status) {
    case "ok":
    case "partial":
      return outcome.summary;
    case "refused":
      return outcome.reason;
    case "failed":
      return outcome.error;
  }
}

function text(args: Args, key: string): string | null {
  const value = args[key];
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

function whole(args: Args, key: string): number | null {
  const value = args[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/**
 * A URL as somewhere you have heard of.
 *
 * A chip is a few words wide and a real URL is not, so the host is what fits
 * and it is also the part that answers "where did it go". Anything that will
 * not parse is shown as written: a model that browsed to `cnn.com` should read
 * as having browsed to cnn.com, not as having browsed to nothing.
 */
function place(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "") || url;
  } catch {
    return url;
  }
}

/** Cuts machine text to something a chip can hold, saying that it was cut. */
function clip(value: string, at = 60): string {
  const flat = value.replace(/\s+/g, " ").trim();
  return flat.length > at ? `${flat.slice(0, at - 1)}…` : flat;
}

/** What one call was, what it acted on, and where. */
interface Described {
  title: string;
  target: string | null;
  where?: string;
}

function describe(tool: string, args: Args): Described {
  switch (tool) {
    case "directory":
      return { title: "Checked who is available", target: null };

    case "update_notes":
      return { title: "Updated its memory", target: text(args, "content") };

    case "create_agent": {
      const name = text(args, "name");
      return { title: name ? `Asked to add ${name}` : "Asked to add an agent", target: null };
    }

    case "request_permission":
      return { title: "Asked for permission", target: text(args, "summary") };

    case "run_command":
      return { title: "Ran a command", target: text(args, "command") };

    case "open_on_desktop": {
      const command = text(args, "command");
      return {
        title: command
          ? `Opened ${clip(command.split(/\s+/)[0]!, 24)} on its screen`
          : "Opened a program on its screen",
        target: command,
      };
    }

    // Only ever reaches here naming nobody: a send with recipients is peer
    // traffic and `transcriptRows` has already lifted it into a burst.
    case "send_message": {
      const named = sendRecipients(args);
      return {
        title: named.length > 0 ? `Sent to ${named.join(", ")}` : "Sent a message to nobody",
        target: null,
      };
    }

    case "schedule": {
      const action = text(args, "action");
      if (action === "cancel") return { title: "Cancelled a routine", target: null };
      if (action === "list") return { title: "Checked its schedule", target: null };
      const what = text(args, "name") ?? text(args, "what");
      return { title: what ? `Scheduled ${clip(what, 40)}` : "Changed its schedule", target: what };
    }

    case "browse": {
      const action = text(args, "action");
      const id = whole(args, "id");
      switch (action) {
        case "open": {
          const url = text(args, "url");
          if (!url) return { title: "Opened a page", target: null };
          return { title: `Opened ${place(url)}`, target: url, where: place(url) };
        }
        case "read":
          return { title: "Read the page", target: null };
        case "click":
          return { title: "Clicked on the page", target: id === null ? null : `element ${id}` };
        case "type": {
          const typed = text(args, "text");
          return { title: "Typed on the page", target: typed };
        }
        case "scroll":
          return { title: "Scrolled the page", target: null };
        case "back":
          return { title: "Went back a page", target: null };
        default:
          return { title: "Used the browser", target: null };
      }
    }

    case "use_screen": {
      const action = text(args, "action");
      const x = whole(args, "x");
      const y = whole(args, "y");
      const at = x !== null && y !== null ? `${x}, ${y}` : null;
      switch (action) {
        case "look":
          return { title: "Looked at its screen", target: null };
        case "click":
        case "double_click":
        case "right_click":
          return { title: "Clicked on its screen", target: at };
        case "move":
          return { title: "Moved the pointer", target: at };
        case "type":
          return { title: "Typed on its screen", target: text(args, "text") };
        case "key":
          return { title: "Pressed a key", target: text(args, "keys") };
        case "scroll":
          return { title: "Scrolled its screen", target: null };
        default:
          return { title: "Used its screen", target: null };
      }
    }

    // A tool this build does not know about reads as a tool nobody has
    // explained yet. Guessing at it is how `update_notes` once drew as a
    // message sent to no one.
    default:
      return { title: `Used ${tool}`, target: null };
  }
}

/** One tool call, read out of a stored message. */
export function trailStep(part: ToolCall, key: string): Step {
  const { title, target, where } = describe(part.name, asRecord(part.arguments));
  const { spent, rest } = readSpent(outcomeText(part.outcome));
  return {
    key,
    tool: part.name,
    title,
    target,
    where: where ?? null,
    said: rest,
    failed: part.outcome.status === "refused" || part.outcome.status === "failed",
    spent,
  };
}

/**
 * How several calls of one kind read as one.
 *
 * Named rather than counted wherever the group can name something, for the
 * reason a burst draws one chip per peer rather than "5 messages with 2
 * agents": a count that names nothing hides what the operator opened the
 * channel to find out. Browsing is the case that has an answer, because a
 * turn spent on one site can say which site.
 */
function manyLabel(group: TrailGroup): string {
  const count = group.steps.length;
  switch (group.steps[0]!.tool) {
    case "run_command":
      return `Ran ${count} commands`;
    case "browse": {
      const places = new Set(group.steps.map((step) => step.where).filter(Boolean));
      const only = places.size === 1 ? [...places][0] : null;
      return only ? `${count} steps on ${only}` : `${count} steps in the browser`;
    }
    case "use_screen":
      return `${count} actions on its screen`;
    case "open_on_desktop":
      return `Opened ${count} programs`;
    case "schedule":
      return `${count} changes to its schedule`;
    case "update_notes":
      return `Updated its memory ${count} times`;
    case "directory":
      return `Checked who is available ${count} times`;
    default:
      return `Used ${group.steps[0]!.tool} ${count} times`;
  }
}

/**
 * A turn's steps, folded into the chips a row draws.
 *
 * Grouped by tool across the whole run of calls rather than only where they
 * are adjacent: the order a model happens to interleave `browse` and
 * `run_command` in is not something the operator asked about, and reading
 * "4 steps on cnn.com · ran 2 commands" is the answer to what the turn did.
 * Failures and credential spends keep their own chips, in place.
 */
export function foldTrail(steps: Step[]): TrailGroup[] {
  const groups: TrailGroup[] = [];
  const open = new Map<string, TrailGroup>();

  for (const step of steps) {
    if (step.failed || step.spent.length > 0) {
      groups.push({
        key: step.key,
        label: step.title,
        steps: [step],
        failed: step.failed,
        spent: step.spent,
      });
      continue;
    }
    const held = open.get(step.tool);
    if (held) {
      held.steps.push(step);
      continue;
    }
    const group: TrailGroup = {
      key: step.key,
      label: step.title,
      steps: [step],
      failed: false,
      spent: [],
    };
    open.set(step.tool, group);
    groups.push(group);
  }

  return groups.map((group) =>
    group.steps.length === 1 ? group : { ...group, label: manyLabel(group) },
  );
}

/**
 * Tools whose summary is the call again, in the runtime's words.
 *
 * `browse` answers `read in the browser` beside a step that already says "Read
 * the page". `open_on_desktop` answers with the command it was handed.
 * `use_screen` answers `clicked at (412, 96)` beside a step that says where it
 * clicked. `update_notes` answers with a character count printed directly above
 * the characters. None of them is wrong; all of them are the line above read
 * back, and nine of those turned a row into a paragraph of grey monospace,
 * which is the shape this was collapsed to get away from.
 *
 * A failure is never an echo. Whatever went wrong is not something the title
 * could have said.
 */
const ECHOES = new Set(["browse", "use_screen", "open_on_desktop", "update_notes"]);

/** Whether what came back is worth reading beside the call it came from. */
export function tellsMore(step: Step): boolean {
  return step.failed || !ECHOES.has(step.tool);
}

/**
 * Whether it earns a place on the collapsed chip, where space is scarcest.
 *
 * Stricter, because a chip is one line shared with everything else the turn
 * did. A call with something to open keeps its answer behind the click, where
 * an exit code is worth reading and a byte count can be ignored in peace. What
 * is left is the call that has nothing else to show, where the summary is the
 * whole of what came back: `2 agents: Chef, Scribe` is the answer to a
 * directory lookup, not a restatement of it.
 */
export function saysMore(step: Step): boolean {
  return step.failed || (step.target === null && tellsMore(step));
}

/**
 * Whether a chip has anything behind it.
 *
 * A directory lookup is one call with nothing to show but the sentence already
 * on the chip, and a button that opens nothing is a button the operator learns
 * to distrust.
 */
export function hasDetail(group: TrailGroup): boolean {
  return group.steps.length > 1 || group.steps[0]!.target !== null;
}
