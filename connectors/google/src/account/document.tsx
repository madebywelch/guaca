import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { COLUMN, SiteFooter, SiteHeader } from "../components/chrome.js";

export interface DocumentProps {
  title: string;
  nav?: ReactNode;
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
        <meta name="robots" content="noindex" />
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

export function renderDocument(props: DocumentProps): string {
  return `<!doctype html>${renderToStaticMarkup(<Document {...props} />)}`;
}
