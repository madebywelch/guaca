/**
 * The colors a chart is allowed to use.
 *
 * Eight hues in one fixed order, assigned to series by position and never
 * cycled. The order is the accessibility mechanism, not a preference: what
 * makes a chart readable to a red-green colorblind operator is that
 * *neighboring* slots stay far apart, because neighboring slots are the ones
 * that touch in a stack, sit side by side in a group, and cross in a line
 * chart. A palette picked by eye fails that test roughly always.
 *
 * So this one was not picked by eye. The eight hexes are a documented set with
 * published separation figures; what this file chose is the order, by
 * enumerating all 40,320 of them and keeping only the 160 that clear every
 * gate in both surfaces. Guaca opens on green because Guaca is green, and that
 * choice cost nothing: of the orders that pass, this is one of the best.
 *
 * Measured on this app's own chart surface (`--raised`: `#ffffff` on paper,
 * `#222622` on ink), simulating protanopia and deuteranopia at full severity,
 * as OKLab distance ×100:
 *
 * | gate                        | target | paper | ink  |
 * |-----------------------------|--------|-------|------|
 * | neighbors, colorblind     | ≥ 8    | 9.1   | 8.4  |
 * | neighbors, full color     | ≥ 15   | 19.6  | 19.3 |
 * | first three, every pair     | ≥ 8    | 13.0  | 13.0 |
 *
 * The last row is the one scatter needs, where any two dots can end up
 * touching and "neighbors" means nothing. Nine hues is not an option: a
 * generated ninth is indistinguishable from one of these eight under
 * colorblindness, so a ninth series folds into an "Other" instead.
 *
 * `palette.test.ts` recomputes every figure above from these hexes. It is the
 * gate, not a note about one: a hex edited here fails the suite.
 *
 * Ink is not paper flipped. It is the same eight hues re-stepped for a dark
 * ground, chosen and measured as its own set, which is why green is the one
 * value that repeats: it already sat in both bands.
 */

/** One hue, in each of the two surfaces the reading column has. */
export interface Slot {
  /** Named for the doc comment above and for a failure message, never drawn. */
  hue: string;
  paper: string;
  ink: string;
}

/**
 * The eight, in the order they are handed out.
 *
 * Read by index, so a series keeps its color when its neighbors are switched
 * off in the legend. Color follows the series, never its rank: an operator who
 * has learned that revenue is green is misled by a chart that repaints the
 * survivors when something is hidden.
 */
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

/** How many series a chart may carry before the tail has to be folded up. */
export const MAX_SERIES = SLOTS.length;

/**
 * How many of them a form where any two marks can touch may carry.
 *
 * Scatter and bubble have no neighbors: every dot is potentially beside every
 * other, so the gate is every pair rather than adjacent pairs, and that is a
 * strictly harder test no ordering of eight hues can pass. Three is what does
 * pass. Past three, a scatter leans on the shape of its marks as well as their
 * color, which is the documented answer to running out of hues.
 */
export const MAX_SCATTER_HUES = 3;

/**
 * The marks a scatter draws its series with.
 *
 * A second channel beside the hue, and it is why a scatter here is not capped
 * at three series. Shape survives every kind of colorblindness, print, and a
 * screenshot pasted into a document, none of which hue does.
 */
export const SCATTER_MARKS = ["circle", "square", "triangle", "diamond"] as const;

export type ScatterMark = (typeof SCATTER_MARKS)[number];

/**
 * The color for a series, as a CSS custom property reference.
 *
 * A property rather than a hex, so the surface decides which of the two values
 * it resolves to and nothing here has to know which one is current. The
 * alternative is reading `data-surface` in every chart and re-rendering all of
 * them when it changes, which is a subscription to a value CSS already has.
 */
export function seriesColor(index: number): string {
  return `var(--series-${index % SLOTS.length})`;
}

/** The mark a scatter series is drawn with, by the same rule. */
export function seriesMark(index: number): ScatterMark {
  return SCATTER_MARKS[index % SCATTER_MARKS.length] as ScatterMark;
}
