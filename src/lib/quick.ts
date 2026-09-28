/**
 * The status bar's quick actions, described for a person.
 *
 * The same sentence the host tells an agent reading the settings
 * (`QuickAction::describe`), so the tooltip on a button and what an agent says
 * about it read alike. A button an agent asked for, from before agents pinned
 * pages instead, still says who asked.
 */
import type { AgentId, QuickAction } from "./types";

export function describeQuick(action: QuickAction, nameOf: (id: AgentId) => string): string {
  const { does } = action;
  if (does.kind === "message") return `Sends ${nameOf(does.agentId)}: “${does.text}”`;
  const place = does.place;
  switch (place.kind) {
    case "channel":
      return `Opens the channel with ${nameOf(place.agentId)}`;
    case "calendar":
      return "Opens the calendar";
    case "forYou":
      return "Opens For You";
    case "settings":
      return place.section ? `Opens Settings on ${place.section}` : "Opens Settings";
    case "crewSettings":
      return "Opens a crew's settings";
  }
}

/** The tooltip: what it does, and who put it there when that was not you. */
export function quickTitle(action: QuickAction, nameOf: (id: AgentId) => string): string {
  const said = describeQuick(action, nameOf);
  return action.addedBy && action.addedBy !== "the operator"
    ? `${said}. Added by ${action.addedBy}, with your approval.`
    : said;
}
