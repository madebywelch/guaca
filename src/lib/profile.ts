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

const SPELLED: Record<string, string> = { gpt: "GPT", ai: "AI", oss: "OSS", vl: "VL" };

export function modelName(id: string): string {
  const slug = id.trim().split("/").at(-1)?.split(":")[0] ?? "";
  const words = slug
    .replace(/[-_]\d{8}$/, "")
    .replace(/(?<![\d.])(\d)[-_](\d)(?![\d.])/g, "$1.$2")
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((word) => {
      const lower = word.toLowerCase();
      if (SPELLED[lower]) return SPELLED[lower];
      if (/^\d+(\.\d+)*[bkm]$/.test(lower)) return lower.toUpperCase();
      if (/^\d/.test(word) || /^o\d/.test(lower)) return lower;
      return word.charAt(0).toUpperCase() + word.slice(1);
    });
  const named = words.join(" ").replace(/^GPT (\d)/, "GPT-$1");
  return named || id.trim();
}
