import { oauthProvider } from "@better-auth/oauth-provider";
import { type BetterAuthOptions, betterAuth } from "better-auth";
import { drizzleAdapter } from "better-auth/adapters/drizzle";
import { emailOTP } from "better-auth/plugins";
import { drizzle } from "drizzle-orm/d1";

import { labelFromIdToken } from "./connections";
import { PROVIDERS } from "./connectors";
import { CODE_TTL_MINUTES, sendSignInCode } from "./email";
import { origin } from "./origin";
import { schema } from "./schema";

const instances = new WeakMap<object, Auth>();

export type Auth = ReturnType<typeof build>;

export function getAuth(env: Env): Auth {
  const cached = instances.get(env);
  if (cached) return cached;
  const auth = build(env);
  instances.set(env, auth);
  return auth;
}

export const SESSION_TTL_DAYS = 7;

export const REFRESH_TOKEN_TTL_SECONDS = 30 * 24 * 60 * 60;

export const ACCOUNT_OPTIONS = {
  encryptOAuthTokens: true,
  accountLinking: {
    enabled: true,
    trustedProviders: ["google", "github"],
    allowDifferentEmails: true,
  },
  additionalFields: {
    label: { type: "string", required: false, input: false },
  },
} satisfies BetterAuthOptions["account"];

export const DISABLED_PATHS = [
  "/email-otp/send-verification-otp",
  "/email-otp/check-verification-otp",
  "/email-otp/verify-email",
  "/sign-in/email-otp",
  "/email-otp/request-password-reset",
  "/forget-password/email-otp",
  "/email-otp/reset-password",
  "/email-otp/request-email-change",
  "/email-otp/change-email",
];

export const SERVED_AUTH_PATHS = [
  "/.well-known/oauth-authorization-server",
  "/.well-known/openid-configuration",
  "/callback/:id",
  "/error",
  "/oauth2/authorize",
  "/oauth2/introspect",
  "/oauth2/revoke",
  "/oauth2/token",
  "/oauth2/userinfo",
] as const;

export const REFUSED_AUTH_PATHS = [
  ...DISABLED_PATHS,
  "/account-info",
  "/admin/oauth2/create-client",
  "/admin/oauth2/resources",
  "/admin/oauth2/resources/:identifier",
  "/admin/oauth2/resources/:identifier/clients/:client_id",
  "/admin/oauth2/update-client",
  "/change-email",
  "/change-password",
  "/delete-user",
  "/delete-user/callback",
  "/get-access-token",
  "/get-session",
  "/link-social",
  "/list-accounts",
  "/list-sessions",
  "/oauth2/client/rotate-secret",
  "/oauth2/consent",
  "/oauth2/continue",
  "/oauth2/create-client",
  "/oauth2/delete-client",
  "/oauth2/delete-consent",
  "/oauth2/end-session",
  "/oauth2/end-session/confirm",
  "/oauth2/get-client",
  "/oauth2/get-clients",
  "/oauth2/get-consent",
  "/oauth2/get-consents",
  "/oauth2/public-client",
  "/oauth2/public-client-prelogin",
  "/oauth2/register",
  "/oauth2/update-client",
  "/oauth2/update-consent",
  "/ok",
  "/refresh-token",
  "/request-password-reset",
  "/reset-password",
  "/reset-password/:token",
  "/revoke-other-sessions",
  "/revoke-session",
  "/revoke-sessions",
  "/send-verification-email",
  "/sign-in/email",
  "/sign-in/social",
  "/sign-out",
  "/sign-up/email",
  "/unlink-account",
  "/update-session",
  "/update-user",
  "/verify-email",
  "/verify-password",
] as const;

export const DESKTOP_CLIENT_ID = "guaca-desktop";

export const REMOTE_CLIENT_ID = "guaca-remote";

export const REMOTE_PATH = "/remote";

export const SCOPES = ["openid", "email", "offline_access", "connectors"] as const;

export const CONNECTORS_SCOPE = "connectors";

export const MCP_SCOPES = ["openid", CONNECTORS_SCOPE] as const;

export const GRANT_TYPES = ["authorization_code", "refresh_token"] as const;

export const USERINFO_CLAIMS = ["sub", "email", "email_verified", "scope"] as const;

export function authPlugins(env?: Env) {
  return [
    oauthProvider({
      scopes: [...SCOPES],
      grantTypes: [...GRANT_TYPES],
      advertisedMetadata: { claims_supported: [...USERINFO_CLAIMS] },
      loginPage: "/signin",
      consentPage: "/consent",
      allowDynamicClientRegistration: false,
      allowUnauthenticatedClientRegistration: false,
      cachedTrustedClients: new Set([DESKTOP_CLIENT_ID, REMOTE_CLIENT_ID]),
      accessTokenExpiresIn: 60 * 60,
      refreshTokenExpiresIn: REFRESH_TOKEN_TTL_SECONDS,
      refreshTokenReuseInterval: REFRESH_TOKEN_TTL_SECONDS,
      customUserInfoClaims: ({ scopes }: { scopes: readonly string[] }) => ({
        scope: scopes.join(" "),
      }),
      disableJwtPlugin: true,
    }),
    emailOTP({
      otpLength: 6,
      expiresIn: CODE_TTL_MINUTES * 60,
      storeOTP: "hashed",
      allowedAttempts: 3,
      sendVerificationOTP: async ({ email, otp, type }) => {
        if (type !== "sign-in") throw new Error(`unexpected OTP type: ${type}`);
        if (!env) throw new Error("no environment to send from");
        await sendSignInCode(env, email, otp);
      },
    }),
  ] as const;
}

function build(env: Env) {
  if (!env.BETTER_AUTH_URL) throw new Error("BETTER_AUTH_URL is not set");
  const baseURL = origin(env.BETTER_AUTH_URL);
  if (!env.BETTER_AUTH_SECRET) throw new Error("BETTER_AUTH_SECRET is not set");

  return betterAuth({
    appName: "Guaca",
    baseURL,
    secret: env.BETTER_AUTH_SECRET,
    basePath: "/api/auth",
    trustedOrigins: [baseURL],
    database: drizzleAdapter(drizzle(env.DB, { schema }), { provider: "sqlite", schema }),

    account: ACCOUNT_OPTIONS,

    databaseHooks: {
      account: {
        create: { before: async (account) => ({ data: withoutIdToken(account) }) },
        update: { before: async (account) => ({ data: withoutIdToken(account) }) },
      },
    },

    session: { expiresIn: SESSION_TTL_DAYS * 24 * 60 * 60 },

    emailAndPassword: { enabled: false },

    socialProviders: socialProviders(env),

    disabledPaths: DISABLED_PATHS,

    plugins: [...authPlugins(env)],
  });
}

function withoutIdToken(account: { idToken?: string | null }) {
  const label = account.idToken ? labelFromIdToken(account.idToken) : undefined;
  return label ? { idToken: null, label } : { idToken: null };
}

function socialProviders(env: Env) {
  const out: Record<string, { clientId: string; clientSecret: string; accessType?: "offline" }> =
    {};
  for (const provider of PROVIDERS) {
    const [idKey, secretKey] = provider.credentials;
    const clientId = (env as unknown as Record<string, string | undefined>)[idKey];
    const clientSecret = (env as unknown as Record<string, string | undefined>)[secretKey];
    if (!clientId || !clientSecret) continue;
    out[provider.id] = { clientId, clientSecret };
  }
  return out;
}
