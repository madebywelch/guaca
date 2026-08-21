# Browsers

An agent can be given a browser: a Kernel session, which is a hosted Chrome and
nothing else. `kernel.rs` starts and ends them, `cdp.rs` drives them over the
DevTools protocol, and the operator watches through Kernel's own live view in an
iframe.

A *browser* is not a *computer*. `MACHINES.md` is the other one: a Linux
machine with a shell, a desktop and a screen, worked by looking at pixels and
aiming a pointer. Both may be configured, either may be, or neither.

## Why they are two things

They were one. Chrome's remote debugging port was opened on the E2B machine and
`browse` drove that, which is the exact way to use a page: Chrome knows where
every element is, what it says and what it does, and a screenshot is a guess at
all three. It did not survive contact with a general-purpose desktop.

The port belongs to a profile, and Chrome ignores `--remote-debugging-port`
whenever it re-attaches to a profile that is already open, so any other route
that opened a window took the interface away. That was fixable, and was fixed,
by pinning every route on the machine to one profile with the port on it. What
was not fixable was having two ways to use the web on one screen: an agent that
read the page and an agent that clicked at coordinates disagreed about which
window was in front, and each fix moved the disagreement rather than ending it.

So the machine kept the half it is good at and lost the other. A computer is now
exactly what it looks like: a screen, a pointer and a keyboard, with no
privileged channel into anything for the two views to fall out of step. A
browser is a provider whose whole product is one browser: there is one, it is
the only thing in its sandbox, the socket is handed out at creation, and there
is no second route to that window at all.

## The socket is read at the point of use, never stored

Only `agents.browser_id` is kept. The `cdp_ws_url` and the live view URL both
change when a browser is replaced, and a browser is replaced often: it goes to
standby seconds after the last action and is deleted some minutes later. A
stored socket outlives the browser it addressed, and connecting to a dead one
does not fail, it hangs until the call times out.

`ensure_browser` therefore asks the provider every time and takes the socket
from the answer. A 404 is not an error there: it is the expected end of a
browser, and the answer is to make another.

## A session outlives the browser it was made in, because of the profile

Each agent gets one Kernel profile, named from its id, and every browser it is
given is created against that profile with `save_changes`. Cookies are written
back when the browser is deleted or times out, so the next one opens signed in
to the same accounts. That is the same property the machine gets from its disk
surviving a sleep, reached a different way.

Two consequences that are not obvious:

- **Closing is what saves.** A browser left to time out saves too, on the
  provider's clock. The Close button in the pane exists so an operator who has
  just signed an agent in can make it durable now rather than in an hour.
- **One writer at a time**, which the provider does not enforce. Two browsers
  open on one profile means the last one closed overwrites the other's cookies.
  So an agent holds at most one browser, recorded on its row, and `ensure_browser`
  never makes a second while the first is alive.

The profile is deleted with the agent. A name is free to reuse the moment an
agent is deleted, and the profile is named from the agent, so leaving it behind
would hand the next agent of that name somebody else's sessions.

## The element numbering lives on the page

`read` returns the page's text and a numbered list of what can be used, and
`click` and `type` take one of those numbers. The numbering is stored in
`window.__guacEls` on the page itself, which is what makes a click refer to the
element the model was shown rather than to whatever now sits at some position.

A page that has re-rendered has forgotten it, and `require` refuses in that case
rather than acting on whatever now holds that number. The refusal says to read
again, because a refusal that only says no gets reworded and retried.

Typing goes in through `Input.insertText` after the element is focused and its
contents selected. Setting `.value` is what this used to do and it quietly fails
on every framework that keeps its own copy of the value: the property changed,
React's copy did not, and the next render put the old text back.

## Two scopes on one socket

The address a provider hands out is the *browser*, not a page. `Target.*` and
`Storage.getCookies` are sent as they are; everything that acts on a page has to
carry the `sessionId` of an attached target. Getting that wrong is not an error:
`Runtime.evaluate` without a session evaluates in the browser's own context,
where there is no `document`, and the reply is a well-formed `undefined`.

`Target.attachToTarget` is called with `flatten: true`. Without it the reply is a
session that can only be spoken to through `Target.sendMessageToTarget`, which
wraps every call in a string and every reply in an event.

## A connection per action

Each action opens a socket, does one thing and closes it. It costs a handshake
and buys statelessness: turns can be minutes apart, the browser sleeps in
between, and a socket held across that is dead in a way nothing notices until an
action hangs on it. The only thing that has to persist across actions is the
element numbering, and that is on the page.

## An action is done once, and the page it produced is read until it answers

A navigation can replace the target a session is attached to, and Chrome then
fails the *wait* and the *description* rather than the navigation that caused
them. An `open` that redirects is enough to do it. The wording that came back
was Chrome's own, `Inspected target navigated or closed`, on a page that had
loaded perfectly well, and it reads to an agent like a browser that cannot see
the web: one met it, abandoned `browse` for the rest of the run, and went to
work the same pages by screenshot on its computer instead. `CdpError::TargetGone`
exists to keep that wording away from a model and to say what to do instead.

So `browse` attaches again and reads again. It never acts again. That asymmetry
is the whole design: the action has already happened by the time the target can
go missing, and a click sent a second time because the answer went astray is the
one mistake a browser tool must not make. `settle_and_collect` is one function
for exactly this reason, and it runs at most twice.

## Sign-ins, and the one thing that is weaker here

Detection is the same rule as the machine's, in `domain/signin.rs`, because "a
cookie's presence is not a login" is a fact about the web rather than about
where a browser runs. Cookie names and flags come from `Storage.getCookies`, and
`CookieMark` has no field a value could arrive in.

The second layer of that rule needs to know a site was *visited*, which is what
separates a site somebody uses from an ad network that sets cookies from inside
someone else's page. On a machine that comes from Chrome's own history file. A
hosted browser has no such file to read, so it comes from the current tab's
navigation history instead: less than a history, and honest about being less.
What it costs is a few second-layer guesses, which was the hedged layer already.
It cannot produce a false claim, only fewer true ones.

A sign-in is recorded against the surface it was found on, and the two are
replaced independently. Without that, asking the computer what it holds would
erase everything the browser reported, and an agent's accounts would flicker
between two halves of the truth depending on which scan ran last.

## The consent gate reads the browser's sessions, not the agent's

`needs_consent` fires on a `click` or `type` in the browser, after this turn has
taken in a page or a screen, on a site the agent holds a session for. The
sessions consulted are the *browser's*, not the union of both surfaces. The URL
the rule is decided from came from the browser, so a session the computer holds
is not the thing that action could spend: gating on it would stop and ask about
an account the action cannot touch, which teaches an operator to click through
the prompt without reading it.

`ARCHITECTURE.md`, *A page that was read this turn cannot quietly press a
button*, is the rest of it.

## The live view is framed directly, and the computer's viewer is not

An E2B sandbox refuses traffic without a header and an iframe cannot set one, so
`proxy.rs` relays the desktop and attaches the token on the way out. A Kernel
live view URL carries its own token in the path, scoped to one browser session,
and dies with that browser. There is nothing to attach, so it goes into the
iframe as it is.

Two CSP entries, not one. The frame loads over HTTPS on port 8443 and then opens
a WebSocket of its own for the pixels, so `frame-src` without a matching
`connect-src` is a frame that loads and never paints. `wss://` has to be there
too. A test in `kernel.rs` reads `tauri.conf.json` and asserts all three,
because every check at the HTTP layer passes when the CSP is wrong: curl does
not enforce CSP, and the pane simply stays blank.

The frame is given `allow="autoplay; clipboard-read; clipboard-write"`. Signing
in means pasting a password out of a manager, and without it the paste silently
does nothing, which reads as a password manager that will not fill.

## Stealth is the operator's switch and defaults off

On, sites that block automation are far more likely to let an agent through and
the provider solves the captchas. It also costs more and needs a plan that
includes it, so switching it on for everyone would make the first browser fail
to start on accounts that do not have it. Off by default, one checkbox in
settings, and the reason is in the hint beside it.
