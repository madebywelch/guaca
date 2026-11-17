# Guaca verification map

The maintained list of journeys `scripts/verify.py` drives, one file per
feature. Read the index, then the file for the feature you are about to prove.

## Baseline preconditions

- `scripts/verify.py doctor` reports `"ok": true`, or at least `web`,
  `guacad`, `chromium` (and `guacadClock` for `lapse`).
- Every scenario starts from an empty workspace on its own port. Nothing is
  seeded: the first step of each is the onboarding a person sees on a first
  visit, which ends with an agent named Executive Assistant.
- The model is the stand-in on loopback, chosen in Connect AI as API key,
  Custom endpoint, with the endpoint filled before the key.

## Driving conventions

- Find elements by role and accessible name with `w.ui`. Names here are
  exact unless a file says otherwise.
- Every wait is on a row, an element or an event. Never sleep.
- Script the model by what was said to it, never by call count.
- Read rows only through `w.host.rows`, which opens the database read only.

## Proof and failure reporting

- Each feature asserts its failure path beside its success path.
- A step's `observed` value is the same on every run of one commit; ids,
  times and ports go in `detail`.
- A step the product cannot satisfy fails with the names it did show. Never
  weaken the step to pass; write the gap in that feature's Gotchas.

## Features

| Feature | Scenario | File |
|---|---|---|
| First run, writing to an agent, its reply | `reply` | [first-run.md](first-run.md) |
| Agents talking to each other, and the guard ending it | `peers` | [peers.md](peers.md) |
| A permission request in the conversation | `permission` | [permission.md](permission.md) |
| A request that lapses into For you | `lapse` | [for-you.md](for-you.md) |
| Work that survives a Host stopping or crashing | `restart` | [restart.md](restart.md) |
| A routine fired by an event | `routine` | [routine.md](routine.md) |
| The Mac-only shell: window, banners, menu bar | `scripts/verify.py mac` | `docs/PROBE.md` |
