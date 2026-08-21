# Machines

An agent can be given a computer: an E2B sandbox with a Linux desktop, a
browser, and a viewer the operator can watch. `e2b.rs` starts them, `proxy.rs`
serves the viewer, `browser.py` drives Chrome over the DevTools protocol, and
`sessions.py` reports what that Chrome is signed in to.

Why an agent is told what it already has access to, why a session is scoped to
one agent while a credential is scoped to the group, and why an observed
capability that overclaims is worse than none, are in `PROTOCOL.md`,
*Connectors*. What follows is what will bite you in the code.

## The frame points at this app's page, not at noVNC's

noVNC narrates its own transport. Every time it connects it slides a bar across
the top of the picture reading "Connected (unencrypted) to" and the desktop's
name, and connecting is not a rare event: the frame is rebuilt whenever the
operator opens a different agent's panel. So the bar arrived in the middle of
ordinary work and read as a stall that had not happened. It is also wrong about
the hop that matters, which is TLS from `proxy.rs` to E2B.

The address in the frame is `viewer.html`, which `proxy.rs` answers itself
rather than relaying. That page frames noVNC from the same origin, which is the
only way to reach into a document this app does not own, and appends one rule:
`#noVNC_status.noVNC_status_normal` is hidden. Only the normal kind. The same
bar carries noVNC's errors, those never time out on their own, and they are the
only notice an operator gets that a desktop stopped answering.

Two things drift quietly if you let them. The options deciding autoconnect,
scaling and reconnection live in the address, so the page hands its query
straight on instead of holding a copy of them; and `e2b.rs` builds that address
from `proxy::VIEWER_DOCUMENT`, so the two halves cannot disagree about which
page the frame is pointed at.

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

## The route an agent actually takes names no browser

Every shim above matches on a browser's name, and that is why they all missed
the button on the dock, which has none: it runs `exo-open --launch
WebBrowser`, and which browser that is sits three files away in `helpers.rc`,
whose shipped answer walks `debian-sensible-browser` to `sensible-browser` to
the `x-www-browser` alternative, which the template points at Firefox. An
agent that reads the screen and sees a browser icon clicks it, so this was the
commonest route on the machine and the only unshimmed one, and the operator
watched Chrome, then Firefox, then Chrome again as the next tool call evicted
it. `web_browser_helper` is that shim, and its command is an absolute path
rather than a name, because the process reading it is a panel belonging to a
session whose `PATH` was fixed when it started: these machines sleep and wake
for weeks, so a desktop that came up before a shim existed is reachable by a
file and never by `PATH`. `helpers.rc` is edited rather than written, since
the same file says which terminal and which file manager the desktop opens.

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
