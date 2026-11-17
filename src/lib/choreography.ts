import { useEffect, useRef, useState } from "react";

import { prefersReducedMotion } from "./motion";
import type { Pulse } from "./store";
import type { AgentId } from "./types";

export const AIM_MS = 650;
export const FLIGHT_MS = 1200;
export const CATCH_MS = 700;
export const STAGGER_MS = 850;

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
  inFlight: StagedPulse[];
  staged: StagedPulse[];
}

export function usePulseChoreography(pulses: Pulse[], onDone: (id: number) => void): Choreography {
  const [staged, setStaged] = useState<StagedPulse[]>([]);

  const queue = useRef<Pulse[]>([]);
  const seen = useRef(new Set<number>());
  const lastStart = useRef(0);
  const timers = useRef(new Set<number>());
  const wait = useRef<number | null>(null);
  const done = useRef(onDone);
  done.current = onDone;

  const schedule = useRef<() => void>(() => {});

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

    const book = () => {
      if (wait.current !== null || queue.current.length === 0) return;

      wait.current = window.setTimeout(
        () => {
          wait.current = null;

          const next = queue.current.shift();
          if (next) {
            lastStart.current = Date.now();
            start(next);
          }
          book();
        },
        Math.max(0, STAGGER_MS - (Date.now() - lastStart.current)),
      );
    };

    schedule.current = book;

    return () => {
      if (wait.current !== null) window.clearTimeout(wait.current);
      wait.current = null;
      for (const id of timers.current) window.clearTimeout(id);
      timers.current.clear();
    };
  }, []);

  useEffect(() => {
    for (const pulse of pulses) {
      if (seen.current.has(pulse.id)) continue;
      seen.current.add(pulse.id);

      if (prefersReducedMotion()) {
        done.current(pulse.id);
        continue;
      }
      if (queue.current.length >= MAX_QUEUE) {
        done.current(pulse.id);
        continue;
      }
      queue.current.push(pulse);
    }

    schedule.current();
  }, [pulses]);

  return { staged, inFlight: staged.filter((p) => p.phase === "flight") };
}

export function roleOf(
  staged: StagedPulse[],
  agent: AgentId,
): { gesture: "send" | "receive" | null; facing: AgentId | null; says: string | null } {
  for (const pulse of staged) {
    if (pulse.from === agent) {
      return {
        gesture: pulse.phase === "aim" ? "send" : null,
        facing: pulse.to,
        says: pulse.phase === "aim" ? "!" : null,
      };
    }
    if (pulse.to === agent) {
      return {
        gesture: pulse.phase === "catch" ? "receive" : null,
        facing: pulse.from,
        says: null,
      };
    }
  }
  return { gesture: null, facing: null, says: null };
}
