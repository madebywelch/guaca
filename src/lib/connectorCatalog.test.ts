import { describe, expect, it } from "vitest";

import { CATALOG, entryFor } from "./connectorCatalog";

/**
 * The catalog is data, and every field of it is a promise the Rust side has to
 * be able to keep. A tile whose variable name or note the backend rejects looks
 * fine in the grid and fails at the moment the operator pastes a token in,
 * which is the worst place to find out.
 */

/** `ConnectorError::BadEnvVar` in `domain/connector.rs`. */
const ENV_VAR = /^[A-Za-z_][A-Za-z0-9_]*$/;
/** `MAX_NOTE_LEN` in `domain/connector.rs`. */
const MAX_NOTE = 240;

describe("the connector catalog", () => {
  it("names a variable the backend will accept", () => {
    for (const entry of CATALOG) {
      expect(entry.envVar, `${entry.service} claims ${entry.envVar}`).toMatch(ENV_VAR);
    }
  });

  it("keeps every note inside what a connector can store", () => {
    // A note is copied onto the credential when the tile is used, and the
    // backend refuses one over this length. It is also read by every agent in
    // the group on every turn, so the cap is a budget rather than a formality.
    for (const entry of CATALOG) {
      expect(entry.note?.length ?? 0, `${entry.service}'s note`).toBeLessThanOrEqual(MAX_NOTE);
    }
  });

  it("never gives two services the same variable", () => {
    // One variable per group is a unique index in SQLite. Two tiles sharing a
    // name would make the second one unaddable, and the error would name an
    // index rather than the tile that caused it.
    const vars = CATALOG.map((entry) => entry.envVar);
    expect(new Set(vars).size).toBe(vars.length);

    const services = CATALOG.map((entry) => entry.service);
    expect(new Set(services).size).toBe(services.length);
  });

  it("gives every entry a colour and something to draw", () => {
    for (const entry of CATALOG) {
      expect(entry.color, entry.service).toMatch(/^#[0-9a-f]{6}$/);
      expect(entry.mark.length, entry.service).toBeGreaterThan(0);
    }
  });

  it("offers Mistral, and says the part of it that is not guessable", () => {
    // An agent holding this key would reach for chat completions. The OCR call
    // is a different endpoint, a different model id, and wants the file
    // uploaded first, so the note carries all three.
    const mistral = entryFor("Mistral");
    expect(mistral?.envVar).toBe("MISTRAL_API_KEY");
    expect(mistral?.note).toContain("mistral-ocr-4-1");
    expect(mistral?.note).toContain("/v1/ocr");
    expect(mistral?.note).toContain("/v1/files");
  });
});
