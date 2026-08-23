/**
 * Endpoints that are known to speak the protocol this app speaks.
 *
 * Guaca talks to one shape of API: OpenAI-compatible `/chat/completions`, with
 * streaming and tool calls. That is a wide field and the field is the problem —
 * the setting is a text box, and a base URL that is off by a path segment fails
 * on every turn of every agent with an error from a server rather than from
 * here. This is a list of the ones worth having spelled correctly.
 *
 * It is a starting point, never a restriction. Anything else is typed in, which
 * is what the box was for; a preset only fills it. `Custom` is not an entry in
 * the list, it is the absence of one, so a hand-typed endpoint reads as chosen
 * rather than as unrecognized.
 *
 * Local endpoints are marked because they change what the API key field means:
 * a server on this machine wants no key, and an empty key field beside a
 * warning about a missing key is the state an operator will try to fix forever.
 *
 * A ChatGPT subscription is deliberately not in this list. It is not an endpoint
 * with a different URL: it speaks a different protocol, authenticates by signing
 * in rather than by pasting, offers its own models and has no per-call price.
 * Putting it in a list of base URLs would say it is one row different from
 * OpenRouter, and the first thing an operator would do is look for the key field
 * it does not have. It gets its own block in the pane, above this list.
 */

import type { Group, Settings } from "./types";

export interface Provider {
  id: string;
  name: string;
  baseUrl: string;
  /** A model id that exists there, for the field beside it. */
  model: string;
  /** On this machine, and therefore wanting no key. */
  local?: boolean;
}

export const PROVIDERS: Provider[] = [
  {
    id: "openrouter",
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    model: "anthropic/claude-sonnet-4.5",
  },
  {
    id: "openai",
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1",
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

/**
 * The preset an endpoint is, if it is one.
 *
 * Normalized the same three ways `normalize_base_url` in `config.rs` does,
 * because the endpoint it stores is the one this has to recognize afterward.
 * Whitespace and case are the two ways a pasted URL differs from the same URL;
 * a trailing `/chat/completions` is the third and the most common, since it is
 * the URL a provider's own documentation prints. Recognizing only two of the
 * three had the chosen row flip across a save: the box read as unrecognized,
 * the backend trimmed the path, and the same setting then read as the preset.
 */
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

/**
 * Whether this endpoint is ready to be used, as far as this side can tell.
 *
 * "As far as this side can tell" is the whole caveat: a key that is present can
 * still be wrong, which is what Test connection is for. This answers the
 * cheaper question of whether there is any point pressing it.
 *
 * The key is one key, app-wide, belonging to whichever endpoint is configured.
 * So this is only a true statement about the provider currently chosen; asked
 * about any other row it reports on somebody else's key, which is why the UI
 * only shows it for the chosen one.
 */
export function providerReady(provider: Provider, keySet: boolean): boolean {
  return provider.local === true || keySet;
}

/**
 * Whether an agent in this group has its turns paid for by OpenRouter.
 *
 * Asked before OpenRouter's model rankings are offered as suggestions, because
 * a suggestion is a slug and a slug only means something at the endpoint it was
 * ranked at. `anthropic/claude-opus-5` pasted into a field pointed at
 * `api.openai.com` is a refusal by name on the agent's next turn, and the
 * operator has no way to connect that refusal to a button they pressed in a
 * dialog an hour earlier.
 *
 * Resolves group over app, the same order the backend resolves in. A subscription
 * is not this endpoint at all: it offers its own models, and none of them are
 * OpenRouter's to rank.
 */
export function onOpenRouter(group: Group | undefined, settings: Settings | null): boolean {
  // Optional all the way down, including a field the type says is always
  // there. This decides whether an extra appears beside a field; a group that
  // arrives half-shaped should cost the operator a suggestion, not the dialog.
  const provider = group?.inference?.provider ?? settings?.provider;
  if (provider !== "compatible") return false;
  const baseUrl = group?.inference?.baseUrl || settings?.baseUrl || "";
  return providerFor(baseUrl)?.id === "openrouter";
}

/**
 * How a ChatGPT plan is written for a person.
 *
 * The service sends lowercase identifiers, and the point of showing one at all
 * is so an operator can tell which of their accounts they signed in to. An
 * unrecognized plan is title-cased rather than replaced: a plan this list has
 * not heard of still works, and "Unknown" beside a working sign-in reads as a
 * problem.
 */
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
