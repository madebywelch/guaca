/**
 * What a person can authorize Guaca to reach, and what that costs them.
 *
 * Signing in to guaca.bot is not one of these: that is an email address and a
 * code. Everything here hangs off an account that already exists, which is
 * what keeps one Google consent screen from having to mean two things.
 *
 * Guaca's own answer to connectors is to sign a browser in on the agent's
 * computer, and that answer is still the better one wherever it works: the app
 * never handles a password and there is no token to keep. It stops working at
 * exactly one place, which is where a service will only issue access through
 * OAuth to a registered, verified application. A local open-source app cannot
 * be that application. A hosted origin can, so this is the part of guaca.bot
 * that exists for a reason other than marketing.
 *
 * ## One grant per provider, not one per capability
 *
 * Google and GitHub both hold a single grant per (user, OAuth client). There
 * is no such thing as revoking Gmail while keeping Calendar: revoking a token
 * revokes the grant. Better Auth models the same fact, keying `account` on
 * `(issuer, accountId)` so one external identity is one row with one token set.
 *
 * So a connector here is a *scope bundle inside a provider's grant*, not an
 * independent connection. Widening is incremental and costs one consent
 * screen. Narrowing revokes the grant first, because a provider offers no way
 * to hand a scope back. The UI says both of those out loud rather than
 * offering per-capability switches that would quietly lie.
 *
 * ## `review`
 *
 * Operator-facing, and the reason this file is worth reading before promising
 * a connector to anyone. Google gates scopes in three tiers, and the tier
 * decides whether shipping a capability is a form submission or a paid annual
 * security assessment. It is recorded next to the scope it applies to because
 * that is the only place it will be read in time. See README, "Connectors".
 */

export type Review =
  /** No review. Ships as soon as the consent screen is published. */
  | "none"
  /** Google's verification: a form, a demo video, and weeks of waiting. */
  | "sensitive"
  /** Verification plus a CASA security assessment, annually, for money. */
  | "restricted";

export interface Capability {
  id: string;
  label: string;
  /** What an agent can do once this is granted. Shown on the card. */
  grants: string;
  /** A limit of the scope that a person would otherwise discover by failing. */
  caveat?: string;
  scopes: readonly string[];
  review: Review;
}

export interface Provider {
  id: string;
  label: string;
  /**
   * Both must be present in the environment or the provider is not offered at
   * all. A card that cannot complete its flow is worse than an absent one.
   */
  credentials: readonly [string, string];
  /**
   * Where the consent screen lives.
   *
   * Authorizing is a form post that answers with a redirect to the provider,
   * and a browser checks `form-action` against where a submission lands, not
   * only against the action it was posted to. So this origin has to be named
   * in the policy or the redirect is dropped and the button does nothing. See
   * `CSP` in `worker/index.ts`; `worker.itest.ts` checks it against the URL
   * Better Auth actually produces rather than against this string.
   */
  consentOrigin: string;
  /**
   * Requested on every authorization, alongside whatever was ticked.
   *
   * Nobody signs in with a provider here: identity is the email address a
   * code was sent to. These are still asked for because a grant with no
   * identity attached cannot be matched to the account that owns it.
   */
  identityScopes: readonly string[];
  capabilities: readonly Capability[];
}

export const PROVIDERS: readonly Provider[] = [
  {
    id: "google",
    label: "Google",
    credentials: ["GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET"],
    consentOrigin: "https://accounts.google.com",
    identityScopes: ["openid", "email", "profile"],
    capabilities: [
      {
        id: "gmail",
        label: "Gmail",
        grants: "Read your mail and send mail as you.",
        scopes: [
          "https://www.googleapis.com/auth/gmail.readonly",
          "https://www.googleapis.com/auth/gmail.send",
        ],
        review: "restricted",
      },
      {
        id: "calendar",
        label: "Calendar",
        grants: "Read your calendars and create, move and cancel events.",
        scopes: ["https://www.googleapis.com/auth/calendar"],
        review: "sensitive",
      },
      {
        id: "drive",
        label: "Drive",
        grants: "Read files across your Drive and create new files.",
        caveat: "Existing files are read-only unless you shared them with Guaca.",
        scopes: [
          "https://www.googleapis.com/auth/drive.file",
          "https://www.googleapis.com/auth/drive.readonly",
        ],
        review: "restricted",
      },
    ],
  },
  {
    id: "github",
    label: "GitHub",
    credentials: ["GITHUB_CLIENT_ID", "GITHUB_CLIENT_SECRET"],
    consentOrigin: "https://github.com",
    identityScopes: ["read:user", "user:email"],
    capabilities: [
      {
        id: "repos",
        label: "Repositories",
        grants: "Read and write your repositories, issues and pull requests.",
        caveat: "GitHub has no read-only form of this scope. Write comes with read.",
        scopes: ["repo"],
        review: "none",
      },
      {
        id: "org",
        label: "Organizations",
        grants: "See which organizations and teams you belong to.",
        scopes: ["read:org"],
        review: "none",
      },
      {
        id: "gists",
        label: "Gists",
        grants: "Read and write your gists.",
        scopes: ["gist"],
        review: "none",
      },
    ],
  },
];

export function findProvider(id: string): Provider | undefined {
  return PROVIDERS.find((p) => p.id === id);
}

/** The providers this deployment actually holds credentials for. */
export function configuredProviders(env: Record<string, unknown>): Provider[] {
  return PROVIDERS.filter((p) =>
    p.credentials.every((key) => typeof env[key] === "string" && env[key]),
  );
}

/** The scopes on a grant, as the provider returned them. */
export function parseScopes(scope: string | null | undefined): Set<string> {
  return new Set((scope ?? "").split(/[\s,]+/).filter(Boolean));
}

/**
 * The capabilities a grant currently covers.
 *
 * A capability counts as granted only when every scope it asked for came back.
 * A provider is free to return fewer than were requested, and a half-granted
 * capability shown as connected is an agent that fails on its first real call.
 */
export function grantedCapabilities(
  provider: Provider,
  scope: string | null | undefined,
): Set<string> {
  const held = parseScopes(scope);
  return new Set(
    provider.capabilities.filter((c) => c.scopes.every((s) => held.has(s))).map((c) => c.id),
  );
}

/** Everything to ask the provider for, given the capabilities that were ticked. */
export function scopesFor(provider: Provider, capabilityIds: Iterable<string>): string[] {
  const wanted = new Set(capabilityIds);
  const out = new Set<string>(provider.identityScopes);
  for (const capability of provider.capabilities) {
    if (wanted.has(capability.id)) for (const scope of capability.scopes) out.add(scope);
  }
  return [...out];
}

/**
 * Whether reaching the desired set means giving something back.
 *
 * Neither provider can narrow a grant in place, so this is the question of
 * whether the grant has to be revoked and rebuilt rather than widened. Asked
 * about capabilities rather than raw scopes: a scope the provider added on its
 * own, which Google does, is not something the person asked to keep.
 */
export function narrows(granted: Set<string>, desired: Set<string>): boolean {
  for (const id of granted) if (!desired.has(id)) return true;
  return false;
}

/**
 * Hand the token back to the provider.
 *
 * Deleting our row is not revocation: the grant would still be listed on the
 * person's Google or GitHub account with nothing here to show for it, and any
 * token already handed to a paired machine would keep working. Failure is
 * reported rather than swallowed, because a disconnect that silently left the
 * grant standing is the one failure of this page that actually matters.
 */
export async function revokeGrant(
  providerId: string,
  token: string,
  env: Record<string, unknown>,
): Promise<void> {
  if (providerId === "google") {
    const res = await fetch("https://oauth2.googleapis.com/revoke", {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ token }),
    });
    // Google answers 400 `invalid_token` for a token it has already forgotten,
    // which is the state we were asking for.
    if (!res.ok && res.status !== 400) {
      throw new Error(`google revoke failed: ${res.status} ${await res.text()}`);
    }
    return;
  }

  if (providerId === "github") {
    const id = env.GITHUB_CLIENT_ID as string;
    const secret = env.GITHUB_CLIENT_SECRET as string;
    const res = await fetch(`https://api.github.com/applications/${id}/grant`, {
      method: "DELETE",
      headers: {
        accept: "application/vnd.github+json",
        authorization: `Basic ${btoa(`${id}:${secret}`)}`,
        "content-type": "application/json",
        "user-agent": "guaca.bot",
      },
      body: JSON.stringify({ access_token: token }),
    });
    if (!res.ok && res.status !== 404) {
      throw new Error(`github revoke failed: ${res.status} ${await res.text()}`);
    }
    return;
  }

  throw new Error(`no revocation endpoint for provider ${providerId}`);
}
