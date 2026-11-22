import type { AgentCard, Group, Provider as ProviderKind, Settings } from "./types";

export interface Provider {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  local?: boolean;
  keyPage?: string;
  keySteps?: string[];
}

export const PROVIDERS: Provider[] = [
  {
    id: "openrouter",
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    model: "anthropic/claude-sonnet-4.5",
    keyPage: "https://openrouter.ai/settings/keys",
    keySteps: [
      "Sign in or create an OpenRouter account.",
      "Create a key, name it, and copy it.",
      "Add credits to your account; most models need them.",
    ],
  },
  {
    id: "openai",
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1",
    keyPage: "https://platform.openai.com/api-keys",
    keySteps: [
      "Sign in or create an OpenAI Platform account.",
      "Create a new secret key and copy it. It is shown only once.",
      "Add a payment method under Billing. A ChatGPT plan does not cover API use.",
    ],
  },
  {
    id: "groq",
    name: "Groq",
    baseUrl: "https://api.groq.com/openai/v1",
    model: "llama-3.3-70b-versatile",
  },
  {
    id: "together",
    name: "Together",
    baseUrl: "https://api.together.xyz/v1",
    model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
  },
  {
    id: "fireworks",
    name: "Fireworks",
    baseUrl: "https://api.fireworks.ai/inference/v1",
    model: "accounts/fireworks/models/llama-v3p3-70b-instruct",
  },
  {
    id: "lmstudio",
    name: "LM Studio",
    baseUrl: "http://localhost:1234/v1",
    model: "",
    local: true,
  },
  {
    id: "ollama",
    name: "Ollama",
    baseUrl: "http://localhost:11434/v1",
    model: "",
    local: true,
  },
];

export function providerFor(baseUrl: string): Provider | undefined {
  const needle = baseUrl
    .trim()
    .toLowerCase()
    .replace(/\/+$/, "")
    .replace(/\/chat\/completions$/, "")
    .replace(/\/+$/, "");
  if (!needle) return undefined;
  return PROVIDERS.find((provider) => provider.baseUrl.toLowerCase() === needle);
}

export function providerReady(provider: Provider, keySet: boolean): boolean {
  return provider.local === true || keySet;
}

export function inferenceOf(
  group: { inference: { provider: ProviderKind | null }; apiKeySet: boolean } | undefined,
  settings: Pick<Settings, "provider" | "apiKeySet">,
): { provider: ProviderKind; keyed: boolean } {
  return {
    provider: group?.inference.provider ?? settings.provider,
    keyed: (group?.apiKeySet ?? false) || settings.apiKeySet,
  };
}

export function onOpenRouter(group: Group | undefined, settings: Settings | null): boolean {
  const provider = group?.inference?.provider ?? settings?.provider;
  if (provider !== "compatible") return false;
  const baseUrl = group?.inference?.baseUrl || settings?.baseUrl || "";
  return providerFor(baseUrl)?.id === "openrouter";
}

export function ownModel(agent: AgentCard, provider: ProviderKind | null | undefined): string {
  if (provider === "claude") return "";
  return (provider === "chatgpt" ? agent.subscriptionModel : agent.model).trim();
}

export function planLabel(plan: string): string {
  const known: Record<string, string> = {
    free: "Free",
    plus: "Plus",
    pro: "Pro",
    team: "Team",
    business: "Business",
    enterprise: "Enterprise",
    edu: "Edu",
  };
  const key = plan.trim().toLowerCase();
  if (!key) return "";
  return known[key] ?? key.charAt(0).toUpperCase() + key.slice(1);
}
