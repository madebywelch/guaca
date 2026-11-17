# A request that lapses into For you

A request nobody answers lapses after the approval window and is filed in For
you as a decision. An Allow there grants the same change once, so the agent's
next identical ask goes through without parking. A Deny grants nothing.

## Sub-features

- `lapse` expires the card and files a pending decision in one commit.
- `count` shows the waiting decision on the For you place.
- `allow-grant` writes a one-time grant that the next identical ask spends.
- `deny-nothing` leaves the grant ungranted; the next ask parks again.

## How to get to it (user POV)

- The `For you, 1 waiting` button in the Places at the foot of the window.

## Driving it with scripts/verify.py

Preconditions: `first_run` has finished, on the clock Host with a one-second
minute (the scenario sets it), so the window is ten seconds.

- **Lapse.** Ask for a setting change and answer nothing. `approvals` turns
  `expired`, `decisions` gains a `pending` row with origin `approval`, the run's
  outcome is `needs_you`, and the card says `It is waiting in For you`.
- **Open.** Press `For you, 1 waiting`. Dialog `For you` lists the request with
  `Allow`, `Deny`, `Write an answer`.
- **Allow.** Press `Allow`. The agent is sent the answer as a message holding
  `"answer":"Allow"`; script it to ask for the same change. The setting changes,
  no second `approvals` row appears, and `permission_grants` shows granted and used.
- **Deny.** Repeat with another value and press `Deny`. The agent's next ask
  parks: a new `pending` row, and the grant row stays ungranted.

## Gotchas

- The window is `APPROVAL_WINDOW` scaled by `probe_clock::scaled`; only a
  `probe-host` build started with `GUACA_PROBE_MINUTE_MS` shortens it.
- The text the person reads still says ten minutes. That is the product's
  sentence, not the run's window.
- A grant row is written ungranted at the lapse, so a Deny leaves one behind.
- Escape closes the For you dialog before writing in the field again.
