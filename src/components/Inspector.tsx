import { useEffect, useState } from "react";

import { useStore } from "../lib/store";
import type { AgentCard, RoutineId } from "../lib/types";
import { BrowserScreen } from "./BrowserScreen";
import { ComputerScreen } from "./ComputerScreen";
import { RoutineDetail } from "./RoutineDetail";
import { RoutineList } from "./RoutineList";

interface Props {
  agent: AgentCard | undefined;
  onEditProfile: (agent: AgentCard) => void;
}

const REMEMBERED = "guac.inspector";

/**
 * Whether the panel was open last time.
 *
 * One answer for the whole app rather than one per agent: this is a statement
 * about how the operator likes to work, not about any particular agent, and a
 * panel that reappeared on some rows and not others would read as a bug.
 */
function remembered(): boolean {
  try {
    return localStorage.getItem(REMEMBERED) !== "closed";
  } catch {
    // Private modes and hardened webviews can refuse storage. A forgotten
    // preference is a much smaller problem than a panel that will not draw.
    return true;
  }
}

/**
 * What an agent has, beside what it says.
 *
 * The transcript answers "what happened"; this answers "what is this agent set
 * up to do" — the machine it works on and the schedule it keeps. Both used to
 * be in the dialog you reached by right-clicking, which meant a standing
 * commitment and a live desktop were things you had to go and open a modal to
 * find, and could not watch while reading.
 *
 * Two levels, in the same column. The top one is the agent; opening a routine
 * replaces it rather than growing it, because a routine's instruction is
 * several sentences long and a list that showed it was one routine tall.
 *
 * The agent's name, look and instructions are deliberately not here. Those are
 * edited rarely and all at once, which is what a dialog is for; these are
 * glanced at constantly.
 */
export function Inspector({ agent, onEditProfile }: Props) {
  const [open, setOpen] = useState(remembered);
  const [routine, setRoutine] = useState<RoutineId | "new" | null>(null);
  // Bumped to make the list read itself again after a routine was edited.
  const [listVersion, setListVersion] = useState(0);

  useEffect(() => {
    try {
      localStorage.setItem(REMEMBERED, open ? "open" : "closed");
    } catch {
      // As above: not worth telling the operator about.
    }
  }, [open]);

  // Switching agent comes back to the top of the panel. A routine id belongs
  // to one agent, and leaving it open would show another agent's schedule
  // under this agent's name until it failed to load.
  const agentId = agent?.id;
  useEffect(() => {
    setRoutine(null);
  }, [agentId]);

  // A fired routine in the transcript, clicked. The panel is opened along with
  // it: the request came from a row the operator could see, so answering it by
  // changing something behind a closed panel would read as the click doing
  // nothing. Cleared as it is taken, so the same chip can be clicked again
  // after navigating away.
  const asked = useStore((state) => state.openingRoutine);
  const taken = useStore((state) => state.routineOpened);
  useEffect(() => {
    if (asked === null) return;
    setRoutine(asked);
    setOpen(true);
    taken();
  }, [asked, taken]);

  // Nothing to inspect on the activity board, which is about the whole crew.
  if (!agent) return null;

  if (!open) {
    return (
      <aside className="inspector inspector--closed">
        <button
          type="button"
          className="inspector__tab"
          onClick={() => setOpen(true)}
          title={`Show ${agent.name}'s screen and routines`}
          aria-label={`Show ${agent.name}'s screen and routines`}
          aria-expanded={false}
        >
          «
        </button>
      </aside>
    );
  }

  return (
    <aside className="inspector" aria-label={`${agent.name}: screen and routines`}>
      <div className="inspector__head">
        {routine !== null ? (
          <>
            <button
              type="button"
              className="inspector__tab"
              onClick={() => setRoutine(null)}
              title="Back to the agent"
              aria-label="Back to the agent"
            >
              ‹
            </button>
            <h2 className="inspector__title">Routine</h2>
          </>
        ) : (
          <>
            <span style={{ flex: 1 }} />
            <button
              type="button"
              className="inspector__gear"
              onClick={() => onEditProfile(agent)}
              title={`Edit ${agent.name}'s profile`}
              aria-label={`Edit ${agent.name}'s profile`}
            >
              ⚙
            </button>
          </>
        )}
        <button
          type="button"
          className="inspector__tab"
          onClick={() => setOpen(false)}
          title="Hide this panel"
          aria-label="Hide this panel"
          aria-expanded={true}
        >
          »
        </button>
      </div>

      <div className="inspector__body">
        {/* Hidden rather than unmounted while a routine is open. The screen
            holds a live connection to the agent's desktop, and rebuilding it
            every time somebody looked at a schedule would drop and reconnect
            it. The list beside it is remounted deliberately, by its key. */}
        <div className="inspector__level" hidden={routine !== null}>
          <ComputerScreen agent={agent} key={agent.id} />
          <BrowserScreen agent={agent} key={`browser:${agent.id}`} />
          <RoutineList agentId={agent.id} onOpen={setRoutine} key={`${agent.id}:${listVersion}`} />
        </div>

        {routine !== null && (
          <RoutineDetail
            key={`${agent.id}:${routine}`}
            agentId={agent.id}
            routineId={routine}
            onBack={() => {
              setRoutine(null);
              // A routine can come back renamed, retimed, switched off or
              // gone, and the list behind this was drawn before any of that.
              setListVersion((n) => n + 1);
            }}
          />
        )}
      </div>
    </aside>
  );
}
