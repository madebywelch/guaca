import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { describeTrigger, parseTrigger, routineTitle } from "../lib/routine";
import { useStore } from "../lib/store";
import { relativeTime, useNow } from "../lib/time";
import { type AgentId, errorMessage, type Routine, type RoutineId } from "../lib/types";

/**
 * The end of the second line: where this routine stands, right now.
 *
 * Three states, and the row has to be honest about all of them. A switched-off
 * routine must not claim a next firing, and one waiting on an event has no
 * next firing to claim: a countdown drawn there would be to a moment nothing
 * is holding. When it last ran carries the weight instead, because on a
 * routine that is not about to do anything that is the only news left.
 */
function standing(routine: Routine, now: number): string {
  if (!routine.active) {
    return routine.lastRunAt === null
      ? " · off"
      : ` · off, last ran ${relativeTime(routine.lastRunAt, now)}`;
  }
  if (routine.nextRunAt !== null) {
    // Arguments swapped on purpose: this one is ahead of now.
    return ` · next in ${relativeTime(now, routine.nextRunAt)}`;
  }
  return routine.lastRunAt === null
    ? " · waiting"
    : ` · waiting, last ran ${relativeTime(routine.lastRunAt, now)}`;
}

interface Props {
  agentId: AgentId;
  /** `"new"` opens an empty one. */
  onOpen: (routine: RoutineId | "new") => void;
}

/**
 * An agent's schedule, as one line per standing commitment.
 *
 * Agents set these for themselves with the `schedule` tool, which used to mean
 * the only record of what a crew had promised to keep doing was inside the
 * agents. A routine that fires every weekday is a standing commitment, and a
 * standing commitment nobody can see or cancel is not one anybody chose.
 *
 * A row is a name and a cadence and stops there. The instruction is written to
 * be acted on with no other context, so it runs to several sentences, and a
 * list that showed it was one routine tall.
 */
export function RoutineList({ agentId, onOpen }: Props) {
  const [routines, setRoutines] = useState<Routine[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const now = useNow(30_000);
  // A routine set, retimed, canceled or fired since this was drawn. The
  // operator watching an agent work is the one most likely to be looking at
  // this list while it changes, and until this existed the only way to see the
  // change was to close the panel and open it again.
  const changed = useStore((state) => state.routineVersion[agentId] ?? 0);

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
  }, [load, changed]);

  return (
    <section className="routines">
      <div className="routines__head">
        <h3 className="routines__title">Routines</h3>
        <button
          type="button"
          className="routines__add"
          title="Add a routine"
          aria-label="Add a routine"
          onClick={() => onOpen("new")}
        >
          +
        </button>
      </div>

      {routines === null ? (
        <p className="routines__note">Loading…</p>
      ) : routines.length === 0 ? (
        <p className="routines__note">
          Nothing standing. Agents set these for themselves, and you can set one here: it reaches
          the agent as an instruction when it fires.
        </p>
      ) : (
        routines.map((routine) => (
          <button
            key={routine.id}
            type="button"
            className="routine"
            data-off={routine.active ? undefined : "true"}
            onClick={() => onOpen(routine.id)}
            title={routine.what}
          >
            <span
              aria-hidden="true"
              className="routine__mark"
              // A clock face is a lie on something that does not wait on one.
              data-waiting={parseTrigger(routine.trigger).kind === "event" || undefined}
            />
            <span className="routine__body">
              <span className="routine__name">{routineTitle(routine)}</span>
              <span className="routine__when">
                {describeTrigger(routine.trigger, routine.nextRunAt)}
                <span className="routine__next">{standing(routine, now)}</span>
              </span>
            </span>
          </button>
        ))
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.5rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </section>
  );
}
