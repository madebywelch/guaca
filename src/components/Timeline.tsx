import { useEffect, useMemo, useState } from "react";

import { api } from "../lib/ipc";
import { layoutRibbon, RIBBON_CHAR_PX, RIBBON_FACE_PX } from "../lib/ribbon";
import { useStore } from "../lib/store";
import { midnight, useNow } from "../lib/time";
import { errorMessage, type Occasion, type Outcome, type UpcomingRun } from "../lib/types";
import { Ribbon } from "./Ribbon";

interface Props {
  onOpenAgent: (agentId: string) => void;
  onOpenGroups: () => void;
  openAgentId?: string | null;
}

const AHEAD_MS = 8 * 86_400_000;
const TODAY_RUNS = 100;

export function Timeline({ onOpenAgent, onOpenGroups, openAgentId }: Props) {
  const agents = useStore((state) => state.agents);
  const groups = useStore((state) => state.groups);
  const activity = useStore((state) => state.activity);
  const outcomes = useStore((state) => state.outcomes);
  const pending = useStore((state) => state.pending);
  const routines = useStore((state) => state.routineVersion);
  const dates = useStore((state) => state.calendarVersion);
  const now = useNow(60_000);
  const [upcoming, setUpcoming] = useState<UpcomingRun[]>([]);
  const [occasions, setOccasions] = useState<Occasion[]>([]);
  const [width, setWidth] = useState(0);
  const [today, setToday] = useState<Outcome[] | null>(null);
  const hour = Math.floor(now / 3_600_000);
  const day = midnight(now);

  useEffect(() => {
    let current = true;
    api
      .listOutcomes({ since: day, limit: TODAY_RUNS })
      .then((rows) => {
        if (current) setToday(rows);
      })
      .catch((error: unknown) => {
        console.warn(`Today's finished runs could not be read: ${errorMessage(error)}`);
        if (current) setToday((read) => read ?? []);
      });
    return () => {
      current = false;
    };
  }, [day, outcomes]);

  const runs = useMemo(() => {
    const unseen = new Set(outcomes.map((one) => one.runId));
    return [...outcomes, ...(today ?? []).filter((one) => !unseen.has(one.runId))];
  }, [outcomes, today]);

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
        groups: groups.map((group) => group.id),
        activity,
        outcomes: runs,
        approvals: pending,
        upcoming,
        occasions,
        faceSpan: width > 0 ? RIBBON_FACE_PX / width : 0,
        charSpan: width > 0 ? RIBBON_CHAR_PX / width : 0,
      }),
    [now, agents, groups, activity, runs, pending, upcoming, occasions, width],
  );

  return (
    <Ribbon
      ribbon={ribbon}
      onOpenAgent={onOpenAgent}
      onOpenGroups={onOpenGroups}
      openAgentId={openAgentId}
      onTrackWidth={setWidth}
      pending={today === null}
    />
  );
}
