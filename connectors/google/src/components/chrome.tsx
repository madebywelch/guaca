import type { ReactNode } from "react";

import { SITE } from "../content.js";

/**
 * The header and footer, shared by both halves of the origin.
 *
 * The marketing page and the signed-in area used to draw these separately, in
 * two languages, and it showed: signing in took a person from a site to a
 * form. They are the same site, so the chrome around them is one component and
 * the difference between the two is what goes in the nav.
 */

/** The marketing page's column. The other two halves pass their own, narrower. */
export const SHELL = "mx-auto w-full max-w-6xl px-6";

/**
 * The reading column: the signed-in area's forms, and the legal pages.
 *
 * Both are things a person reads or fills in one line at a time, which wants a
 * measure the marketing page's six-column shell does not give. It lives here
 * rather than beside either of them because it is now shared by both.
 */
export const COLUMN = "mx-auto w-full max-w-[46rem] px-6";

/**
 * The wordmark, which is the logo.
 *
 * Ledger's identity is Eye contact: the word `guaca` drawn as circles and one
 * round stroke, with each a's bowl an eye and the two pupils meeting
 * across the c. It replaced the app's square icon beside a tracked name, which
 * was a mark *and* a word side by side and read as two things; the wordmark
 * already spells the name, so the name beside it would say it twice.
 *
 * Two files rather than one that recolors itself, because an `<img>` cannot
 * read `currentColor`. `prefers-color-scheme` picks the ink variant, which is
 * the same geometry drawn in the site's paper ink, and the browser only
 * fetches the one it draws. `alt` is empty because the link's own label is the
 * accessible name: a screen reader should hear "Guaca, home", not the mark
 * followed by the same word.
 *
 * Always a link home. The header stays on screen while the page scrolls, and
 * the logo is where a person reaches to get back to the top.
 */
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

/** GitHub's own mark, from Octicons (MIT), for the buttons that go there. */
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

/**
 * The right-hand side of every public page: the way to the app, and nothing
 * else. No "Sign in": the app needs no account, and a header offering one
 * tells a stranger it might.
 */
export function PublicActions() {
  return (
    <a className="btn btn--primary btn--compact" href={SITE.repo}>
      <GitHubMark />
      GitHub
    </a>
  );
}

/**
 * `nav` sits beside the logo and is the page's own sections; only the marketing
 * page has any, and below a tablet's width they go, because a phone scrolls to
 * them faster than it can aim at them. `actions` sits at the far end and never
 * goes: it is how a person leaves this page for the thing they came for.
 *
 * `shell` is the column, and it is a prop because the two halves do not share
 * one. The signed-in area is forms, which want a narrow measure, and a logo
 * sitting a hundred pixels left of the heading under it is most of what made
 * signing in feel like leaving the site.
 */
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

/**
 * What the footer lists, grouped the way a person looks for it.
 *
 * Paths into the marketing page are written from the root, because the footer
 * is also worn by the legal pages and the signed-in area, where a bare anchor
 * would point at nothing.
 */
const FOOTER = [
  {
    heading: "Product",
    links: [
      { label: "Features", href: "/#features" },
      { label: "Install", href: SITE.install },
      { label: "Releases", href: `${SITE.repo}/releases` },
      // The one door to the account, for the few services that need it.
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
    // The legal pages have nowhere else to be linked from. Every page on this
    // origin wears this footer, which is what makes one link here reach them
    // from the marketing page, a form and a consent screen.
    heading: "Legal",
    links: [
      { label: "Privacy", href: SITE.privacy },
      { label: "Terms", href: SITE.terms },
      { label: SITE.license.name, href: SITE.license.url },
    ],
  },
] as const;

/**
 * The grid answers to the column it is in rather than to the window, because
 * the same footer sits under a 72rem page and a 46rem form, and at a given
 * window width those two have very different room for five columns.
 */
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
