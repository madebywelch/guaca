import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { relativeTime, splitGap, toSeconds, type Unit, useNow } from "../lib/time";
import { type AgentId, errorMessage, type Routine, type RoutineDraft } from "../lib/types";

interface Props {
  agentId: AgentId;
}

interface Editing {
  what: string;
  repeats: boolean;
  value: number;
  unit: Unit;
  /** Set only when the operator asked for the next firing to move. */
  inValue: string;
  inUnit: Unit;
}

function editingFor(routine: Routine): Editing {
  const gap =
    routine.everySecs === null ? { value: 1, unit: "hours" as Unit } : splitGap(routine.everySecs);
  return {
    what: routine.what,
    repeats: routine.everySecs !== null,
    value: gap.value,
    unit: gap.unit,
    inValue: "",
    inUnit: "hours",
  };
}

const BLANK: Editing = {
  what: "",
  repeats: true,
  value: 6,
  unit: "hours",
  inValue: "",
  inUnit: "hours",
};

function draftOf(editing: Editing): RoutineDraft {
  return {
    what: editing.what,
    everySecs: editing.repeats ? toSeconds(editing.value, editing.unit) : null,
    inSecs: editing.inValue.trim() ? toSeconds(Number(editing.inValue), editing.inUnit) : null,
  };
}

/**
 * An agent's schedule, as the operator sees and edits it.
 *
 * Agents set these for themselves with the `schedule` tool, which used to mean
 * the only record of what a crew had promised to keep doing was inside the
 * agents. A routine that fires every five hours is a standing commitment, and
 * a standing commitment nobody can see or cancel is not one anybody chose.
 */
export function RoutineList({ agentId }: Props) {
  const [routines, setRoutines] = useState<Routine[] | null>(null);
  const [editing, setEditing] = useState<Record<string, Editing>>({});
  const [adding, setAdding] = useState<Editing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const now = useNow(30_000);

  const load = useCallback(async () => {
    try {
      setRoutines(await api.agentRoutines(agentId));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setRoutines([]);
    }
  }, [agentId]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await load();
      return true;
    } catch (caught) {
      setError(errorMessage(caught));
      return false;
    } finally {
      setBusy(false);
    }
  };

  if (routines === null) return <p className="field__hint">Loading routines…</p>;

  return (
    <div className="routines">
      <div className="routines__head">
        <span className="field__label">Routines</span>
        {adding === null && (
          <button
            type="button"
            className="btn btn--ghost btn--small"
            onClick={() => setAdding({ ...BLANK })}
          >
            Add
          </button>
        )}
      </div>

      {routines.length === 0 && adding === null && (
        <p className="field__hint">
          None. Agents set these for themselves, and you can set one here: it is delivered to the
          agent as an instruction when it fires.
        </p>
      )}

      {routines.map((routine) => {
        const state = editing[routine.id];
        const dirty = state !== undefined;
        return (
          <div className="routine" key={routine.id}>
            <input
              className="input"
              value={state?.what ?? routine.what}
              onChange={(event) =>
                setEditing((e) => ({
                  ...e,
                  [routine.id]: {
                    ...(e[routine.id] ?? editingFor(routine)),
                    what: event.target.value,
                  },
                }))
              }
            />
            <div className="routine__row">
              <Cadence
                value={state ?? editingFor(routine)}
                onChange={(next) => setEditing((e) => ({ ...e, [routine.id]: next }))}
              />
              <span className="routine__when">
                {routine.lastRunAt ? `ran ${relativeTime(routine.lastRunAt, now)} ago, ` : ""}
                {/* Arguments swapped on purpose: this one is ahead of now. */}
                next in {relativeTime(now, routine.nextRunAt)}
              </span>
              {dirty && (
                <button
                  type="button"
                  className="btn btn--small btn--primary"
                  disabled={busy}
                  onClick={() =>
                    void run(() => api.updateRoutine(routine.id, draftOf(state))).then((ok) => {
                      if (ok) setEditing((e) => ({ ...e, [routine.id]: undefined as never }));
                    })
                  }
                >
                  Save
                </button>
              )}
              <button
                type="button"
                className="btn btn--small btn--ghost"
                disabled={busy}
                onClick={() => void run(() => api.deleteRoutine(routine.id))}
              >
                Delete
              </button>
            </div>
          </div>
        );
      })}

      {adding !== null && (
        <div className="routine">
          <input
            className="input"
            placeholder="Check the listings and tell me what is new"
            value={adding.what}
            onChange={(event) => setAdding({ ...adding, what: event.target.value })}
          />
          <div className="routine__row">
            <Cadence value={adding} onChange={setAdding} />
            <span style={{ flex: 1 }} />
            <button
              type="button"
              className="btn btn--small btn--primary"
              disabled={busy || !adding.what.trim()}
              onClick={() =>
                void run(() => api.createRoutine(agentId, draftOf(adding))).then((ok) => {
                  if (ok) setAdding(null);
                })
              }
            >
              Add
            </button>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              onClick={() => setAdding(null)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}

/** Repeat or not, and how often. */
function Cadence({ value, onChange }: { value: Editing; onChange: (next: Editing) => void }) {
  return (
    <>
      <select
        className="input input--slim"
        value={value.repeats ? "every" : "once"}
        onChange={(event) => onChange({ ...value, repeats: event.target.value === "every" })}
      >
        <option value="every">every</option>
        <option value="once">once, in</option>
      </select>
      {value.repeats ? (
        <>
          <input
            className="input input--slim input--number"
            type="number"
            min={1}
            value={value.value}
            onChange={(event) => onChange({ ...value, value: Number(event.target.value) })}
          />
          <UnitPicker unit={value.unit} onChange={(unit) => onChange({ ...value, unit })} />
        </>
      ) : (
        <>
          <input
            className="input input--slim input--number"
            type="number"
            min={1}
            placeholder="1"
            value={value.inValue}
            onChange={(event) => onChange({ ...value, inValue: event.target.value })}
          />
          <UnitPicker unit={value.inUnit} onChange={(inUnit) => onChange({ ...value, inUnit })} />
        </>
      )}
    </>
  );
}

function UnitPicker({ unit, onChange }: { unit: Unit; onChange: (unit: Unit) => void }) {
  return (
    <select
      className="input input--slim"
      value={unit}
      onChange={(event) => onChange(event.target.value as Unit)}
    >
      <option value="minutes">minutes</option>
      <option value="hours">hours</option>
      <option value="days">days</option>
    </select>
  );
}
