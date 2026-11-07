import { defineWorkersConfig, readD1Migrations } from "@cloudflare/vitest-pool-workers/config";

const migrations = await readD1Migrations("./migrations");

export default defineWorkersConfig({
  test: {
    include: ["worker/selfhost.itest.ts"],
    poolOptions: {
      workers: {
        wrangler: { configPath: "./wrangler.selfhost.jsonc" },
        miniflare: {
          bindings: {
            MIGRATIONS: migrations,
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
