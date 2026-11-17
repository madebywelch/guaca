import type { ReactNode } from "react";

export interface DisplayCapability {
  id: string;
  label: string;
  grants: string;
  caveat?: string;
}

export interface DisplayProvider {
  id: string;
  label: string;
  capabilities: readonly DisplayCapability[];
}

export interface DisplayConnection {
  id: string;
  providerId: string;
  label: string;
  capabilities: Set<string>;
}

function Note({ children, bad = false }: { children: ReactNode; bad?: boolean }) {
  const tone = bad ? "border-danger text-danger" : "border-rule text-muted";
  return (
    <p role={bad ? "alert" : undefined} className={`m-0 border-l-[3px] ${tone} pl-4 text-body`}>
      {children}
    </p>
  );
}

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

export function signIn(input: {
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
