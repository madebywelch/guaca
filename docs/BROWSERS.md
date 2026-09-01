# Browsers

An agent can be given a browser: a Kernel session, which is a hosted Chrome and
nothing else. `kernel.rs` starts and ends them, `cdp.rs` drives them over the
DevTools protocol, and the operator watches through Kernel's own live view in an
iframe.

A *browser* is not a *computer*. `MACHINES.md` is the other one: a Linux
machine with a shell, a desktop and a screen, worked by looking at pixels and
aiming a pointer. Both may be configured, either may be, or neither.

## A browser is given to one agent, and separately from the computer

The same rule as the machine's, and the reason it is a second column rather
than one switch over both: a crew where one agent reads the web and nobody else
leaves the workspace is the ordinary shape, not a special case. A Kernel key
says the workspace can hand a browser out; `agents.has_browser` says who was
given one. `Surfaces::given_to` is where the two meet.

*A computer is given to one agent, not to the workspace* in `MACHINES.md` has
the argument in full. What is different here is what taking one back does: the
browser is closed rather than slept, because closing is what writes the cookies
back to the agent's profile. The profile outlives every browser made against it
and is deleted with the agent, so a browser given back opens signed in to the
same accounts. Nothing is lost by taking one back, which is why it does not ask
twice.

The gate is in `ensure_browser`, the only function that makes a browser, and a
model that calls `browse` without one is refused by `Runtime::not_given` before
anything is opened.

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

## What an agent holds is read from its row, not from its turn

A turn carries one `AgentCard`, read once in `run_turn` and passed through every
round. That is right for everything the operator decides and wrong for the one
thing a turn changes about itself: the browser made by the first `browse` is
written to the row and not to that snapshot. Read from the snapshot, the second
`browse` of a turn sees an agent holding nothing and asks for another browser,
under a name only one live browser may have. Kernel answers 409, and it answers
it to every page the agent tries to open for the rest of the turn.

So `ensure_browser` reads `browser_id` from the store. `Runtime::held` is the
whole of it, and `ensure_computer` reads the sandbox the same way for the same
reason, where the provider does not refuse the second one and simply bills for
it.

A conflict is still possible without that mistake, because the row can be lost
while the browser is up: a crash between creating one and writing it down leaves
a browser running, holding the agent's name, with nothing in the app pointing at
it. `KernelClient::create` treats the 409 as the answer it wanted and adopts
that browser, found by its `guac-agent` tag rather than by the name the conflict
was about, and asked for by id so the reply carries a socket. The alternative is
a `browse` tool that refuses every call until the orphan times out.

`tests/machines.rs` drives both of these against a scripted control plane: a
provider that refuses a duplicate name is the only thing that can tell one
browser per agent from two.

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

## A dialog is answered, because nobody else is there to answer it

`alert`, `confirm`, `prompt` and the *Leave site?* box block the renderer.
Nothing on that page runs while one is up, so the call that raised it waits out
the 30-second call timeout and fails, and so does every action after it: the box
outlives the socket that caused it, and the connection-per-action design means
there is no client sitting there to notice. An unanswered dialog is not a slow
action. It is a browser that has stopped until the session times out, and what
reached the agent was `the browser took too long to answer` on every page it
tried afterward.

So `Page.enable` is called on attach, and `Page::call` answers
`Page.javascriptDialogOpening` out of its own read loop. It has to be there
rather than after the action, because the call that raised the dialog is exactly
the call that will not return.

Three of the four are answered yes. `beforeunload` asks whether the navigation
the agent just requested was meant; `confirm` asks whether the button it just
pressed was meant, and that press has already been through the consent gate;
`alert` has no other answer. `prompt` is declined, because it is the one that
wants a value rather than a decision: yes submits the empty string as though the
agent had typed it, where no is the answer a page is written to expect from
somebody with nothing to say.

Neither answer is silent. A dialog is a decision taken on the agent's behalf
between the action it asked for and the page it reads afterward, and the page
alone does not record that one happened: an agent that meant to leave a form
sees the page it wanted, and an agent whose `prompt` was declined sees a page
where nothing changed and no reason why. `note_dialogs` puts what was answered
into the description, and `render_page` prints it under the same untrusted label
as the rest of the page, because the wording is the site's.

What this does not reach is a dialog raised while nothing is driving the
browser: a `setTimeout` that fires between turns is handed to Chrome's own
manager before this client attaches, and there is no event left to answer. That
still arrives as a timeout.

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

## Whether the gate fires at all is a decision about the agent

`Consent` sits on the card next to `has_browser`, and it is `open`. An agent
with an open browser is never stopped: what the browser is signed in to is what
the operator handed over when they gave it the browser, and the agent spending
it is the arrangement rather than a surprise.

`askBeforeActing` is the other answer, taken per agent, from the switch under
the browser pane. It is per agent because per site is a question with no useful
answer. The interesting instruction an operator holds is about accounts and
intent, not domains: *post on the company LinkedIn, never on my own* is one
site, two accounts and a rule the gate cannot see. The model can hold that,
told plainly. What the operator decides here is which agents they trust to
follow it, which is the same decision they took when they handed one a browser.

The default is open for a reason with a measurement behind it. An agent doing
research presses something on a search engine every few seconds, leaves the
domain to read a result, and comes back: the grant below is taken back on every
cycle, so the gate asked again on every cycle. A question in that shape is not a
stricter gate. It is a gate the operator has learned to click through, which
costs the one thing the whole mechanism buys.

That also fixes what the second detection layer in `signin::detect` costs here.
A domain the browser has visited, holding a durable `httpOnly` cookie whose name
looks like an identity, is reported as a sign-in: that is the right hedge for a
roster, and it means an ordinary search engine can be a "session" the gate
fires on. With the gate off by default, a hedged guess no longer buys the
operator a dialog.

## The consent gate reads the browser's sessions, not the agent's

Once an operator has asked to be asked, `needs_consent` fires on a `click` or
`type` in the browser, after this turn has taken in a page or a screen, on a
site the agent holds a session for. The
sessions consulted are the *browser's*, not the union of both surfaces. The URL
the rule is decided from came from the browser, so a session the computer holds
is not the thing that action could spend: gating on it would stop and ask about
an account the action cannot touch, which teaches an operator to click through
the prompt without reading it.

A yes is remembered against that site until the turn ends or the browser reads
something off another one, so an agent working through an inbox is asked once
rather than once per press. It is held on the turn's `Reading` and never
written down: the next turn asks again, and so does the first press after the
agent has been anywhere else.

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

## Two hosts, and a runtime check because a test cannot see the provider

Kernel serves a live view from `kernel.sh` today and served it from
`onkernel.com` before that. Both are in the CSP and in `LIVE_VIEW_HOSTS`, which
is one list rather than two: the test above builds the entries it looks for out
of that constant, so the config and the runtime cannot drift apart.

The old host stays because which one an account is issued is the provider's
decision and not one it announces. An entry for a host nobody is issued costs a
line in an allowlist. A missing one costs the whole pane, silently: an iframe
the CSP refuses draws the surface behind it and reports nothing, so a browser
that is running, signed in and working looks exactly like one that failed to
start. Black full screen, gray in the panel, no error anywhere.

That is why `framable` asks the same question of the URL that actually arrived,
and `Browser::running` hands the pane the origin instead of the URL when the
answer is no. The pane then says which address it could not show, and says that
the agent can still use the web, which is true: only watching is refused. The
test can only prove the config agrees with what this build believes, and what
moved the last two times was what the provider sends.

The frame is given `allow="autoplay; clipboard-read; clipboard-write"`. Signing
in means pasting a password out of a manager, and without it the paste silently
does nothing, which reads as a password manager that will not fill.

## Stealth is the operator's switch and defaults off

On, sites that block automation are far more likely to let an agent through and
the provider solves the captchas. It also costs more and needs a plan that
includes it, so switching it on for everyone would make the first browser fail
to start on accounts that do not have it. Off by default, one checkbox in
settings, and the reason is in the hint beside it.
