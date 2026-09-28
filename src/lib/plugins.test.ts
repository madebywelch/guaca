import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import { BRANDS, hostOf, markFor, nameFor, reportLine } from "./plugins";
import type { ServerReport } from "./types";

function report(over: Partial<ServerReport> = {}): ServerReport {
  return {
    endpoint: "https://box.example.com/mcp",
    transport: "streamable HTTP",
    protocol: "2025-11-25",
    handshake: true,
    signin: "none",
    server: "Home Assistant",
    tools: ["turn_on", "turn_off"],
    ms: 84,
    ...over,
  };
}

describe("markFor", () => {
  it("draws a server nobody vouched for as a plug rather than nothing", () => {
    // A lookup with a truthy fallback finds `constructor` on the prototype and
    // draws an empty square, which reads as a broken row.
    expect(markFor("home_assistant").color).toBe("var(--accent)");
    expect(markFor("constructor" as never).color).toBe("var(--accent)");
    expect(markFor("neon").color).toBe("#34d59a");
  });
});

/**
 * The catalog as Rust ships it: every `PluginKind::X => "..."` arm of one
 * method, read out of `domain/plugin.rs`.
 *
 * Read from the source rather than listed again here, because a server added
 * to the Rust catalog and not to `BRANDS` compiles on both sides and draws as a
 * plug with a lowercase name: a vendor Guaca vouches for, shown as one nobody
 * did.
 */
function rustArms(method: string): Map<string, string> {
  const source = readFileSync(resolve(__dirname, "../../src-tauri/src/domain/plugin.rs"), "utf8");
  const body = source.match(
    new RegExp(`pub fn ${method}\\(&self\\) -> &str \\{([\\s\\S]*?)\\n    \\}`),
  );
  if (!body) throw new Error(`could not find PluginKind::${method} in domain/plugin.rs`);
  return new Map(
    [...body[1]!.matchAll(/PluginKind::(\w+) => "([^"]+)"/g)].map((m) => [m[1]!, m[2]!]),
  );
}

describe("the catalog", () => {
  it("draws and names every server Rust ships, and nothing else", () => {
    const slugs = rustArms("slug");
    const labels = rustArms("label");
    // A regex that stopped matching would pass the rest vacuously.
    expect(slugs.size).toBeGreaterThan(5);
    expect(labels.size).toBe(slugs.size);
    expect(Object.keys(BRANDS).sort()).toEqual([...slugs.values()].sort());
    for (const [variant, slug] of slugs) {
      expect(nameFor(slug)).toBe(labels.get(variant));
    }
  });
});

describe("nameFor", () => {
  it("spells the eight the way their vendors do", () => {
    // The one thing a slug cannot supply. A transcript is the only place that
    // needs this, because it is the only place with no row to read a name off.
    expect(nameFor("agentmail")).toBe("AgentMail");
    expect(nameFor("google")).toBe("Google");
  });

  it("calls a server the operator added what they called it", () => {
    // The same answer `PluginKind::label` gives for a custom row, and for the
    // same reason: nobody else has a name for it.
    expect(nameFor("home_assistant")).toBe("home_assistant");
    expect(nameFor("constructor" as never)).toBe("constructor");
  });
});

describe("hostOf", () => {
  it("falls back to the address when it is not one", () => {
    expect(hostOf("https://mcp.neon.tech/mcp")).toBe("mcp.neon.tech");
    expect(hostOf("stdio:npx -y @modelcontextprotocol/server-github")).toBe("npx on the host");
    expect(hostOf("not a url")).toBe("not a url");
  });
});

describe("reportLine", () => {
  it("names the tools rather than counting them", () => {
    // The operator is checking that what they expect is what this address
    // publishes. A count answers a question nobody has.
    const line = reportLine(report());
    expect(line).toContain("Home Assistant answered in 84 ms");
    expect(line).toContain("streamable HTTP, MCP 2025-11-25, with a handshake");
    expect(line).toContain("2 tools: turn_on, turn_off");
  });

  it("says which transport it was, because that is the field that costs the most", () => {
    const line = reportLine(report({ transport: "HTTP+SSE (2024-11-05)", handshake: true }));
    expect(line).toContain("HTTP+SSE (2024-11-05)");
  });

  it("tells a server that wants a sign-in apart from a key it refused", () => {
    // One status code, opposite problems. Told apart wrongly, an operator
    // re-pastes a key at a server that never wanted one.
    const wanted = reportLine(report({ signin: "wanted", transport: "", protocol: "", tools: [] }));
    expect(wanted).toContain("wants a sign-in");
    expect(wanted).not.toContain("MCP ");

    const refused = reportLine(
      report({ signin: "refused", transport: "", protocol: "", tools: [] }),
    );
    expect(refused).toContain("refused what you gave it");
    expect(refused).not.toContain("MCP ");
  });

  it("says that a server publishing nothing would offer the crew nothing", () => {
    // Reachable and useless is a real outcome, and "0 tools" reads as a bug in
    // Guaca rather than as the server having nothing on it.
    expect(reportLine(report({ tools: [] }))).toContain("would be offered nothing");
  });

  it("stands in for a server that does not name itself", () => {
    expect(reportLine(report({ server: "" }))).toMatch(/^It answered/);
  });
});
