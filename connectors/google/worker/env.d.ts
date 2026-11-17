declare global {
  interface Env {
    ASSETS: Fetcher;
    DB: D1Database;

    EMAIL: SendEmail;
    EMAIL_FROM?: string;

    BETTER_AUTH_URL: string;
    BETTER_AUTH_SECRET: string;

    GOOGLE_CLIENT_ID?: string;
    GOOGLE_CLIENT_SECRET?: string;
    GITHUB_CLIENT_ID?: string;
    GITHUB_CLIENT_SECRET?: string;
    SELFHOST_BEARER?: string;
  }
}

export type {};
