-- Better Auth's tables, for D1.
--
-- The authority for this shape is Better Auth's own `getAuthTables()`, not
-- this file and not `worker/schema.ts`. `worker/schema.test.ts` calls it and
-- fails when any of the three disagree, which is what makes it safe to keep
-- the migration hand-written: a library upgrade that adds a column fails a
-- test rather than every request after a deploy.
--
-- Dates are epoch seconds (Drizzle `mode: "timestamp"`), booleans are 0/1.

CREATE TABLE user (
  id            TEXT PRIMARY KEY,
  name          TEXT    NOT NULL,
  email         TEXT    NOT NULL UNIQUE,
  emailVerified INTEGER NOT NULL DEFAULT 0,
  image         TEXT,
  createdAt     INTEGER NOT NULL,
  updatedAt     INTEGER NOT NULL
);

CREATE TABLE session (
  id        TEXT PRIMARY KEY,
  expiresAt INTEGER NOT NULL,
  token     TEXT    NOT NULL UNIQUE,
  createdAt INTEGER NOT NULL,
  updatedAt INTEGER NOT NULL,
  ipAddress TEXT,
  userAgent TEXT,
  userId    TEXT    NOT NULL REFERENCES user(id) ON DELETE CASCADE
);

CREATE INDEX session_userId_idx ON session (userId);

-- One row per external identity, which is one OAuth grant. The unique key is
-- (issuer, accountId) rather than (providerId, accountId): two provider
-- configurations pointing at the same issuer are the same grant and share
-- these tokens. `worker/connectors.ts` is built on that.
--
-- accessToken, refreshToken and idToken are ciphertext: `encryptOAuthTokens`
-- is on, so a copy of this table is not a set of live Google credentials.
CREATE TABLE account (
  id                    TEXT PRIMARY KEY,
  issuer                TEXT    NOT NULL,
  accountId             TEXT    NOT NULL,
  providerId            TEXT    NOT NULL,
  userId                TEXT    NOT NULL REFERENCES user(id) ON DELETE CASCADE,
  accessToken           TEXT,
  refreshToken          TEXT,
  idToken               TEXT,
  accessTokenExpiresAt  INTEGER,
  refreshTokenExpiresAt INTEGER,
  scope                 TEXT,
  password              TEXT,
  createdAt             INTEGER NOT NULL,
  updatedAt             INTEGER NOT NULL
);

CREATE UNIQUE INDEX account_issuer_accountId_idx ON account (issuer, accountId);
CREATE INDEX account_userId_idx ON account (userId);

CREATE TABLE verification (
  id         TEXT PRIMARY KEY,
  identifier TEXT    NOT NULL,
  value      TEXT    NOT NULL,
  expiresAt  INTEGER NOT NULL,
  createdAt  INTEGER NOT NULL,
  updatedAt  INTEGER NOT NULL
);

CREATE INDEX verification_identifier_idx ON verification (identifier);

-- RFC 8628. One row per attempt to pair a copy of the desktop app.
CREATE TABLE deviceCode (
  id              TEXT PRIMARY KEY,
  deviceCode      TEXT    NOT NULL,
  userCode        TEXT    NOT NULL,
  userId          TEXT,
  expiresAt       INTEGER NOT NULL,
  status          TEXT    NOT NULL,
  lastPolledAt    INTEGER,
  pollingInterval INTEGER,
  clientId        TEXT,
  scope           TEXT
);

CREATE UNIQUE INDEX deviceCode_deviceCode_idx ON deviceCode (deviceCode);
CREATE UNIQUE INDEX deviceCode_userCode_idx ON deviceCode (userCode);
