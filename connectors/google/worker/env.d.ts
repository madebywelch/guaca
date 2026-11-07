/**
 * What the Worker is handed at runtime.
 *
 * Hand-written rather than generated from `wrangler.jsonc`, because half of it
 * is secrets, which are deliberately absent from that file. This is the list a
 * person reads to find out what has to be set before a deployment works, so it
 * says so here and `README.md` says where each one comes from.
 */
declare global {
  interface Env {
    /** Static assets: the prerendered marketing page and its stylesheet. */
    ASSETS: Fetcher;
    /** Users, sessions, OAuth grants, pending device codes. */
    DB: D1Database;

    /** Cloudflare Email Sending. Sends one thing: the sign-in code. */
    EMAIL: SendEmail;
    /** Who that code comes from. The domain must be onboarded to Email Sending. */
    EMAIL_FROM?: string;

    /** The origin, exactly as registered with every provider. No trailing slash. */
    BETTER_AUTH_URL: string;
    /** Signs sessions and encrypts stored provider tokens. Rotating it logs everyone out. */
    BETTER_AUTH_SECRET: string;

    /** Both halves, or the provider is not offered. See `connectors.ts`. */
    GOOGLE_CLIENT_ID?: string;
    GOOGLE_CLIENT_SECRET?: string;
    GITHUB_CLIENT_ID?: string;
    GITHUB_CLIENT_SECRET?: string;
    SELFHOST_BEARER?: string;
  }
}

export type {};
