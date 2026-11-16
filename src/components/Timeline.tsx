import { useEffect, useMemo, useState } from "react";

import { api } from "../lib/ipc";
import { layoutRibbon, RIBBON_CHAR_PX, RIBBON_FACE_PX } from "../lib/ribbon";
import { useStore } from "../lib/store";
import { useNow } from "../lib/time";
import { errorMessage, type Occasion, type UpcomingRun } from "../lib/types";
import { Ribbon } from "./Ribbon";

interface Props {
  onOpenAgent: (agentId: string) => void;
  onOpenPile: () => void;
  /** Whose page is open, so the strip can mark them. */
  openAgentId?: string | null;
}

/** How far ahead the timeline reads: through the end of next week, which covers its day columns. */
const AHEAD_MS = 8 * 86_400_000;

/**
 * The ribbon with what it is drawn from: the store's live state, plus the two
 * reads only it needs, every agent's upcoming runs and the calendar. Read
 * again when a routine or a date changes and as the clock moves on, and a read
 * that fails leaves the timeline drawing what it has: it is a picture of the
 * day, and a picture with one row missing beats a banner over it.
 */
export function Timeline({ onOpenAgent, onOpenPile, openAgentId }: Props) {
  const agents = useStore((state) => state.agents);
  const activity = useStore((state) => state.activity);
  const outcomes = useStore((state) => state.outcomes);
  const pending = useStore((state) => state.pending);
  const routines = useStore((state) => state.routineVersion);
  const dates = useStore((state) => state.calendarVersion);
  const now = useNow(60_000);
  const [upcoming, setUpcoming] = useState<UpcomingRun[]>([]);
  const [occasions, setOccasions] = useState<Occasion[]>([]);
  const [width, setWidth] = useState(0);
  const hour = Math.floor(now / 3_600_000);

  useEffect(() => {
    let current = true;
    const from = Date.now();
    void Promise.allSettled([
      api.upcomingRuns(from + AHEAD_MS),
      api.calendar(from, from + AHEAD_MS),
    ]).then(([runs, dated]) => {
      if (!current) return;
      if (runs.status === "fulfilled") setUpcoming(runs.value);
      else console.warn(`Upcoming runs could not be read: ${errorMessage(runs.reason)}`);
      if (dated.status === "fulfilled") setOccasions(dated.value);
      else console.warn(`The calendar could not be read: ${errorMessage(dated.reason)}`);
    });
    return () => {
      current = false;
    };
  }, [routines, dates, hour]);

  const ribbon = useMemo(
    () =>
      layoutRibbon({
        now,
        agents: agents.filter((agent) => agent.lifecycle !== "terminated"),
        activity,
        outcomes,
        approvals: pending,
        upcoming,
        occasions,
        faceSpan: width > 0 ? RIBBON_FACE_PX / width : 0,
        charSpan: width > 0 ? RIBBON_CHAR_PX / width : 0,
      }),
    [now, agents, activity, outcomes, pending, upcoming, occasions, width],
  );

  return (
    <Ribbon
      ribbon={ribbon}
      onOpenAgent={onOpenAgent}
      onOpenPile={onOpenPile}
      openAgentId={openAgentId}
      onTrackWidth={setWidth}
    />
  );
}
