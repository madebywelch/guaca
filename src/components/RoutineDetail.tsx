import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import {
  anchorFor,
  clockTime,
  firstRunDelay,
  type Moment,
  momentOf,
  nextRoundMoment,
  ordinal,
  parseTrigger,
  repeatLabel,
  repeats,
  routineTitle,
  TRIGGER_CHOICES,
  WEEKDAYS,
} from "../lib/routine";
import { relativeTime, useNow } from "../lib/time";
import {
  type AgentId,
  errorMessage,
  type Routine,
  type RoutineDraft,
  type RoutineId,
  type RoutineRun,
} from "../lib/types";
import { compact, money, priced } from "./Spend";

interface Props {
  agentId: AgentId;
  /** `"new"` starts an empty one, which exists only once it is created. */
  routineId: RoutineId | "new";
  onBack: () => void;
}

/** A routine as it is being written. */
interface Editing {
  name: string;
  what: string;
  trigger: string;
  /** When it should fire. Only the parts the trigger asks for are read. */
  moment: Moment;
  /** Drop a firing that lands on an agent already working. Repeats only. */
  skipIfWorking: boolean;
}

const DEFAULT_TRIGGER = "daily";

function editingFor(routine: Routine): Editing {
  return {
    name: routine.name,
    what: routine.what,
    trigger: routine.trigger,
    // A routine holding no moment still needs something in the fields, in case
    // the operator switches it to a trigger that has one.
    moment: routine.nextRunAt === null ? nextRoundMoment() : momentOf(routine.nextRunAt),
    skipIfWorking: routine.skipIfWorking,
  };
}

function blank(): Editing {
  return {
    name: "",
    what: "",
    trigger: DEFAULT_TRIGGER,
    moment: nextRoundMoment(),
    // Off unless it is asked for: a routine that has to happen even if it has
    // to wait is the ordinary one.
    skipIfWorking: false,
  };
}

/**
 * What to send.
 *
 * `inSecs` is how a moment reaches the backend: it anchors the first firing,
 * and every repeat after that inherits the hour, and for a weekly or monthly
 * repeat the day as well. Null on an edit leaves the schedule exactly where it
 * was, which is what fixing a typo should do.
 */
function draftOf(editing: Editing, moved: boolean): RoutineDraft {
  return {
    name: editing.name.trim(),
    what: editing.what.trim(),
    trigger: editing.trigger,
    inSecs: moved ? firstRunDelay(editing.trigger, editing.moment) : null,
    // Dropped rather than sent on a trigger that does not repeat, where the
    // backend refuses the pair. The tick is hidden there, so a routine ticked
    // as a repeat and then switched to Once would otherwise fail to save with
    // its reason on a control nobody can see.
    skipIfWorking: repeats(editing.trigger) && editing.skipIfWorking,
  };
}

/** Why the moment on screen is not one, or null when it is. */
function momentProblem(editing: Editing): string | null {
  const anchor = anchorFor(editing.trigger);
  if (anchor === "none") return null;
  if (firstRunDelay(editing.trigger, editing.moment) !== null) return null;
  return anchor === "date"
    ? "That date and time have already passed. Pick one still ahead."
    : "That is not a time of day.";
}

/**
 * One routine, with the panel given over to it.
 *
 * Everything about a routine is here rather than in a row that has to stay
 * one line: what it is called, what it actually says, when it runs, whether it
 * runs at all, and what it has done. A routine is a standing commitment made
 * to an agent, and deciding whether to keep one means reading the instruction
 * that will be delivered, not a summary of it.
 *
 * Active, Delete and Test run act at once and are deliberately not part of the
 * draft: turning something off is not an edit to what it says, and must not
 * wait on a Save the operator has not pressed.
 */
export function RoutineDetail({ agentId, routineId, onBack }: Props) {
  const isNew = routineId === "new";
  const [routine, setRoutine] = useState<Routine | null>(null);
  const [draft, setDraft] = useState<Editing>(() => blank());
  const [runs, setRuns] = useState<RoutineRun[] | null>(null);
  // Whether the moment was touched. An untouched one on an edit has to leave
  // the next firing where it is: rewording an instruction should not silently
  // push the schedule to tomorrow.
  const [moved, setMoved] = useState(false);
  const [loading, setLoading] = useState(!isNew);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const now = useNow(30_000);

  const load = useCallback(async () => {
    if (isNew) return;
    try {
      // Read through the agent's list rather than adding a command for one
      // row: the panel has just drawn that list, so this is already warm.
      const found = (await api.agentRoutines(agentId)).find((r) => r.id === routineId);
      if (!found) {
        // Deleted from under us, or belonging to another agent.
        onBack();
        return;
      }
      setRoutine(found);
      setDraft(editingFor(found));
      setMoved(false);
      setRuns(await api.routineRuns(found.id));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }, [agentId, routineId, isNew, onBack]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      await action();
      return true;
    } catch (caught) {
      setError(errorMessage(caught));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const patch = (fields: Partial<Editing>) => setDraft((current) => ({ ...current, ...fields }));
  /** Any change to when it fires is a change to the moment it is anchored on. */
  const patchMoment = (fields: Partial<Moment>) => {
    setDraft((current) => ({ ...current, moment: { ...current.moment, ...fields } }));
    setMoved(true);
  };

  const dirty =
    routine === null ||
    moved ||
    draft.name.trim() !== routine.name ||
    draft.what.trim() !== routine.what ||
    draft.trigger !== routine.trigger ||
    draftOf(draft, moved).skipIfWorking !== routine.skipIfWorking;

  const problem = momentProblem(draft);

  const save = () =>
    void run(async () => {
      if (routine) {
        await api.updateRoutine(routine.id, draftOf(draft, moved));
        await load();
      } else {
        const made = await api.createRoutine(agentId, draftOf(draft, true));
        setRoutine(made);
        setDraft(editingFor(made));
        setMoved(false);
        setRuns([]);
      }
    });

  if (loading) return <p className="routines__note">Loading…</p>;

  const anchor = anchorFor(draft.trigger);
  const waiting = parseTrigger(draft.trigger).kind === "event";

  return (
    <div className="detail">
      <div className="detail__actions">
        {routine ? (
          <>
            <button
              type="button"
              role="switch"
              aria-checked={routine.active}
              className="toggle"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  setRoutine(await api.setRoutineActive(routine.id, !routine.active));
                })
              }
            >
              <span aria-hidden="true" className="toggle__track" />
              <span className="toggle__label">{routine.active ? "Active" : "Off"}</span>
            </button>

            <span style={{ flex: 1 }} />

            {confirmDelete ? (
              <>
                <button
                  type="button"
                  className="btn btn--small btn--danger"
                  disabled={busy}
                  onClick={() =>
                    void run(() => api.deleteRoutine(routine.id)).then((ok) => {
                      if (ok) onBack();
                    })
                  }
                >
                  Delete it
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  onClick={() => setConfirmDelete(false)}
                >
                  Keep
                </button>
              </>
            ) : (
              <>
                <button
                  type="button"
                  className="btn btn--small"
                  disabled={busy}
                  onClick={() => setConfirmDelete(true)}
                >
                  Delete
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--primary"
                  disabled={busy || dirty}
                  title={
                    dirty
                      ? "Save first, so what runs is what you are looking at"
                      : "Run it now. The schedule does not move."
                  }
                  onClick={() =>
                    void run(async () => {
                      await api.testRoutine(routine.id);
                      setRuns(await api.routineRuns(routine.id));
                      setNote("Sent. Watch the conversation.");
                    })
                  }
                >
                  Test run
                </button>
              </>
            )}
          </>
        ) : (
          <p className="routines__note" style={{ margin: 0 }}>
            New routine. It starts running as soon as you create it.
          </p>
        )}
      </div>

      <label className="field">
        <span className="field__label">Name</span>
        <input
          className="input"
          // Named explicitly because the hint below the next field is inside
          // its label, so the accessible name would otherwise be the label
          // plus a sentence of explanation.
          aria-label="Name"
          maxLength={64}
          value={draft.name}
          // What the list will call it if it is left blank, so nothing about
          // the row is a surprise.
          placeholder={
            draft.what.trim() ? routineTitle({ name: "", what: draft.what }) : "Listings sweep"
          }
          onChange={(event) => patch({ name: event.target.value })}
        />
      </label>

      <label className="field">
        <span className="field__label">Instruction</span>
        <textarea
          className="textarea"
          aria-label="Instruction"
          rows={7}
          placeholder="Check the listings and tell me what is new"
          value={draft.what}
          onChange={(event) => patch({ what: event.target.value })}
        />
        <span className="field__hint">
          Delivered to the agent as a message when it fires, in a fresh run with nothing else in it.
          Write it as something the agent can act on with no other context.
        </span>
      </label>

      <div className="field">
        <span className="field__label">When to run</span>
        <div className="when">
          <span aria-hidden="true" className="routine__mark" data-waiting={waiting || undefined} />
          <select
            className="input input--slim"
            aria-label="Trigger"
            value={draft.trigger}
            onChange={(event) => {
              patch({ trigger: event.target.value });
              // Choosing a trigger with a moment is a decision about when, so
              // the moment goes with it rather than staying where the old row
              // was. A trigger with none leaves the schedule alone.
              if (anchorFor(event.target.value) !== "none") setMoved(true);
            }}
          >
            {TRIGGER_CHOICES.map((choice) => (
              <option key={choice.spec} value={choice.spec}>
                {choice.label}
              </option>
            ))}
            {/* A trigger this picker does not offer: a gap an agent chose for
                itself, or an event. Dropping it would silently rewrite the
                agent's schedule the first time an operator saved an unrelated
                edit. */}
            {!TRIGGER_CHOICES.some((choice) => choice.spec === draft.trigger) && (
              <option value={draft.trigger}>{repeatLabel(draft.trigger)}</option>
            )}
          </select>

          {anchor === "weekday" && (
            <select
              className="input input--slim"
              aria-label="Day of the week"
              value={draft.moment.weekday}
              onChange={(event) => patchMoment({ weekday: Number(event.target.value) })}
            >
              {WEEKDAYS.map((day) => (
                <option key={day.day} value={day.day}>
                  {day.label}
                </option>
              ))}
            </select>
          )}

          {anchor === "monthday" && (
            <select
              className="input input--slim"
              aria-label="Day of the month"
              value={draft.moment.monthday}
              onChange={(event) => patchMoment({ monthday: Number(event.target.value) })}
            >
              {Array.from({ length: 31 }, (_, index) => index + 1).map((day) => (
                <option key={day} value={day}>
                  {ordinal(day)}
                </option>
              ))}
            </select>
          )}

          {anchor === "date" && (
            <input
              className="input input--slim"
              type="date"
              aria-label="Date"
              value={draft.moment.date}
              onChange={(event) => patchMoment({ date: event.target.value })}
            />
          )}

          {anchor !== "none" && (
            <>
              <span className="hint">at</span>
              <input
                className="input input--slim"
                type="time"
                aria-label="Time of day"
                value={draft.moment.time}
                onChange={(event) => patchMoment({ time: event.target.value })}
              />
            </>
          )}
        </div>

        {anchor === "monthday" && draft.moment.monthday > 28 && (
          <span className="field__hint">
            Months without a {ordinal(draft.moment.monthday)} are skipped rather than moved, so it
            stays on the {ordinal(draft.moment.monthday)} of the months that have one.
          </span>
        )}
        {waiting && (
          <span className="field__hint">
            This one keeps no place in the clock: nothing fires until that happens. Test run
            delivers it now.
          </span>
        )}
        {routine?.active && !dirty && routine.nextRunAt !== null && (
          <span className="field__hint">
            Next in {relativeTime(now, routine.nextRunAt)}, on{" "}
            {new Date(routine.nextRunAt).toLocaleDateString(undefined, {
              weekday: "long",
              month: "short",
              day: "numeric",
            })}
            .
          </span>
        )}
      </div>

      {/* Only where there is a next firing to fall back on. A one-off that
          skipped would be a one-off that never happened, and the backend
          refuses the pair rather than storing it. */}
      {repeats(draft.trigger) && (
        <label className="field field--row">
          <input
            type="checkbox"
            checked={draft.skipIfWorking}
            onChange={(event) => patch({ skipIfWorking: event.target.checked })}
          />
          <span>
            <span className="field__label">Skip it if the agent is already working</span>
            <span className="field__hint">
              A firing that comes due while this agent is mid-turn or has work waiting is dropped
              rather than queued behind it, and the next one comes at its usual time. For a sweep
              there is no point doing twice over. Leave it off for anything that has to happen even
              if it has to wait.
            </span>
          </span>
        </label>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0 0 0.7rem" }}>
          <span>{error}</span>
        </div>
      )}
      {note && !error && <p className="field__hint">{note}</p>}

      {dirty && (
        <div className="detail__save">
          <button
            type="button"
            className="btn btn--small btn--primary"
            disabled={busy || !draft.what.trim() || problem !== null}
            onClick={save}
          >
            {routine ? "Save changes" : "Create routine"}
          </button>
          {routine && (
            <button
              type="button"
              className="btn btn--small btn--ghost"
              onClick={() => {
                setDraft(editingFor(routine));
                setMoved(false);
              }}
            >
              Discard
            </button>
          )}
          {problem && <span className="field__hint">{problem}</span>}
        </div>
      )}

      {routine && <History runs={runs} now={now} />}
    </div>
  );
}

/**
 * What a routine has actually done.
 *
 * Every firing carries what it spent, because that is the difference between a
 * routine that is working and one that is being delivered to an agent which
 * never runs: the two rows are otherwise identical, and the operator's next
 * move is not the same.
 */
function History({ runs, now }: { runs: RoutineRun[] | null; now: number }) {
  return (
    <div className="field">
      <span className="field__label">Run history</span>
      {runs === null || runs.length === 0 ? (
        <p className="routines__note">No runs yet.</p>
      ) : (
        <ul className="history">
          {runs.map((entry, index) => (
            // A skipped firing has no run to be keyed by, and two of them in
            // one second are two rows. The list is read-only and redrawn whole,
            // so its position is as good an identity as it has.
            <li key={entry.runId ?? `${entry.kind}-${entry.at}-${index}`} className="history__row">
              <span className="history__when">
                {new Date(entry.at).toLocaleDateString(undefined, {
                  month: "short",
                  day: "numeric",
                })}{" "}
                at {clockTime(entry.at)}
              </span>
              {/* A button press and a real firing look the same in the
                  transcript, so which one this was is said here. */}
              {entry.kind === "test" && <span className="history__kind">test run</span>}
              {/* Recorded because the alternative is a gap, and a gap in this
                  list is what a scheduler that has stopped working looks
                  like. */}
              {entry.kind === "skipped" ? (
                <span
                  className="history__quiet"
                  title="This routine skips a firing that lands while the agent is already working, so nothing was delivered"
                >
                  skipped, already working
                </span>
              ) : entry.spent.calls === 0 ? (
                <span
                  className="history__quiet"
                  title="Delivered, but no model call was made under it"
                >
                  nothing ran
                </span>
              ) : (
                <span
                  className="history__spend"
                  title={`${entry.spent.prompt.toLocaleString()} in, ${entry.spent.completion.toLocaleString()} out, over ${entry.spent.calls} model call(s)`}
                >
                  {compact(entry.spent.prompt + entry.spent.completion)}
                  {priced(entry.spent.cost) && (
                    <span className="history__cost">{money(entry.spent.cost)}</span>
                  )}
                </span>
              )}
              <span className="history__ago">{relativeTime(entry.at, now)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
