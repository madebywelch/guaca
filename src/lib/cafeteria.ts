import presets from "./mirrors/hireable.json";
import type { AgentDraft } from "./types";

export type Station =
  | "Front office"
  | "Product and design"
  | "Engineering"
  | "Revenue"
  | "Customers and research"
  | "Finance and legal";

export const STATIONS: Station[] = [
  "Front office",
  "Product and design",
  "Engineering",
  "Revenue",
  "Customers and research",
  "Finance and legal",
];

export interface Hireable {
  id: string;
  name: string;
  station: Station;
  tagline: string;
  avatar: string;
  color: string;
  skills: string[];
  systemPrompt: string;
}

const STATION = new Set<string>(STATIONS);

function isStation(station: string): station is Station {
  return STATION.has(station);
}

function hireable(preset: (typeof presets)[number]): Hireable {
  if (!isStation(preset.station)) {
    throw new Error(`${preset.id} is at ${preset.station}, which is not a station`);
  }
  return { ...preset, station: preset.station };
}

export const HIREABLE: Hireable[] = presets.map(hireable);

export const STARTER_CREW = [
  "chief-of-staff",
  "market-researcher",
  "content-marketer",
  "executive-assistant",
];

export function toDraft(preset: Hireable): AgentDraft {
  return {
    name: preset.name,
    avatar: preset.avatar,
    color: preset.color,
    model: "",
    systemPrompt: preset.systemPrompt,
    skills: preset.skills,
  };
}

export function pick(ids: Iterable<string>): Hireable[] {
  const wanted = new Set(ids);
  return HIREABLE.filter((preset) => wanted.has(preset.id));
}
