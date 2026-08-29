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

/**
 * Command names registered with Tauri.
 *
 * The handler list names the generated wrappers rather than the commands
 * themselves, because `surface!` is what writes them. It is still its own list
 * and still has to be kept in step: the macro cannot register anything with
 * Tauri on its own.
 */
function registeredCommands(): Set<string> {
  const source = read("src-tauri/src/app.rs");
  const block = source.match(/generate_handler!\[([\s\S]*?)\]/);
  if (!block) throw new Error("could not find generate_handler! in app.rs");
  return new Set([...block[1]!.matchAll(/ipc::desktop::([a-z_]+)/g)].map((m) => m[1]!));
}

/**
 * The command surface, which is one list in Rust serving two transports.
 *
 * `surface!` in ipc.rs generates the Tauri wrappers, the HTTP dispatch and the
 * `NAMES` constant from this. It is the authority for what a build answers to,
 * and it is why the desktop and a server cannot answer to different sets.
 */
function surfaceCommands(): Set<string> {
  const source = read("src-tauri/src/ipc.rs");
  const block = source.match(/\nsurface! \{([\s\S]*?)\n\}/);
  if (!block) throw new Error("could not find the surface! list in ipc.rs");
  return new Set([...block[1]!.matchAll(/^\s{4}([a-z_]+)\(/gm)].map((m) => m[1]!));
}

/** Functions in commands.rs that a transport can call. */
function definedCommands(): Set<string> {
  const source = read("src-tauri/src/commands.rs");
  return new Set(
    [...source.matchAll(/^pub async fn ([a-z_]+)\(\n?\s*_?state: &AppState/gm)].map((m) => m[1]!),
  );
}

describe("IPC contract", () => {
  it("finds commands on every side", () => {
    // If a regex stops matching, the rest of this file would pass vacuously.
    expect(calledCommands().size).toBeGreaterThan(5);
    expect(registeredCommands().size).toBeGreaterThan(5);
    expect(definedCommands().size).toBeGreaterThan(5);
    expect(surfaceCommands().size).toBeGreaterThan(5);
  });

  it("answers to the same commands over both transports", () => {
    // The failure this exists for is a command reachable from the desktop and
    // not from a server, which is a panel that works at your desk and fails on
    // your box with nothing on screen saying which half is missing. `surface!`
    // makes that impossible inside Rust; this is the check that the Tauri
    // handler list, which the macro cannot write, still names all of them.
    const surface = surfaceCommands();
    const registered = registeredCommands();
    expect(
      [...surface].filter((name) => !registered.has(name)),
      "on the surface, so served over HTTP, but never registered with Tauri",
    ).toEqual([]);
    expect(
      [...registered].filter((name) => !surface.has(name)),
      "registered with Tauri but not on the surface, so it cannot compile",
    ).toEqual([]);
  });

  it("implements every command on the surface", () => {
    const missing = [...surfaceCommands()].filter((name) => !definedCommands().has(name));
    expect(missing, "listed in surface! but not defined in commands.rs").toEqual([]);
  });

  it("registers every command the frontend calls", () => {
    const missing = [...calledCommands()].filter((name) => !registeredCommands().has(name));
    expect(missing, "called from ipc.ts but not registered in app.rs").toEqual([]);
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
    // And the surface must not be able to name it either.
    expect(read("src-tauri/src/ipc.rs")).not.toMatch(/->\s*AppConfig,/);
    expect(source).not.toMatch(/->\s*Reply<AppConfig>/);
  });

  it("knows the same coding harnesses on both sides", () => {
    // A harness is a program, and adding one is two edits: a variant in Rust
    // and a row in `HARNESSES`. Only the Rust half decides what a job starts,
    // so a variant added there and forgotten here is a harness the operator
    // cannot choose, and a row added here and forgotten there is a choice that
    // stores a value the store reads back as `pi`. Neither shows up until
    // somebody's coding job runs the wrong program.
    const rust = new Set(
      [
        ...read("src-tauri/src/domain/repository.rs").matchAll(/Harness::(\w+) => "([a-z]+)",/g),
      ].map((m) => m[2]!),
    );
    const web = new Set(
      [...read("src/lib/types.ts").matchAll(/\{ id: "([a-z]+)", label: /g)].map((m) => m[1]!),
    );
    expect(rust.size, "the Rust harnesses did not parse").toBeGreaterThan(1);
    expect([...web].sort(), "HARNESSES and Harness disagree").toEqual([...rust].sort());
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

  it("knows the same activity states on both sides", () => {
    // The group column folds this enum into two marks: a count of the agents a
    // person has to unblock, and whether anybody is working. `presenceOf` names
    // every state rather than defaulting, so once the union here matches the
    // enum there, a variant nobody weighed fails the typecheck. This is the
    // half the typechecker cannot see: a variant added in Rust and never
    // written down in TypeScript at all.
    const rust = read("src-tauri/src/runtime/events.rs");
    const block = rust.match(/pub enum Activity \{([\s\S]*?)\n\}/);
    if (!block) throw new Error("could not find enum Activity in events.rs");
    const variants = [...block[1]!.matchAll(/^\s{4}([A-Z][A-Za-z]*)/gm)].map(
      (found) => found[1]![0]!.toLowerCase() + found[1]!.slice(1),
    );
    expect(variants.length).toBeGreaterThan(3);

    const ours = read("src/lib/types.ts");
    const union = ours.match(/export type Activity =([\s\S]*?);\n/);
    if (!union) throw new Error("could not find the Activity union in types.ts");
    const known = [...union[1]!.matchAll(/state: "([a-zA-Z]+)"/g)].map((found) => found[1]!);

    expect([...variants].sort()).toEqual([...known].sort());
  });

  it("agrees on where the menu bar can send the window", () => {
    // The strip points at two things and the window answers them with two
    // different calls. A variant added in Rust and never written down here is a
    // click that arrives and does nothing: the payload lands, no branch matches
    // it, and nothing else in the build has an opinion about that.
    const block = read("src-tauri/src/tray.rs").match(/pub enum Reveal \{([\s\S]*?)\n\}/);
    if (!block) throw new Error("could not find enum Reveal in tray.rs");
    const variants = [...block[1]!.matchAll(/^\s{4}([A-Z][A-Za-z]*)/gm)].map(
      (found) => found[1]![0]!.toLowerCase() + found[1]!.slice(1),
    );
    expect(variants.length).toBeGreaterThan(1);

    const union = read("src/lib/types.ts").match(/export type Reveal =([\s\S]*?);\n/);
    if (!union) throw new Error("could not find the Reveal union in types.ts");
    const known = [...union[1]!.matchAll(/kind: "([a-zA-Z]+)"/g)].map((found) => found[1]!);

    expect([...variants].sort()).toEqual([...known].sort());
  });

  it("draws the same floor under a price on both sides", () => {
    // The rail's meters and the menu bar are two readings of one number, and
    // each decides on its own whether a cost is worth the width it takes. A
    // free model prices every call at a real zero, so the two agreeing is what
    // keeps `$0.0000` off one surface after it was taken off the other. Nothing
    // else in the build compares them: the rule is written twice because one
    // reader is TypeScript and the other is Rust.
    const web = read("src/components/Spend.tsx").match(/MIN_PRICE = ([\d.e-]+)/);
    const rust = read("src-tauri/src/menubar.rs").match(/MIN_PRICE: f64 = ([\d._e-]+)/);
    expect(web, "no MIN_PRICE in Spend.tsx").not.toBeNull();
    expect(rust, "no MIN_PRICE in menubar.rs").not.toBeNull();
    // Rust spells long numbers with underscores; the value is what has to match.
    expect(Number(rust![1]!.replaceAll("_", ""))).toBe(Number(web![1]!));
  });
});
