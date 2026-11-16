import { afterEach, expect, test, vi } from "vitest";

afterEach(() => {
  vi.unstubAllEnvs();
  vi.resetModules();
});

test("ordinary builds retain no diagnostic events", async () => {
  vi.stubEnv("MODE", "production");
  const trace = await import("./diagnostic");
  trace.diagnostic("select.requested", "private-agent-id");
  expect(trace.diagnosticRows().rows).toEqual([]);
});

test("probe events are bounded and retain opaque IDs rather than their source", async () => {
  vi.stubEnv("MODE", "probe");
  const trace = await import("./diagnostic");
  expect(trace.opaque("test")).toBe(2949673445);
  for (let index = 0; index < 300; index++)
    trace.diagnostic("select.committed", "private-agent-id", index);
  const result = trace.diagnosticRows();
  expect(result.rows).toHaveLength(256);
  expect(result.rows[0]?.value).toBe(44);
  expect(result.rows.at(-1)).toMatchObject({ phase: "select.committed", value: 299 });
  expect(JSON.stringify(result)).not.toContain("private-agent-id");
});
