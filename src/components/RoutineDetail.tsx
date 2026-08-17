import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import {
  clockTime,
  isTimed,
  routineTitle,
  secondsUntil,
  TRIGGER_CHOICES,
  toTimeField,
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
  /** Local `HH:MM`. Ignored by triggers with no time of day. */
  time: string;
}

const DEFAULT_TRIGGER = "daily";

function editingFor(routine: Routine): Editing {
  return {
    name: routine.name,
    what: routine.what,
    trigger: routine.trigger,
    time: toTimeField(routine.nextRunAt),
  };
}

function blank(): Editing {
  // Now, rounded up to the next five minutes. A routine set at 9:28 first
  // firing at 9:28 tomorrow is what the operator meant, and a round number is
  // easier to recognise in the list afterwards.
  const now = new Date();
  now.setMinutes(Math.ceil(now.getMinutes() / 5) * 5, 0, 0);
  return { name: "", what: "", trigger: DEFAULT_TRIGGER, time: toTimeField(now.getTime()) };
}

/**
 * What to send.
 *
 * `inSecs` is how a time of day reaches the backend: it anchors the first
 * firing, and every repeat after that keeps the hour. Null on an edit leaves
 * the schedule exactly where it was, which is what fixing a typo should do.
 */
function draftOf(editing: Editing, moved: boolean): RoutineDraft {
  return {
    name: editing.name.trim(),
    what: editing.what.trim(),
    trigger: editing.trigger,
    inSecs: moved && isTimed(editing.trigger) ? secondsUntil(editing.time) : null,
  };
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
  // Whether the time was touched. An untouched time on an edit has to leave
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

  const dirty =
    routine === null ||
    moved ||
    draft.name.trim() !== routine.name ||
    draft.what.trim() !== routine.what ||
    draft.trigger !== routine.trigger;

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

  const timed = isTimed(draft.trigger);

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
          <span aria-hidden="true" className="routine__mark" />
          <select
            className="input input--slim"
            aria-label="Trigger"
            value={draft.trigger}
            onChange={(event) => {
              patch({ trigger: event.target.value });
              // Choosing a timed repeat is a decision about when, so the time
              // goes with it rather than staying where the old row was.
              if (isTimed(event.target.value)) setMoved(true);
            }}
          >
            {TRIGGER_CHOICES.map((choice) => (
              <option key={choice.spec} value={choice.spec}>
                {choice.label}
              </option>
            ))}
            {/* A gap an agent chose for itself is not in the list, and dropping
                it from the picker would silently rewrite the agent's schedule
                the first time an operator saved an unrelated edit. */}
            {!TRIGGER_CHOICES.some((choice) => choice.spec === draft.trigger) && (
              <option value={draft.trigger}>{draft.trigger}</option>
            )}
          </select>
          {timed && (
            <>
              <span className="hint">at</span>
              <input
                className="input input--slim"
                type="time"
                aria-label="Time of day"
                value={draft.time}
                onChange={(event) => {
                  patch({ time: event.target.value });
                  setMoved(true);
                }}
              />
            </>
          )}
        </div>
        {routine?.active && !dirty && (
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
            disabled={busy || !draft.what.trim()}
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
        </div>
      )}

      {routine && (
        <div className="field">
          <span className="field__label">Run history</span>
          {runs === null || runs.length === 0 ? (
            <p className="routines__note">No runs yet.</p>
          ) : (
            <ul className="history">
              {runs.map((entry) => (
                <li key={entry.runId} className="history__row">
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
                  <span className="history__ago">{relativeTime(entry.at, now)}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
