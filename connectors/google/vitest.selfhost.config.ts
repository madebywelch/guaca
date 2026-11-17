import { readFileSync } from "node:fs";

import { defineWorkersConfig, readD1Migrations } from "@cloudflare/vitest-pool-workers/config";

const migrations = await readD1Migrations("./migrations");

export default defineWorkersConfig({
  test: {
    include: ["worker/*.itest.ts"],
    poolOptions: {
      workers: {
        wrangler: { configPath: "./wrangler.selfhost.jsonc" },
        miniflare: {
          d1Databases: { PREVIOUS_DB: { id: "previous" } },
          bindings: {
            MIGRATIONS: migrations,
            FINALIZE_ACCOUNT_LABELS: readFileSync("./finalize-account-labels.sql", "utf8"),
            BETTER_AUTH_URL: "http://localhost:8787",
            BETTER_AUTH_SECRET: "test-secret-that-is-long-enough-for-testing",
            SELFHOST_BEARER: "test-bearer-that-is-long-enough-for-testing",
            GOOGLE_CLIENT_ID: "test-google-client",
            GOOGLE_CLIENT_SECRET: "test-google-secret",
          },
        },
      },
    },
  },
});
