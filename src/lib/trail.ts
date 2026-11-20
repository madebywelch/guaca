import { type DiffLine, lineDiff } from "./diff";
import { nameFor } from "./plugins";
import { plainRefusal } from "./refusal";
import { asRecord, attachedNames, sendRecipients } from "./toolArgs";
import type { ToolCallPart, ToolOutcome } from "./types";

type Args = Record<string, unknown>;

export interface LiveCall {
  callId: string;
  name: string;
  arguments: unknown;
  done: ToolCallPart | null;
  startedAt: number;
  reasoningOffset?: number;
}

export interface Step {
  key: string;
  tool: string;
  title: string;
  target: string | null;
  replaced: string | null;
  where: string | null;
  said: string;
  told: string | null;
  failed: boolean;
  spent: string[];
  ruled: string | null;
}

export interface TrailGroup {
  key: string;
  label: string;
  steps: Step[];
  failed: boolean;
  spent: string[];
}

const CREDENTIAL = String.raw`(?:"((?:[^"]|"")*)"|([^",·]+?)) \(\$([A-Za-z_][A-Za-z0-9_]*)\)`;
const SPENT = new RegExp(`^used (${CREDENTIAL}(?:, ${CREDENTIAL})*) · `);
const ONE_SPENT = new RegExp(`${CREDENTIAL}(?:, |$)`, "gy");

export function readSpent(summary: string): { spent: string[]; rest: string } {
  const found = SPENT.exec(summary);
  if (!found) return { spent: [], rest: summary };
  return {
    spent: [...found[1]!.matchAll(ONE_SPENT)].map(
      ([, quoted, plain, envVar]) => `${quoted?.replaceAll('""', '"') ?? plain} ($${envVar})`,
    ),
    rest: summary.slice(found[0].length),
  };
}

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

function briefList(args: Args): string[] {
  const value = args.briefs;
  const listed = Array.isArray(value) ? value : [value];
  return listed.filter(
    (brief): brief is string => typeof brief === "string" && brief.trim() !== "",
  );
}

function whole(args: Args, key: string): number | null {
  const value = args[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function place(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "") || url;
  } catch {
    return url;
  }
}

function clip(value: string, at = 60): string {
  const flat = value.replace(/\s+/g, " ").trim();
  return flat.length > at ? `${flat.slice(0, at - 1)}…` : flat;
}

const PLUGIN_SEPARATOR = "__";

export function splitPluginTool(name: string): { server: string; tool: string } | null {
  const at = name.indexOf(PLUGIN_SEPARATOR);
  if (at <= 0) return null;
  const tool = name.slice(at + PLUGIN_SEPARATOR.length);
  return tool.length > 0 ? { server: name.slice(0, at), tool } : null;
}

export function pluginCall(name: string): { plugin: string; tool: string } | null {
  const split = splitPluginTool(name);
  return split ? { plugin: nameFor(split.server), tool: split.tool } : null;
}

interface Described {
  title: string;
  target: string | null;
  where?: string;
}

function describe(tool: string, args: Args): Described {
  switch (tool) {
    case "directory":
      return { title: "Checked who is available", target: null };

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

    case "ask_operator":
      return { title: "Asked you", target: text(args, "question") };

    case "decision": {
      const action = text(args, "action");
      if (action === "list") return { title: "Checked its questions", target: null };
      if (action === "complete")
        return { title: "Closed a question", target: text(args, "outcome") };
      if (action === "withdraw")
        return { title: "Withdrew a question", target: text(args, "outcome") };
      return { title: "Left a question for you", target: text(args, "question") };
    }

    case "escalate":
      return { title: "Escalated to the operator", target: text(args, "summary") };

    case "settings": {
      const action = text(args, "action");
      if (action === "update") {
        const changes = asRecord(args.changes);
        const named = Object.keys(changes);
        return {
          title: "Asked you to change a setting",
          target: named.length > 0 ? clip(named.join(", "), 48) : null,
        };
      }
      return { title: "Read the settings", target: null };
    }

    case "run_command":
      return { title: "Ran a command", target: text(args, "command") };

    case "errand": {
      const briefs = briefList(args);
      return {
        title: briefs.length === 1 ? "Sent an errand" : `Sent ${briefs.length} errands`,
        target: briefs.length > 0 ? briefs.join("\n\n") : null,
      };
    }

    case "open_on_desktop": {
      const command = text(args, "command");
      return {
        title: command
          ? `Opened ${clip(command.split(/\s+/)[0]!, 24)} on its screen`
          : "Opened a program on its screen",
        target: command,
      };
    }

    case "send_message": {
      const named = sendRecipients(args);
      return {
        title: named.length > 0 ? `Sent to ${named.join(", ")}` : "Sent a message to nobody",
        target: null,
      };
    }

    case "attach_file": {
      const named = attachedNames(args);
      return {
        title: named.length > 0 ? `Attached ${named.join(", ")}` : "Attached a file",
        target: null,
      };
    }

    case "write_document": {
      const name = text(args, "name");
      return { title: name ? `Wrote ${clip(name, 40)}` : "Wrote a document", target: null };
    }

    case "read_file": {
      const name = text(args, "name");
      return { title: name ? `Read ${clip(name, 40)}` : "Read a saved file", target: null };
    }

    case "artifact": {
      const action = text(args, "action") ?? "list";
      const title = text(args, "title");
      if (["create", "add", "new", "make"].includes(action))
        return { title: title ? `Made ${clip(title, 40)}` : "Made an artifact", target: null };
      if (["update", "edit", "change", "revise", "rename"].includes(action))
        return { title: "Updated an artifact", target: text(args, "note") };
      if (["take", "take_over", "take_ownership", "claim", "own"].includes(action))
        return { title: "Took over an artifact", target: text(args, "note") };
      if (action === "list") return { title: "Checked its group's artifacts", target: null };
      return { title: "Opened an artifact", target: null };
    }

    case "notebook": {
      const action = text(args, "action") ?? "list";
      const path = text(args, "path");
      if (!path || action === "list" || action === "ls")
        return { title: "Checked its notebook", target: null };
      const file = clip(path, 40);
      if (["write", "create", "save", "replace"].includes(action))
        return { title: `Wrote ${file} in its notebook`, target: null };
      if (["append", "add", "log"].includes(action))
        return { title: `Added to ${file} in its notebook`, target: text(args, "content") };
      if (["move", "rename", "mv"].includes(action))
        return { title: `Moved ${file} in its notebook`, target: text(args, "to") };
      if (["delete", "remove", "rm"].includes(action))
        return { title: `Deleted ${file} from its notebook`, target: null };
      return { title: `Read ${file} from its notebook`, target: null };
    }

    case "skill": {
      const action = text(args, "action") ?? "list";
      const name = text(args, "name");
      if (!name || action === "list") return { title: "Checked its skills", target: null };
      if (["write", "create", "update", "save", "edit"].includes(action))
        return { title: `Wrote the ${clip(name, 40)} skill`, target: text(args, "description") };
      if (action === "delete" || action === "remove")
        return { title: `Deleted the ${clip(name, 40)} skill`, target: null };
      return { title: `Read the ${clip(name, 40)} skill`, target: null };
    }

    case "schedule": {
      const action = text(args, "action");
      if (action === "cancel") return { title: "Canceled a routine", target: null };
      if (action === "list") return { title: "Checked its schedule", target: null };
      const what = text(args, "name") ?? text(args, "what");
      return { title: what ? `Scheduled ${clip(what, 40)}` : "Changed its schedule", target: what };
    }

    case "calendar": {
      const action = text(args, "action");
      if (action === "list") return { title: "Checked the calendar", target: null };
      if (action === "cancel")
        return { title: "Took something off the calendar", target: text(args, "id") };
      const what = text(args, "title");
      const when = text(args, "starts_at");
      if (action === "update") {
        return {
          title: what
            ? `Changed ${clip(what, 40)} on the calendar`
            : "Changed a date on the calendar",
          target: when,
        };
      }
      return {
        title: what ? `Put ${clip(what, 40)} on the calendar` : "Added a date to the calendar",
        target: when,
      };
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

    default: {
      const from = pluginCall(tool);
      if (!from) return { title: `Used ${tool}`, target: null };
      return { title: `Used ${readable(from.tool)}`, target: null, where: from.plugin };
    }
  }
}

function readable(name: string): string {
  const words = name.replace(/[_-]+/g, " ").trim();
  return words === "" ? name : words[0]!.toUpperCase() + words.slice(1);
}

export function trailStep(part: ToolCallPart, key: string): Step {
  const { title, target, where } = describe(part.name, asRecord(part.arguments));
  const { spent, rest } = readSpent(outcomeText(part.outcome));
  const failed = part.outcome.status === "refused" || part.outcome.status === "failed";
  const plain = failed ? plainRefusal(part.name, rest) : null;
  return {
    key,
    tool: part.name,
    title: failed ? attempt(title) : title,
    target,
    replaced: typeof part.replaced === "string" ? part.replaced : null,
    where: where ?? null,
    said: plain ?? rest,
    told: plain === null ? null : rest,
    failed,
    spent,
    ruled: part.ruled ?? null,
  };
}

const ATTEMPT: Record<string, string> = {
  Added: "add",
  Asked: "ask",
  Attached: "attach",
  Canceled: "cancel",
  Changed: "change",
  Checked: "check",
  Clicked: "click",
  Closed: "close",
  Deleted: "delete",
  Escalated: "escalate",
  Left: "leave",
  Looked: "look",
  Made: "make",
  Moved: "move",
  Noted: "note",
  Opened: "open",
  Pressed: "press",
  Put: "put",
  Ran: "run",
  Read: "read",
  Scheduled: "schedule",
  Scrolled: "scroll",
  Sent: "send",
  Took: "take",
  Typed: "type",
  Updated: "update",
  Used: "use",
  Went: "go",
  Withdrew: "withdraw",
  Wrote: "write",
};

export function attempt(title: string): string {
  const at = title.indexOf(" ");
  const first = at === -1 ? title : title.slice(0, at);
  const rest = at === -1 ? "" : title.slice(at + 1);
  const verb = ATTEMPT[first];
  return verb ? `Tried to ${verb}${rest ? ` ${rest}` : ""}` : `Failed: ${title}`;
}

export type Machine = "computer" | "browser";

export function surfaceOf(tool: string): Machine | null {
  if (tool === "browse") return "browser";
  if (tool === "run_command" || tool === "open_on_desktop" || tool === "use_screen") {
    return "computer";
  }
  return null;
}

export function machineInUse(work: LiveCall[] | undefined): Machine | null {
  for (const call of work ?? []) {
    if (call.done !== null) continue;
    const surface = surfaceOf(call.name);
    if (surface) return surface;
  }
  return null;
}

export function callInFlight(name: string, raw: unknown): string {
  const args = asRecord(raw);
  switch (name) {
    case "run_command":
      return "Running a command";
    case "errand":
      return "Waiting on errands";
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
    case "skill":
      return "Reading its skills";
    case "notebook":
      return "Working in its notebook";
    case "artifact":
      return "Working on an artifact";
    case "create_agent":
      return "Asking to add an agent";
    case "request_permission":
      return "Asking for permission";
    case "escalate":
      return "Escalating to the operator";
    case "calendar":
      return "Working on the calendar";
    case "settings":
      return text(args, "action") === "update"
        ? "Asking you to change a setting"
        : "Reading the settings";
    case "ask_operator":
      return "Asking you";
    case "decision":
      return "Working on a question";
    case "read_file":
      return "Reading a saved file";
    case "write_document":
      return "Writing a document";
    default: {
      const from = pluginCall(name);
      return from ? `Using ${readable(from.tool)} on ${from.plugin}` : `Using ${name}`;
    }
  }
}

function manyLabel(group: TrailGroup): string {
  const count = group.steps.length;
  switch (group.steps[0]!.tool) {
    case "run_command":
      return `Ran ${count} commands`;
    case "errand":
      return `Sent errands ${count} times`;
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
    case "skill":
      return `Used its skills ${count} times`;
    case "notebook":
      return `Used its notebook ${count} times`;
    case "artifact":
      return `Used its group's artifacts ${count} times`;
    case "attach_file":
      return `Attached ${count} files`;
    case "write_document":
      return `Wrote ${count} documents`;
    case "read_file":
      return `Read ${count} files`;
    case "calendar":
      return `Used the calendar ${count} times`;
    case "settings":
      return `Used the settings ${count} times`;
    case "decision":
      return `Worked on its questions ${count} times`;
    case "ask_operator":
      return `Asked you ${count} times`;
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

function chipLabel(group: TrailGroup): string {
  if (group.steps.length > 1) return manyLabel(group);
  const only = group.steps[0]!;
  if (only.ruled !== null) return `Allowed by a rule · ${only.ruled}`;
  const from = pluginCall(only.tool);
  return from ? `${only.title} on ${from.plugin}` : only.title;
}

export function foldTrail(steps: Step[]): TrailGroup[] {
  const groups: TrailGroup[] = [];
  const open = new Map<string, TrailGroup>();

  for (const step of steps) {
    if (step.failed || step.spent.length > 0 || step.ruled !== null) {
      groups.push({
        key: step.key,
        label: step.title,
        steps: [step],
        failed: step.failed,
        spent: step.spent,
      });
      continue;
    }
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

export interface TrailTally {
  done: number;
  failed: number;
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

export function tallyLabel(tally: TrailTally): string {
  const steps = tally.done === 1 ? "1 step" : `${tally.done} steps`;
  return tally.failed > 0 ? `${steps}, ${tally.failed} failed` : steps;
}

const ECHOES = new Set([
  "browse",
  "use_screen",
  "open_on_desktop",
  "update_memory",
  "update_notes",
  "note_progress",
  "attach_file",
  "read_file",
  "write_document",
  "escalate",
]);

export function tellsMore(step: Step): boolean {
  return step.failed || !(ECHOES.has(step.tool) || pluginCall(step.tool) !== null);
}

export function saysMore(step: Step): boolean {
  if (step.ruled !== null && step.said === "allowed by a rule") return false;
  return step.failed || (step.target === null && tellsMore(step));
}

export function stepDiff(step: Step): DiffLine[] | null {
  if (step.replaced === null) return null;
  const diff = lineDiff(step.replaced, step.target ?? "");
  return diff.length > 0 ? diff : null;
}

export function stepReason(step: Step): string | null {
  return step.failed && step.target === null && step.said.length > 0 ? step.said : null;
}

export function hasDetail(group: TrailGroup): boolean {
  const only = group.steps[0]!;
  return (
    group.steps.length > 1 ||
    only.target !== null ||
    stepDiff(only) !== null ||
    stepReason(only) !== null
  );
}
