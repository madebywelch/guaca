/**
 * Finding the stylesheet the signed-in area shares with the marketing page.
 *
 * All that is left of a module that used to hold a tagged template, an
 * `escapeHtml`, and a `raw()` opt-out. React renders these pages now and
 * escapes by default, so the only thing the Worker still has to do by hand is
 * work out which hashed file to link.
 */

/**
 * The marketing build's stylesheet, found by reading the page it is linked
 * from rather than by importing a build manifest.
 *
 * The filename is content-hashed, so something has to carry it across the gap
 * between `vite build` and this Worker. The choice is a generated module in
 * the repository, a manifest import that couples the two builds, or one asset
 * read per isolate. The read wins: it cannot go stale, it needs no build step
 * to remember, and what it finds is exactly the stylesheet a visitor gets.
 *
 * That stylesheet carries the tokens, the self-hosted fonts and the character
 * classes, so this area is the same design as the page in front of it without
 * a second copy of any of it. The components that read it live under `src/`, so
 * Tailwind's source scan covers them and the utilities they use are in that
 * same file: there is no second stylesheet for this half of the origin.
 */
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
      // A page that renders unstyled is worth more than a 500. `verify-dist`
      // already fails the build if the stylesheet is missing, so reaching here
      // means the asset binding is broken, not that the page is wrong.
      return null;
    }
  })();
  stylesheets.set(env, found);
  return found;
}
