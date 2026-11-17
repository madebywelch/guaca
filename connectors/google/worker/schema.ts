import { index, integer, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";

const timestamp = (name: string) => integer(name, { mode: "timestamp" });

export const user = sqliteTable("user", {
  id: text("id").primaryKey(),
  name: text("name").notNull(),
  email: text("email").notNull().unique(),
  emailVerified: integer("emailVerified", { mode: "boolean" }).notNull().default(false),
  image: text("image"),
  createdAt: timestamp("createdAt").notNull(),
  updatedAt: timestamp("updatedAt").notNull(),
});

export const session = sqliteTable(
  "session",
  {
    id: text("id").primaryKey(),
    expiresAt: timestamp("expiresAt").notNull(),
    token: text("token").notNull().unique(),
    createdAt: timestamp("createdAt").notNull(),
    updatedAt: timestamp("updatedAt").notNull(),
    ipAddress: text("ipAddress"),
    userAgent: text("userAgent"),
    userId: text("userId")
      .notNull()
      .references(() => user.id, { onDelete: "cascade" }),
  },
  (table) => [index("session_userId_idx").on(table.userId)],
);

export const account = sqliteTable(
  "account",
  {
    id: text("id").primaryKey(),
    issuer: text("issuer").notNull(),
    accountId: text("accountId").notNull(),
    providerId: text("providerId").notNull(),
    userId: text("userId")
      .notNull()
      .references(() => user.id, { onDelete: "cascade" }),
    accessToken: text("accessToken"),
    refreshToken: text("refreshToken"),
    idToken: text("idToken"),
    accessTokenExpiresAt: timestamp("accessTokenExpiresAt"),
    refreshTokenExpiresAt: timestamp("refreshTokenExpiresAt"),
    scope: text("scope"),
    password: text("password"),
    createdAt: timestamp("createdAt").notNull(),
    updatedAt: timestamp("updatedAt").notNull(),
    label: text("label"),
  },
  (table) => [
    uniqueIndex("account_issuer_accountId_idx").on(table.issuer, table.accountId),
    index("account_userId_idx").on(table.userId),
  ],
);

export const verification = sqliteTable(
  "verification",
  {
    id: text("id").primaryKey(),
    identifier: text("identifier").notNull(),
    value: text("value").notNull(),
    expiresAt: timestamp("expiresAt").notNull(),
    createdAt: timestamp("createdAt").notNull(),
    updatedAt: timestamp("updatedAt").notNull(),
  },
  (table) => [index("verification_identifier_idx").on(table.identifier)],
);

export const oauthClient = sqliteTable("oauthClient", {
  id: text("id").primaryKey(),
  clientId: text("clientId").notNull().unique(),
  clientSecret: text("clientSecret"),
  clientDiscoveryId: text("clientDiscoveryId"),
  disabled: integer("disabled", { mode: "boolean" }),
  skipConsent: integer("skipConsent", { mode: "boolean" }),
  enableEndSession: integer("enableEndSession", { mode: "boolean" }),
  subjectType: text("subjectType"),
  scopes: text("scopes", { mode: "json" }).$type<string[]>(),
  clientCredentialsScopes: text("clientCredentialsScopes", { mode: "json" }).$type<string[]>(),
  userId: text("userId").references(() => user.id),
  createdAt: timestamp("createdAt"),
  updatedAt: timestamp("updatedAt"),
  name: text("name"),
  uri: text("uri"),
  icon: text("icon"),
  contacts: text("contacts", { mode: "json" }).$type<string[]>(),
  tos: text("tos"),
  policy: text("policy"),
  softwareId: text("softwareId"),
  softwareVersion: text("softwareVersion"),
  softwareStatement: text("softwareStatement"),
  redirectUris: text("redirectUris", { mode: "json" }).$type<string[]>().notNull(),
  postLogoutRedirectUris: text("postLogoutRedirectUris", { mode: "json" }).$type<string[]>(),
  backchannelLogoutUri: text("backchannelLogoutUri"),
  backchannelLogoutSessionRequired: integer("backchannelLogoutSessionRequired", {
    mode: "boolean",
  }),
  tokenEndpointAuthMethod: text("tokenEndpointAuthMethod"),
  applicationType: text("applicationType"),
  jwks: text("jwks"),
  jwksUri: text("jwksUri"),
  grantTypes: text("grantTypes", { mode: "json" }).$type<string[]>(),
  responseTypes: text("responseTypes", { mode: "json" }).$type<string[]>(),
  requirePKCE: integer("requirePKCE", { mode: "boolean" }),
  dpopBoundAccessTokens: integer("dpopBoundAccessTokens", { mode: "boolean" }),
  referenceId: text("referenceId"),
  metadata: text("metadata", { mode: "json" }),
});

export const oauthResource = sqliteTable("oauthResource", {
  id: text("id").primaryKey(),
  identifier: text("identifier").notNull().unique(),
  name: text("name").notNull(),
  accessTokenTtl: integer("accessTokenTtl"),
  refreshTokenTtl: integer("refreshTokenTtl"),
  signingAlgorithm: text("signingAlgorithm"),
  signingKeyId: text("signingKeyId"),
  allowedScopes: text("allowedScopes", { mode: "json" }).$type<string[]>(),
  customClaims: text("customClaims", { mode: "json" }),
  dpopBoundAccessTokensRequired: integer("dpopBoundAccessTokensRequired", { mode: "boolean" }),
  disabled: integer("disabled", { mode: "boolean" }),
  createdAt: timestamp("createdAt"),
  updatedAt: timestamp("updatedAt"),
  policyVersion: integer("policyVersion"),
  metadata: text("metadata", { mode: "json" }),
});

export const oauthClientResource = sqliteTable(
  "oauthClientResource",
  {
    id: text("id").primaryKey(),
    clientId: text("clientId")
      .references(() => oauthClient.clientId, { onDelete: "cascade" })
      .notNull(),
    resourceId: text("resourceId")
      .references(() => oauthResource.identifier, { onDelete: "cascade" })
      .notNull(),
    metadata: text("metadata", { mode: "json" }),
    createdAt: timestamp("createdAt"),
  },
  (table) => [
    uniqueIndex("oauthClientResource_clientId_resourceId_idx").on(table.clientId, table.resourceId),
  ],
);

export const oauthRefreshToken = sqliteTable("oauthRefreshToken", {
  id: text("id").primaryKey(),
  token: text("token").notNull().unique(),
  clientId: text("clientId")
    .references(() => oauthClient.clientId)
    .notNull(),
  sessionId: text("sessionId").references(() => session.id, { onDelete: "set null" }),
  userId: text("userId")
    .references(() => user.id)
    .notNull(),
  referenceId: text("referenceId"),
  authorizationCodeId: text("authorizationCodeId"),
  resources: text("resources", { mode: "json" }).$type<string[]>(),
  requestedUserInfoClaims: text("requestedUserInfoClaims", { mode: "json" }).$type<string[]>(),
  expiresAt: timestamp("expiresAt"),
  createdAt: timestamp("createdAt"),
  revoked: timestamp("revoked"),
  rotatedAt: timestamp("rotatedAt"),
  rotationReplayResponse: text("rotationReplayResponse"),
  rotationReplayExpiresAt: timestamp("rotationReplayExpiresAt"),
  authTime: timestamp("authTime"),
  confirmation: text("confirmation", { mode: "json" }),
  scopes: text("scopes", { mode: "json" }).$type<string[]>().notNull(),
});

export const oauthAccessToken = sqliteTable("oauthAccessToken", {
  id: text("id").primaryKey(),
  token: text("token").unique(),
  clientId: text("clientId")
    .references(() => oauthClient.clientId)
    .notNull(),
  sessionId: text("sessionId").references(() => session.id, { onDelete: "set null" }),
  userId: text("userId").references(() => user.id),
  referenceId: text("referenceId"),
  authorizationCodeId: text("authorizationCodeId"),
  resources: text("resources", { mode: "json" }).$type<string[]>(),
  requestedUserInfoClaims: text("requestedUserInfoClaims", { mode: "json" }).$type<string[]>(),
  refreshId: text("refreshId").references(() => oauthRefreshToken.id),
  expiresAt: timestamp("expiresAt"),
  createdAt: timestamp("createdAt"),
  revoked: timestamp("revoked"),
  confirmation: text("confirmation", { mode: "json" }),
  scopes: text("scopes", { mode: "json" }).$type<string[]>().notNull(),
});

export const oauthConsent = sqliteTable("oauthConsent", {
  id: text("id").primaryKey(),
  clientId: text("clientId")
    .references(() => oauthClient.clientId)
    .notNull(),
  userId: text("userId").references(() => user.id),
  referenceId: text("referenceId"),
  resources: text("resources", { mode: "json" }).$type<string[]>(),
  requestedUserInfoClaims: text("requestedUserInfoClaims", { mode: "json" }).$type<string[]>(),
  scopes: text("scopes", { mode: "json" }).$type<string[]>().notNull(),
  createdAt: timestamp("createdAt"),
  updatedAt: timestamp("updatedAt"),
});

export const oauthClientAssertion = sqliteTable("oauthClientAssertion", {
  id: text("id").primaryKey(),
  expiresAt: timestamp("expiresAt").notNull(),
});
export const schema = {
  user,
  session,
  account,
  verification,
  oauthClient,
  oauthResource,
  oauthClientResource,
  oauthRefreshToken,
  oauthAccessToken,
  oauthConsent,
  oauthClientAssertion,
};
