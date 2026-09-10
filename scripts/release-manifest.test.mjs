import { strict as assert } from "node:assert";
import { test } from "node:test";
import { manifest } from "./release-manifest.mjs";
const commit = "a".repeat(40), image = `ghcr.io/madebywelch/guaca/guacad@sha256:${"b".repeat(64)}`;
const protocol = {generation: 1, minimum: 1, maximum: 1};
test("release metadata refuses mutable images, dirty commits, and incompatible ranges", () => {
  assert.throws(() => manifest("0.2.0", commit, "guacad:latest", protocol));
  assert.throws(() => manifest("0.2.0", `${commit}-dirty`, image, protocol));
  assert.throws(() => manifest("0.2.0-beta.1", commit, image, protocol));
  assert.throws(() => manifest("0.2.0", commit, image, {...protocol, maximum: 0}));
  assert.equal(manifest("0.2.0", commit, image, protocol).notes, "https://github.com/madebywelch/guaca/releases/tag/v0.2.0");
});
