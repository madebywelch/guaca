import { describe, expect, it } from "vitest";

import { hostOf, markFor, reportLine } from "./plugins";
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

describe("hostOf", () => {
  it("falls back to the address when it is not one", () => {
    expect(hostOf("https://mcp.neon.tech/mcp")).toBe("mcp.neon.tech");
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
