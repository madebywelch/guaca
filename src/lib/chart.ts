import { MAX_SERIES } from "./palette";

export type ChartKind = "bar" | "line" | "area" | "pie" | "donut" | "scatter";

const KINDS: ChartKind[] = ["bar", "line", "area", "pie", "donut", "scatter"];

interface Common {
  title: string;
  prefix: string;
  unit: string;
  captionX: string;
  captionY: string;
}

export interface Series {
  name: string;
  values: (number | null)[];
}

export interface Slice {
  label: string;
  value: number;
}

export interface Cloud {
  name: string;
  points: { x: number; y: number }[];
}

export type Chart = CartesianChart | RadialChart | ScatterChart;

export type CartesianChart = Common & {
  kind: "bar" | "line" | "area";
  labels: string[];
  series: Series[];
  stacked: boolean;
  horizontal: boolean;
};

export type RadialChart = Common & { kind: "pie" | "donut"; slices: Slice[] };

export type ScatterChart = Common & { kind: "scatter"; series: Cloud[] };

export interface ChartFault {
  why: string;
}

export type ChartRead = { chart: Chart } | ChartFault;

export function isFault(read: ChartRead): read is ChartFault {
  return "why" in read;
}

export function isCartesian(chart: Chart): chart is CartesianChart {
  return chart.kind === "bar" || chart.kind === "line" || chart.kind === "area";
}

export function isRadial(chart: Chart): chart is RadialChart {
  return chart.kind === "pie" || chart.kind === "donut";
}

export function isScatter(chart: Chart): chart is ScatterChart {
  return chart.kind === "scatter";
}

const NO_READING = "no reading";

const MAX_SLICES = 6;

const LABELED_SHARE = 0.04;

export const WIDTH = 640;

function str(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function num(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  return null;
}

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

export function niceTicks(low: number, high: number, wanted = 5): number[] {
  if (!(Number.isFinite(low) && Number.isFinite(high)) || high === low) {
    return [low || 0];
  }
  const rough = (high - low) / Math.max(1, wanted);
  const power = 10 ** Math.floor(Math.log10(rough));
  const step =
    [1, 2, 2.5, 5, 10].map((factor) => factor * power).find((size) => size >= rough) ?? power;
  const first = Math.floor(low / step) * step;
  const ticks: number[] = [];
  for (let at = first; ; at += step) {
    ticks.push(Number(at.toFixed(10)));
    if (at >= high - step / 1000) break;
  }
  return ticks;
}

export const MARK = {
  maxBarThickness: 24,
  gap: 2,
  barRadius: 4,
  lineWidth: 2,
  dotRadius: 4.5,
  hitRadius: 16,
} as const;

export interface Frame {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface Tick {
  at: number;
  text: string;
  value: number;
}

export interface Band {
  index: number;
  label: string;
  center: number;
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

export interface Stroke {
  series: number;
  line: string;
  fill: string;
}

export interface DirectLabel {
  x: number;
  y: number;
  text: string;
  anchor: "start" | "middle" | "end";
  onMark?: boolean;
}

export interface CartesianPlot {
  family: "cartesian";
  height: number;
  frame: Frame;
  horizontal: boolean;
  ticks: Tick[];
  baseline: number;
  bands: Band[];
  bars: Bar[];
  strokes: Stroke[];
  dots: Point[];
  labels: DirectLabel[];
  shown: number[];
}

export interface Wedge {
  slice: number;
  path: string;
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

const PAD = { top: 16, right: 20, bottom: 34, left: 52 };

const END_LABEL_ROOM = 46;
const PLOT_HEIGHT = 250;

export function layout(chart: Chart): Plot {
  if (isRadial(chart)) return radial(chart);
  if (isScatter(chart)) return scatter(chart);
  return cartesian(chart);
}

function span(chart: CartesianChart): [number, number] {
  const stacks = stacked(chart);
  let low = Math.min(...stacks.flat());
  let high = Math.max(...stacks.flat());
  if (chart.kind !== "line") {
    low = Math.min(0, low);
    high = Math.max(0, high);
  }
  if (low === high) {
    const room = Math.abs(high) || 1;
    return [low - room / 2, high + room / 2];
  }
  return [low, high];
}

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

  const left = flip ? 92 : PAD.left;
  const right = chart.kind === "bar" ? PAD.right : END_LABEL_ROOM;
  const frame: Frame = {
    left,
    top: PAD.top,
    width: WIDTH - left - right,
    height: PLOT_HEIGHT,
  };

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
  const room = band * 0.72;
  const thickness = Math.max(
    1,
    Math.min(MARK.maxBarThickness, (room - MARK.gap * (lanes - 1)) / lanes),
  );
  const runWidth = thickness * lanes + MARK.gap * (lanes - 1);

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

function showable(bands: Band[], flip: boolean): number[] {
  if (bands.length === 0) return [];
  const size = bands[0]?.size ?? 0;
  const widest = Math.max(...bands.map((band) => band.label.length));
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

export function chartSeriesNames(chart: Chart): string[] {
  if (isRadial(chart)) return chart.slices.map((slice) => slice.label);
  return chart.series.map((one) => one.name);
}

export function chartSummary(chart: Chart): string {
  const names = chartSeriesNames(chart);
  const what = chart.title || `${chart.kind} chart`;
  const of = isRadial(chart)
    ? `${names.length} slices`
    : `${names.length} series: ${names.join(", ")}`;
  return `${what}. A ${chart.kind} chart of ${of}. The figures are in the table below.`;
}
