import { paneName } from "./sections";
import type { AgentId, QuickAction } from "./types";

export function describeQuick(action: QuickAction, nameOf: (id: AgentId) => string): string {
  const { does } = action;
  if (does.kind === "message") return `Sends ${nameOf(does.agentId)}: “${does.text}”`;
  const place = does.place;
  switch (place.kind) {
    case "channel":
      return `Opens the conversation with ${nameOf(place.agentId)}`;
    case "calendar":
      return "Opens the calendar";
    case "forYou":
      return "Opens For you";
    case "settings": {
      const pane = paneName(place.section);
      return pane ? `Opens Settings on ${pane}` : "Opens Settings";
    }
    case "crewSettings":
      return "Opens a group's settings";
  }
}

export function quickTitle(action: QuickAction, nameOf: (id: AgentId) => string): string {
  const said = describeQuick(action, nameOf);
  return action.addedBy && action.addedBy !== "the operator"
    ? `${said}. Added by ${action.addedBy}, with your approval.`
    : said;
}
