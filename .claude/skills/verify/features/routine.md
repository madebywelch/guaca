# A routine fired by an event

A person adds a routine that waits on an event. A post with the routine's
secret fires it once, in a run of its own, with the post's body; a retry of
the same delivery does not fire it again. A post without the secret, or to an
event nobody waits on, fires nothing.

## Sub-features

- `create` adds a routine from the agent's Routines tab.
- `address` shows the curl command with the address and secret.
- `fire` delivers a post to the agent with its body.
- `dedupe` ignores a second post with the same `Idempotency-Key`.
- `refuse` answers 401 without the secret and 404 for an unknown event.
- `history` lists the firing under Run history.

## How to get to it (user POV)

- An agent's page, button `Routines`, button `Add a routine`.

## Driving it with scripts/verify.py

Preconditions: `first_run` has finished.

- **Create.** Fill `Name` and `Instruction`, choose `When an event is posted` in
  combobox `Trigger`, fill `Service` and `Event`, press `Create routine`.
  `routines` has `fires = event:billing/invoice.failed`.
- **Address.** Read `curl -X POST <address> ... Authorization: Bearer <secret>`
  from the page and add the secret to `w.secrets` before any capture.
- **Refuse.** Post with no secret, a wrong one, and to `invoice.paid`:
  401, 401, 404, and `routine_runs` stays empty.
- **Fire.** Post with the secret and `Idempotency-Key: delivery-1`. The answer
  says `delivered: 1`, the agent's reply names the invoice from the body, and
  the outcome's trigger is `event`.
- **Dedupe.** Post the same again: `delivered: 0`, still one `routine_runs` row.
- **History.** Open the routine's row (its name starts `Invoice watch When`);
  Run history no longer says `No runs yet.`

## Gotchas

- The routine panel shows the secret. Switch to `Conversation` before a
  screenshot; a capture with a secret on screen is withheld.
- The row's name carries a relative time (`last ran 1m`); match its prefix.
- A routine on the clock (`Once`, `Every hour`) is swept every twenty seconds
  of real time, which is why this feature is proven on an event.
