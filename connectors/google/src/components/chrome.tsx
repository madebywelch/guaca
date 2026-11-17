import type { ReactNode } from "react";

import { SITE } from "../content.js";

export const SHELL = "mx-auto w-full max-w-6xl px-6";

export const COLUMN = "mx-auto w-full max-w-[46rem] px-6";

export function Brand() {
  return (
    <a className="brand" href="/" aria-label={`${SITE.name}, home`}>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcSet="/wordmark-dark.svg" />
        <img
          className="brand__wordmark"
          src="/wordmark.svg"
          alt=""
          width={543}
          height={148}
          decoding="async"
        />
      </picture>
    </a>
  );
}

export function GitHubMark() {
  return (
    <svg className="glyph" viewBox="0 0 16 16" aria-hidden="true">
      <path
        fill="currentColor"
        d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z"
      />
    </svg>
  );
}

export function PublicActions() {
  return (
    <a className="btn btn--primary btn--compact" href={SITE.repo}>
      <GitHubMark />
      GitHub
    </a>
  );
}

export function SiteHeader({
  nav,
  actions,
  shell = SHELL,
}: {
  nav?: ReactNode;
  actions?: ReactNode;
  shell?: string;
}) {
  return (
    <header className="masthead">
      <div className={`${shell} flex h-(--masthead-h) items-center gap-10`}>
        <Brand />
        {nav ? (
          <nav aria-label="Sections" className="hidden items-center gap-7 md:flex">
            {nav}
          </nav>
        ) : null}
        {actions ? <div className="ml-auto flex items-center gap-5">{actions}</div> : null}
      </div>
    </header>
  );
}

const FOOTER = [
  {
    heading: "Product",
    links: [
      { label: "Features", href: "/#features" },
      { label: "Install", href: SITE.install },
      { label: "Releases", href: `${SITE.repo}/releases` },
      { label: "Connectors", href: SITE.signIn },
    ],
  },
  {
    heading: "Open source",
    links: [
      { label: "Source", href: SITE.repo },
      { label: "Documentation", href: `${SITE.repo}#readme` },
      { label: "Issues", href: `${SITE.repo}/issues` },
    ],
  },
  {
    heading: "Legal",
    links: [
      { label: "Privacy", href: SITE.privacy },
      { label: "Terms", href: SITE.terms },
      { label: SITE.license.name, href: SITE.license.url },
    ],
  },
] as const;

export function SiteFooter({ shell = SHELL }: { shell?: string } = {}) {
  return (
    <footer className="footer">
      <div className={`${shell} @container pt-16 pb-10`}>
        <div className="grid grid-cols-2 gap-x-6 gap-y-10 @xl:grid-cols-3 @4xl:grid-cols-[minmax(0,2fr)_repeat(3,minmax(0,1fr))]">
          <div className="col-span-2 @xl:col-span-3 @4xl:col-span-1">
            <Brand />
            <p className="footer__line">Local AI agents that talk to each other.</p>
          </div>
          {FOOTER.map((group) => (
            <div key={group.heading}>
              <h2 className="footer__heading">{group.heading}</h2>
              <ul className="footer__links">
                {group.links.map((link) => (
                  <li key={link.label}>
                    <a className="footer__link" href={link.href}>
                      {link.label}
                    </a>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
        <p className="footer__base">
          {SITE.name} by <a href={`https://github.com/${SITE.author}`}>{SITE.author}</a>. Free
          software under {SITE.license.name}.
        </p>
      </div>
    </footer>
  );
}
