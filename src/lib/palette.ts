export interface Slot {
  hue: string;
  paper: string;
  ink: string;
}

export const SLOTS: readonly Slot[] = [
  { hue: "green", paper: "#008300", ink: "#008300" },
  { hue: "blue", paper: "#2a78d6", ink: "#3987e5" },
  { hue: "magenta", paper: "#e87ba4", ink: "#d55181" },
  { hue: "yellow", paper: "#eda100", ink: "#c98500" },
  { hue: "aqua", paper: "#1baf7a", ink: "#199e70" },
  { hue: "orange", paper: "#eb6834", ink: "#d95926" },
  { hue: "violet", paper: "#4a3aa7", ink: "#9085e9" },
  { hue: "red", paper: "#e34948", ink: "#e66767" },
];

export const MAX_SERIES = SLOTS.length;

export const MAX_SCATTER_HUES = 3;

export const SCATTER_MARKS = ["circle", "square", "triangle", "diamond"] as const;

export type ScatterMark = (typeof SCATTER_MARKS)[number];

export function seriesColor(index: number): string {
  return `var(--series-${index % SLOTS.length})`;
}

export function seriesMark(index: number): ScatterMark {
  return SCATTER_MARKS[index % SCATTER_MARKS.length] as ScatterMark;
}
