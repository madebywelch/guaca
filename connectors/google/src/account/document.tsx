import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { COLUMN, SiteFooter, SiteHeader } from "../components/chrome.js";

/**
 * The document the signed-in area is served as.
 *
 * `renderToStaticMarkup` rather than `renderToString`: there is no client to
 * hydrate, so hydration markers would be bytes with nothing to do, and it is
 * the renderer that leaves adjacent text alone instead of splitting it with
 * comment nodes. React escapes every interpolated value, which is what this
 * file replaced a hand-written `escapeHtml` and a `raw()` opt-out with.
 *
 * The stylesheet is the marketing build's, found at request time by reading
 * the prerendered page through the asset binding. That is the whole reason
 * both halves look alike: one build, one hashed file, no second copy of the
 * tokens or the fonts.
 */
export interface DocumentProps {
  title: string;
  /** The right-hand side of the header. Whatever this page's exit is. */
  nav?: ReactNode;
  /** The marketing build's hashed stylesheet, or null if it could not be read. */
  stylesheet: string | null;
  children: ReactNode;
}

export function Document({ title, nav, stylesheet, children }: DocumentProps) {
  return (
    <html lang="en">
      <head>
        <meta charSet="UTF-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>{title}</title>
        {/* Not the marketing page. Nothing here should be a search result:
            every page is one person's grants or a form that changes them. */}
        <meta name="robots" content="noindex" />
        {/* The signed-in area links the marketing build's stylesheet, so it has
            both surfaces too. Declaring one is the browser drawing light form
            controls and a white canvas around a page that is ink. */}
        <meta name="color-scheme" content="light dark" />
        <meta name="theme-color" content="#fbf9f3" media="(prefers-color-scheme: light)" />
        <meta name="theme-color" content="#12110d" media="(prefers-color-scheme: dark)" />
        <link rel="icon" href="/favicon.svg" type="image/svg+xml" />
        <link rel="apple-touch-icon" href="/apple-touch-icon.png" />
        {stylesheet ? <link rel="stylesheet" href={stylesheet} /> : null}
      </head>
      <body className="flex min-h-dvh flex-col">
        <SiteHeader actions={nav} shell={COLUMN} />
        <main id="main" className="grow py-14 sm:py-20">
          <div className={COLUMN}>{children}</div>
        </main>
        <SiteFooter shell={COLUMN} />
      </body>
    </html>
  );
}

/** The document as bytes, doctype included. */
export function renderDocument(props: DocumentProps): string {
  return `<!doctype html>${renderToStaticMarkup(<Document {...props} />)}`;
}
