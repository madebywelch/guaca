const stylesheets = new WeakMap<object, Promise<string | null>>();

export function stylesheetHref(env: Env): Promise<string | null> {
  if (env.SELFHOST_BEARER) return Promise.resolve("/assets/index-DVbVPvcI.css");
  const cached = stylesheets.get(env);
  if (cached) return cached;
  const found = (async () => {
    try {
      const res = await env.ASSETS.fetch(new Request("https://assets.local/index.html"));
      if (!res.ok) return null;
      const match = /<link\b[^>]*\brel="stylesheet"[^>]*\bhref="([^"]+)"/.exec(await res.text());
      return match?.[1] ?? null;
    } catch {
      return null;
    }
  })();
  stylesheets.set(env, found);
  return found;
}
