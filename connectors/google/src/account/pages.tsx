import type { ReactNode } from "react";

/**
 * Every page of the signed-in area, as React.
 *
 * These were markup strings behind a tagged template until the two halves of
 * the origin drifted far enough apart that signing in felt like leaving the
 * site. They are components now, in the same language, reading the same
 * stylesheet and the same tokens, so the design is one thing rather than two
 * that resemble each other.
 *
 * Pure functions from state to elements, called by the routes in
 * `worker/index.ts` and rendered to static markup by `document.tsx`. Nothing
 * here reads a request, touches a database, or ships to a browser: this area
 * runs no JavaScript on purpose, because it is an origin whose job is holding
 * other people's OAuth tokens.
 *
 * The provider types below are the display shape, not the catalog. The
 * Worker passes `PROVIDERS` straight in and the compiler checks it still fits,
 * which keeps the direction of dependency one way: the UI knows what a
 * connector looks like on a page, not what it costs to ship one.
 */

export interface DisplayCapability {
  id: string;
  label: string;
  /** What an agent can do once this is granted. */
  grants: string;
  /** A limit of the scope a person would otherwise discover by failing. */
  caveat?: string;
}

export interface DisplayProvider {
  id: string;
  label: string;
  capabilities: readonly DisplayCapability[];
}

/**
 * One authorized identity, as a card on this page.
 *
 * A person can authorize the same provider twice — a work Google and a personal
 * one — and each is its own grant with its own scopes and its own revocation.
 * `worker/connections.ts` argues why that is the unit rather than the provider.
 */
export interface DisplayConnection {
  id: string;
  providerId: string;
  /** The provider's own name for it, which is how a person tells two apart. */
  label: string;
  capabilities: Set<string>;
}

/**
 * An aside about state: what just failed, what is not configured, what a
 * provider will do next.
 *
 * A bad one is announced. These pages have no JavaScript, so every notice
 * arrives with a fresh document and `role="alert"` is what tells a screen
 * reader that the thing it is about to read is why the last action did not
 * work, rather than more of the page.
 */
function Note({ children, bad = false }: { children: ReactNode; bad?: boolean }) {
  const tone = bad ? "border-danger text-danger" : "border-rule text-muted";
  return (
    <p role={bad ? "alert" : undefined} className={`m-0 border-l-[3px] ${tone} pl-4 text-body`}>
      {children}
    </p>
  );
}

/**
 * A code field: six digits, or the pairing code the desktop app shows.
 *
 * `autoComplete="one-time-code"` is the whole reason this is one component.
 * It is what makes a phone offer the code from the notification, and getting
 * it wrong costs a person a trip to another app and back with six digits in
 * their head.
 *
 * Deliberately not autofocused. On a page whose entire content is a heading
 * saying what happened, a line saying where the code went, and this field,
 * moving focus past the first two costs the people who cannot see them the
 * only context there is, and saves everyone else one tap.
 */
function CodeField(input: {
  name: string;
  label: string;
  autoComplete: "one-time-code" | "off";
  defaultValue?: string;
  inputMode?: "numeric" | "text";
  maxLength?: number;
}) {
  return (
    <label className="block max-w-[15rem]">
      <span className="field__label">{input.label}</span>
      <input
        name={input.name}
        defaultValue={input.defaultValue ?? ""}
        autoComplete={input.autoComplete}
        autoCapitalize="characters"
        inputMode={input.inputMode ?? "text"}
        maxLength={input.maxLength}
        spellCheck="false"
        required
        className="input input--code"
      />
    </label>
  );
}

/**
 * Step one: the address.
 *
 * There is no provider button here and no password field. Identity on this
 * origin is an email address that received a code, which is what keeps a
 * Google consent screen meaning exactly one thing.
 */
export function signIn(input: {
  /** Where to land afterwards. Already checked to be a path on this origin. */
  next?: string;
  error?: string;
}) {
  return (
    <div className="grid gap-5">
      <p className="label m-0">Connectors</p>
      <h1 className="display m-0 text-head">Sign in to authorize a service.</h1>
      <p className="lede m-0">
        Guaca itself needs no account. This is only for the services that will not hand access to an
        app running on your own machine, and it is where their tokens are kept.
      </p>
      {input.error ? <Note bad>{input.error}</Note> : null}
      <form method="post" action="/signin/email" className="grid gap-5">
        {/* Round-tripped through the form rather than the button's href, so it
            survives the post that follows. Losing it here is invisible: sign-in
            works, and the person lands on the wrong page still holding a code. */}
        {input.next ? <input type="hidden" name="next" value={input.next} /> : null}
        <label className="block max-w-[22rem]">
          <span className="field__label">Email</span>
          <input
            type="email"
            name="email"
            autoComplete="email"
            spellCheck="false"
            required
            className="input"
          />
        </label>
        <div className="flex flex-wrap items-center gap-3">
          <button className="btn btn--primary" type="submit">
            Email me a code
          </button>
        </div>
      </form>
      {/* The one page on this origin where an account comes into existence, so
          the one page where the two documents belong in the text rather than
          only in the footer under it. */}
      <p className="m-0 text-ui text-faint">
        No password. A six-digit code arrives by email, and entering it creates the account the
        first time. The account holds your email and the grants you authorize, nothing else. The{" "}
        <a className="link" href="/privacy">
          privacy policy
        </a>{" "}
        says that in full, and the{" "}
        <a className="link" href="/terms">
          terms
        </a>{" "}
        cover what making one commits you to.
      </p>
    </div>
  );
}

/** Step two: the code. The address it went to is in a cookie, not the URL. */
export function signInCode(input: { email: string; next?: string; error?: string }) {
  return (
    <div className="grid gap-5">
      <p className="label m-0">Sign in</p>
      <h1 className="display m-0 text-head">Enter the code we sent.</h1>
      <p className="lede m-0">
        Six digits, on their way to <span className="mono text-lede">{input.email}</span>. The code
        expires in ten minutes and works once.
      </p>
      {input.error ? <Note bad>{input.error}</Note> : null}
      <form method="post" action="/signin/code" className="grid gap-5">
        {input.next ? <input type="hidden" name="next" value={input.next} /> : null}
        <CodeField
          name="code"
          label="Code"
          autoComplete="one-time-code"
          inputMode="numeric"
          maxLength={6}
        />
        <div className="flex flex-wrap items-center gap-3">
          <button className="btn btn--primary" type="submit">
            Continue
          </button>
        </div>
      </form>
      <p className="m-0 text-ui text-faint">
        Nothing arrived, or wrong address?{" "}
        <a className="link" href="/signin">
          Start again
        </a>
        . An unused code stops working as soon as you ask for another.
      </p>
    </div>
  );
}

function capabilityRow(
  capability: DisplayCapability,
  granted: boolean,
  // Two cards for the same provider render the same capabilities, and a
  // duplicated `id` would point every label at the first card's checkbox: a
  // click on the second card's "Gmail" would tick the first card's box. So the
  // id is scoped to the card rather than to the provider.
  scope: string,
) {
  const id = `${scope}-${capability.id}`;
  return (
    <li key={capability.id} className="grid grid-cols-[1.1rem_1fr] items-start gap-3">
      <input
        type="checkbox"
        id={id}
        name="capability"
        value={capability.id}
        defaultChecked={granted}
        className="mt-1.5 h-[1.1rem] w-[1.1rem] accent-text"
      />
      <label htmlFor={id}>
        <span className="font-medium">{capability.label}</span>
        <span className="block text-body text-muted">{capability.grants}</span>
        {capability.caveat ? (
          <span className="mt-0.5 block text-ui text-faint">{capability.caveat}</span>
        ) : null}
      </label>
    </li>
  );
}

/**
 * One authorized identity, and what it may do.
 *
 * The unit on this page is a connection rather than a provider, because two
 * Google accounts are two grants: revoking one says nothing about the other,
 * and a group has to be able to point at one of them. Everything that acts —
 * updating the ticks, disconnecting — carries the connection's id, so a page
 * showing two Googles cannot act on the wrong one.
 */
function connectionCard(provider: DisplayProvider, connection: DisplayConnection) {
  const count = connection.capabilities.size;
  const pill = count > 0 ? "bg-system-wash text-system" : "bg-sunken text-faint";
  return (
    <section key={connection.id} className="card">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="card__title m-0">
          {provider.label}
          <span className="mono block text-aside font-normal text-muted">{connection.label}</span>
        </h2>
        <span
          className={`rounded-pill ${pill} px-2.5 py-1 font-medium text-meta whitespace-nowrap`}
        >
          {count > 0 ? `${count} authorized` : "Nothing authorized"}
        </span>
      </div>
      <form method="post" action="/app/authorize">
        <input type="hidden" name="provider" value={provider.id} />
        <input type="hidden" name="connection" value={connection.id} />
        <ul className="m-0 mt-4 grid list-none gap-3.5 p-0">
          {provider.capabilities.map((capability) =>
            capabilityRow(capability, connection.capabilities.has(capability.id), connection.id),
          )}
        </ul>
        <div className="mt-5 flex flex-wrap items-center gap-3">
          <button className="btn btn--primary" type="submit">
            Update access
          </button>
        </div>
      </form>
      <p className="mt-4 mb-0 text-ui text-faint">
        {provider.label} issues these as one grant, so removing a capability revokes the whole thing
        and asks you to authorize the rest again. Adding one does not.
      </p>
      <form method="post" action="/app/disconnect">
        <input type="hidden" name="provider" value={provider.id} />
        <input type="hidden" name="connection" value={connection.id} />
        <button className="btn btn--quiet" type="submit">
          Disconnect {connection.label} and revoke the token
        </button>
      </form>
    </section>
  );
}

/**
 * The card that authorizes a new identity at a provider.
 *
 * Always offered, even when one is already connected, because that is the whole
 * point: a person with a work Google and a personal one needs a second grant
 * rather than a way to overwrite the first. Google is sent `select_account`, so
 * this reaches its account chooser rather than silently re-authorizing whoever
 * the browser is signed in as.
 */
function addConnectionCard(provider: DisplayProvider, existing: number) {
  const scope = `add-${provider.id}`;
  return (
    <section key={scope} className="card">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="card__title m-0">
          {existing > 0 ? `Another ${provider.label} account` : provider.label}
        </h2>
      </div>
      {existing > 0 ? (
        <p className="mt-3 mb-0 text-body text-muted">
          Authorizing a second account adds a grant beside the {existing === 1 ? "one" : "ones"}{" "}
          above rather than replacing {existing === 1 ? "it" : "them"}. Each of your groups chooses
          which one it uses.
        </p>
      ) : null}
      <form method="post" action="/app/authorize">
        <input type="hidden" name="provider" value={provider.id} />
        <ul className="m-0 mt-4 grid list-none gap-3.5 p-0">
          {provider.capabilities.map((capability) => capabilityRow(capability, false, scope))}
        </ul>
        <div className="mt-5 flex flex-wrap items-center gap-3">
          <button className="btn btn--primary" type="submit">
            {existing > 0 ? `Authorize another ${provider.label} account` : "Authorize"}
          </button>
        </div>
      </form>
    </section>
  );
}

export function connectors(input: {
  providers: readonly DisplayProvider[];
  connections: readonly DisplayConnection[];
  notice?: { text: string; bad?: boolean };
  selfHosted?: boolean;
}) {
  return (
    <div className="grid gap-9">
      <div className="grid gap-5">
        <p className="label m-0">Connectors</p>
        <h1 className="display m-0 text-head">What your agents may reach.</h1>
        <p className="lede m-0">
          Authorizing a service here stores a token for it. Guaca hands that token to no one: your
          agents call the service through this origin, and the token stays.
        </p>
        {input.notice ? <Note bad={input.notice.bad}>{input.notice.text}</Note> : null}
      </div>

      {input.providers.length === 0 ? (
        <Note>No provider is configured on this deployment.</Note>
      ) : (
        <div className="grid gap-5">
          {input.providers.flatMap((provider) => {
            const mine = input.connections.filter(
              (connection) => connection.providerId === provider.id,
            );
            return [
              ...mine.map((connection) => connectionCard(provider, connection)),
              addConnectionCard(provider, mine.length),
            ];
          })}
        </div>
      )}

      <div className="grid gap-3">
        <h2 className="card__title m-0">Using these in Guaca</h2>
        <p className="m-0 text-ui text-faint">
          {input.selfHosted
            ? "Each group can connect an authorized provider under its Connectors and choose which account it uses."
            : "Sign in to this account from Guaca, under Settings, Account. Each of your groups then connects the provider it needs under that group's Connectors, and chooses which of the accounts above it uses."}
        </p>
      </div>
    </div>
  );
}

/**
 * The one screen in the sign-in that this origin writes.
 *
 * Everything before it belongs to the authorization server: it has already
 * checked which application is asking, that the redirect it named is one this
 * origin registered for it, and that the request carries a PKCE challenge. What
 * is left is the only question a machine cannot answer, which is whether the
 * person in front of the browser meant to hand an account to a copy of Guaca.
 *
 * `query` is that request, signed by the authorization server, and it is passed
 * straight back on the form. The page cannot widen what was asked for: the
 * signature covers the scopes, and `oauth2Consent` checks it before it mints
 * anything.
 *
 * Deny posts as well as allow. A refusal that just closed the tab would leave
 * Guaca listening on a port until it gave up, so the answer goes back either
 * way and the app says it was refused.
 */
export function consent(input: { email: string; scopes: readonly string[]; query: string }) {
  return (
    <div className="grid gap-5">
      <p className="label m-0">Link a machine</p>
      <h1 className="display m-0 text-head">Let Guaca use this account?</h1>
      <p className="lede m-0">
        A copy of Guaca on this machine is asking to link to{" "}
        <span className="font-medium">{input.email}</span>. It stays linked until you sign it out,
        here or in the app.
      </p>
      {input.scopes.length > 0 ? (
        <ul className="m-0 grid list-none gap-3.5 p-0">
          {input.scopes.map((line) => (
            <li key={line} className="text-body text-muted">
              {line}
            </li>
          ))}
        </ul>
      ) : null}
      <Note>
        Only continue if you started this from Guaca on this machine. Nothing else can have opened
        this page.
      </Note>
      <div className="flex flex-wrap items-center gap-3">
        <form method="post" action="/consent">
          <input type="hidden" name="oauth_query" value={input.query} />
          <input type="hidden" name="decision" value="allow" />
          <button className="btn btn--primary" type="submit">
            Link it
          </button>
        </form>
        <form method="post" action="/consent">
          <input type="hidden" name="oauth_query" value={input.query} />
          <input type="hidden" name="decision" value="deny" />
          <button className="btn btn--quiet" type="submit">
            Cancel
          </button>
        </form>
      </div>
    </div>
  );
}

export function notice(input: { heading: string; body: string; bad?: boolean; back?: string }) {
  return (
    <div className="grid gap-5">
      <h1 className="display m-0 text-head">{input.heading}</h1>
      {input.bad ? <Note bad>{input.body}</Note> : <p className="lede m-0">{input.body}</p>}
      {input.back ? (
        <p className="m-0">
          <a className="link" href={input.back}>
            Back
          </a>
        </p>
      ) : null}
    </div>
  );
}

/**
 * The header's right-hand side once there is a session.
 *
 * The email belongs here rather than over the heading. This origin holds
 * grants for one account and a person may well have two, so which one they are
 * looking at has to be on every page, not announced once.
 */
export function signedInNav(email: string) {
  return (
    <>
      <span className="mono text-small text-faint">{email}</span>
      <form method="post" action="/signout">
        <button className="btn btn--quiet" type="submit">
          Sign out
        </button>
      </form>
    </>
  );
}
