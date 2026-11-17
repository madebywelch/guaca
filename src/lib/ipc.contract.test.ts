import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(__dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

function calledCommands(): Set<string> {
  const source =
    read("src/lib/ipc.ts") +
    read("src/lib/host.ts") +
    read("src/lib/transfer.ts") +
    read("src/lib/transport.ts");
  return new Set([...source.matchAll(/invoke(?:Local)?<[^(]*\(\s*"([a-z_]+)"/g)].map((m) => m[1]!));
}

function registeredCommands(): Set<string> {
  const source = read("src-tauri/src/app.rs");
  const block = source.match(/generate_handler!\[([\s\S]*?)\]/);
  if (!block) throw new Error("could not find generate_handler! in app.rs");
  return new Set([...block[1]!.matchAll(/\b([a-z_]+)\b/g)].map((m) => m[1]!));
}

function surfaceCommands(): Set<string> {
  const source = read("src-tauri/src/ipc.rs");
  const block = source.match(/\nsurface! \{([\s\S]*?)\n\}/);
  if (!block) throw new Error("could not find the surface! list in ipc.rs");
  return new Set([...block[1]!.matchAll(/^\s{4}([a-z_]+)\(/gm)].map((m) => m[1]!));
}

function definedCommands(): Set<string> {
  const source = read("src-tauri/src/commands.rs");
  return new Set(
    [...source.matchAll(/^pub async fn ([a-z_]+)\(\n?\s*_?state: &AppState/gm)].map((m) => m[1]!),
  );
}

describe("IPC contract", () => {
  it("makes idempotency keys optional for older hire and send clients", () => {
    const surface = read("src-tauri/src/ipc.rs");
    expect(surface).toMatch(
      /hire_agents\(group_id: GroupId, drafts: Vec<AgentDraft>, idempotency_key: Option<String>\)/,
    );
    expect(surface).toMatch(
      /send_message\(agent_id: AgentId, text: String, files: Option<Vec<FileRef>>, idempotency_key: Option<String>\)/,
    );
  });

  it("allows older clients to save memory without an expected version", () => {
    expect(read("src-tauri/src/ipc.rs")).toMatch(
      /set_agent_memory\(id: AgentId, content: String, expected: Option<String>\)/,
    );
    expect(read("src/lib/ipc.ts")).toMatch(
      /setAgentMemory: \(id: AgentId, content: string, expected\?: string\)/,
    );
    expect(read("src-tauri/src/commands.rs")).toContain(
      ".write_versioned(id, &card.name, &content, expected.as_deref())",
    );
    expect(read("src-tauri/tests/memory_write_race.rs")).toMatch(
      /fn a_save_without_an_expected_version_stores_over_any_content\(\)/,
    );
  });
  it("requires the displayed artifact version and sources to approve reads", () => {
    const surface = read("src-tauri/src/ipc.rs");
    expect(surface).toMatch(
      /approve_artifact_sources\([^\n]*version:[^\n]*owner_id:[^\n]*sources:/,
    );
    expect(read("src/lib/ipc.ts")).toMatch(
      /invoke<Artifact>\("approve_artifact_sources",\s*\{\s*id,\s*version,\s*ownerId,\s*sources/,
    );
    expect(read("src-tauri/src/commands.rs")).toMatch(
      /fn allow_artifact_sources\([\s\S]*?Update Guaca on this device to approve this page/,
    );
  });
  it("reaches the desktop's own commands only through transport.ts", () => {
    const sources = (dir: string): string[] =>
      readdirSync(resolve(root, dir), { withFileTypes: true }).flatMap((entry) => {
        const path = `${dir}/${entry.name}`;
        if (entry.isDirectory()) return sources(path);
        return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [path] : [];
      });
    const bridged = sources("src").filter(
      (path) =>
        path !== "src/lib/transport.ts" &&
        /import\s*\{[^}]*\binvoke\b[^}]*\}\s*from\s*"@tauri-apps\/api\/core"|import\("@tauri-apps\/api\/core"\)/.test(
          read(path),
        ),
    );
    expect(bridged, "call the command through api in ipc.ts instead").toEqual([]);
  });

  it("finds commands on every side", () => {
    expect(calledCommands().size).toBeGreaterThan(5);
    expect(registeredCommands().size).toBeGreaterThan(2);
    expect(definedCommands().size).toBeGreaterThan(5);
    expect(surfaceCommands().size).toBeGreaterThan(5);
  });

  it("keeps runtime construction out of the native client", () => {
    const app = read("src-tauri/src/app.rs");
    expect(app).not.toMatch(/Runtime::|Store::open|start_scheduler|start_all/);
    expect(read("src/lib/transport.ts")).not.toContain("OVER_THE_BRIDGE");
  });

  it("implements every command on the surface", () => {
    const missing = [...surfaceCommands()].filter((name) => !definedCommands().has(name));
    expect(missing, "listed in surface! but not defined in commands.rs").toEqual([]);
  });

  it("registers every command the frontend calls", () => {
    const missing = [...calledCommands()].filter(
      (name) => !registeredCommands().has(name) && !surfaceCommands().has(name),
    );
    expect(missing, "called from ipc.ts but not registered in app.rs").toEqual([]);
  });

  it("registers every command it defines", () => {
    const orphans = [...definedCommands()].filter(
      (name) => !registeredCommands().has(name) && !surfaceCommands().has(name),
    );
    expect(orphans, "defined in commands.rs but never registered, so unreachable").toEqual([]);
  });

  it("exposes no command the frontend cannot reach", () => {
    const unused = [...registeredCommands()].filter((name) => !calledCommands().has(name));
    expect(unused, "registered but never called from the frontend").toEqual([]);
  });

  it("never exposes a command that could return the API key", () => {
    const source = read("src-tauri/src/commands.rs");
    expect(source).toMatch(/fn get_settings\([^)]*\)\s*->\s*Reply<RedactedConfig>/);
    expect(read("src-tauri/src/ipc.rs")).not.toMatch(/->\s*AppConfig,/);
    expect(source).not.toMatch(/->\s*Reply<AppConfig>/);
  });

  it("asks OpenRouter for its ranking only from the suggestions in an agent's dialog", () => {
    const callers = readdirSync(resolve(root, "src"), { recursive: true, encoding: "utf8" })
      .filter((path) => /\.tsx?$/.test(path) && !/\.test\.tsx?$/.test(path))
      .filter((path) => /\.rankedModels\(/.test(read(`src/${path}`)))
      .sort();

    expect(callers).toEqual(["components/ModelSuggestions.tsx"]);
  });

  it("knows the same twelve use cases on both sides", () => {
    const rust = read("src-tauri/src/commands/catalog.rs").match(
      /CATEGORIES: \[&str; \d+\] = \[([\s\S]*?)\];/,
    );
    const web = read("src/lib/roles.ts").match(/export const ROLES: Role\[\] = \[([\s\S]*?)\];/);
    expect(rust, "no CATEGORIES in catalog.rs").not.toBeNull();
    expect(web, "no ROLES in roles.ts").not.toBeNull();

    const quoted = (block: string) => [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]!);
    const ids = [...web![1]!.matchAll(/id:\s*"([^"]+)"/g)].map((m) => m[1]!);

    expect(ids).toEqual(quoted(rust![1]!));
  });

  it("knows the same activity states on both sides", () => {
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

  it("reports the menu bar the fields it reads, and no fewer", () => {
    const block = read("src-tauri/src/menubar.rs").match(/pub struct Presence \{([\s\S]*?)\n\}/);
    if (!block) throw new Error("could not find struct Presence in menubar.rs");
    const rust = [...block[1]!.matchAll(/^\s{4}pub ([a-z_]+):/gm)].map((found) =>
      found[1]!.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase()),
    );
    expect(rust.length).toBeGreaterThan(3);

    const shape = read("src/lib/types.ts").match(/export interface Presence \{([\s\S]*?)\n\}/);
    if (!shape) throw new Error("could not find interface Presence in types.ts");
    const web = [...shape[1]!.matchAll(/^\s{2}([a-zA-Z]+):/gm)].map((found) => found[1]!);

    expect([...web].sort()).toEqual([...rust].sort());
  });

  it("draws the same floor under a price on both sides", () => {
    const web = read("src/components/Spend.tsx").match(/MIN_PRICE = ([\d.e-]+)/);
    const rust = read("src-tauri/src/menubar.rs").match(/MIN_PRICE: f64 = ([\d._e-]+)/);
    expect(web, "no MIN_PRICE in Spend.tsx").not.toBeNull();
    expect(rust, "no MIN_PRICE in menubar.rs").not.toBeNull();
    expect(Number(rust![1]!.replaceAll("_", ""))).toBe(Number(web![1]!));
  });
});
