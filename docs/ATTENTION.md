# Attention

Guaca runs unattended. A choice found during an email check must still be
answerable when the operator returns, without keeping the agent's turn alive.
`decision` records that choice in SQLite. **For you** is its permanent destination
in the rail, shared by every crew.

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

Crew circles still report their agents' activity and parked requests. The menu
bar adds the durable decision count and opens **For you** when the window is
shut. Operating-system notifications are reminders to visit the destination;
reading or dismissing one never settles a decision.

## What earns a place on the desk

The old desk held only stopped work. For you also holds work that can wait for
a preference: an agent can file a meeting-time choice and finish the rest of an
email sweep. Ordinary completions, routine firings and chat updates do not
become decision cards.

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
The configured browser and repository gates still enforce their own decisions.
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

## An answer is not completion

`pending → answered → completed` is the normal lifecycle. An obsolete pending
question can instead become `withdrawn`, with a reason. Neither terminal state
erases history. Only the owning agent can record completion or withdrawal.

Answer acceptance, the transcript envelope and `pending_runs` recovery point
commit in one immediate transaction. The runtime books the run under its run
lock before enqueueing. Duplicate clicks have one winner. Answers arriving
during another turn use ordinary inbox intake and can be read at its next round.

Completion requires a separate tool call with a concrete outcome after the work
is done. It cannot be inferred from a turn ending. Outstanding answered records
remain visible and continue to receive review reminders.

On restart, answered records become interrupted. The operator's **Resume
follow-through** re-delivers the saved answer with instructions to inspect prior
actions before continuing. Nothing silently replays an external effect. Pending
questions survive without a time limit; completed receipts remain completed.

## Reminders

The Rust scheduler claims one reminder batch in an immediate transaction and
persists the next reminder before emitting `decisionReminder`. Default cadence
is 9 AM and 4 PM in the runtime host's local timezone, plus one hour before a
known deadline. A newly filed decision due within an hour is immediately
eligible. A decision past its deadline stays answerable and joins the next
briefing rather than disappearing.

The UI sends one generic notification for the batch, without source content.
Notification preferences, launch quiet time and burst suppression still apply.
The one-hour snooze changes the next reminder, survives restart and is preserved
across rescans. A reconnect always restores the rows even if the notification
event was missed. Delivery while every client is closed, remote push channels,
and configurable briefing times are not implemented here.

## What an escalation is, and what it deliberately is not

An escalation is an operational blocker, not an unanswered preference. Nothing
parks and nothing expires. One row per agent preserves `raised_at` and increments
`times` when the same agent raises again. The prompt shows its own open
escalation so the agent knows it already reported the wall.

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

Existing parked permissions and live questions still lapse after ten minutes.
That bound does not apply to durable decisions. Agents should use `decision`
for unattended work instead of relying on a live question reaching someone.

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
