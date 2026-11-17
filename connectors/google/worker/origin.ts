export function origin(value: string): string {
  if (URL.canParse(value) && new URL(value).origin === value) return value;
  throw new Error(
    `BETTER_AUTH_URL is ${JSON.stringify(value)}. It must be the origin exactly as registered with ` +
      "every provider: a scheme and a host, with no path and no trailing slash.",
  );
}
