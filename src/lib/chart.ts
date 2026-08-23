/**
 * A chart, from what a model wrote to where every mark goes.
 *
 * Two halves, and the split is the point. {@link readChart} turns a model's
 * JSON into a value that cannot be wrong: the shapes are discriminated by
 * chart type, so a pie with two series or a scatter with categories is not a
 * thing this file can produce. {@link layout} then turns that into
 * coordinates. Neither half touches the DOM, which is what makes a chart
 * testable in a suite where nothing has a size: jsdom performs no layout, so a
 * chart that worked out its own geometry from a measured element would be
 * exercised by no test in this repo and checked by nobody but the operator.
 *
 * The spec deliberately looks like the one every plotting library uses:
 * `type`, `labels`, `series: [{ name, data }]`. That is not a lack of
 * imagination. A model has seen that shape ten thousand times and writes it
 * correctly first try, and a novel schema of our own would be a thing every
 * agent has to be taught in a prompt it is already skimming.
 *
 * Coordinates are worked out against a fixed {@link WIDTH}, and the drawing is
 * scaled by CSS to whatever room it has. So a "pixel" here is roughly a real
 * one in a transcript and rather more than one in the full view, which is the
 * behavior a figure should have: opened larger, it is larger.
 */

import { MAX_SERIES } from "./palette";

export type ChartKind = "bar" | "line" | "area" | "pie" | "donut" | "scatter";

const KINDS: ChartKind[] = ["bar", "line", "area", "pie", "donut", "scatter"];

/** Everything a chart carries whatever shape it is. */
interface Common {
  title: string;
  /** Written before every number: a currency, usually. */
  prefix: string;
  /** Written after every number: a unit, usually. */
  unit: string;
  /** What the two axes are of. Empty is common and draws nothing. */
  captionX: string;
  captionY: string;
}

/** One line, one area, or one run of bars. */
export interface Series {
  name: string;
  /** One per category. `null` is a gap, which is not the same as a zero. */
  values: (number | null)[];
}

/** One wedge. */
export interface Slice {
  label: string;
  value: number;
}

/** One cloud of points. */
export interface Cloud {
  name: string;
  points: { x: number; y: number }[];
}

/**
 * A chart that has been checked.
 *
 * Discriminated on `kind` all the way down, so the renderer never asks whether
 * a pie has categories. The alternative, one wide record with everything
 * optional, puts that question at every use site and gets it wrong at one.
 */
export type Chart = CartesianChart | RadialChart | ScatterChart;

/** Anything with categories along one axis and a value scale up the other. */
export type CartesianChart = Common & {
  kind: "bar" | "line" | "area";
  labels: string[];
  series: Series[];
  stacked: boolean;
  /** Bars along the x axis instead of up it. What long names want. */
  horizontal: boolean;
};

/** One series against its own total. */
export type RadialChart = Common & { kind: "pie" | "donut"; slices: Slice[] };

/** Points with no categories at all. */
export type ScatterChart = Common & { kind: "scatter"; series: Cloud[] };

/**
 * Why a chart was refused, in a sentence the model can act on.
 *
 * Shown under the spec rather than instead of it: an operator looking at a
 * figure that did not draw needs to see what was asked for, and the agent that
 * wrote it needs to be told what to change. A red box saying "invalid chart" is
 * neither.
 */
export interface ChartFault {
  why: string;
}

export type ChartRead = { chart: Chart } | ChartFault;

export function isFault(read: ChartRead): read is ChartFault {
  return "why" in read;
}

/**
 * Which of the three families a chart belongs to.
 *
 * Predicates rather than comparisons at each use site, and not only for
 * tidiness: TypeScript will not narrow an intersection whose discriminant is
 * itself two literals, so `kind !== "pie" && kind !== "donut"` leaves the pie
 * arm in the type and every read of `series` after it is an error. These say
 * the same thing in a form the compiler acts on.
 */
export function isCartesian(chart: Chart): chart is CartesianChart {
  return chart.kind === "bar" || chart.kind === "line" || chart.kind === "area";
}

export function isRadial(chart: Chart): chart is RadialChart {
  return chart.kind === "pie" || chart.kind === "donut";
}

export function isScatter(chart: Chart): chart is ScatterChart {
  return chart.kind === "scatter";
}

/**
 * What a table says where a series has no value for a category.
 *
 * Words rather than a dash. In a column of numbers a dash reads as a minus
 * sign, and a screen reader announces it as punctuation or as nothing at all,
 * which is the one cell in the table where saying nothing is wrong: a gap and a
 * zero are different facts and this column is where the difference is read.
 */
const NO_READING = "no reading";

/** Part-to-whole stops being readable well before it stops being drawable. */
const MAX_SLICES = 6;

/** Under this share, a wedge is too thin to write beside without collisions. */
const LABELED_SHARE = 0.04;

/** The width every coordinate below is worked out against. */
export const WIDTH = 640;

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

function str(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * A number, or `null` for a gap.
 *
 * `null` and `0` are different facts and a chart that confuses them lies: a
 * month with no reading yet is not a month that sold nothing. Anything that is
 * neither is a gap too, because a model that emits `"12"` or `NaN` for one
 * point should still get the other eleven drawn.
 */
function num(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  return null;
}

/**
 * Turns a model's JSON into a chart, or says why it will not.
 *
 * Every refusal names the field and what to do about it. These are read by the
 * agent that wrote the spec, on a turn where it can still fix it, so "invalid
 * spec" costs a whole turn and teaches nothing.
 */
export function readChart(value: unknown): ChartRead {
  if (!isRecord(value)) {
    return { why: 'A chart must be a JSON object, like {"type": "bar", …}.' };
  }

  const kind = str(value.type).toLowerCase() as ChartKind;
  if (!KINDS.includes(kind)) {
    return {
      why: `"${str(value.type) || "(missing)"}" is not a chart type. Use one of: ${KINDS.join(", ")}.`,
    };
  }

  const common: Common = {
    title: str(value.title),
    prefix: str(value.prefix),
    unit: str(value.unit),
    captionX: str(value.x),
    captionY: str(value.y),
  };

  const raw = Array.isArray(value.series) ? value.series : [];
  if (raw.length === 0) {
    return { why: '"series" is required and must hold at least one series.' };
  }
  if (raw.length > MAX_SERIES && kind !== "pie" && kind !== "donut") {
    return {
      why: `${raw.length} series is more than a reader can tell apart. Keep the largest ${MAX_SERIES} and fold the rest into one called "Other".`,
    };
  }

  const labels = Array.isArray(value.labels) ? value.labels.map((label) => str(label)) : [];

  if (kind === "scatter") return readScatter(common, raw);
  if (kind === "pie" || kind === "donut") return readRadial(common, kind, labels, raw);
  return readCartesian(common, kind, labels, raw, value);
}

function readCartesian(
  common: Common,
  kind: "bar" | "line" | "area",
  labels: string[],
  raw: unknown[],
  value: Record<string, unknown>,
): ChartRead {
  const series: Series[] = [];
  for (const [at, entry] of raw.entries()) {
    if (!isRecord(entry)) return { why: `Series ${at + 1} must be an object with "data".` };
    const data = entry.data;
    if (!Array.isArray(data) || data.length === 0) {
      return { why: `Series ${at + 1} needs "data": an array of numbers, one per label.` };
    }
    series.push({ name: str(entry.name) || `Series ${at + 1}`, values: data.map(num) });
  }

  const width = Math.max(...series.map((one) => one.values.length));
  if (series.some((one) => one.values.every((point) => point === null))) {
    return { why: "A series whose values are all missing has nothing to draw." };
  }

  // Named rather than numbered where the model said nothing, because an axis
  // reading 1..12 under a chart of months is worse than one reading nothing.
  const named =
    labels.length === 0
      ? Array.from({ length: width }, (_, at) => String(at + 1))
      : labels.length === width
        ? labels
        : null;
  if (named === null) {
    return {
      why: `"labels" has ${labels.length} entries but the longest series has ${width}. They have to match: one label per point.`,
    };
  }

  // Ragged series are padded rather than refused. A model writing four
  // quarters of one year and three of the next has said something true, and a
  // refusal there loses the whole figure over the part it got right.
  for (const one of series) {
    while (one.values.length < width) one.values.push(null);
  }

  return {
    chart: {
      ...common,
      kind,
      labels: named,
      series,
      stacked: value.stacked === true,
      horizontal: value.horizontal === true,
    },
  };
}

function readRadial(
  common: Common,
  kind: "pie" | "donut",
  labels: string[],
  raw: unknown[],
): ChartRead {
  const first = raw[0];
  if (!isRecord(first) || !Array.isArray(first.data)) {
    return { why: `A ${kind} needs one series whose "data" holds one number per slice.` };
  }
  if (raw.length > 1) {
    return {
      why: `A ${kind} shows one series against its own total. For several series, use a stacked bar.`,
    };
  }

  const values = first.data.map(num);
  if (values.some((one) => one !== null && one < 0)) {
    return { why: `A ${kind} cannot show a negative share. Use a bar chart.` };
  }

  const named = values.map((_, at) => labels[at] ?? `Slice ${at + 1}`);
  let slices: Slice[] = values
    .map((one, at) => ({ label: named[at] as string, value: one ?? 0 }))
    .filter((slice) => slice.value > 0);

  if (slices.length === 0) return { why: `A ${kind} needs at least one slice above zero.` };

  // Folded rather than refused, which is the documented answer to too many
  // classes and is what the operator wanted anyway: past six wedges nobody is
  // comparing them, they are reading the big ones and the remainder.
  if (slices.length > MAX_SLICES) {
    const ranked = [...slices].sort((a, b) => b.value - a.value);
    const kept = ranked.slice(0, MAX_SLICES - 1);
    const rest = ranked.slice(MAX_SLICES - 1).reduce((sum, slice) => sum + slice.value, 0);
    slices = [...kept, { label: "Other", value: rest }];
  }

  return { chart: { ...common, kind, slices } };
}

function readScatter(common: Common, raw: unknown[]): ChartRead {
  const series: Cloud[] = [];
  for (const [at, entry] of raw.entries()) {
    if (!isRecord(entry) || !Array.isArray(entry.data)) {
      return { why: `Series ${at + 1} needs "data": an array of [x, y] pairs.` };
    }
    const points = entry.data
      .map((pair) => {
        if (!Array.isArray(pair)) return null;
        const x = num(pair[0]);
        const y = num(pair[1]);
        return x === null || y === null ? null : { x, y };
      })
      .filter((point): point is { x: number; y: number } => point !== null);
    if (points.length === 0) {
      return { why: `Series ${at + 1} has no usable points. Each one is a pair, like [1, 4.5].` };
    }
    series.push({ name: str(entry.name) || `Series ${at + 1}`, points });
  }
  return { chart: { ...common, kind: "scatter", series } };
}

// ---------------------------------------------------------------------------
// Numbers, as people read them
// ---------------------------------------------------------------------------

/**
 * A value, short enough to sit on a mark.
 *
 * Compacted above ten thousand, because an axis of `1,250,000` steals the
 * width the chart was drawn in, and rounded to the precision the number
 * actually has rather than to two decimals always: `4` should not draw as
 * `4.00`.
 */
export function formatValue(chart: Common, value: number, compact = true): string {
  const magnitude = Math.abs(value);
  let text: string;
  if (compact && magnitude >= 1_000_000) text = `${trim(value / 1_000_000)}M`;
  else if (compact && magnitude >= 10_000) text = `${trim(value / 1000)}K`;
  else text = trim(value).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `${chart.prefix}${text}${chart.unit}`;
}

function trim(value: number): string {
  const magnitude = Math.abs(value);
  const places = magnitude >= 100 ? 0 : magnitude >= 10 ? 1 : magnitude >= 1 ? 2 : 3;
  return String(Number(value.toFixed(places)));
}

/**
 * Round numbers a reader can do arithmetic with, spanning the data.
 *
 * The 1-2-5 rule, which is the only tick algorithm anybody recognizes: an axis
 * that steps by 37 is technically a fit and practically unreadable.
 */
export function niceTicks(low: number, high: number, wanted = 5): number[] {
  if (!(Number.isFinite(low) && Number.isFinite(high)) || high === low) {
    return [low || 0];
  }
  const rough = (high - low) / Math.max(1, wanted);
  const power = 10 ** Math.floor(Math.log10(rough));
  // 2.5 is in there because without it a range of 1.24M steps by 500K, which is
  // three gridlines for the whole chart. The classic set, and the reason every
  // axis you have ever read steps by one of these five.
  const step =
    [1, 2, 2.5, 5, 10].map((factor) => factor * power).find((size) => size >= rough) ?? power;
  const first = Math.floor(low / step) * step;
  const ticks: number[] = [];
  // Carried past the data rather than stopping under it. An axis whose top
  // gridline is below the tallest bar leaves that bar sticking out of its own
  // frame, which reads as a chart that did not finish drawing.
  //
  // Nudged at the comparison, because floating point turns three tenths into
  // 0.30000000000000004 and an axis that says so is an axis nobody trusts.
  for (let at = first; ; at += step) {
    ticks.push(Number(at.toFixed(10)));
    if (at >= high - step / 1000) break;
  }
  return ticks;
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/** The mark specs, which are fixed for every chart this app draws. */
export const MARK = {
  /** A bar never fills its band: the leftover is what separates two of them. */
  maxBarThickness: 24,
  /** The surface showing through is what separates touching marks. */
  gap: 2,
  /** Rounded at the end the data reaches, square where it leaves the baseline. */
  barRadius: 4,
  lineWidth: 2,
  dotRadius: 4.5,
  /** Big enough that a pointer aimed near a dot counts as aimed at it. */
  hitRadius: 16,
} as const;

export interface Frame {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface Tick {
  /** Along the value axis, in drawing coordinates. */
  at: number;
  text: string;
  value: number;
}

/** One category's slot, and the whole-height target that reads it out. */
export interface Band {
  /** Its own position, carried so a band is a value rather than a position. */
  index: number;
  label: string;
  /** Where the label and the crosshair sit. */
  center: number;
  /** The pointer target, which is the whole band and not the marks in it. */
  from: number;
  size: number;
}

export interface Bar {
  series: number;
  band: number;
  x: number;
  y: number;
  width: number;
  height: number;
  /** Which corners are rounded: the data end only. */
  round: "top" | "bottom" | "left" | "right";
  value: number;
}

export interface Point {
  series: number;
  band: number;
  x: number;
  y: number;
  value: number;
}

/** A run of points with no gap in it. `null` breaks a line rather than bridging it. */
export interface Stroke {
  series: number;
  line: string;
  /** The same run closed down to the baseline, for an area fill. */
  fill: string;
}

export interface DirectLabel {
  x: number;
  y: number;
  text: string;
  anchor: "start" | "middle" | "end";
  /** Set where the label sits on top of a filled mark and must not be ink. */
  onMark?: boolean;
}

export interface CartesianPlot {
  family: "cartesian";
  height: number;
  frame: Frame;
  horizontal: boolean;
  ticks: Tick[];
  /** Where zero sits, which is where every bar leaves from. */
  baseline: number;
  bands: Band[];
  bars: Bar[];
  strokes: Stroke[];
  dots: Point[];
  labels: DirectLabel[];
  /** Bands whose label is drawn. The rest are dropped rather than overlapped. */
  shown: number[];
}

export interface Wedge {
  slice: number;
  path: string;
  /** Written beside the wedge, or `null` where there is no room for one. */
  label: DirectLabel | null;
  share: number;
}

export interface RadialPlot {
  family: "radial";
  height: number;
  center: { x: number; y: number };
  radius: number;
  wedges: Wedge[];
  total: number;
}

export interface ScatterPlot {
  family: "scatter";
  height: number;
  frame: Frame;
  ticks: Tick[];
  across: Tick[];
  dots: Point[];
}

export type Plot = CartesianPlot | RadialPlot | ScatterPlot;

/** Room for the numbers down the side, the names along the bottom, and air. */
const PAD = { top: 16, right: 20, bottom: 34, left: 52 };

/**
 * More of it where a line writes its last value in the margin.
 *
 * The alternative is a number half off the edge of the card, which is the one
 * label a reader was most likely looking for.
 */
const END_LABEL_ROOM = 46;
const PLOT_HEIGHT = 250;

export function layout(chart: Chart): Plot {
  if (isRadial(chart)) return radial(chart);
  if (isScatter(chart)) return scatter(chart);
  return cartesian(chart);
}

/**
 * What the value axis has to span.
 *
 * Bars and areas are anchored to zero because their length is the quantity:
 * a bar chart from 90 to 100 makes a 2% difference look like everything. A
 * line is not, because a line encodes change and a temperature series forced
 * down to zero is a flat line across the top of the frame.
 */
function span(chart: CartesianChart): [number, number] {
  const stacks = stacked(chart);
  let low = Math.min(...stacks.flat());
  let high = Math.max(...stacks.flat());
  if (chart.kind !== "line") {
    low = Math.min(0, low);
    high = Math.max(0, high);
  }
  if (low === high) {
    // A flat series still needs a frame to sit in.
    const room = Math.abs(high) || 1;
    return [low - room / 2, high + room / 2];
  }
  return [low, high];
}

/** Every value the axis has to reach, which for a stack is the running total. */
function stacked(chart: CartesianChart): number[][] {
  if (!chart.stacked) {
    return chart.series.map((one) => one.values.filter((v): v is number => v !== null));
  }
  return chart.labels.map((_, band) => {
    let up = 0;
    let down = 0;
    for (const one of chart.series) {
      const value = one.values[band] ?? 0;
      if (value >= 0) up += value;
      else down += value;
    }
    return [up, down];
  });
}

function cartesian(chart: CartesianChart): CartesianPlot {
  const flip = chart.horizontal && chart.kind === "bar";
  const [low, high] = span(chart);
  const values = niceTicks(low, high);
  const min = Math.min(low, values[0] as number);
  const max = Math.max(high, values[values.length - 1] as number);

  // Horizontal bars give the names the left margin instead of the numbers, and
  // widen it: a category axis holds words, and the numbers axis holds numbers.
  const left = flip ? 92 : PAD.left;
  const right = chart.kind === "bar" ? PAD.right : END_LABEL_ROOM;
  const frame: Frame = {
    left,
    top: PAD.top,
    width: WIDTH - left - right,
    height: PLOT_HEIGHT,
  };

  /** A value, as a coordinate along whichever axis carries values. */
  const place = (value: number) => {
    const ratio = (value - min) / (max - min || 1);
    return flip ? frame.left + ratio * frame.width : frame.top + (1 - ratio) * frame.height;
  };

  const ticks: Tick[] = values.map((value) => ({
    at: place(value),
    text: formatValue(chart, value),
    value,
  }));
  const baseline = place(Math.min(Math.max(0, min), max));

  const count = chart.labels.length;
  const size = (flip ? frame.height : frame.width) / Math.max(1, count);
  const origin = flip ? frame.top : frame.left;
  const bands: Band[] = chart.labels.map((label, at) => ({
    index: at,
    label,
    center: origin + size * (at + 0.5),
    from: origin + size * at,
    size,
  }));

  const bars: Bar[] = [];
  const dots: Point[] = [];
  const strokes: Stroke[] = [];
  const labels: DirectLabel[] = [];

  if (chart.kind === "bar") {
    bars.push(...barMarks(chart, bands, place, flip));
    labels.push(...barLabels(chart, bars, flip));
  } else {
    for (const [at, one] of chart.series.entries()) {
      // Stacked, a series is drawn at the height of everything up to and
      // including it, but it still *reports* its own value: the readout and the
      // table both say what this series was worth, and a label saying something
      // else is the chart contradicting itself two inches lower down.
      const running = chart.stacked ? runningTotals(chart, at) : one.values;
      const points = running.map((value, band) =>
        value === null
          ? null
          : {
              x: bands[band]?.center ?? 0,
              y: place(value),
              value: one.values[band] ?? value,
              band,
            },
      );

      // And its fill stops at the series underneath rather than at zero. Every
      // band drawn down to the axis is every band drawn over the one before it,
      // which at a tenth opacity is a stack whose colors are all mixtures.
      const under = chart.stacked && at > 0 ? runningTotals(chart, at - 1) : null;
      const floorAt = (band: number) => {
        const below = under?.[band];
        return below === null || below === undefined ? baseline : place(below);
      };

      strokes.push(...strokeRuns(at, points, floorAt));
      for (const point of points) {
        if (point) {
          dots.push({ series: at, band: point.band, x: point.x, y: point.y, value: point.value });
        }
      }
      labels.push(...endLabel(chart, points));
    }
  }

  return {
    family: "cartesian",
    height: frame.top + frame.height + PAD.bottom,
    frame,
    horizontal: flip,
    ticks,
    baseline,
    bands,
    bars,
    strokes,
    dots,
    labels,
    shown: showable(bands, flip),
  };
}

/** A stacked series sits on top of the ones before it. */
function runningTotals(chart: CartesianChart, upTo: number): (number | null)[] {
  return chart.labels.map((_, band) => {
    let sum = 0;
    let seen = false;
    for (let at = 0; at <= upTo; at++) {
      const value = chart.series[at]?.values[band];
      if (value === null || value === undefined) continue;
      seen = true;
      sum += value;
    }
    return seen ? sum : null;
  });
}

function barMarks(
  chart: CartesianChart,
  bands: Band[],
  place: (value: number) => number,
  flip: boolean,
): Bar[] {
  const bars: Bar[] = [];
  const lanes = chart.stacked ? 1 : chart.series.length;
  const band = bands[0]?.size ?? 0;
  // Never the whole band: the air left over is what separates one category
  // from the next, and it is doing that job better than a stroke would.
  const room = band * 0.72;
  const thickness = Math.max(
    1,
    Math.min(MARK.maxBarThickness, (room - MARK.gap * (lanes - 1)) / lanes),
  );
  const runWidth = thickness * lanes + MARK.gap * (lanes - 1);

  // Categories outside, series inside, because a stack's running total belongs
  // to the category. Written the other way round each series starts again from
  // the baseline, and a stacked chart draws every series on top of the axis
  // rather than on top of the one before it.
  for (const [at, slot] of bands.entries()) {
    let up = 0;
    let down = 0;
    for (const [index, one] of chart.series.entries()) {
      const value = one.values[at];
      if (value === null || value === undefined) continue;

      const lane = chart.stacked ? 0 : index;
      const offset = slot.center - runWidth / 2 + lane * (thickness + MARK.gap);
      const from = chart.stacked ? (value >= 0 ? up : down) : 0;
      const to = from + value;
      if (chart.stacked) {
        if (value >= 0) up = to;
        else down = to;
      }

      const a = place(from);
      const b = place(to);
      const near = Math.min(a, b);
      const length = Math.abs(a - b);
      // Trimmed by the surface gap so touching segments never share an edge, and
      // trimmed off the end nearest the baseline so the far end stays where the
      // value says it is. A stroke around each segment would add ink that is not
      // data; this removes some instead. Skipped on a segment too short to lose
      // it, which would otherwise vanish entirely.
      const trim = chart.stacked && length > MARK.gap * 2 ? MARK.gap : 0;
      const rising = value >= 0;

      bars.push(
        flip
          ? {
              series: index,
              band: at,
              x: near + (rising ? trim : 0),
              y: offset,
              width: Math.max(1, length - trim),
              height: thickness,
              round: rising ? "right" : "left",
              value,
            }
          : {
              series: index,
              band: at,
              x: offset,
              y: near + (rising ? 0 : trim),
              width: thickness,
              height: Math.max(1, length - trim),
              round: rising ? "top" : "bottom",
              value,
            },
      );
    }
  }
  return bars;
}

/**
 * The values written on the marks.
 *
 * One series only, and only while they fit. A number on every bar of a grouped
 * chart is a wall of digits nobody reads, and the tooltip and the table below
 * already hold every value. On a single series it is the opposite: the numbers
 * are the point, and they are also what makes a pale fill legible on paper.
 */
function barLabels(chart: CartesianChart, bars: Bar[], flip: boolean): DirectLabel[] {
  if (chart.series.length !== 1 || bars.length > 14) return [];
  return bars.map((bar) => {
    const text = formatValue(chart, bar.value);
    if (flip) {
      const outside = bar.round === "right";
      return {
        x: outside ? bar.x + bar.width + 5 : bar.x - 5,
        y: bar.y + bar.height / 2 + 4,
        text,
        anchor: outside ? "start" : "end",
      };
    }
    const above = bar.round === "top";
    return {
      x: bar.x + bar.width / 2,
      y: above ? bar.y - 6 : bar.y + bar.height + 13,
      text,
      anchor: "middle",
    };
  });
}

/** The value at the end of a line, which is the one a reader looks for. */
function endLabel(
  chart: CartesianChart,
  points: ({ x: number; y: number; value: number } | null)[],
): DirectLabel[] {
  if (chart.series.length > 4) return [];
  const last = [...points].reverse().find((point) => point !== null);
  if (!last) return [];
  return [
    {
      x: last.x + 7,
      y: last.y + 4,
      text: formatValue(chart, last.value),
      anchor: "start",
    },
  ];
}

/**
 * A line, broken wherever the data is.
 *
 * One path per run of present values rather than one per series, because a
 * missing month drawn as a straight line between the months either side of it
 * is the chart inventing a reading.
 */
function strokeRuns(
  series: number,
  points: ({ x: number; y: number; band: number } | null)[],
  floorAt: (band: number) => number,
): Stroke[] {
  const strokes: Stroke[] = [];
  let run: { x: number; y: number; band: number }[] = [];
  const close = () => {
    if (run.length === 0) return;
    const line = run.map((point, at) => `${at === 0 ? "M" : "L"}${point.x} ${point.y}`).join("");
    // Back along the floor rather than straight across it, so a fill follows
    // the shape of whatever it is sitting on.
    const floor = [...run]
      .reverse()
      .map((point) => `L${point.x} ${floorAt(point.band)}`)
      .join("");
    strokes.push({ series, line, fill: `${line}${floor}Z` });
    run = [];
  };
  for (const point of points) {
    if (point) run.push(point);
    else close();
  }
  close();
  return strokes;
}

/**
 * Which category names are drawn.
 *
 * Every one that fits, and then every other one, and so on. A label is never
 * shrunk, rotated or clipped to make it fit: rotated axis text is unreadable
 * and a clipped one drops the characters that told them apart. Dropping whole
 * labels at an even interval keeps the axis honest about what it is showing.
 */
function showable(bands: Band[], flip: boolean): number[] {
  if (bands.length === 0) return [];
  const size = bands[0]?.size ?? 0;
  const widest = Math.max(...bands.map((band) => band.label.length));
  // Roughly, for the axis type size. Measuring properly needs a laid-out
  // document, which the suite does not have and a first paint does not either.
  const needed = flip ? 14 : widest * 5.6 + 8;
  const every = Math.max(1, Math.ceil(needed / Math.max(1, size)));
  return bands.map((_, at) => at).filter((at) => at % every === 0);
}

function radial(chart: RadialChart): RadialPlot {
  const total = chart.slices.reduce((sum, slice) => sum + slice.value, 0);
  const height = 260;
  const center = { x: WIDTH / 2, y: height / 2 };
  const radius = 96;
  const inner = chart.kind === "donut" ? radius * 0.58 : 0;

  // Two pixels of surface between wedges, expressed as the angle that subtends
  // at the rim. The same separator every other mark in this file uses.
  const pad = Math.min(0.06, MARK.gap / radius);

  let angle = -Math.PI / 2;
  const wedges: Wedge[] = chart.slices.map((slice, at) => {
    const share = total > 0 ? slice.value / total : 0;
    const sweep = share * Math.PI * 2;
    const from = angle + pad / 2;
    const to = angle + sweep - pad / 2;
    angle += sweep;
    const middle = (from + to) / 2;
    const out = radius + 16;
    const right = Math.cos(middle) >= 0;
    return {
      slice: at,
      path: arc(center, radius, inner, from, Math.max(from, to)),
      share,
      // A one percent wedge is a sliver, and two of them side by side put two
      // labels in the same place: the shot of that reads "Partne Other 1%".
      // Under the threshold the legend and the table carry the name, which they
      // were already doing, and nothing is written on top of anything.
      label:
        share < LABELED_SHARE
          ? null
          : {
              x: center.x + Math.cos(middle) * out,
              y: center.y + Math.sin(middle) * out + 4,
              text: `${slice.label} ${Math.round(share * 100)}%`,
              anchor: right ? "start" : "end",
            },
    };
  });

  return { family: "radial", height, center, radius, wedges, total };
}

function arc(
  center: { x: number; y: number },
  outer: number,
  inner: number,
  from: number,
  to: number,
): string {
  const point = (radius: number, angle: number) => ({
    x: center.x + Math.cos(angle) * radius,
    y: center.y + Math.sin(angle) * radius,
  });
  const big = to - from > Math.PI ? 1 : 0;
  const a = point(outer, from);
  const b = point(outer, to);
  // A full circle has no arc to draw: start and end land on the same point and
  // the path collapses to nothing. Two half arcs is the standard way round it.
  if (to - from >= Math.PI * 2 - 1e-6) {
    const half = point(outer, from + Math.PI);
    const ring = `M${a.x} ${a.y}A${outer} ${outer} 0 1 1 ${half.x} ${half.y}A${outer} ${outer} 0 1 1 ${a.x} ${a.y}Z`;
    if (inner <= 0) return ring;
    const c = point(inner, from);
    const d = point(inner, from + Math.PI);
    return `${ring}M${c.x} ${c.y}A${inner} ${inner} 0 1 0 ${d.x} ${d.y}A${inner} ${inner} 0 1 0 ${c.x} ${c.y}Z`;
  }
  if (inner <= 0) {
    return `M${center.x} ${center.y}L${a.x} ${a.y}A${outer} ${outer} 0 ${big} 1 ${b.x} ${b.y}Z`;
  }
  const c = point(inner, to);
  const d = point(inner, from);
  return `M${a.x} ${a.y}A${outer} ${outer} 0 ${big} 1 ${b.x} ${b.y}L${c.x} ${c.y}A${inner} ${inner} 0 ${big} 0 ${d.x} ${d.y}Z`;
}

function scatter(chart: ScatterChart): ScatterPlot {
  const all = chart.series.flatMap((one) => one.points);
  const xs = all.map((point) => point.x);
  const ys = all.map((point) => point.y);
  const acrossTicks = niceTicks(Math.min(...xs), Math.max(...xs), 6);
  const upTicks = niceTicks(Math.min(...ys), Math.max(...ys));

  const frame: Frame = {
    left: PAD.left,
    top: PAD.top,
    width: WIDTH - PAD.left - PAD.right,
    height: PLOT_HEIGHT,
  };
  const range = (ticks: number[], values: number[]) => {
    const low = Math.min(ticks[0] as number, ...values);
    const high = Math.max(ticks[ticks.length - 1] as number, ...values);
    return [low, high - low || 1] as const;
  };
  const [xLow, xSpan] = range(acrossTicks, xs);
  const [yLow, ySpan] = range(upTicks, ys);
  const toX = (value: number) => frame.left + ((value - xLow) / xSpan) * frame.width;
  const toY = (value: number) => frame.top + (1 - (value - yLow) / ySpan) * frame.height;

  const dots: Point[] = chart.series.flatMap((one, series) =>
    one.points.map((point, band) => ({
      series,
      band,
      x: toX(point.x),
      y: toY(point.y),
      value: point.y,
    })),
  );

  return {
    family: "scatter",
    height: frame.top + frame.height + PAD.bottom,
    frame,
    ticks: upTicks.map((value) => ({ at: toY(value), text: formatValue(chart, value), value })),
    across: acrossTicks.map((value) => ({
      at: toX(value),
      text: formatValue(chart, value),
      value,
    })),
    dots,
  };
}

/**
 * A bar's outline, rounded only where the data ends.
 *
 * Square at the baseline on purpose: a bar rounded at both ends floats, and
 * every bar in the chart appears to start slightly above the axis it is
 * measured from.
 */
export function barPath(bar: Bar): string {
  const { x, y, width: w, height: h } = bar;
  const r = Math.min(MARK.barRadius, w / 2, h / 2);
  if (r <= 0.5) return `M${x} ${y}h${w}v${h}h${-w}Z`;
  switch (bar.round) {
    case "top":
      return `M${x} ${y + h}V${y + r}A${r} ${r} 0 0 1 ${x + r} ${y}H${x + w - r}A${r} ${r} 0 0 1 ${x + w} ${y + r}V${y + h}Z`;
    case "bottom":
      return `M${x} ${y}V${y + h - r}A${r} ${r} 0 0 0 ${x + r} ${y + h}H${x + w - r}A${r} ${r} 0 0 0 ${x + w} ${y + h - r}V${y}Z`;
    case "right":
      return `M${x} ${y}H${x + w - r}A${r} ${r} 0 0 1 ${x + w} ${y + r}V${y + h - r}A${r} ${r} 0 0 1 ${x + w - r} ${y + h}H${x}Z`;
    default:
      return `M${x + w} ${y}H${x + r}A${r} ${r} 0 0 0 ${x} ${y + r}V${y + h - r}A${r} ${r} 0 0 0 ${x + r} ${y + h}H${x + w}Z`;
  }
}

/**
 * The chart as its own numbers.
 *
 * Every figure has one, and it is not a fallback. It is how a value is read by
 * a screen reader, by an operator who cannot separate two of the hues, and by
 * anybody who wants the number rather than the shape. A tooltip may never be
 * the only way to find out what a mark is worth.
 */
export function chartTable(chart: Chart): { head: string[]; rows: string[][] } {
  if (isRadial(chart)) {
    const total = chart.slices.reduce((sum, slice) => sum + slice.value, 0) || 1;
    return {
      head: [chart.captionX || "Slice", chart.captionY || "Value", "Share"],
      rows: chart.slices.map((slice) => [
        slice.label,
        formatValue(chart, slice.value, false),
        `${Math.round((slice.value / total) * 100)}%`,
      ]),
    };
  }
  if (isScatter(chart)) {
    return {
      head: ["Series", chart.captionX || "x", chart.captionY || "y"],
      rows: chart.series.flatMap((one) =>
        one.points.map((point) => [
          one.name,
          formatValue(chart, point.x, false),
          formatValue(chart, point.y, false),
        ]),
      ),
    };
  }
  return {
    head: [chart.captionX || "", ...chart.series.map((one) => one.name)],
    rows: chart.labels.map((label, at) => [
      label,
      ...chart.series.map((one) => {
        const value = one.values[at];
        return value === null || value === undefined
          ? NO_READING
          : formatValue(chart, value, false);
      }),
    ]),
  };
}

/** What the series are called, for a legend and for a readout. */
export function chartSeriesNames(chart: Chart): string[] {
  if (isRadial(chart)) return chart.slices.map((slice) => slice.label);
  return chart.series.map((one) => one.name);
}

/**
 * What a chart is, said in one sentence.
 *
 * The whole of what a screen reader gets from the drawing itself, which is why
 * it says what is plotted rather than "chart": the numbers are in the table
 * underneath, and repeating them here would be a hundred announcements nobody
 * asked for.
 */
export function chartSummary(chart: Chart): string {
  const names = chartSeriesNames(chart);
  const what = chart.title || `${chart.kind} chart`;
  const of = isRadial(chart)
    ? `${names.length} slices`
    : `${names.length} series: ${names.join(", ")}`;
  return `${what}. A ${chart.kind} chart of ${of}. The figures are in the table below.`;
}
