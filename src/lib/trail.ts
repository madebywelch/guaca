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
 *
 * A plugin's tools are its own and arrive after this build shipped, so nothing
 * here can say what one of them does. What it can say is where the work went,
 * which is the half of the name the runtime prefixed on for that purpose. So a
 * chip reads `Used gmail_search on Google` rather than `Used
 * google__gmail_search`, and several of them gather by the server rather than
 * by the tool, the way a browsing turn gathers by the browser.
 */

import { type DiffLine, lineDiff } from "./diff";
import { nameFor } from "./plugins";
import { asRecord, attachedNames, sendRecipients } from "./toolArgs";
import type { ToolCallPart, ToolOutcome } from "./types";

type Args = Record<string, unknown>;

/**
 * One tool call while the turn making it is still going.
 *
 * What it is, said before the call so that a wait can be named while it is
 * being waited on, and the record of it once it has come back. A call that has
 * not come back is the one thing a turn can be doing that neither the
 * transcript nor the thinking says a word about, and it is the one that can
 * take a minute.
 */
export interface LiveCall {
  callId: string;
  name: string;
  arguments: unknown;
  /**
   * The record of the call once it has come back, and null until then.
   *
   * The whole part rather than the outcome alone, because this is what the chip
   * is drawn from and the transcript draws the same chip from the same shape a
   * minute later. A memory rewrite carries what it overwrote, and a live chip
   * assembled from the fields somebody thought to list would quietly have
   * stopped showing it.
   */
  done: ToolCallPart | null;
  startedAt: number;
}

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
   * The version this call overwrote, where it overwrote something whole.
   *
   * Only a memory rewrite has one, and only the runtime could have supplied it:
   * one agent's memory is written from its wall and from every thread it holds,
   * so a previous version read back out of the channel this call happens to sit
   * in is wrong exactly when something interesting happened. Empty string where
   * there was nothing to overwrite, which is a first memory and not a missing
   * one; null where nothing was overwritten at all.
   */
  replaced: string | null;
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

/**
 * A plugin call, as the server it went to and the tool it called there.
 *
 * Mirrors `split_plugin_tool` in `llm/tools.rs`: two underscores, split on the
 * first pair, both halves non-empty. Which servers exist is deliberately not
 * checked, and cannot be from here: a crew's plugins are whatever the operator
 * connected, and a transcript outlives the connection. A prefix this build has
 * never heard of is drawn as the name it was called by, which is what the
 * operator typed when they added the server.
 *
 * The cost of not checking is a model that invents `use_screen__click` and has
 * its refusal drawn as a call to a server called Use_screen. That row is a
 * failure with its reason on it either way, and the alternative is a second
 * list of this build's own tool names, kept in step with the switch below.
 */
export function pluginCall(name: string): { plugin: string; tool: string } | null {
  const at = name.indexOf("__");
  if (at <= 0) return null;
  const tool = name.slice(at + 2);
  return tool.length > 0 ? { plugin: nameFor(name.slice(0, at)), tool } : null;
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

    // Both names, and the old one is not decoration. A tool call is stored as
    // JSON on the message, so every turn recorded before the rename still says
    // `update_notes`, and a renderer that knows only the new name draws a year
    // of transcripts as "Used update_notes". Same argument `Part::Approval` has
    // for never being widened: old rows cannot be migrated into a new spelling.
    case "update_memory":
    case "update_notes":
      return { title: "Updated its memory", target: text(args, "content") };

    case "note_progress":
      return { title: "Noted where its work stands", target: text(args, "note") };

    case "create_agent": {
      const name = text(args, "name");
      return { title: name ? `Asked to add ${name}` : "Asked to add an agent", target: null };
    }

    case "request_permission":
      return { title: "Asked for permission", target: text(args, "summary") };

    // Not "asked" and not "said": nothing was asked, and this is the one call
    // that reaches the operator wherever they are without stopping the turn.
    case "escalate":
      return { title: "Escalated to the operator", target: text(args, "summary") };

    case "run_command":
      return { title: "Ran a command", target: text(args, "command") };

    // Named apart from `run_command` rather than sharing its label, because
    // the two run on different machines and the chip is where an operator
    // finds out which. An agent that has both draws both in one turn.
    case "shell":
      return { title: "Ran a command in its repository", target: text(args, "command") };

    case "code":
      return { title: "Started a coding agent", target: text(args, "task") };

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

    // Named, because the file itself is drawn under the message and the chip's
    // job is to say which call put it there. `2 files attached` beside two
    // cards is a count of what the operator is already looking at.
    case "attach_file": {
      const named = attachedNames(args);
      return {
        title: named.length > 0 ? `Attached ${named.join(", ")}` : "Attached a file",
        target: null,
      };
    }

    case "schedule": {
      const action = text(args, "action");
      if (action === "cancel") return { title: "Canceled a routine", target: null };
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
    // explained yet. Guessing at it is how `update_memory` once drew as a
    // message sent to no one. A plugin's is that tool with a place behind it,
    // and the place is worth keeping: `run_sql` alone is not a chip anybody
    // can read a week later, which is why the runtime writes the name
    // prefixed at all. It is held as the place rather than said in the title,
    // for the reason `chipLabel` gives.
    default: {
      const from = pluginCall(tool);
      if (!from) return { title: `Used ${tool}`, target: null };
      return { title: `Used ${from.tool}`, target: null, where: from.plugin };
    }
  }
}

/** One tool call, read out of a stored message. */
export function trailStep(part: ToolCallPart, key: string): Step {
  const { title, target, where } = describe(part.name, asRecord(part.arguments));
  const { spent, rest } = readSpent(outcomeText(part.outcome));
  return {
    key,
    tool: part.name,
    title,
    target,
    // Checked for the type rather than for truth: an empty previous version is
    // an agent's first memory, and reading it as nothing to compare against
    // draws that page as a document with no history instead of as all new.
    replaced: typeof part.replaced === "string" ? part.replaced : null,
    where: where ?? null,
    said: rest,
    failed: part.outcome.status === "refused" || part.outcome.status === "failed",
    spent,
  };
}

/**
 * What a call that has not come back yet is, in the present tense.
 *
 * Coarser than the label a finished call gets, deliberately. `describe` says
 * what a call *was*, which is the wrong tense for the one thing the operator is
 * waiting on, and a second copy of it in the present would be forty more arms
 * to keep in step with the first forty. What is worth knowing while a call is
 * in flight is which machine is being waited on, and that is the tool. The one
 * exception is a page being opened, because the wait is the site and the site
 * is in the arguments.
 */
/**
 * Which machine a turn is on right now, read off its live calls, or null.
 *
 * Mirrors `tools::surface_of` in Rust, which is what the menu bar reads. The
 * rail reads it from here because the trail is already on this side: a call
 * that has not come back on one of the four machine tools is an agent driving
 * a rented desktop or a hosted browser, which is the moment there is a screen
 * to go and watch.
 */
export function machineInUse(work: LiveCall[] | undefined): "computer" | "browser" | null {
  for (const call of work ?? []) {
    if (call.done !== null) continue;
    if (call.name === "browse") return "browser";
    if (
      call.name === "run_command" ||
      call.name === "open_on_desktop" ||
      call.name === "use_screen"
    ) {
      return "computer";
    }
  }
  return null;
}

export function callInFlight(name: string, raw: unknown): string {
  const args = asRecord(raw);
  switch (name) {
    case "run_command":
      return "Running a command";
    case "shell":
      return "Running a command in its repository";
    case "code":
      return "Starting a coding agent";
    case "browse": {
      const url = text(args, "url");
      return url && text(args, "action") === "open"
        ? `Opening ${place(url)}`
        : "Working the browser";
    }
    case "use_screen":
      return "Working its screen";
    case "open_on_desktop":
      return "Opening a program on its screen";
    case "send_message":
      return "Sending a message";
    case "attach_file":
      return "Attaching a file";
    case "update_memory":
    case "update_notes":
      return "Updating its memory";
    case "note_progress":
      return "Noting where its work stands";
    case "directory":
      return "Checking who is available";
    case "schedule":
      return "Changing its schedule";
    case "create_agent":
      return "Asking to add an agent";
    case "request_permission":
      return "Asking for permission";
    case "escalate":
      return "Escalating to the operator";
    default: {
      const from = pluginCall(name);
      return from ? `Using ${from.tool} on ${from.plugin}` : `Using ${name}`;
    }
  }
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
    case "shell":
      return `Ran ${count} commands in its repository`;
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
    case "attach_file":
      return `Attached ${count} files`;
    case "update_memory":
    case "update_notes":
      return `Updated its memory ${count} times`;
    case "note_progress":
      return `Noted where its work stands ${count} times`;
    case "directory":
      return `Checked who is available ${count} times`;
    case "escalate":
      return `Escalated to the operator ${count} times`;
    default: {
      const from = pluginCall(group.steps[0]!.tool);
      return from
        ? `${count} calls to ${from.plugin}`
        : `Used ${group.steps[0]!.tool} ${count} times`;
    }
  }
}

/**
 * What one chip says, whether it stands for one call or several.
 *
 * A plugin's server is named here rather than in the step's own title because
 * the chip is the only place it is missing. A step is only ever drawn under a
 * chip, and that chip already says where the work went: a column of `Used
 * drive_read_file on Google` under one reading `4 calls to Google` says the
 * place five times over.
 */
function chipLabel(group: TrailGroup): string {
  if (group.steps.length > 1) return manyLabel(group);
  const only = group.steps[0]!;
  const from = pluginCall(only.tool);
  return from ? `${only.title} on ${from.plugin}` : only.title;
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
    // Gathered by the server a plugin call went to rather than by the tool it
    // called there, for the reason a browsing turn is gathered by the browser:
    // the plugin is the place, its tools are what the turn did there, and four
    // chips reading `Used drive_read_file on Google` beside each other say the
    // place four times and the work once.
    const family = pluginCall(step.tool)?.plugin ?? step.tool;
    const held = open.get(family);
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
    open.set(family, group);
    groups.push(group);
  }

  return groups.map((group) => ({ ...group, label: chipLabel(group) }));
}

/**
 * What a turn has done so far, as the one line that can stand for all of it.
 *
 * The chips are the record, and the transcript draws every one of them the
 * moment the turn ends. While the turn is still running they are a wall: seven
 * kinds of work wrapped across four rows directly above the composer, growing
 * and reflowing every time a call comes back, with the box the operator is
 * typing into moving underneath. What is worth knowing live is narrower than
 * the record: that the work is moving, whether any of it went wrong, and what
 * it is spending. The rest is one click away and permanent a minute later.
 */
export interface TrailTally {
  /** Calls that have come back. A call still in flight is not one of them. */
  done: number;
  /** How many of those the runtime refused or that failed outright. */
  failed: number;
  /**
   * Credentials spent, by name, first spend first.
   *
   * Never folded into a count and never behind a click, live or recorded: this
   * is the operator's audit trail for their own tokens, and it is the one part
   * of the trail that stays on the line while the turn runs.
   */
  spent: string[];
}

export function tallyTrail(steps: Step[]): TrailTally {
  const spent = new Set<string>();
  let failed = 0;
  for (const step of steps) {
    if (step.failed) failed += 1;
    for (const credential of step.spent) spent.add(credential);
  }
  return { done: steps.length, failed, spent: [...spent] };
}

/**
 * What the counter above the composer says.
 *
 * A count, because a turn's kinds of work are what the chips behind it are for
 * and naming one of them here would be a chip that is wrong about the rest. A
 * failure is the exception, and it is on the line rather than behind the click
 * for the reason a failure never joins a burst: it is the one thing on the
 * trail the operator may have to do something about.
 */
export function tallyLabel(tally: TrailTally): string {
  const steps = tally.done === 1 ? "1 step" : `${tally.done} steps`;
  return tally.failed > 0 ? `${steps}, ${tally.failed} failed` : steps;
}

/**
 * Tools whose summary is the call again, in the runtime's words.
 *
 * `browse` answers `read in the browser` beside a step that already says "Read
 * the page". `open_on_desktop` answers with the command it was handed.
 * `use_screen` answers `clicked at (412, 96)` beside a step that says where it
 * clicked. `update_memory` answers with a character count printed directly above
 * the characters. `note_progress` answers with how many notes the agent now
 * holds, which is a number the model needs and the operator can already see by
 * looking at the panel that lists them. `attach_file` answers `attached brief.md` beside a chip that
 * says "Attached brief.md" and a card drawing brief.md. None of them is wrong;
 * all of them are the line above read back, and nine of those turned a row into
 * a paragraph of gray monospace, which is the shape this was collapsed to get
 * away from.
 *
 * A failure is never an echo. Whatever went wrong is not something the title
 * could have said.
 */
const ECHOES = new Set([
  "browse",
  "use_screen",
  "open_on_desktop",
  "update_memory",
  "update_notes",
  "note_progress",
  "attach_file",
  // Every word of what comes back is an instruction to the model -- do not
  // wait, do what you can, do not raise it again -- and the two numbers in it
  // that the operator would want are on the desk and the rail, read live from
  // the row. Frozen into a transcript, "2d ago" is wrong by the next morning.
  "escalate",
]);

/** Whether what came back is worth reading beside the call it came from. */
export function tellsMore(step: Step): boolean {
  // A plugin answers `Google · gmail_search`, which is the chip again. The
  // runtime writes it so that whatever draws the call can say which server the
  // work went to, and the chip says that itself now.
  return step.failed || !(ECHOES.has(step.tool) || pluginCall(step.tool) !== null);
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
 * What the call changed about what it overwrote, where it overwrote something.
 *
 * A memory rewrite is the whole page every time, because replacing is the only
 * write the tool has: that is right for the agent, which has to reconcile what
 * it believed against what it just learned, and useless for the operator, who
 * is handed two near-identical pages and left to compare them by eye.
 *
 * Null where there is nothing to compare, which includes a call that replaced
 * nothing and the one degenerate case of clearing a memory that was already
 * empty. An empty diff drawn as a panel is a control that opens onto nothing.
 */
export function stepDiff(step: Step): DiffLine[] | null {
  if (step.replaced === null) return null;
  const diff = lineDiff(step.replaced, step.target ?? "");
  return diff.length > 0 ? diff : null;
}

/**
 * What a call that went wrong said, where the reason is the whole of it.
 *
 * A refusal is written to be acted on and runs to a paragraph, and a paragraph
 * on a chip is a chip clipped at the pane: `U… a coding agent is already
 * working in whizzworks-site, started by…`. So it comes off the head line and
 * is drawn where a command is drawn — whole, wrapped, and scrolled if it runs
 * long — and the clipped copy on the chip becomes a summary of something the
 * operator can now open.
 *
 * Only where there is nothing else behind the call. A `run_command` that failed
 * has its command to show, and that is what somebody opening it came for.
 */
export function stepReason(step: Step): string | null {
  return step.failed && step.target === null && step.said.length > 0 ? step.said : null;
}

/**
 * Whether a chip has anything behind it.
 *
 * A directory lookup is one call with nothing to show but the sentence already
 * on the chip, and a button that opens nothing is a button the operator learns
 * to distrust.
 *
 * A memory that was cleared has no content to show and is still worth opening:
 * what was thrown away is the whole of what happened. So is a refusal with
 * nothing but its reason, which is the one case where the sentence on the chip
 * is a clipped copy rather than the whole thing.
 */
export function hasDetail(group: TrailGroup): boolean {
  const only = group.steps[0]!;
  return (
    group.steps.length > 1 ||
    only.target !== null ||
    stepDiff(only) !== null ||
    stepReason(only) !== null
  );
}
