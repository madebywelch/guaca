import { oauthProvider } from "@better-auth/oauth-provider";
import { betterAuth } from "better-auth";
import { drizzleAdapter } from "better-auth/adapters/drizzle";
import { emailOTP } from "better-auth/plugins";
import { drizzle } from "drizzle-orm/d1";

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
};

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

export const DESKTOP_CLIENT_ID = "guaca-desktop";

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
      cachedTrustedClients: new Set([DESKTOP_CLIENT_ID]),
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

    session: { expiresIn: SESSION_TTL_DAYS * 24 * 60 * 60 },

    emailAndPassword: { enabled: false },

    socialProviders: socialProviders(env),

    disabledPaths: [...DISABLED_PATHS, "/update-user"],

    plugins: [...authPlugins(env)],
  });
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
