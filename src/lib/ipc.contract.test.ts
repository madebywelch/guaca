import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Guards the seam between Rust and TypeScript.
 *
 * A misspelled command name compiles cleanly on both sides and fails only when
 * a user clicks the thing. Nothing else in the build catches it, so it is
 * checked here by reading the two sources and comparing them.
 */

const root = resolve(__dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

/** Command names the frontend calls. */
function calledCommands(): Set<string> {
  const source = read("src/lib/ipc.ts");
  // Matches up to the opening paren rather than the first `>`, so a nested
  // generic like `Record<AgentId, Activity>` does not cut the match short.
  return new Set([...source.matchAll(/invoke<[^(]*\(\s*"([a-z_]+)"/g)].map((m) => m[1]!));
}

/** Command names registered with Tauri. */
function registeredCommands(): Set<string> {
  const source = read("src-tauri/src/app.rs");
  const block = source.match(/generate_handler!\[([\s\S]*?)\]/);
  if (!block) throw new Error("could not find generate_handler! in app.rs");
  return new Set([...block[1]!.matchAll(/commands::([a-z_]+)/g)].map((m) => m[1]!));
}

/** Functions in commands.rs annotated as Tauri commands. */
function definedCommands(): Set<string> {
  const source = read("src-tauri/src/commands.rs");
  return new Set(
    [...source.matchAll(/#\[tauri::command\][\s\S]{0,80}?fn\s+([a-z_]+)/g)].map((m) => m[1]!),
  );
}

describe("IPC contract", () => {
  it("finds commands on both sides", () => {
    // If a regex stops matching, the rest of this file would pass vacuously.
    expect(calledCommands().size).toBeGreaterThan(5);
    expect(registeredCommands().size).toBeGreaterThan(5);
    expect(definedCommands().size).toBeGreaterThan(5);
  });

  it("registers every command the frontend calls", () => {
    const missing = [...calledCommands()].filter((name) => !registeredCommands().has(name));
    expect(missing, "called from ipc.ts but not registered in app.rs").toEqual([]);
  });

  it("defines every command it registers", () => {
    const missing = [...registeredCommands()].filter((name) => !definedCommands().has(name));
    expect(missing, "registered in app.rs but not defined in commands.rs").toEqual([]);
  });

  it("registers every command it defines", () => {
    const orphans = [...definedCommands()].filter((name) => !registeredCommands().has(name));
    expect(orphans, "defined in commands.rs but never registered, so unreachable").toEqual([]);
  });

  it("exposes no command the frontend cannot reach", () => {
    // The IPC surface is the app's entire attack surface. Anything registered
    // and unused is surface with no purpose.
    const unused = [...registeredCommands()].filter((name) => !calledCommands().has(name));
    expect(unused, "registered but never called from the frontend").toEqual([]);
  });

  it("never exposes a command that could return the API key", () => {
    const source = read("src-tauri/src/commands.rs");
    // get_settings must hand back the redacted view, never AppConfig.
    expect(source).toMatch(/fn get_settings\([^)]*\)\s*->\s*Reply<RedactedConfig>/);
    expect(source).not.toMatch(/->\s*Reply<AppConfig>/);
  });

  it("knows the same twelve use cases on both sides", () => {
    // A suggestion is asked for by use case, and the backend refuses anything
    // that is not one of these before it spends a request. So a category
    // renamed at OpenRouter and updated on one side only is a dialog that draws
    // nothing, silently, for exactly the agents it was built for. Neither list
    // is the source — OpenRouter is — and the pair failing together is how a
    // rename there gets noticed here.
    const rust = read("src-tauri/src/llm/catalog.rs").match(
      /CATEGORIES: \[&str; \d+\] = \[([\s\S]*?)\];/,
    );
    const web = read("src/lib/roles.ts").match(/export const ROLES: Role\[\] = \[([\s\S]*?)\];/);
    expect(rust, "no CATEGORIES in catalog.rs").not.toBeNull();
    expect(web, "no ROLES in roles.ts").not.toBeNull();

    const quoted = (block: string) => [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]!);
    // The web list is `{ id, label }` pairs and only the id crosses IPC.
    const ids = [...web![1]!.matchAll(/id:\s*"([^"]+)"/g)].map((m) => m[1]!);

    expect(ids).toEqual(quoted(rust![1]!));
  });

  it("draws the same floor under a price on both sides", () => {
    // The rail's meters and the menu bar are two readings of one number, and
    // each decides on its own whether a cost is worth the width it takes. A
    // free model prices every call at a real zero, so the two agreeing is what
    // keeps `$0.0000` off one surface after it was taken off the other. Nothing
    // else in the build compares them: the rule is written twice because one
    // reader is TypeScript and the other is Rust.
    const web = read("src/components/TokenMeter.tsx").match(/MIN_PRICE = ([\d.e-]+)/);
    const rust = read("src-tauri/src/menubar.rs").match(/MIN_PRICE: f64 = ([\d._e-]+)/);
    expect(web, "no MIN_PRICE in TokenMeter.tsx").not.toBeNull();
    expect(rust, "no MIN_PRICE in menubar.rs").not.toBeNull();
    // Rust spells long numbers with underscores; the value is what has to match.
    expect(Number(rust![1]!.replaceAll("_", ""))).toBe(Number(web![1]!));
  });
});
