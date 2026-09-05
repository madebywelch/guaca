import { describe, expect, it } from "vitest";
import { parseGroupFile } from "./transfer";

const good = {
  format: "guaca-group",
  version: 1,
  tables: { groups: [{ name: "Crew" }] },
  memories: {},
  files: {},
  reconnect: [],
};
describe("group file preview", () => {
  it("accepts a versioned group file", () =>
    expect(parseGroupFile(JSON.stringify(good)).tables.groups?.[0]?.name).toBe("Crew"));
  it.each([
    null,
    {},
    { ...good, version: 2 },
    { ...good, tables: { groups: [] } },
    { ...good, reconnect: [null] },
    { ...good, reconnect: [{ kind: "plugin", name: "Demo", details: {}, agents: "bad" }] },
  ])("refuses malformed data before rendering it", (value) =>
    expect(() => parseGroupFile(JSON.stringify(value))).toThrow(),
  );
});
