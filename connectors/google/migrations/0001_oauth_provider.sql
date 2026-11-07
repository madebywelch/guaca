-- The OAuth 2.1 authorization server, and the end of the device grant.
--
-- The desktop app used to pair over RFC 8628: a code a person carried from the
-- app to a browser by hand. That is the flow for a television, and RFC 8628
-- section 5.4 names its weakness — nothing binds the code to the machine that
-- asked for it, so anybody who can talk an operator into approving one walks
-- away with a token that mints Gmail access tokens. This origin exists to hold
-- other people's refresh tokens, so that is not a trade worth making.
--
-- What replaces it is what RFC 8252 says a native application should do:
-- authorization code with PKCE, redirected to a loopback port the app bound
-- before it asked for anything. The code is delivered to 127.0.0.1 on the
-- machine that started the flow and is useless without a verifier that never
-- left it.
--
-- `deviceCode` is dropped rather than left standing. Two doors to one account
-- means the weaker one decides what the account is worth.
--
-- Forward-only and numbered, like every migration here. The authority for the
-- shape below is Better Auth's own `getAuthTables()`; `worker/schema.test.ts`
-- calls it and fails when this file, `worker/schema.ts` and the library
-- disagree. Dates are epoch seconds, booleans are 0/1, and every `string[]`
-- and `json` field is JSON text.

DROP INDEX IF EXISTS deviceCode_deviceCode_idx;
DROP INDEX IF EXISTS deviceCode_userCode_idx;
DROP TABLE IF EXISTS deviceCode;

CREATE TABLE oauthClient (
  id TEXT PRIMARY KEY,
  clientId TEXT NOT NULL UNIQUE,
  clientSecret TEXT,
  clientDiscoveryId TEXT,
  disabled INTEGER,
  skipConsent INTEGER,
  enableEndSession INTEGER,
  subjectType TEXT,
  scopes TEXT,
  clientCredentialsScopes TEXT,
  userId TEXT REFERENCES user(id),
  createdAt INTEGER,
  updatedAt INTEGER,
  name TEXT,
  uri TEXT,
  icon TEXT,
  contacts TEXT,
  tos TEXT,
  policy TEXT,
  softwareId TEXT,
  softwareVersion TEXT,
  softwareStatement TEXT,
  redirectUris TEXT NOT NULL,
  postLogoutRedirectUris TEXT,
  backchannelLogoutUri TEXT,
  backchannelLogoutSessionRequired INTEGER,
  tokenEndpointAuthMethod TEXT,
  applicationType TEXT,
  jwks TEXT,
  jwksUri TEXT,
  grantTypes TEXT,
  responseTypes TEXT,
  requirePKCE INTEGER,
  dpopBoundAccessTokens INTEGER,
  referenceId TEXT,
  metadata TEXT
);

CREATE TABLE oauthResource (
  id TEXT PRIMARY KEY,
  identifier TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  accessTokenTtl INTEGER,
  refreshTokenTtl INTEGER,
  signingAlgorithm TEXT,
  signingKeyId TEXT,
  allowedScopes TEXT,
  customClaims TEXT,
  dpopBoundAccessTokensRequired INTEGER,
  disabled INTEGER,
  createdAt INTEGER,
  updatedAt INTEGER,
  policyVersion INTEGER,
  metadata TEXT
);

CREATE TABLE oauthClientResource (
  id TEXT PRIMARY KEY,
  clientId TEXT NOT NULL REFERENCES oauthClient(clientId) ON DELETE CASCADE,
  resourceId TEXT NOT NULL REFERENCES oauthResource(identifier) ON DELETE CASCADE,
  metadata TEXT,
  createdAt INTEGER
);

CREATE UNIQUE INDEX oauthClientResource_clientId_resourceId_idx ON oauthClientResource (clientId, resourceId);

CREATE TABLE oauthRefreshToken (
  id TEXT PRIMARY KEY,
  token TEXT NOT NULL UNIQUE,
  clientId TEXT NOT NULL REFERENCES oauthClient(clientId),
  sessionId TEXT REFERENCES session(id) ON DELETE SET NULL,
  userId TEXT NOT NULL REFERENCES user(id),
  referenceId TEXT,
  authorizationCodeId TEXT,
  resources TEXT,
  requestedUserInfoClaims TEXT,
  expiresAt INTEGER,
  createdAt INTEGER,
  revoked INTEGER,
  rotatedAt INTEGER,
  rotationReplayResponse TEXT,
  rotationReplayExpiresAt INTEGER,
  authTime INTEGER,
  confirmation TEXT,
  scopes TEXT NOT NULL
);

CREATE TABLE oauthAccessToken (
  id TEXT PRIMARY KEY,
  token TEXT UNIQUE,
  clientId TEXT NOT NULL REFERENCES oauthClient(clientId),
  sessionId TEXT REFERENCES session(id) ON DELETE SET NULL,
  userId TEXT REFERENCES user(id),
  referenceId TEXT,
  authorizationCodeId TEXT,
  resources TEXT,
  requestedUserInfoClaims TEXT,
  refreshId TEXT REFERENCES oauthRefreshToken(id),
  expiresAt INTEGER,
  createdAt INTEGER,
  revoked INTEGER,
  confirmation TEXT,
  scopes TEXT NOT NULL
);

CREATE TABLE oauthConsent (
  id TEXT PRIMARY KEY,
  clientId TEXT NOT NULL REFERENCES oauthClient(clientId),
  userId TEXT REFERENCES user(id),
  referenceId TEXT,
  resources TEXT,
  requestedUserInfoClaims TEXT,
  scopes TEXT NOT NULL,
  createdAt INTEGER,
  updatedAt INTEGER
);

CREATE TABLE oauthClientAssertion (
  id TEXT PRIMARY KEY,
  expiresAt INTEGER NOT NULL
);

-- The one client, seeded rather than registered.
--
-- Dynamic client registration is off, so this row is the only application that
-- can ever complete the flow, and the consent screen names something this
-- origin chose instead of something a stranger asserted about itself.
--
-- `clientSecret` is null and `tokenEndpointAuthMethod` is `none` because Guaca
-- ships as an open-source download: a secret in it would be a secret in every
-- copy, which is not a secret. PKCE is what takes its place, and `requirePKCE`
-- says so rather than relying on the default.
--
-- The redirect URI carries no port on purpose. RFC 8252 section 7.3 has the
-- authorization server ignore the port on a loopback redirect, which is the
-- whole reason the app can bind `127.0.0.1:0`, take whatever the operating
-- system gives it, and still match. Scheme, host, path and query are all
-- compared exactly.
INSERT INTO oauthClient (
  id, clientId, clientSecret, name, uri,
  redirectUris, tokenEndpointAuthMethod, applicationType,
  grantTypes, responseTypes, requirePKCE, skipConsent, disabled,
  scopes, createdAt, updatedAt
) VALUES (
  'guaca-desktop',
  'guaca-desktop',
  NULL,
  'Guaca',
  'https://github.com/madebywelch/guaca',
  '["http://127.0.0.1/callback"]',
  'none',
  'native',
  '["authorization_code","refresh_token"]',
  '["code"]',
  1,
  0,
  0,
  '["openid","email","offline_access","connectors"]',
  unixepoch(),
  unixepoch()
);
