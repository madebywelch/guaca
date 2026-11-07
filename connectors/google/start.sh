#!/bin/sh
set -eu
umask 077
node -e 'const fs = require("node:fs"); const names = ["BETTER_AUTH_URL", "BETTER_AUTH_SECRET", "SELFHOST_BEARER", "GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET"]; if (names.some((name) => !process.env[name])) process.exit(1); fs.writeFileSync(".dev.vars", names.map((name) => `${name}=${JSON.stringify(process.env[name])}`).join("\n") + "\n", { mode: 0o600 });'
pnpm exec wrangler d1 migrations apply guaca-connectors --local --config wrangler.selfhost.jsonc --persist-to /data
pnpm exec wrangler d1 execute guaca-connectors --local --config wrangler.selfhost.jsonc --persist-to /data --file selfhost.sql
exec pnpm exec wrangler dev --local --config wrangler.selfhost.jsonc --persist-to /data --ip 0.0.0.0 --port 8788 --log-level none
