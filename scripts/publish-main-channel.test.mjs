import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { generateKeyPairSync } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { envelope, signature } from "./release-manifest.mjs";

const workflow = (name) => readFileSync(new URL(`../.github/workflows/${name}.yml`, import.meta.url), "utf8");
const main = workflow("main-channel");
const release = workflow("release-image");
const publish = main.slice(main.indexOf("\n  publish:"));
const guard = publish.match(/id: tip\n        run: \|\n((?: {10}.*\n)+)/)?.[1];
const sha = "a".repeat(40);
const older = "b".repeat(40);
const digest = `sha256:${"c".repeat(64)}`;
const image = "ghcr.io/madebywelch/guaca/guacad";
const { privateKey, publicKey } = generateKeyPairSync("ed25519");

function checkFeed(t, { published, isAncestor = false, failedFetch = false, badSignature = false, remoteMain = older }) {
  const directory = mkdtempSync(join(tmpdir(), "guaca-main-feed-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const output = join(directory, "output");
  const bytes = Buffer.from(JSON.stringify({ schema: 1, channel: "main", commit: published, image: `${image}@${digest}` }));
  const manifest = envelope(bytes, badSignature ? "" : signature(bytes, privateKey)).toString().trim();
  writeFileSync(join(directory, "main-keys.pub"), publicKey.export({ type: "spki", format: "der" }).subarray(-32).toString("base64"));
  symlinkSync(new URL(".", import.meta.url), join(directory, "scripts"));
  const mockGit = `
git() {
  case "$1 $2 ${"$"}{3:-}" in
    'ls-remote origin refs/heads/main-feed') echo 'existing ref' ;;
    'ls-remote --exit-code origin') printf '%s\\trefs/heads/main\\n' "$REMOTE_MAIN" ;;
    'fetch --quiet origin') return "$FETCH_STATUS" ;;
    'rev-parse FETCH_HEAD ') echo feed-tip ;;
    'show FETCH_HEAD:guaca-main.json ') printf '%s\\n' "$MANIFEST" ;;
    'merge-base --is-ancestor'*) return "$ANCESTOR_STATUS" ;;
    *) return 1 ;;
  esac
}
`;
  const run = spawnSync("bash", ["-e", "-o", "pipefail", "-c", mockGit + guard], {
    cwd: directory,
    encoding: "utf8",
    env: {
      PATH: process.env.PATH,
      GITHUB_SHA: sha,
      GITHUB_OUTPUT: output,
      IMAGE: image,
      MANIFEST: manifest,
      REMOTE_MAIN: remoteMain,
      FETCH_STATUS: failedFetch ? "1" : "0",
      ANCESTOR_STATUS: isAncestor ? "0" : "1",
    },
  });
  assert.ifError(run.error);
  return { code: run.status, output: run.status === 0 ? readFileSync(output, "utf8") : "" };
}

test("an old retry leaves both the feed and mutable main tag alone", (t) => {
  assert.match(main, /group: main-channel\n  queue: max/);
  const { code, output } = checkFeed(t, { published: older });
  assert.equal(code, 0);
  assert.equal(output, "feed=feed-tip\n");
  const mainTag = publish.slice(publish.indexOf("- name: Point the main tag"));
  assert.match(mainTag, /if: steps\.tip\.outputs\.current == 'true' \|\| steps\.tip\.outputs\.digest != ''/);
});

test("a retry after the feed push uses the digest already signed, not a rebuilt image", (t) => {
  assert.deepEqual(checkFeed(t, { published: sha }), {
    code: 0,
    output: `feed=feed-tip\ndigest=${digest}\n`,
  });
  assert.ok(publish.indexOf("- name: Point the main tag") > publish.indexOf("- name: Publish the feed"));
  assert.match(publish, /DIGEST: \$\{\{ steps\.tip\.outputs\.digest \|\| steps\.image\.outputs\.digest \}\}/);
});

test("the feed check stops publication on a fetch error", (t) => {
  assert.equal(checkFeed(t, { published: older, failedFetch: true }).code, 1);
});

test("a replay will not point main at an unsigned feed", (t) => {
  assert.equal(checkFeed(t, { published: sha, badSignature: true }).code, 1);
});

test("only current main can replace a feed signed by an old key", (t) => {
  assert.deepEqual(checkFeed(t, { published: older, badSignature: true, remoteMain: sha }), {
    code: 0,
    output: "feed=feed-tip\ncurrent=true\n",
  });
});

test("an unpublished descendant can advance the channel", (t) => {
  assert.deepEqual(checkFeed(t, { published: older, isAncestor: true }), {
    code: 0,
    output: "feed=feed-tip\ncurrent=true\n",
  });
});

test("mutable tags use the release pool, while a recovered architecture can replace its run artifact", () => {
  for (const file of [main, release]) {
    assert.match(file, /publish:\n    needs: build\n    runs-on: aws-release-linux-2cpu-4gb/);
    assert.match(file, /overwrite: true/);
    assert.match(file, /fail-fast: false/);
  }
});

test("release publication refuses to tag an image if a sibling artifact expired", (t) => {
  const directory = mkdtempSync(join(tmpdir(), "guaca-release-digests-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const browser = "ghcr.io/madebywelch/guaca/browser";
  const kept = (arch, name, hex) => {
    const where = join(directory, "digests", `release-digests-${arch}`, name);
    mkdirSync(where, { recursive: true });
    writeFileSync(join(where, hex.repeat(64)), "");
  };
  kept("amd64", "guacad", "a");
  kept("amd64", "browser", "d");
  const script = release.match(/name: Publish the multi-platform images\n[\s\S]*?run: \|\n((?: {10}.*\n)+)/)?.[1];
  assert.ok(script);
  const run = () => spawnSync("bash", ["-e", "-c", 'docker() { printf "%s\\n" "$@"; }\n' + script], {
    encoding: "utf8",
    env: { RUNNER_TEMP: directory, IMAGE: image, BROWSER: browser, VERSION: "1.2.3", GITHUB_RUN_NUMBER: "42" },
  });
  assert.equal(run().status, 1);
  assert.equal(run().stdout, "");
  kept("arm64", "guacad", "b");
  assert.equal(run().status, 1, "a host image is not tagged while its browser is missing");
  assert.equal(run().stdout, "");
  kept("arm64", "browser", "e");
  const result = run();
  assert.equal(result.status, 0);
  for (const named of [`${image}@sha256:${"a".repeat(64)}`, `${image}@sha256:${"b".repeat(64)}`]) {
    assert.ok(result.stdout.includes(`${named}\n`));
  }
  for (const named of [`${browser}@sha256:${"d".repeat(64)}`, `${browser}@sha256:${"e".repeat(64)}`]) {
    assert.ok(result.stdout.includes(`${named}\n`));
  }
  assert.ok(result.stdout.includes(`${image}:1.2.3\n`) && result.stdout.includes(`${browser}:1.2.3\n`));
});

test("a release tags no image until both architectures have checked their browser natively", () => {
  const build = release.slice(release.indexOf("\n  build:"), release.indexOf("\n  publish:"));
  const at = (step) => build.indexOf(`- name: ${step}`);
  assert.match(build, /runner: aws-linux-arm-2cpu-4gb/);
  assert.match(build, /run: \.\/scripts\/browser-isolation\.sh/);
  assert.match(build, /run: python3 scripts\/browser-e2e\.py/);
  assert.ok(at("The browser reaches the public web and nothing else") > at("Build and push the browser"));
  assert.ok(at("Keep the digests") > at("The browser reaches the public web and nothing else"));
  assert.ok(at("Keep the digests") > at("An agent with no Kernel key reads a public page through it"));
  assert.doesNotMatch(build, /continue-on-error/);
  assert.match(build, /BROWSER_IMAGE: \$\{\{ env\.BROWSER \}\}@\$\{\{ steps\.browser\.outputs\.digest \}\}/);
});

test("release publication cannot move a version tag backward", (t) => {
  const directory = mkdtempSync(join(tmpdir(), "guaca-release-version-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const script = release.match(/id: existing\n[\s\S]*?run: \|\n((?: {10}.*\n)+)/)?.[1];
  const run = (number) => spawnSync("bash", ["-e", "-c", `docker() { echo '{"annotations":{"io.github.actions.run-number":"${number}"}}'; }\n` + script], {
    encoding: "utf8",
    env: { PATH: process.env.PATH, RUNNER_TEMP: directory, GITHUB_OUTPUT: join(directory, "out"), IMAGE: image, VERSION: "1.2.3", GITHUB_RUN_NUMBER: "42" },
  });
  assert.equal(run(43).status, 1);
  assert.equal(run(41).status, 0);
  assert.equal(run(42).status, 0);
  assert.equal(readFileSync(join(directory, "out"), "utf8"), "published=true\n");
});
