# The account

Guaca runs on your machine, with your keys, and an account changes none of that.
This file is about the one thing it does change, why that thing cannot be done
locally, and what had to be true before it was worth building at all.

## Nobody has to sign in

The app is fully usable with no account, and the code is arranged so that stays
true rather than so it reads well in a README:

- `Account` is a field on `AppState` that nothing else in the app depends on.
  No turn, no prompt, no tool, no guard reads it. Deleting the module would
  remove one Settings pane and break nothing else.
- Both reads happen when the Account pane is opened, not at startup. An install
  that never opens that pane never sends `guaca.bot` a request, ever.
- The pane's first sentence says it is optional. That is not marketing copy; it
  is the answer to the question an operator is actually asking when they find a
  sign-in inside a local app.

If a change makes any of those three false, it is a change to what Guaca is, and
it needs arguing on those terms rather than landing as a refactor.

## What an account is for

One thing: a hosted OAuth client.

Guaca's answer to reaching an operator's accounts is normally to sign a browser
in on the agent's own computer. Where that works it is the better answer — the
app never handles a password, there is no token to keep, and logging out ends it
(`docs/MACHINES.md`). It stops working at exactly one place, which is a service
that will only issue programmatic access to a registered OAuth application.
Gmail is the example everybody hits.

Guaca cannot be that application. Its client secret would ship inside a download
anybody can read, which is not a secret, and no amount of local cleverness
changes that. A hosted origin can be, and `guaca.bot` is: it holds the client,
holds the refresh token, and hands a paired machine a short-lived access token.
The refresh token never reaches this machine, which is the whole reason storing
it there beats storing it here.

Everything else that has ever been proposed for an account — agents that run in
the cloud, managed provider keys — is a separate argument that has not been had.
Nothing in this file or in `account.rs` assumes them.

## Signing in is authorization code with PKCE, on a loopback port

This is the third OAuth flow in the app and the second of its kind:

| | Flow | Why |
|---|---|---|
| `subscription.rs` | Device code | OpenAI operates the client; the device flow is what they publish |
| `oauth.rs` | Code + PKCE, loopback | MCP mandates it, and vendors issue a client on the spot |
| `account.rs` | Code + PKCE, loopback | RFC 8252, and the asset is somebody's mail |

`subscription.rs` argues against a loopback redirect: a fixed port may already
belong to something else, and a URL scheme is claimed by whichever build
registered last. Both objections are about a port or a scheme **chosen in
advance**. `oauth.rs` answers them by binding `127.0.0.1:0` *before* naming the
redirect, so the port is one the operating system has already handed out and
nothing can take it in between. This module is that answer pointed at one known
server.

### Why not the device grant, given one was already built

`guaca.bot` shipped an RFC 8628 device grant before the app half existed, and it
was removed rather than kept beside this one.

A device code is a bearer secret carried by a human. RFC 8628 §5.4 names what
follows: nothing binds the code to the machine that asked for it, so anyone who
can talk an operator into approving a code walks away with a token that mints
Gmail access tokens on that operator's account. "Your Guaca needs re-pairing,
enter this code" is the whole attack, and it works over a chat message.

That is the correct trade for a television. It is not the correct trade for an
application that has a browser on the same machine, which is the case RFC 8252
is written about. A loopback redirect cannot be phished across machines: the
code is delivered to `127.0.0.1` on the machine that started the flow, and it is
worth nothing without a verifier that never left the process.

Both were not kept, because two doors to one account means the weaker one
decides what the account is worth. `migrations/0001_oauth_provider.sql` on the
service drops the device table.

### The dance

1. `GET https://guaca.bot/.well-known/oauth-authorization-server` (RFC 8414).
2. Bind `127.0.0.1:0`. Only now is the redirect URI known.
3. Open the operator's browser at the authorization endpoint, with a PKCE
   challenge and a state.
4. Catch the redirect on that port. Check the issuer and the state before
   writing a success page, so a mismatch is never told it worked.
5. `POST` the code and the verifier to the token endpoint.
6. Spend the token once, on `/api/connectors`, before anything is written.

Steps 3 to 5 are `oauth.rs`'s own helpers, reused rather than reimplemented.
Step 6 is this module's, and it is the difference between a sign-in that reports
success and one that works: it is also where the account's email comes from, so
there is no second call to make.

**The client is fixed and public.** `guaca-desktop`, seeded by the service's
migration, `token_endpoint_auth_method: "none"`, no secret. `oauth.rs` registers
a client dynamically because MCP vendors require it; here that would mean an
open registration endpoint on a database whose job is holding other people's
refresh tokens, and a consent screen naming an application a stranger asserted.
The fixed client is the smaller surface and the more honest screen.

**The redirect URI is registered with no port.** RFC 8252 §7.3 has the
authorization server compare a loopback redirect on scheme, host, path and query
while ignoring the port. That is what makes "bind first, then ask" possible at
all, and it is the one thing the service must not stop honoring.

### The issuer is read, never assumed

RFC 9207 puts an `iss` on the redirect, and that check is worth exactly what the
value it is compared against is worth. The value is the `issuer` field of the
metadata document from step 1, and nothing else.

It used to be the origin, on the reasoning that the document was fetched from the
root of the origin so the two had to be the same string. They are the same string
only for an authorization server mounted at that root, and `guaca.bot` mounts its
own under a path:

```json
{
  "issuer": "https://guaca.bot/api/auth",
  "authorization_endpoint": "https://guaca.bot/api/auth/oauth2/authorize",
  "authorization_response_iss_parameter_supported": true
}
```

So every sign-in opened a browser, reached the consent screen, was issued a code,
and was refused on the way back for naming the issuer the service publishes. RFC
8414 §3.3 wants a document served at the root well-known path to claim the bare
origin, so the service is bending that rule as well, but where the two disagree
the published value is the one that wins: it is also the one the server will
send.

The origin is still what an *absent* `issuer` means, because that is what the
address the document arrived from implies. Substituting it unconditionally was
the bug; substituting it when nothing was published is the reading.

Both are then checked to be on the configured origin, alongside the two
endpoints. An unchecked issuer is worse than no check at all: a metadata document
naming a third party would have Guaca accept a code minted by that third party.

## Where the service is

`DEFAULT_ORIGIN`, a constant. `GUACA_ACCOUNT_ORIGIN` moves it and exists so the
flow can be run end to end against a Worker on this machine.

It is an environment variable rather than a setting, and `subscription.rs` gives
the reason: a sign-in service an operator can type into a box is a credential
sent somewhere nobody chose. Three things guard it anyway, because an
environment variable is still an input:

- **HTTPS, or loopback.** Anything else is refused before a request is made. The
  failure this prevents is a credential on a plaintext connection across a
  network; loopback is exempt because it does not cross one.
- **Everything discovered must be on the origin that published it.** Both
  endpoints and the issuer. A metadata document is the one place discovery could
  move a credential to a third party, or name one whose codes Guaca would then
  accept, and the check costs three string comparisons.
- **An override is logged at startup, and shown in the pane.** A machine left
  pointed at a development service is not a silent state, and the two hold
  different accounts.

## What is stored

Its own file, `account.json`, 0600, temp-then-rename.

Not `config.json`, for the reason `subscription.rs` gives: the token set rotates
on refresh, which is Guaca writing in the background, while `config.json` is
rewritten wholesale whenever the operator presses Save. Two writers on one file
lose a refreshed token to a stale in-memory copy, and the symptom is a sign-in
that works until an unrelated setting changes.

Plaintext, with the same caveat and more force. This credential is not Guaca's
to lose: it stands for an account that can mint access tokens for the operator's
mail. The honest fix is the OS keychain, and it is the first thing to reach for
if this file grows a second reader.

`Status` is what crosses IPC, and it has no token and no field one could arrive
in. A test asserts its exact serialization, because a field added there is a
token one serialization away from the webview.

## What the account holds is read, never kept

`/api/connectors` is asked every time the pane needs it. The answer changes when
the operator authorizes something in a browser, not when this app does anything,
and a cached copy would be a list of capabilities an agent is told it has and
does not.

The service is also the only thing that can change that list, because the
consent screens are Google's and GitHub's. The pane has a link and no controls,
which is the truthful shape: a tick box here would be a control that cannot
work.

## How an agent actually reaches it

As a plugin. `guaca.bot` serves an MCP server at `/mcp`, authenticated by the
same account token, and Google appears in a group's Plugins list like any other.
`docs/PLUGINS.md`, *Google is a plugin whose sign-in is the account's*, has the
argument; the short version is that it makes per-agent reach, the trust boundary
and the tool plumbing all work without a line of new machinery.

`POST /api/connectors/:provider/token` still exists on the service and the app
still does not call it. That is deliberate rather than unfinished: it would put
a live Google credential on a laptop for an hour at a time, and the rule from
`docs/MACHINES.md` is the one that decides it — **a secret never reaches a
model.** Keeping the token at the origin that holds the refresh token is the
stronger position, and the app gets tools instead.

## One account, several identities

Signing in to `guaca.bot` is one identity: the address a code reached. What that
account *holds* is not. A person can authorize a work Google and a personal one,
and those are two grants — Better Auth keys them on `(issuer, accountId)`, and
two Google accounts are two subjects.

The first version of this got that wrong by one word. It said "one grant per
provider" and read the first row matching one, so a second account could be
authorized and never seen again, and a crew had no way to name one. Two things
were needed to fix it and only one of them was in the app:

- `prompt=select_account` on Google's consent. `consent` alone forces the screen
  and still silently reuses whichever Google the browser is signed into, so an
  operator could not create the second grant in the first place.
- A connection id on the plugin row, so a group can say which it means.

`docs/PLUGINS.md`, *A crew chooses which identity it uses*, has the app half.

## What is not built yet

Cloud-run agents and managed provider keys, both of which were named as reasons
an account might exist and neither of which has been argued. Nothing in this
module assumes them.
