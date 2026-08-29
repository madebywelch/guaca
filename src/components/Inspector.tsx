import { useEffect, useState } from "react";

import { useStore } from "../lib/store";
import type { AgentCard, RoutineId } from "../lib/types";
import { BrowserScreen } from "./BrowserScreen";
import { ComputerScreen } from "./ComputerScreen";
import { Memory } from "./Memory";
import { RoutineDetail } from "./RoutineDetail";
import { RoutineList } from "./RoutineList";
import { WorkingNotes } from "./WorkingNotes";

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
 *
 * The order is what is worth glancing at, most first. A screen is a picture of
 * something happening right now, so the two of them lead; then the schedule it
 * keeps, which is what is about to happen and is the one section nothing else
 * on screen says; then the working notes, which are what the agent is in the
 * middle of and which the transcript beside them is already showing. Memory is
 * last because it is the one section that is a text box: it is read closely
 * rather than glanced at, edited deliberately, and at the top it spent the
 * panel's vertical budget on a page nobody was looking at while the live
 * desktop under it sat below the fold. Its height cap stays for the same
 * reason it was added, one section further down.
 *
 * Memory and the working notes are the two halves of what an agent carries
 * between conversations, and they are the only sections with something to show
 * on every agent, since a screen is usually an offer rather than a picture and
 * a new agent keeps no routines. They are adjacent, which is what the schedule
 * moving above them buys: the two stores are read against each other, and what
 * each is for is clearest when the other is under it.
 */
export function Inspector({ agent, onEditProfile }: Props) {
  const [open, setOpen] = useState(remembered);
  const [routine, setRoutine] = useState<RoutineId | "new" | null>(null);

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

  // Nothing to inspect with no channel open, which is what going inside a crew
  // the open one was not in leaves behind.
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
            it.

            The key is here rather than on each section, and it is the whole
            mechanism for switching agent: every section below holds one
            agent's thing, a desktop connection, a browser, a list read over
            IPC, and none of them may be left under the next agent's name. One
            key on what they share says that once. Saying it three times, once
            per section, collided: React indexes the outgoing children by key
            to decide what to delete, so the duplicate overwrote the entry the
            screen would have been deleted through. The previous agent's
            screen stayed in the document with no fiber left to update it and
            its polling still running, every switch added another, and only
            closing the panel cleared them. */}
        <div className="inspector__level" hidden={routine !== null} key={agent.id}>
          <BrowserScreen agent={agent} />
          <ComputerScreen agent={agent} />
          <RoutineList agentId={agent.id} onOpen={setRoutine} />
          <WorkingNotes agentId={agent.id} />
          <Memory agentId={agent.id} />
        </div>

        {routine !== null && (
          <RoutineDetail
            key={`${agent.id}:${routine}`}
            agentId={agent.id}
            routineId={routine}
            // A routine can come back renamed, retimed, switched off or gone.
            // The list behind this reads itself again on the event every one of
            // those emits, so there is nothing to bump here.
            onBack={() => setRoutine(null)}
          />
        )}
      </div>
    </aside>
  );
}
