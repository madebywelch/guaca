import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { aGroup } from "../test-fixtures";
import type { Provider } from "./providers";
import { onOpenRouter, PROVIDERS, providerFor, providerReady } from "./providers";
import type { Settings } from "./types";

const root = resolve(__dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

function rustConst(name: string): string {
  const source = read("src-tauri/src/config.rs");
  const match = source.match(new RegExp(`${name}: &str = "([^"]+)"`));
  if (!match) throw new Error(`could not find ${name} in config.rs`);
  return match[1]!;
}

function endpointPlaceholder(): string {
  const source = read("src/components/SettingsPage.tsx");
  const match = source.match(/Inference endpoint[\s\S]{0,400}?placeholder="([^"]+)"/);
  if (!match) throw new Error("could not find the endpoint field in SettingsPage.tsx");
  return match[1]!;
}

function preset(id: string): Provider {
  const found = PROVIDERS.find((provider) => provider.id === id);
  if (!found) throw new Error(`no preset called ${id}`);
  return found;
}

describe("an endpoint that is not a preset", () => {
  it("reads as no preset at all rather than as the first one in the list", () => {
    expect(providerFor("")).toBeUndefined();
  });

  it("reads as no preset when the box holds nothing but whitespace", () => {
    expect(providerFor("   ")).toBeUndefined();
    expect(providerFor("\n\t ")).toBeUndefined();
    expect(providerFor("/")).toBeUndefined();
    expect(providerFor("///")).toBeUndefined();
  });

  it("leaves an endpoint nobody ships unrecognized, because that is what chosen means", () => {
    expect(providerFor("https://llm.internal.example.com/v1")).toBeUndefined();
    expect(providerFor("http://localhost:8080/v1")).toBeUndefined();
  });

  it("refuses a near miss on a preset's own host", () => {
    const openai = preset("openai");
    expect(providerFor(`${openai.baseUrl}/beta`)).toBeUndefined();
    expect(providerFor("https://api.openai.com")).toBeUndefined();
    expect(providerFor("https://api.openai.com/v2")).toBeUndefined();
  });
});

describe("the same endpoint, pasted the way a provider prints it", () => {
  it("resolves a base that still names the completions path", () => {
    for (const provider of PROVIDERS) {
      expect(providerFor(`${provider.baseUrl}/chat/completions`), provider.name).toBe(provider);
    }
  });

  it("resolves it with a trailing slash on top, and in the wrong case", () => {
    const openai = preset("openai");
    expect(providerFor(`${openai.baseUrl}/chat/completions/`)).toBe(openai);
    expect(providerFor(`${openai.baseUrl.toUpperCase()}/CHAT/COMPLETIONS`)).toBe(openai);
  });

  it("still refuses a path that only looks like it", () => {
    expect(providerFor(`${preset("openai").baseUrl}/chat`)).toBeUndefined();
    expect(providerFor(`${preset("openai").baseUrl}/completions`)).toBeUndefined();
    expect(providerFor(`${preset("openai").baseUrl}/chat/completions/extra`)).toBeUndefined();
  });
});

describe("the same endpoint, pasted differently", () => {
  it("resolves a trailing slash, however many were pasted", () => {
    for (const provider of PROVIDERS) {
      expect(providerFor(`${provider.baseUrl}/`), provider.name).toBe(provider);
      expect(providerFor(`${provider.baseUrl}//`), provider.name).toBe(provider);
      expect(providerFor(`${provider.baseUrl}///`), provider.name).toBe(provider);
    }
  });

  it("resolves an endpoint typed in a different case", () => {
    for (const provider of PROVIDERS) {
      expect(providerFor(provider.baseUrl.toUpperCase()), provider.name).toBe(provider);
    }
    expect(providerFor("Https://Api.OpenAI.com/v1")).toBe(preset("openai"));
  });

  it("resolves an endpoint with whitespace around it", () => {
    for (const provider of PROVIDERS) {
      expect(providerFor(`  ${provider.baseUrl}  `), provider.name).toBe(provider);
      expect(providerFor(`\n${provider.baseUrl}\t`), provider.name).toBe(provider);
    }
  });

  it("resolves all three at once", () => {
    expect(providerFor("\n  HTTPS://Api.OpenAI.com/v1//  \t")).toBe(preset("openai"));
  });
});

describe("whether there is any point pressing Test connection", () => {
  it("ships both kinds of endpoint, so neither case below passes vacuously", () => {
    expect(PROVIDERS.some((provider) => provider.local === true)).toBe(true);
    expect(PROVIDERS.some((provider) => provider.local !== true)).toBe(true);
  });

  it("says a hosted endpoint is not ready until a key is stored", () => {
    for (const provider of PROVIDERS.filter((entry) => entry.local !== true)) {
      expect(providerReady(provider, false), `${provider.name} with no key`).toBe(false);
    }
  });

  it("says a local endpoint is ready with no key, because the server wants none", () => {
    for (const provider of PROVIDERS.filter((entry) => entry.local === true)) {
      expect(providerReady(provider, false), `${provider.name} with no key`).toBe(true);
      expect(providerReady(provider, true), `${provider.name} with a key`).toBe(true);
    }
  });

  it("says a hosted endpoint is ready once there is a key", () => {
    for (const provider of PROVIDERS.filter((entry) => entry.local !== true)) {
      expect(providerReady(provider, true), `${provider.name} with a key`).toBe(true);
    }
  });
});

describe("the preset list", () => {
  it("has presets to offer", () => {
    expect(PROVIDERS.length).toBeGreaterThan(3);
  });

  it("gives every preset an id nothing else uses", () => {
    const ids = PROVIDERS.map((provider) => provider.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("gives every preset a name to show", () => {
    for (const provider of PROVIDERS) {
      expect(provider.name.trim(), `${provider.id}'s name`).not.toBe("");
    }
  });

  it("gives every preset a base URL that parses", () => {
    for (const provider of PROVIDERS) {
      expect(
        () => new URL(provider.baseUrl),
        `${provider.name} claims ${provider.baseUrl}`,
      ).not.toThrow();
    }
  });

  it("spells every base URL the way the backend would store it", () => {
    for (const provider of PROVIDERS) {
      const url = provider.baseUrl;
      expect(url.startsWith("http://") || url.startsWith("https://"), `${url} scheme`).toBe(true);
      expect(url.endsWith("/"), `${url} ends with a slash`).toBe(false);
      expect(url.endsWith("/chat/completions"), `${url} names the endpoint`).toBe(false);
    }
  });

  it("lets every preset find itself", () => {
    for (const provider of PROVIDERS) {
      expect(providerFor(provider.baseUrl), `${provider.name} lost itself`).toBe(provider);
    }
  });

  it("does not have two presets claiming one endpoint", () => {
    const urls = PROVIDERS.map((provider) => provider.baseUrl.toLowerCase());
    expect(new Set(urls).size).toBe(urls.length);
  });

  it("names a model on every hosted preset", () => {
    for (const provider of PROVIDERS.filter((entry) => entry.local !== true)) {
      expect(provider.model.trim(), `${provider.name}'s model`).not.toBe("");
    }
  });
});

describe("what a fresh install starts on", () => {
  it("starts on a preset, endpoint and model together", () => {
    const chosen = providerFor(rustConst("DEFAULT_BASE_URL"));
    expect(chosen, `no preset ships ${rustConst("DEFAULT_BASE_URL")}`).toBeDefined();
    expect(chosen?.model).toBe(rustConst("DEFAULT_MODEL"));
  });

  it("suggests that same endpoint in the empty box", () => {
    expect(endpointPlaceholder()).toBe(rustConst("DEFAULT_BASE_URL"));
  });
});

describe("whether OpenRouter is what pays", () => {
  const app = (over: Partial<Settings> = {}) =>
    ({ provider: "compatible", baseUrl: "https://openrouter.ai/api/v1", ...over }) as Settings;

  it("follows the app when the crew overrides nothing", () => {
    expect(onOpenRouter(aGroup(), app())).toBe(true);
    expect(onOpenRouter(aGroup(), app({ baseUrl: "https://api.openai.com/v1" }))).toBe(false);
  });

  it("lets a crew's own endpoint win, in both directions", () => {
    const elsewhere = aGroup({
      inference: { ...aGroup().inference, baseUrl: "https://api.groq.com/openai/v1" },
    });
    expect(onOpenRouter(elsewhere, app())).toBe(false);

    const here = aGroup({
      inference: { ...aGroup().inference, baseUrl: "https://openrouter.ai/api/v1" },
    });
    expect(onOpenRouter(here, app({ baseUrl: "http://localhost:1234/v1" }))).toBe(true);
  });

  it("is never true on a subscription, whatever endpoint is stored beside it", () => {
    expect(onOpenRouter(aGroup(), app({ provider: "chatgpt" }))).toBe(false);

    const paid = aGroup({ inference: { ...aGroup().inference, provider: "chatgpt" } });
    expect(onOpenRouter(paid, app())).toBe(false);
  });

  it("says no before anything is known", () => {
    expect(onOpenRouter(undefined, null)).toBe(false);
    expect(onOpenRouter(aGroup(), null)).toBe(false);
  });

  it("recognizes the endpoint the way it is actually pasted", () => {
    expect(
      onOpenRouter(aGroup(), app({ baseUrl: "https://openrouter.ai/api/v1/chat/completions" })),
    ).toBe(true);
    expect(onOpenRouter(aGroup(), app({ baseUrl: " HTTPS://OpenRouter.ai/api/v1/ " }))).toBe(true);
  });
});

describe("where to get a key", () => {
  it("links only to a key page on the provider's own site, with at most three steps", () => {
    const linked = PROVIDERS.filter((provider) => provider.keyPage);
    expect(linked.map((provider) => provider.id)).toEqual(["openrouter", "openai"]);
    for (const provider of linked) {
      const page = new URL(provider.keyPage!);
      const api = new URL(provider.baseUrl);
      const site = api.hostname.split(".").slice(-2).join(".");
      expect(page.protocol, provider.id).toBe("https:");
      expect(page.hostname === site || page.hostname.endsWith(`.${site}`), provider.id).toBe(true);
      expect(provider.keySteps?.length ?? 0, provider.id).toBeGreaterThan(0);
      expect(provider.keySteps!.length, provider.id).toBeLessThanOrEqual(3);
    }
    expect(PROVIDERS.filter((provider) => provider.local).some((p) => p.keyPage)).toBe(false);
  });
});
