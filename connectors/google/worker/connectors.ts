export type Review =
  | "none"
  | "sensitive"
  | "restricted";

export interface Capability {
  id: string;
  label: string;
  grants: string;
  caveat?: string;
  scopes: readonly string[];
  review: Review;
}

export interface Provider {
  id: string;
  label: string;
  credentials: readonly [string, string];
  consentOrigin: string;
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

export function configuredProviders(env: Record<string, unknown>): Provider[] {
  return PROVIDERS.filter((p) =>
    p.credentials.every((key) => typeof env[key] === "string" && env[key]),
  );
}

export function parseScopes(scope: string | null | undefined): Set<string> {
  return new Set((scope ?? "").split(/[\s,]+/).filter(Boolean));
}

export function grantedCapabilities(
  provider: Provider,
  scope: string | null | undefined,
): Set<string> {
  const held = parseScopes(scope);
  return new Set(
    provider.capabilities.filter((c) => c.scopes.every((s) => held.has(s))).map((c) => c.id),
  );
}

export function scopesFor(provider: Provider, capabilityIds: Iterable<string>): string[] {
  const wanted = new Set(capabilityIds);
  const out = new Set<string>(provider.identityScopes);
  for (const capability of provider.capabilities) {
    if (wanted.has(capability.id)) for (const scope of capability.scopes) out.add(scope);
  }
  return [...out];
}

export function narrows(granted: Set<string>, desired: Set<string>): boolean {
  for (const id of granted) if (!desired.has(id)) return true;
  return false;
}

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
