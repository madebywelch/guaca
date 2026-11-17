import { ownModel } from "./providers";
import type { AgentCard, Group, Settings } from "./types";

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
