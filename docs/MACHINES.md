# Machines

An agent can be given a computer: an E2B sandbox with a Linux desktop, a
browser, and a viewer the operator can watch. `e2b.rs` starts them, `proxy.rs`
serves the viewer, `browser.py` drives Chrome over the DevTools protocol, and
`sessions.py` reports what that Chrome is signed in to.

Why an agent is told what it already has access to, why a session is scoped to
one agent while a credential is scoped to the group, and why an observed
capability that overclaims is worse than none, are in `PROTOCOL.md`,
*Connectors*. What follows is what will bite you in the code.

## There is one browser on every machine, and one profile in it

Chrome ignores `--remote-debugging-port` when it re-attaches to an existing
profile, so `browse` needs a profile it controls, and a sign-in performed on the
default one was invisible to every agent with nothing reporting an error. The
other half is the same failure wearing a different name: the template ships a
second browser, with a binary on `PATH`, a menu entry and an icon on the
desktop, and an agent told to send mail opened it, drove it by coordinates, and
read the page with `browse`, which was on Chrome the whole time. Neither is
something an agent can be asked to remember. A prompt saying "use Chrome" was
already there.

So every route is shimmed onto `google-chrome` and `/home/user/.guac/chrome`: a
wrapper first on `PATH` with every other browser's name symlinked to it, a
`.desktop` entry in the user's own XDG directory shadowing each packaged one, a
launcher on the desktop rewritten in place because it is a file rather than an
entry anything looks up, and `as_chrome` at the call site, which replaces the
browser's name as well as adding the flags. The session is started with that
directory first on `PATH`, since every icon, menu entry and terminal on the
screen inherits it. Anything still running on another profile or of another kind
is closed when the desktop starts, the operator's own window included, because a
sign-in there is one no agent can ever use.

If you add a way to open a browser it goes through `as_chrome`, and the port
goes with the profile or `browse` loses its remote interface.

## An agent that named another browser is told which one opened

The rewrite is silent on the machine and must not be silent in the turn:
handing an agent back the name it asked for leaves it describing a window nobody
can see and reaching for that name again. The flags do not travel with it
either, in the result or in the transcript, because a model reads its own tool
results back and copies them.

## Sign-ins are detected, never declared

The browser is holding the cookies, so `domain/signin.rs` asks it rather than
asking the operator to keep a list. The whole set for an agent is replaced on
every scan: a row that outlives the logout it should have noticed keeps the crew
routing work to a machine that will hit a login wall.

## A cookie's presence is not a login, and this is the trap

A profile that has browsed for an hour holds a thousand cookies across three
hundred domains, most of them durable and `httpOnly`. `google.com` sets `NID` on
a browser that has never seen an account, and `PHPSESSID` is handed to every
anonymous visitor. Both were real false positives from a live machine. Detection
is therefore a signature table plus a rule that needs the browser to have
*visited* the site and to hold a cookie implying an identity rather than a
session. The tests carry the real cookie names, and they are the whole
defence: do not loosen them without a fresh capture.

## A cookie value must never leave the sandbox

`browser.py` drops it at the only point in the system that sees one, and
`CookieMark` has no field it could arrive in. The same holds one level up: a
credential's secret has no field on `Connector`, so it cannot be serialized to
the webview or rendered into a prompt. It goes from SQLite into the `envs` of
one sandbox command and stops there, and deliberately not into a dotfile on the
sandbox either, because that disk survives the sleep this app relies on.
