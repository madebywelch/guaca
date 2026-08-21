# Machines

An agent can be given a computer: an E2B sandbox with a Linux desktop, a shell,
a browser, and a viewer the operator can watch. `e2b.rs` starts them, `proxy.rs`
serves the viewer, and `sessions.py` reports what the machine's browser is
signed in to.

A *computer* is not a *browser*. `BROWSERS.md` is the other one: a hosted Chrome
on a different provider, driven over the DevTools protocol, which is where
anything on the web belongs. This file is the machine, and the machine is worked
by looking at it.

Why an agent is told what it already has access to, why a session is scoped to
one agent while a credential is scoped to the group, and why an observed
capability that overclaims is worse than none, are in `PROTOCOL.md`,
*Connectors*. What follows is what will bite you in the code.

## A computer is looked at, never asked

There used to be a second way to use the web on one of these machines. Chrome
was started with its remote debugging port open and driven over the DevTools
protocol, which is exact where a screenshot is a guess. It is gone, and
`BROWSERS.md` explains what replaced it and why. What matters here is the
consequence: `screenshot` and `act_on_desktop` are the entire interface to a
machine's screen, and there is no privileged channel into the browser for
anything to fall out of sync with.

Three things follow, and each was a real failure of the coordinate path that the
exact path used to paper over:

- **The screen is 1024x768**, and clicks are always in true screen pixels. Both
  vendors who ship a computer-use tool train and evaluate at about that size;
  above it the image is resized somewhere out of Guaca's control and every
  coordinate that comes back is in a space nothing here can name. The
  alternative is a larger screen and a scaled screenshot, which is two
  coordinate spaces and a conversion at every call site. One space, chosen so
  nothing downstream wants to resize it, has no such failure. A machine made
  before this changed keeps the screen it started with, which is safe: the
  screenshot reports the geometry it captured.
- **Every action answers with a fresh picture**, not just `look`. The tool used
  to say "look again after anything that changes the screen" and models did not:
  they clicked, were told "clicked at 412, 300", and typed into a form they had
  last seen two actions ago. Prose describing a click cannot carry "the click
  opened a dialog"; a picture can. Only the newest screenshot stays in the
  conversation, replaced by a line saying one was dropped, or a turn spent
  filling a form carries a dozen near-identical images.
- **`xdotool mousemove` carries `--sync`.** Without it the move and the click
  are two requests to X and the second can be delivered first, so a click lands
  wherever the pointer happened to be. On an idle machine they arrive in order
  and the flag looks like superstition. Under load they do not, and it reads as
  a model that cannot aim.

Typing is chunked, `scrot` is called with `--pointer` so the model can see where
the pointer is, and an action that hands work to the screen is given a moment to
settle before the picture is taken. All three are in `e2b.rs` with the failure
each one closes.

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

The template ships a second browser, with a binary on `PATH`, a menu entry and
an icon on the desktop, and an agent told to send mail opened it. That is not
something an agent can be asked to remember, and it is not a brand preference:
only one profile on that machine is read when Guaca asks what it is signed in
to, so a sign-in in any other window is one nothing can see. The operator signs
in on the screen, the roster keeps saying the agent has no account, and the crew
routes work to a machine that will hit a login wall.

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

If you add a way to open a browser it goes through `as_chrome`. It must not open
a debugging port: that is the thing this stopped doing, and one route that
reintroduced it would be a second way to use the web that nobody else knows
about.

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

It is also told that this window is not the browser `browse` uses. They are two
places with two cookie jars, and an agent that reads them as one opens a page
here and looks for it there.

## Sign-ins are detected, never declared

The browser is holding the cookies, so `domain/signin.rs` asks rather than
asking the operator to keep a list. `sessions.py` reads Chrome's own cookie and
history files off the disk, which works with the browser closed and cannot leak
a value: the `value` and `encrypted_value` columns are never selected, so no
token is read at all rather than being read and dropped.

The whole set for one *surface* is replaced on every scan. Scoped to the
surface, because a computer and a browser are scanned independently: a replace
that took the agent's whole set would mean asking one erases everything the
other reported.

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

`sessions.py` drops it at the only point on the machine that sees one, and
`CookieMark` has no field it could arrive in. The same holds one level up: a
credential's secret has no field on `Connector`, so it cannot be serialized to
the webview or rendered into a prompt. It goes from SQLite into the `envs` of
one sandbox command and stops there, and deliberately not into a dotfile on the
sandbox either, because that disk survives the sleep this app relies on.

## What is not gated here

The consent gate in `runtime/mod.rs` fires on a `click` or `type` in the
*browser*, because that is where an action is addressed to a domain and can be
matched against a session. A screenshot carries no URL, so `use_screen` is not
gated: an agent that reads its own screen and then clicks a button on a page the
operator signed in to on that screen is not stopped. That was true before this
split and is true after it. Wording is the only thing holding that path, and it
is listed under *Known limitations* in `ARCHITECTURE.md`.
