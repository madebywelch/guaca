---
name: verify-recovery
description: Model-check how a conversation survives a host crash, a drain for an update, and an operator stop, against the TLA+ model in formal/recovery. Use when a change touches pending_runs, start_deliveries, take_in, put_down, Store::recover, Runtime::drain or pick_up, stop_run, settle_run_as, or finish_turn, or when asked whether a message can be lost or repeated across a restart.
---

# Verify conversation recovery

Tests check the orderings someone wrote down. This check covers every ordering of
the steps below, with up to two host processes ending at any point, and it finishes in
under a minute.

## What is promised

| Property | The promise, in one sentence | The harm if it breaks |
|---|---|---|
| `NoBlindRepeat` | Work a turn started is never repeated on its own: a model sees a message an earlier turn acted on only when it is that turn's conversation carried on with the record of what it did. | An email sent twice, a file written twice, a booking made twice. |
| `NothingLost` | Every message to an agent is answered, called off by the operator, or still owed somewhere: queued, in a turn, in `pending_runs`, put down, or reported interrupted. | The operator asked for something, and nothing ever answers or says why. |
| `OnlyStartedReported` | A conversation no turn started is delivered again, never reported interrupted. | The operator is told to review actions that never happened. |
| `ReportedOnce` | A conversation is reported interrupted at most once, across any number of restarts. | Duplicate Try again notices. |
| `StoppedStaysStopped` | A conversation the operator stopped is never picked up by the next host. | Called-off work is carried out after an update. |

## Does my change touch it?

```sh
git diff --name-only origin/main... -- src-tauri/src/db/put_down.rs src-tauri/src/runtime/drain.rs   src-tauri/src/boot.rs src-tauri/src/db/store.rs src-tauri/src/runtime/mod.rs
```

A hit in `put_down.rs`, `drain.rs` or `boot::recover` always counts. In `store.rs` and
`runtime/mod.rs`, it counts when the diff changes a function named in the table below,
or adds a new writer or deleter of `pending_runs`, `put_down` or `put_down_runs`
(`git diff origin/main... | grep -nE 'pending_runs|put_down'`).

## Run it

```sh
./scripts/formal.sh
./scripts/ci.sh formal
```

Both commands run the same check. The script needs Java 11 or newer. On first use it downloads
`tla2tools.jar` v1.7.4 into `~/.cache/guaca` and checks it against a pinned SHA-256. Each row
prints `ok` or `FAIL`. The rows come in three kinds:

- **holds**: the code as it is satisfies the property. A FAIL here means the change broke a promise, or the model no longer describes the code.
- **finding**: the code as it is breaks the property, and the row expects that. A FAIL here saying it "now holds" means a fix landed. Change the row to `holds` and extend the first holds row's configuration to cover it.
- **broken**: a switch is set to FALSE, which removes one piece of the code, and the property must break. A FAIL here means the model lost the step that piece guards.

To see a counterexample, run `./scripts/formal.sh --trace "OperatorStops=TRUE StartCanFail=FALSE" NothingLost`. It prints the raw TLC output for the code's switches with those two scope settings.
Read every result, passing or failing, with the `/statechart` skill on `formal/recovery/Recovery.tla`. It turns a violation into its shortest path in plain words, and when the check passes it draws how a conversation moves between states.

## Model to code

Each action is one SQLite commit, or one section the code runs under `Runtime.inner.runs`. Paths are under `src-tauri/src`.

| Model | Code |
|---|---|
| `up`, `ends` | a host process, and how many have ended (crash, kill, or exit after a drain) |
| `draining`, `stopped`, `pdMem`, `outstanding` | `Runs.draining`, `Runs.stopped`, `Runs.put_down`, `Runs.outstanding` in `runtime/mod.rs` |
| `inbox` | the agent's mpsc receiver, with `carry` in front (`actor_loop`) |
| `turn` | the actor inside `Runtime::run_turn`. `carries` is whether `Runtime::resumed` returned records, `did` is whether the turn made a call, `fresh` is what it took in since its last call (`acted_on` in `run_turn`), and `owed` is the `track_inflight` calls `finish_turn` makes |
| `resuming` | `Inner.resuming`, filled by `Runtime::pick_up` |
| `riders` | messages a resumed turn carries without taking them in again: the non-routine ids in its `Inner.resuming` entry, booked from `Recovered::carried` |
| `accepted` | `messages` rows addressed to the agent |
| `pending` | the `pending_runs` row: `none`, `unstarted` (`started_at IS NULL`), `started` |
| `put`, `whole` | `put_down` rows (`rec` when `records` is not empty, `link` for a message in its carrier's `absorbed` list) and `put_down_runs` rows |
| `extra` | other messages in the run (records, notices, replies), which `put_down::UNTOUCHED` tests; it skips the system notice a failed start mark writes, which `SkipStartNote` models |
| `notices` | Interrupted notices, written by `Store::recover`, or at once by `Runtime::cut_short_notice` for a message a stopped turn had acted on, which then settles as `Settled::Interrupted` |
| `read`, `acted`, `answered`, `blind`, `marked`, `calledOff`, `resumedOff` | ghosts: what really happened, which the code cannot see |
| `Send` | `Runtime::deliver_with_expectation`: `Store::append_delivery` and `book`, or while draining `Store::append_put_down` and `hand_on` |
| `Dequeue`, `PutDownQueued` | `actor_loop` taking `first`, and `Runtime::put_down_queue` while draining |
| `MarkBatch`, `MarkFails` | `Store::start_deliveries` at the top of `run_turn`, and its error path, which puts the batch down |
| `Begin` | `run_turn` from `self.resumed(&batch)` to the first `if self.stopped(run_id)` |
| `TakeIn`, `Think` | `Runtime::take_in` (greedy, one `start_deliveries` per message) and the model call of a round |
| `Call`, `Returned` | a tool call from the completion, and its return |
| `CalledOff` | the round loop's `self.stopped(run_id)` checks, and the stop select in `stream_with_retries` |
| `Answer`, `HaltStopped`, `FinishOne` | the end of `run_turn`: the reply or the GuardStop notice, then for a stopped turn `Runtime::hand_back`, then `finish_turn`, where each `track_inflight` is either `Store::settle_run_as` or `Runtime::mark_whole` |
| `PutTurnDown`, `PutTurnDownCommit` | `Runtime::put_turn_down`: the record `deliver`, then `Runtime::put_down` and `Store::put_down_with_absorbed` |
| `Stop` | `Runtime::stop_run` and `Store::stop_routine_run` |
| `Drain` | `Runtime::drain` setting `draining` |
| `End` | the process ending: everything in `Runtime` is gone |
| `Recover` | `boot::recover` (`Store::recover`), then `Runtime::pick_up` |

The six switches `MarkBeforeRead`, `AcceptOneCommit`, `RecoverUntouched`, `RecoverClears`, `KeepLink`, `SkipStartNote` are TRUE in the code. `OperatorStops` and `StartCanFail` are scope settings: they decide whether the operator may stop a conversation, and whether the `start_deliveries` commit may fail.

## What it does not cover

The model has one agent and two operator messages, each in its own conversation, with at most two host processes ending. It leaves out the following:

- Peer messages and cascades across agents.
- Routine occurrences (`routine_occurrences`) and absorbed routines.
- Decision answers (`Store::recover_decisions`).
- Paused and deleted agents.
- Budget exhaustion.
- Approvals that park a turn.
- Failures of any commit other than `start_deliveries`.
- SQLite itself: each commit is assumed atomic and durable, so power loss under `synchronous = NORMAL` is out.

A pass covers this model, not the whole runtime.

## Bug or modeling error

When a row fails, replay the trace against the real code before believing either side. Write a scratch test in `src-tauri/tests/cascade.rs` that uses the harness there (`taken_in_mid_turn`, `h.restart`, `h.runtime.stop_run`, and a SQLite trigger for a failing commit), with one step per action, and run it with `cargo test --test cascade <name> -- --nocapture`.

- If the code reproduces it, it is a bug. Report it with the steps written in the code's words. Do not weaken the property or the model to make it pass.
- If the code does not reproduce it, the model is wrong. Find the action that does something the code cannot do, such as two commits treated as one, or a step the code makes atomic split in two, and fix it against the table above. In the first version, a turn put down again lost the records it had been resumed with, because the model left out that `put_turn_down` starts from `resumed`. In the same version, `Think` ran before a greedy `take_in` had emptied the inbox.

## Updating it for an intended change

1. Read the diff against the table, and change the action that stands for each changed function so it does what the code now does. Commit boundaries matter most.
2. Run `./scripts/formal.sh`. Every broken row must still break. If one no longer does, the model has lost the step that row guards.
3. If a holds row now fails, either the change breaks a promise, or the promise itself changed. Only a person can decide the second case. Ask before you change a property.
4. Move the sync commit below forward only after you have checked the model against the code at the new commit.

## Coverage record

Last confirmed against commit `79ae97296e7c83b4737c38a7abba9bf308de232b` (`main`). Files covered: `db/put_down.rs`, `runtime/drain.rs`, `boot.rs` (`recover`), `db/store.rs` (`append_delivery`, `claim_send`, `start_deliveries`, `settle_run_as`, `stop_routine_run`) and `runtime/mod.rs` (`deliver_with_expectation`, `run_turn`, `take_in`, `hand_back`, `finish_turn`, `track_inflight`, `stop_run`, `actor_loop`).

Open findings, held as finding rows in `scripts/formal.sh` and each replayed in the real runtime:

1. **NoBlindRepeat (stops).** A drain puts a message a turn took in and acted on down bare, beside the turn's own message and its record. If the operator stops the turn's conversation on the next host before that turn takes the message in again, the message gets a turn of its own with no record of the calls already made for it. Replayed with the agent paused across the restart, which holds the window open.
