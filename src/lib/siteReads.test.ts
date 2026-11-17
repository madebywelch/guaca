import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import { SITE_READS, siteText, siteValue } from "./siteReads";

const ROOT = resolve(__dirname, "../..");

const read = (path: string) => readFileSync(resolve(ROOT, path), "utf8");

describe("what guaca.bot's tests read from this repository", () => {
  it.each(Object.entries(SITE_READS))("%s is still at the path the list names", (_, { path }) => {
    expect(existsSync(resolve(ROOT, path)), path).toBe(true);
  });

  it.each(Object.keys(SITE_READS))("%s still matches its pattern", (name) => {
    expect(siteText(name, read).trim()).not.toBe("");
  });

  it.each(Object.entries(SITE_READS).filter(([, { literal }]) => literal))(
    "%s still parses as a literal",
    (name) => {
      expect(siteValue(name, read)).toBeTypeOf("object");
    },
  );

  it("refuses a name it does not list, so guaca.bot cannot read around it", () => {
    expect(() => siteText("src/App.tsx", read)).toThrow(/does not list/);
  });
});
