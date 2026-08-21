import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { Provider } from "./providers";
import { PROVIDERS, providerFor, providerReady } from "./providers";

/**
 * The preset list is data, and both of its jobs are answered from one string.
 *
 * `providerFor` decides which row the dialog draws as chosen, so it has to
 * survive the two ways an operator's paste differs from the same URL and it
 * has to refuse everything else: a hand-typed endpoint must read as chosen,
 * not as one of these with the highlight on the wrong row. And because the
 * Rust side normalises a base URL before it stores one, a preset that is not
 * already in that canonical form saves fine and comes back unrecognised, which
 * looks like the app forgetting the choice that was just made.
 *
 * Nothing here needs a running app. A preset the backend would rewrite, or one
 * that cannot find itself, is wrong the moment it is written.
 */

const root = resolve(__dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

/** A constant from `src-tauri/src/config.rs`, read rather than copied. */
function rustConst(name: string): string {
  const source = read("src-tauri/src/config.rs");
  const match = source.match(new RegExp(`${name}: &str = "([^"]+)"`));
  if (!match) throw new Error(`could not find ${name} in config.rs`);
  return match[1]!;
}

/** What the endpoint box suggests when it is empty. */
function endpointPlaceholder(): string {
  const source = read("src/components/SettingsDialog.tsx");
  const match = source.match(/Inference endpoint[\s\S]{0,400}?placeholder="([^"]+)"/);
  if (!match) throw new Error("could not find the endpoint field in SettingsDialog.tsx");
  return match[1]!;
}

function preset(id: string): Provider {
  const found = PROVIDERS.find((provider) => provider.id === id);
  if (!found) throw new Error(`no preset called ${id}`);
  return found;
}

describe("an endpoint that is not a preset", () => {
  it("reads as no preset at all rather than as the first one in the list", () => {
    // First run hands this an empty setting. Falling back to PROVIDERS[0] would
    // draw OpenRouter as chosen before anyone chose anything, and the operator
    // would be looking at a highlighted row beside an empty box.
    expect(providerFor("")).toBeUndefined();
  });

  it("reads as no preset when the box holds nothing but whitespace", () => {
    expect(providerFor("   ")).toBeUndefined();
    expect(providerFor("\n\t ")).toBeUndefined();
    // Trimming leaves a bare separator empty too, so it must not match either.
    expect(providerFor("/")).toBeUndefined();
    expect(providerFor("///")).toBeUndefined();
  });

  it("leaves an endpoint nobody ships unrecognised, because that is what chosen means", () => {
    // `Custom` is the absence of an entry. Answering with the nearest preset
    // would put the highlight on a row that is not what the app would call.
    expect(providerFor("https://llm.internal.example.com/v1")).toBeUndefined();
    expect(providerFor("http://localhost:8080/v1")).toBeUndefined();
  });

  it("refuses a near miss on a preset's own host", () => {
    // The comparison has to be the whole string. A `startsWith` or `endsWith`
    // implementation would claim both of these, and an endpoint one path
    // segment off fails on every turn of every agent.
    const openai = preset("openai");
    expect(providerFor(`${openai.baseUrl}/beta`)).toBeUndefined();
    expect(providerFor("https://api.openai.com")).toBeUndefined();
    expect(providerFor("https://api.openai.com/v2")).toBeUndefined();
  });
});

describe("the same endpoint, pasted the way a provider prints it", () => {
  it("resolves a base that still names the completions path", () => {
    // The most common paste there is, because it is the URL in a provider's own
    // documentation. The Rust normaliser strips it before storing, so
    // recognising it here is what stops the chosen row flipping across a save:
    // the box read as unrecognised, the backend trimmed the path, and the very
    // same setting then read as the preset.
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
    // Only a trailing `/chat/completions` is a known paste error. Anything else
    // after the base is a different endpoint and must read as chosen.
    expect(providerFor(`${preset("openai").baseUrl}/chat`)).toBeUndefined();
    expect(providerFor(`${preset("openai").baseUrl}/completions`)).toBeUndefined();
    expect(providerFor(`${preset("openai").baseUrl}/chat/completions/extra`)).toBeUndefined();
  });
});

describe("the same endpoint, pasted differently", () => {
  it("resolves a trailing slash, however many were pasted", () => {
    // A copied base URL arrives with one; a hand-assembled one arrives with
    // two. Neither is a different endpoint, and the Rust side stores them the
    // same way, so every preset has to survive both.
    for (const provider of PROVIDERS) {
      expect(providerFor(`${provider.baseUrl}/`), provider.name).toBe(provider);
      expect(providerFor(`${provider.baseUrl}//`), provider.name).toBe(provider);
      expect(providerFor(`${provider.baseUrl}///`), provider.name).toBe(provider);
    }
  });

  it("resolves an endpoint typed in a different case", () => {
    // Local endpoints get typed out by hand rather than pasted, which is where
    // the case comes from, and a host is case-insensitive anyway.
    for (const provider of PROVIDERS) {
      expect(providerFor(provider.baseUrl.toUpperCase()), provider.name).toBe(provider);
    }
    expect(providerFor("Https://Api.OpenAI.com/v1")).toBe(preset("openai"));
  });

  it("resolves an endpoint with whitespace around it", () => {
    // A paste out of a terminal or a docs page brings its own padding.
    for (const provider of PROVIDERS) {
      expect(providerFor(`  ${provider.baseUrl}  `), provider.name).toBe(provider);
      expect(providerFor(`\n${provider.baseUrl}\t`), provider.name).toBe(provider);
    }
  });

  it("resolves all three at once", () => {
    // The realistic paste is not one of these, it is all of them.
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
    // The reason the flag exists: a server on this machine takes no key, and an
    // empty key field beside a missing-key warning is a state an operator will
    // try to fix forever.
    for (const provider of PROVIDERS.filter((entry) => entry.local === true)) {
      expect(providerReady(provider, false), `${provider.name} with no key`).toBe(true);
      // And a key stored for some other endpoint does not change the answer.
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
    // Without this the loops below pass on an empty array.
    expect(PROVIDERS.length).toBeGreaterThan(3);
  });

  it("gives every preset an id nothing else uses", () => {
    // The id is the key of the rendered row and the answer to "which one is
    // chosen". Two presets sharing one draws the highlight on both.
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
    // `normalize_base_url` in `src-tauri/src/config.rs` refuses anything without
    // one of these schemes, and strips a trailing slash and a trailing
    // `/chat/completions`. A preset that is not already in that form is a
    // choice the operator makes, saves, and watches come back as Custom.
    for (const provider of PROVIDERS) {
      const url = provider.baseUrl;
      expect(url.startsWith("http://") || url.startsWith("https://"), `${url} scheme`).toBe(true);
      expect(url.endsWith("/"), `${url} ends with a slash`).toBe(false);
      expect(url.endsWith("/chat/completions"), `${url} names the endpoint`).toBe(false);
    }
  });

  it("lets every preset find itself", () => {
    // The property the rest of this file rests on: whatever `choose` puts in
    // the box, and whatever the backend gives back after a save, must resolve
    // to the row that was clicked.
    for (const provider of PROVIDERS) {
      expect(providerFor(provider.baseUrl), `${provider.name} lost itself`).toBe(provider);
    }
  });

  it("does not have two presets claiming one endpoint", () => {
    // `providerFor` can only answer with one of them, so the other can never be
    // drawn as chosen no matter how it was picked.
    const urls = PROVIDERS.map((provider) => provider.baseUrl.toLowerCase());
    expect(new Set(urls).size).toBe(urls.length);
  });

  it("names a model on every hosted preset", () => {
    // Choosing a preset fills the endpoint and the model, which is what the
    // dialog promises. A hosted preset with no model leaves the model box
    // holding whatever the last endpoint used. Local presets carry none on
    // purpose: the model is whatever that server has loaded.
    for (const provider of PROVIDERS.filter((entry) => entry.local !== true)) {
      expect(provider.model.trim(), `${provider.name}'s model`).not.toBe("");
    }
  });
});

describe("what a fresh install starts on", () => {
  it("starts on a preset, endpoint and model together", () => {
    // The backend's defaults are what an operator sees before touching
    // anything. If they are not one of these rows, the dialog opens on Custom
    // for a configuration nobody typed.
    const chosen = providerFor(rustConst("DEFAULT_BASE_URL"));
    expect(chosen, `no preset ships ${rustConst("DEFAULT_BASE_URL")}`).toBeDefined();
    expect(chosen?.model).toBe(rustConst("DEFAULT_MODEL"));
  });

  it("suggests that same endpoint in the empty box", () => {
    // The placeholder is a third copy of the default. One that drifts advertises
    // an endpoint the app would not actually call.
    expect(endpointPlaceholder()).toBe(rustConst("DEFAULT_BASE_URL"));
  });
});
