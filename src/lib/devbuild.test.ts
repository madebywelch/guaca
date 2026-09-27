import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Guards `pnpm app`, which nothing else in CI runs.
 *
 * Both of these broke without a failing build: one meant the dev app would not
 * start, the other meant it started as the installed app and managed the
 * operator's own local host. Checked by reading the files the tools read.
 */

const root = resolve(__dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

/** `[features]` from Cargo.toml, as name to entries. */
function features(): Map<string, string[]> {
  const section = read("src-tauri/Cargo.toml").match(/^\[features\]\n([\s\S]*?)(?=^\[)/m);
  if (!section) throw new Error("could not find [features] in Cargo.toml");
  const body = section[1]!.replace(/#.*$/gm, "");
  return new Map(
    [...body.matchAll(/^([\w-]+)\s*=\s*\[([\s\S]*?)\]/gm)].map((m) => [
      m[1]!,
      [...m[2]!.matchAll(/"([^"]+)"/g)].map((e) => e[1]!),
    ]),
  );
}

/** What `tauri dev` puts in place of the defaults: every default feature whose
 *  entries do not name `tauri/custom-protocol`, after `--no-default-features`. */
function tauriDevFeatures(): string[] {
  const all = features();
  return (all.get("default") ?? []).filter(
    (name) => !(all.get(name) ?? [name]).includes("tauri/custom-protocol"),
  );
}

/** The container name `LocalHost::new` derives from a bundle identifier. */
const hostName = (identifier: string) => `${identifier.replace(/[^A-Za-z0-9]/g, "-")}-host`;

describe("pnpm app", () => {
  it("keeps the desktop feature when tauri dev drops custom-protocol", () => {
    expect(tauriDevFeatures()).toContain("desktop");
  });

  it("names the binary a bare cargo run starts", () => {
    expect(read("src-tauri/Cargo.toml")).toMatch(/^default-run = "guac"$/m);
  });

  it("runs under an identifier whose local host is not the installed app's", () => {
    const installed = JSON.parse(read("src-tauri/tauri.conf.json")).identifier as string;
    const dev = JSON.parse(read("src-tauri/tauri.dev.conf.json")).identifier as string;
    expect(hostName(dev)).not.toBe(hostName(installed));

    const scripts = JSON.parse(read("package.json")).scripts as Record<string, string>;
    expect(scripts.app).toBe("./scripts/app.sh");
    expect(read("scripts/app.sh")).toMatch(
      /^exec pnpm tauri dev --config src-tauri\/tauri\.dev\.conf\.json/m,
    );
  });

  it("compiles in a host image this machine built rather than the unpublished tag", () => {
    const script = read("scripts/app.sh");
    const exported = script.indexOf("export GUACA_BACKEND_IMAGE=");
    expect(exported).toBeGreaterThan(-1);
    expect(exported).toBeLessThan(script.indexOf("exec pnpm tauri dev"));
  });
});
