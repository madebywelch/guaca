import { ownModel } from "./providers";
import type { AgentCard, Group, Settings } from "./types";

/**
 * The one line under an agent's name: whose crew it is in, and what answers.
 *
 * The same resolution a turn makes, agent over crew over app, read back as a
 * sentence rather than as three fields. The model is the agent's own where it
 * set one, and the crew's or the app's default where it did not; `inherited`
 * says which, because the profile draws an inherited model a shade lighter
 * than one the agent chose. A Claude subscription chooses nothing, so it is
 * named rather than a model. A key with no model anywhere is a profile that
 * has to say so: the agent cannot answer until somebody sets one.
 */
export interface RunsOn {
  crew: string;
  model: string;
  inherited: boolean;
}

export function runsOn(
  agent: AgentCard,
  group: Group | undefined,
  settings: Settings | null,
): RunsOn {
  const crew = group?.name ?? "Independent agent";
  const provider = group?.inference.provider ?? settings?.provider ?? null;
  const own = ownModel(agent, provider);
  if (provider === "claude") return { crew, model: "Claude subscription", inherited: true };
  if (own) return { crew, model: own, inherited: false };
  const fallback =
    provider === "chatgpt"
      ? group?.inference.subscriptionModel || settings?.subscriptionModel || ""
      : group?.inference.defaultModel || settings?.defaultModel || "";
  return { crew, model: fallback || "No model set", inherited: true };
}
