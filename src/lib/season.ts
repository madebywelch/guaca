import { createContext, useEffect, useState } from "react";

import type { Season } from "./theme";

export type { Season } from "./theme";

const NOVEMBER = 10;
const DECEMBER = 11;
const THURSDAY = 4;

export function thanksgiving(year: number): number {
  const first = new Date(year, NOVEMBER, 1).getDay();
  return 1 + ((THURSDAY - first + 7) % 7) + 21;
}

export function seasonOn(date: Date): Season | null {
  const month = date.getMonth();
  const day = date.getDate();
  if (month === DECEMBER && day === 25) return "christmas";
  if (month === NOVEMBER && day === thanksgiving(date.getFullYear())) return "thanksgiving";
  return null;
}

export function untilMidnight(now: Date): number {
  const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  return Math.max(1, next.getTime() - now.getTime());
}

export function watchSeason(
  onChange: (season: Season | null) => void,
  clock: () => Date = () => new Date(),
): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const check = () => {
    clearTimeout(timer);
    const now = clock();
    onChange(seasonOn(now));
    timer = setTimeout(check, untilMidnight(now));
  };
  const shown = () => {
    if (document.visibilityState === "visible") check();
  };
  window.addEventListener("focus", check);
  document.addEventListener("visibilitychange", shown);
  check();
  return () => {
    clearTimeout(timer);
    window.removeEventListener("focus", check);
    document.removeEventListener("visibilitychange", shown);
  };
}

const FORCED: Record<string, Season | null> = {
  thanksgiving: "thanksgiving",
  christmas: "christmas",
  none: null,
};

export function forcedSeason(
  asked: string | null | undefined,
  dev: boolean = import.meta.env.DEV,
): Season | null | undefined {
  if (!dev || !asked || !Object.hasOwn(FORCED, asked)) return undefined;
  return FORCED[asked];
}

function askedSeason(): string | null | undefined {
  return (
    new URLSearchParams(window.location.search).get("season") ?? import.meta.env.VITE_GUACA_SEASON
  );
}

export function useSeason(enabled: boolean): Season | null {
  const [season, setSeason] = useState<Season | null>(() => seasonOn(new Date()));
  const forced = forcedSeason(askedSeason());
  useEffect(() => (forced === undefined ? watchSeason(setSeason) : undefined), [forced]);
  if (!enabled) return null;
  return forced === undefined ? season : forced;
}

export const SeasonContext = createContext<Season | null>(null);
