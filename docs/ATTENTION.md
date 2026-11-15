# Attention

Guaca runs unattended. A choice found during an email check must still be
answerable when the operator returns, without keeping the agent's turn alive.
`decision` records that choice in SQLite. **For you** is its permanent destination
in the rail, shared by every group.

## Three tiers over one queue

The rail's **For you** entry counts unanswered decisions, interrupted
follow-through, live approval requests and operational blockers. Snoozing a
decision does not remove it from the count. Opening the entry shows three views:

- **Needs you:** live permissions first, operational blockers, then decisions
  ordered by deadline and age. Snoozed decisions remain under **Set aside**.
- **Following through:** answers delivered to agents without completion receipts.
- **History:** the latest 100 completed or withdrawn decisions. A receipt carries
  the original question, answer and outcome; it is the decision record itself,
  not a second copy of the transcript.

`ForYou.tsx` owns the surface. `Desk.tsx` renders the existing permission and
blocker cards inside it. The bottom-right floating desk is gone. New arrivals
update the count without taking keyboard focus. The dialog traps focus and
returns it to the control that opened it.

Group circles still report their agents' activity and parked requests. The menu
bar adds the durable decision count and opens **For you** when the window is
shut. Operating-system notifications are reminders to visit the destination;
reading or dismissing one never settles a decision.

## What earns a place on the desk

The old desk held only stopped work. For you also holds work that can wait for
a preference: an agent can file a meeting-time choice and finish the rest of an
email sweep. Ordinary completions, routine firings and chat updates do not
become decision cards.

They are not lost either. Every finished run leaves one outcome (*A run ends in
an outcome* in docs/ARCHITECTURE.md), and **Finished**, under the requests in
Needs you, lists the ones the operator has not seen: one line each, newest
first, under a heading per group when the lines come from more than one. A line
is the agent's face and name, the first line of the closing note, a word when
the run ended some other way than finishing (failed, interrupted, stopped,
needs you) and its age. Clicking it opens that note in its channel and marks
the run seen, as opening the channel any other way does. A run with no note, or
whose note was cleared, opens the channel at its end. **Mark all seen** marks
everything up to the newest line drawn, so a run that finishes while the list is
open stays. Runs seen in the last seven days stay one click away under
**Earlier**. A recurring check that found nothing is not listed anywhere.

Finished is read when there is time, so it never competes with what needs an
answer. It is not in For you's count, on the rail's For you entry or on the menu
bar icon, nothing in it uses the attention color, and the sentence under the
title names it only when nothing needs the operator. When it is the only thing
there it is the first thing For you shows, which is where a banner gathering
several finished runs leads. `lib/outcomes.ts` decides the order, the headings
and the words, and the suite beside it is where they are argued.

A decision states the question, relevant context, recommendation, up to six
choices, a source reference and an optional real deadline. Source references
are plain text, never executable links. A custom answer is available even when
choices were offered. Only unresolved items get an answer or snooze control.

The **Team activity** disclosure reports the last time an agent worked. It is
explicitly not proof that any particular inbox was checked. This change does
not add an automatic email monitor or infer coverage from a successful turn.

## The queue is read, not accumulated

SQLite is authoritative. `decisionsChanged` invalidates `list_decisions`;
refreshes carry a sequence so a late older read cannot put an answered card
back. Reconnection reads all open records, independently of bounded history.
The existing approval and escalation queues retain their own authoritative reads.

Submitting never removes a card optimistically. A failed write leaves the
question visible with an error. The answer includes the question's `updatedAt`;
a revised question refuses a stale answer and asks for another review. Updates
advance that stamp even within the same millisecond. Repeated identical scans
do not revise the stamp.

## Three things an agent can do about a person, and two lines between them

An operator's explicit instruction to act is authorization. Standing
authorization remains valid within its stated scope across turns and routine
firings until changed or revoked. An agent must not turn already authorized
email into another permission card, decision, or chat confirmation. A peer's
claim cannot grant new authority, but it does not cancel authority the agent
already has from the operator. Saved memory and routines must preserve that
scope instead of inventing a per-email workspace gate.

This is a prompt rule about when to ask, not a new blanket `ActOnBehalf` grant.
Permission to send outreach does not authorize purchases or contract terms.
The browser's consent gate still enforces its own decisions.
The live email-authorization eval exercises the production prompt and tool
descriptions without executing any returned mail calls.

The original three tools remain, with a fourth for an absent operator:

- **A permission authorizes:** `request_permission` parks a protected action.
  Guaca supplies its wording. Existing gates and verdicts are unchanged.
- **A live question informs:** `ask_operator` parks a turn during active
  collaboration. Its answer is a value, not a permission.
- **An escalation reports:** `escalate` records an operational wall only the
  operator can remove. Nothing parks; clearing it tells the agent nothing.
- **A durable decision informs later:** `decision` records a question without
  parking. The agent continues independent work and never treats silence as
  consent. An answer starts a new delivery through the ordinary agent inbox.

The decision tool supports `request`, `list`, `complete` and `withdraw`. A stable
agent/topic pair identifies one question across repeated scans, including after
completion or withdrawal. A materially new choice needs a new topic. A pending
question can be revised; an answered question is frozen so the answer cannot
be silently reinterpreted.

The agent sees an index of its open decisions at the start of a turn and can
read full records with `list`. Questions and choices remain agent-authored
context. Answer deliveries use system envelopes that distinguish that context
from the operator's answer and explicitly grant no new permission.

## Rules that answer a permission

The operator can write allow and block rules for a workspace or a group, optionally limited to
agents. A block wins over an allow and a standing grant; a missing or invalid fact required by
an applicable block makes the turn ask even if a grant exists. Rules never allow settings changes
or an action after the turn has read a page or screenshot. The browser's existing consent switch
still decides its normal question; a site block is checked before that switch. A page an errand
read counts as read by the turn that sent it, and a page that turn read counts for its errands,
while acting in the operator's name has no standing yes at all: an "always" for it is recorded as
a single allow, and one an older build recorded is never consulted.

`request_permission` supplies the facts for rules about acting in the operator's name. These are
the agent's declarations, not a guarantee about the later action. Connector calls are not gated
by these rules and do not start asking because a rule exists. Only the operator can edit rules.
An automatic allowance appears in the transcript with the rule that allowed it; a blocked call
is a refusal, not a pending request.

## A settings change is a permission with its own diff

`settings` with `update` parks the turn on `ProtectedAction::ChangeSettings`.
The card lists each setting an agent asked to change, before and after, in the
words the settings pane uses (`config::LIMIT_LABELS`, checked against
`limits.ts` by `manual.test.ts`). Nothing changes until the operator allows it,
and the patch is applied to the settings as they are at that moment, so an edit
another window saved while the card was open is not undone.

It is one of two protected actions with no standing yes; acting in the
operator's name is the other. "Always" is scoped to an agent and an action, and
this action is any setting to any value: a standing
yes given for a timeout would cover a relay depth raised to its ceiling, and
the limits are the guard that ends a cascade. `ProtectedAction::stands` says
so, `decide_approval` records an "always" as a single allow, and
`ask_permission` never consults a standing grant for it.

What an agent may ask for at all is `config::AGENT_WRITABLE`. The provider, the
endpoint and every key are `OPERATOR_ONLY` and refused before anyone is asked:
the endpoint decides where the API key is sent, so an agent that could move it,
even with a click of approval, could send the key anywhere.

## An answer is not completion

`pending → answered → completed` is the normal lifecycle. An obsolete pending
question can instead become `withdrawn`, with a reason. Neither terminal state
erases history. Only the owning agent can record completion or withdrawal.

Answer acceptance, the transcript envelope and `pending_runs` recovery point
commit in one immediate transaction. The runtime books the run under its run
lock before enqueueing. Duplicate clicks have one winner. Answers arriving
during another turn use ordinary inbox intake and can be read at its next round.

The envelope carries a `decision` part, not text. The agent reads what
`decision::delivery` builds from it: a paragraph on what an answer permits and
how to finish, then the request and the answer as JSON. The transcript draws
the operator's answer as their own reply, quoting the question it answers, with
the context, recommendation and source behind the question. It was a text part
first, and every channel drew the paragraph and the JSON as a bubble from Guaca;
migration 61 converts those rows, and the paragraph the agent reads is the same
either way.

A live question is the other kind and leaves the other shape. `ask_operator`
parks the turn, so the question is a part in a channel and its answer is a
column on the `approvals` row, and the card that asked it reads that column back
through `approval_answers` — the same bounded window as `approval_states` — and
draws it under the sentence saying it was answered. That read is for the page
only. What the parked turn is handed when it resumes is the tool result
`runtime::put_to_operator` builds, and the card reading the record neither
changes it nor gives it a second copy to read.

Completion requires a separate tool call with a concrete outcome after the work
is done. It cannot be inferred from a turn ending. Outstanding answered records
remain visible and continue to receive review reminders.

On restart, an answered record becomes interrupted only when the run delivering
its answer was still unsettled, so follow-through that had finished can still be
completed afterward. The operator's **Resume follow-through** re-delivers the
saved answer with instructions to inspect prior actions before continuing. Try
again on that delivery goes through the same Resume and records the new run as
the delivery, so the answer is not delivered twice. Nothing silently replays an
external effect. Pending questions survive without a time limit; completed
receipts remain completed.

## Reminders

The Rust scheduler claims one reminder batch in an immediate transaction. The
same commit moves each decision to its next reminder and writes one outbox row
for the batch, so a batch is never claimed without something left to announce
it. Default cadence is 9 AM and 4 PM in the host's time zone, plus one hour
before a known deadline. A newly filed decision due within an hour is
immediately eligible. A decision past its deadline stays answerable and joins
the next briefing rather than disappearing.

The row says how many decisions need a review and nothing an agent wrote. It
waits in the outbox until a window claims it, so a batch due while every window
is closed, including one due the moment the host boots, is shown when a window
next connects rather than spent on nobody. The one-hour snooze changes the next
reminder, survives restart and is preserved across rescans.
`decisionReminder` is still emitted after the claim, for windows that raise
their own banner from it; it stops mattering once windows draw deliveries.
A reminder offered while the operator is away is also pushed to the address in
`delivery`, if they set one (below). Configurable briefing times are not
implemented here.

## What interrupts the operator

Every interruption is decided on the host, once, by `domain::outbox::decide`.
A producer writes an outbox row in the commit of the thing it is about: an
outcome (`outcomes::record`, and `record_interrupted` after a restart), a
permission request or question (`create_approval`), an escalation (first raise
and at most once a day after), and a reminder batch. Each row has a dedup key,
`outcome:<run>`, `approval:<id>`, `escalation:<id>:<day>` or
`reminder:<remind_at>`, so settling twice or raising twice in a day writes
nothing more.

The dispatcher (`runtime/outbox.rs`) starts at boot after the pickup. It wakes
when a commit writes a row and when a held row comes due, and decides each row
from its kind, the run's ending, the operator's three switches and whether
anybody is there:

| Row | Away | Here, elsewhere | Here, its channel on screen |
|---|---|---|---|
| Permission request, question, stuck agent | Deliver | Deliver | Suppress |
| Decision reminder | Deliver | Deliver | Deliver |
| Run failed or interrupted | Deliver | Deliver | Suppress |
| Run done, or done leaving something open | Gather for 10 minutes, then deliver as one item | Hold until they would count as away: gather if they left, suppress if still here | Suppress |
| Run stopped by the operator, or a check that found nothing | No row is written | No row is written | No row is written |

**Away** means no window has had focus for two minutes. A window reports focus
when it gets it and while it keeps it, and reports when it loses it; the host
keeps the last report of each window in memory and never shows it to an agent.
**On screen** means the window with focus shows that agent's channel with
nothing open over it. A switch that is off suppresses its kind whatever the
presence: `needsYou` covers requests, stuck agents and reminders, `failures`
covers failed and interrupted runs, and `finished` covers finished work.

The switches live in the host's settings as `attention`, one setting for every
window. Only the operator can change them: they are not in
`config::AGENT_WRITABLE`, and an agent's settings read does not include them.

A delivered row is offered, not sent. Every window hears `delivery`; one that
connects later asks `pendingDeliveries`. A window shows the banner only when
`claimDelivery` answers true, which it does for exactly one window. Gathered
work is one item, titled with how many finished and listing a line for each,
claimed with every row it carries. Answering a request, clearing an
escalation, or seeing a run withdraws whatever was waiting to announce it, in
the same commit. An item nobody claims in 24 hours expires; what it said is
still in For you and in its channel.

**A claim can be given back.** A window claims before it shows, because showing
first would raise one banner per open window. On a Mac it learns only after
the claim whether macOS took the banner, so a window that claimed and was
refused calls `releaseDelivery`. Each claim names the page that made it
(`client`, the page's `PAGE_ID`, stored as `claimed_by`), and only that page
can give it back; a release from any other page, or from a page that already
gave it back, changes nothing and answers false. A page that names nothing,
which is an older build, can claim and can never give back.

| Release | What happens |
|---|---|
| First | The item is pending again and offered again to every window. The page that gave it back is never offered it again (`released_by`): `pendingDeliveries` leaves it out and its claim answers false, after a restart too. The other windows have 15 seconds (`RECLAIM_MS`). |
| Second, by another page that claimed it in those 15 seconds or later | No window is offered it again. It goes to the push at once. |
| Third | There is none: no page can hold a claim to give back. |

If no other window takes it in the 15 seconds, or a second page gives it back,
the dispatcher treats it as if nobody were at a window: with an address set it
writes the push, whatever presence says, since the one thing that would have
reached the operator at their window has failed. The push row has the window
row's dedup key, so an item is pushed at most once: a push already written,
sent or failed stays as it is, and one dropped because the operator came back
is due again. With no address nothing is written. Either way the dispatcher
does not look at the row again; it stays pending, claimable by a window that
never gave it back, and expires after its 24 hours like any offer nobody took.
Everything here is in SQLite (migration 78: `claimed_by`, `released_by`,
`releases`), so a restart repeats neither the offer nor the push.

A page that may not notify never claims (`willNotify` is asked first), so it
has nothing to give back, and the Settings test banner is no delivery and
gives back nothing. The menu bar panel never claims at all.

## Reaching the operator with every window closed

A window that is closed claims nothing, so an offer alone reaches nobody while
the operator is away. For that the Host can send to an address the operator
sets in Settings → Notifications ("Send to your phone or another service"): an
ntfy topic, which the free ntfy app shows on a phone, or any receiver that takes
a JSON POST, such as Home Assistant, n8n or a chat bridge. Nothing of ours is
involved, and no account.

**What is pushed.** Exactly what is offered while the operator is away, what
every window that tried gave back (see *A claim can be given back* above), and
nothing else. When the dispatcher offers an item and `presence` is away, and
`delivery.url` is set, the same commit writes a second outbox row with
destination `endpoint`. So gathered finished work is one push per gathering, a
request or a stuck agent is one push at once, and nothing is pushed while a
Guaca window has focus. A push waiting to be tried again is dropped when the
operator comes back (`operator present`), unless the windows gave back what it
is about, when the address is removed
(`turned off`), and when what it is about is answered or seen, in the commit
that answered or saw it, like the window's row.

**What it says.** Decided when the row is written, by `push::say`, from the
`content` setting:

| Row | Title | Body, `outcome` | Body, `generic` |
|---|---|---|---|
| Finished run | The agent's name | Its headline | "Something finished" as the title, and no line |
| Gathered finished work | "N finished" | A line per run, as the window shows it | No lines |
| Request, question, stuck agent | "Needs you" | "Pip has a question." and what the window says under it | No name |
| Decision reminder | "Needs you" | How many decisions, which names nothing either way | The same |
| Failed or interrupted run | "Pip could not finish" | Its headline | No name and no line |

A generic push never has the line stored for it, so switching to generic
cannot be undone for a push already written, and that is the point of it.

**How it is sent.** One POST per row, built by `push::request`. The ntfy
format is the body as plain text with `Title`, `Priority` (4 for what
needs the operator and for failures, 3 for finished work), `Tags: guaca`,
`Click` when `linkBase` is set, and `Authorization: Bearer <token>` when a
token is set; a title that is not ASCII is sent as an RFC 2047 encoded word,
which ntfy reads. The JSON format is `{ id, kind, title, body, group, agent,
status, createdAt, link }` with the same `Authorization`, where `agent` and
`group` are null for gathered work and for generic content. Both carry
`X-Guaca-Delivery: <row id>` on every attempt: delivery is at least once,
because an answer lost after the receiver took a push gets it sent again, and
that header is how a receiver drops the repeat. Redirects are not followed,
since following one would send the token wherever it points.

**Trying again.** `push::next` decides. A 2xx is sent. A 3xx, or a 4xx other
than 408 and 429, fails at once: the same request gets the same answer. A
429 waits for its `Retry-After` in seconds. Anything else, a 5xx, a 408 or no
answer within ten seconds, is tried again after 30 seconds, 2 minutes, 10
minutes, an hour, and hourly after that, until a retry would land more than a
day after the row was written, and then it fails. The rows are in SQLite, so a
restart between attempts resumes them.

**What the operator sees.** The pane draws `pushReport`: "Last sent today 9:02
AM", or "Last attempt failed: 401 Unauthorized. Check the token.", with when it
is tried again if it will be. "Send a test" (`testPush`) sends one push to the
saved address at once, whoever is at a window, writes no row, and says what
came back. Every failure is worded from the status code alone (`push::refused`
and `push::unreachable`), never from the error the HTTP client formats, which
names the address.

**The address and the token are secrets.** A topic on the public ntfy.sh is
readable by anyone who knows it. Both are stored in the Host's settings beside
the API key and reach the webview only as `urlSet` and `urlHint` (scheme,
host, and the last four characters of the rest) and `tokenSet` and
`tokenHint`. They are in no event, no log line and no error, and
`tests/outbox_endpoint.rs` checks all of those, plus everything the model is
sent, with a secret in both. No agent can read or set them: `delivery` is not
in `config::AGENT_WRITABLE`, so `settings` refuses it, and the settings an
agent reads list what they include by name. It is not added to
`OPERATOR_ONLY`, for the reason `attention` is not: that would change the
sentence an agent is refused with.

The pane says so: on ntfy.sh, pick a topic nobody could guess, and better,
reserve it and send with an access token, or run your own ntfy server. A
server of your own reaches an iPhone instantly only by forwarding a wake-up
through ntfy.sh (its `upstream-base-url` setting), and that wake-up carries
the message id and a hash of the topic, never the text: the app then fetches
the message from your server (ntfy's docs, *iOS instant notifications*).

## What an escalation is, and what it deliberately is not

An escalation is an operational blocker, not an unanswered preference. Nothing
parks and nothing expires. One row per agent preserves `raised_at` and increments
`times` when the same agent raises again. The prompt shows its own open
escalation so the agent knows it already reported the wall. The report is historical:
an uncleared row is not evidence that a tool is still unavailable. Current tool
definitions and call results determine whether the work can continue.

**Open channel** is the primary action because that is where the operator can
unblock the agent. **Clear** only removes the report. The agent cannot withdraw
its escalation. Deleted agents lose their escalation; decision rows are hidden
while their owner is deleted and become available when restored.

## What bounds the asking

A parked turn can ask one question at a time and remains subject to the existing
run limits. Durable decisions are capped at 100 open records per agent. Request
fields and answers have explicit length bounds. All open records are read;
history is bounded separately so old receipts cannot evict live work.

## The ten minutes

A parked permission or live question holds its turn for ten minutes
(`APPROVAL_WINDOW`). When nobody answers in that time the request is not lost:
`Store::lapse_approval` expires the card and files the same request as a
decision in one commit, so there is no moment when it is in neither place or
answerable in both. The question is the card's own: Guaca's sentence for a
permission, with its details as the context, and Allow and Deny as the choices;
the agent's question, with its own choices, for `ask_operator`. The decision
records where it came from (`origin` and `origin_ref`, migration 74), and the
run that asked is the run that filed it, so that run's outcome is "needs you"
and the outbox treats it as any other filed decision: reminders, and the
interruption policy. The turn is told the request is on For you and carries on
without it. Whenever the operator answers, the answer reaches the agent as a new
message, as every decision's does.

Four kinds lapse this way: `request_permission`, `ask_operator`, `create_agent`
and `settings`. Browser consent does not: that press belonged to a page the
turn has since left, so it expires as it always did, and so does a page pinned
to the status bar. A stop or a restart that closes a request first is a
different fact from a lapse. Nothing is filed, and the turn, or the turn the
next host picks up, is told the restart ended it and to ask again.

An Allow on a decision filed from a lapsed permission is a one-time grant. The
row is written ungranted when the request lapses, bound to the agent and to a
fingerprint of exactly what was asked (`approval::fingerprint`): for acting in
the operator's name, the action, the tool, the recipients, the domain and the
amount; for a new agent, its name, instructions, skills and starting memory;
for a settings change, each setting with its before and after, so the grant
stops matching once any of those settings has changed. The Allow sets it
granted for 24 hours, and the agent is told so after the answer. The next
request from that agent with the same fingerprint is let through without
parking, once (`Store::take_grant` spends it in one statement). It is read
after the rules, so a block rule wins over it, and it is not read when a block
might apply and its fact is missing, for the same reason a standing grant is
not. A Deny, and an answer to a question, grant nothing.

The window is a constant in the product. `Runtime::set_approval_window` exists
so the cascade suite can watch a lapse happen in a second; nothing else calls
it. The bound never applies to a decision an agent files itself, which is still
the way to put something to an operator who is not there.

## Testing

`db/decisions.rs` covers concurrent scans and answers, transaction rollback,
late answers, stale versions, ownership, deduplication after completion,
snoozes and restart recovery. `tests/decisions.rs` drives the real actor and wire
through filing, settling, late answering and follow-through, including an answer
arriving during unrelated work.

`ForYou.test.tsx` checks authoritative refreshes, failed answers, written choices,
interrupted work and snoozed visibility. Menubar tests check the attention count.
The ordinary IPC contract, migration, desktop and server suites cover the seams.

The new live eval asks an assistant to file a meeting-time choice for an absent
operator while finishing an agenda, then checks later completion after answering.
Run `./scripts/evals.sh` after changing the tools or their descriptions. A model
endpoint failure is not evidence that the model followed these instructions.
