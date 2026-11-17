# Work that survives a Host restart

A Host stopped while a turn is waiting on the model puts the conversation down
and the next Host delivers it again, so the person gets the reply once. A Host
that crashes mid-turn instead marks the turn interrupted, repeats nothing on
its own, and offers Try again.

## Sub-features

- `drain` puts a conversation down on SIGTERM and exits 0.
- `pickup` delivers the put-down message again on the next start.
- `crash` reports an interrupted turn with a Try again button.
- `retry` runs the interrupted turn once more.

## How to get to it (user POV)

- An update or a service restart stops the Host while an agent is working.
- After a crash, the agent's conversation shows the interruption and `Try again`.

## Driving it with scripts/verify.py

Preconditions: `first_run` has finished.

- **Stop.** Script the reply with `held(gate, ...)` and wait until the call is
  in the air. `w.host.stop()` returns 0 and `host.log` says
  `putting work down for the next one conversations=1`.
- **Pick up.** Open the gate, `w.host.start()`, `w.open_browser()`,
  `open_agent`. The model sees the same message a second time, the reply is
  shown once and the outcome is `done`.
- **Crash.** Hold another call and `w.host.kill()`. After the next start the
  conversation has button `Try again`, the outcome is `interrupted`, and the
  model has seen the message once.
- **Retry.** Press `Try again`. The model sees it a second time and the reply
  arrives once.

## Gotchas

- After a restart the page opens on Today; use `open_agent`, which presses
  the agent's face on the timeline (`Executive Assistant, ...`).
- The held call's connection is dropped by the stopping Host; the stand-in
  counts it in `w.model.dropped` and that is expected.
