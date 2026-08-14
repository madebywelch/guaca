import { useEffect, useRef, useState } from "react";

import type { Pulse } from "./store";
import type { AgentId } from "./types";

/**
 * Paces inter-agent messages so you can actually watch one happen.
 *
 * The runtime is real-time: a fan-out to four agents lands four messages within
 * a few milliseconds of each other. Animating them the moment they arrive means
 * four parcels crossing at once and every egg reacting simultaneously, which is
 * over before the eye can follow it.
 *
 * So delivery and choreography are decoupled. Messages are already delivered by
 * the time they reach here; this only decides when the drawing of each one
 * plays. Each message gets a visible beat of its own:
 *
 *   aim     both eggs turn to face each other, sender shouts
 *   flight  the parcel crosses the wire
 *   catch   the recipient takes the hit and settles
 *
 * Simultaneous messages are staggered rather than serialised, so a burst still
 * finishes promptly while each throw stays individually legible.
 */

export const AIM_MS = 650;
export const FLIGHT_MS = 1200;
export const CATCH_MS = 700;
/** Gap between the starts of consecutive throws. */
export const STAGGER_MS = 850;

/**
 * Beyond this, a backlog would take longer to play than anyone will watch.
 * Excess messages are still delivered and still in the transcript; they just do
 * not each get their own animation.
 */
const MAX_QUEUE = 12;

export type Phase = "aim" | "flight" | "catch";

export interface StagedPulse {
  id: number;
  from: AgentId;
  to: AgentId;
  color: string;
  phase: Phase;
}

export interface Choreography {
  /** Parcels currently crossing the wire. */
  inFlight: StagedPulse[];
  /** Every message being animated right now, in any phase. */
  staged: StagedPulse[];
}

function prefersReducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

/**
 * Consumes pulses from the store and hands back only the ones that should be
 * drawn at this moment. Calls `onDone` when a message has finished playing so
 * the store can drop it.
 */
export function usePulseChoreography(pulses: Pulse[], onDone: (id: number) => void): Choreography {
  const [staged, setStaged] = useState<StagedPulse[]>([]);

  const queue = useRef<Pulse[]>([]);
  const seen = useRef(new Set<number>());
  const lastStart = useRef(0);
  const timers = useRef(new Set<number>());
  // Kept in a ref so the pump does not need `onDone` in its dependencies, which
  // would restart the scheduler on every render.
  const done = useRef(onDone);
  done.current = onDone;

  useEffect(() => {
    for (const pulse of pulses) {
      if (seen.current.has(pulse.id)) continue;
      seen.current.add(pulse.id);

      if (prefersReducedMotion()) {
        done.current(pulse.id);
        continue;
      }
      if (queue.current.length >= MAX_QUEUE) {
        // Dropped from the animation only. The message itself arrived.
        done.current(pulse.id);
        continue;
      }
      queue.current.push(pulse);
    }
  }, [pulses]);

  useEffect(() => {
    const after = (ms: number, run: () => void) => {
      const id = window.setTimeout(() => {
        timers.current.delete(id);
        run();
      }, ms);
      timers.current.add(id);
    };

    const setPhase = (id: number, phase: Phase) =>
      setStaged((current) => current.map((p) => (p.id === id ? { ...p, phase } : p)));

    const start = (pulse: Pulse) => {
      setStaged((current) => [...current, { ...pulse, phase: "aim" }]);
      after(AIM_MS, () => setPhase(pulse.id, "flight"));
      after(AIM_MS + FLIGHT_MS, () => setPhase(pulse.id, "catch"));
      after(AIM_MS + FLIGHT_MS + CATCH_MS, () => {
        setStaged((current) => current.filter((p) => p.id !== pulse.id));
        done.current(pulse.id);
      });
    };

    // A single interval is enough: it only ever starts the next throw once the
    // stagger has elapsed, so the cost is one cheap check per tick.
    const pump = window.setInterval(() => {
      if (queue.current.length === 0) return;
      const now = Date.now();
      if (now - lastStart.current < STAGGER_MS) return;

      const next = queue.current.shift();
      if (!next) return;
      lastStart.current = now;
      start(next);
    }, 80);

    return () => {
      window.clearInterval(pump);
      for (const id of timers.current) window.clearTimeout(id);
      timers.current.clear();
    };
  }, []);

  return { staged, inFlight: staged.filter((p) => p.phase === "flight") };
}

/**
 * What one agent is doing in the current choreography.
 *
 * A sender aims and throws; a recipient watches it come and then takes it.
 * Both look at each other for the whole aim and flight, which is what makes the
 * exchange read as two characters rather than two independent animations.
 */
export function roleOf(
  staged: StagedPulse[],
  agent: AgentId,
): { gesture: "send" | "receive" | null; facing: AgentId | null; says: string | null } {
  for (const pulse of staged) {
    if (pulse.from === agent) {
      return {
        gesture: pulse.phase === "aim" ? "send" : null,
        facing: pulse.phase === "catch" ? null : pulse.to,
        says: pulse.phase === "aim" ? "!" : null,
      };
    }
    if (pulse.to === agent) {
      return {
        gesture: pulse.phase === "catch" ? "receive" : null,
        facing: pulse.phase === "catch" ? null : pulse.from,
        says: null,
      };
    }
  }
  return { gesture: null, facing: null, says: null };
}
