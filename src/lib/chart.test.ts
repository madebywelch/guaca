import { describe, expect, it } from "vitest";

import {
  barPath,
  type CartesianPlot,
  type Chart,
  chartTable,
  formatValue,
  isFault,
  layout,
  MARK,
  niceTicks,
  type RadialPlot,
  readChart,
} from "./chart";

/** Reads a spec that is expected to be good, and hands back the chart. */
function good(value: unknown): Chart {
  const read = readChart(value);
  if (isFault(read)) throw new Error(`expected a chart, got: ${read.why}`);
  return read.chart;
}

/** Reads one that is expected to be refused, and hands back the sentence. */
function refused(value: unknown): string {
  const read = readChart(value);
  if (!isFault(read)) throw new Error("expected a refusal");
  return read.why;
}

const bars = {
  type: "bar",
  title: "Revenue",
  labels: ["Q1", "Q2", "Q3"],
  series: [{ name: "2026", data: [10, 20, 15] }],
};

describe("reading a spec", () => {
  it("reads the shape every plotting library uses", () => {
    const chart = good(bars);
    expect(chart.kind).toBe("bar");
    expect(chart.title).toBe("Revenue");
    if (chart.kind === "bar") {
      expect(chart.labels).toEqual(["Q1", "Q2", "Q3"]);
      expect(chart.series[0]?.values).toEqual([10, 20, 15]);
    }
  });

  it("names the field and the fix in every refusal", () => {
    // These are read by the agent that wrote the spec, on a turn where it can
    // still put it right. "Invalid chart" costs a whole turn and teaches it
    // nothing, so every one of these has to say what to do.
    expect(refused({ type: "sunburst", series: [{ data: [1] }] })).toContain("bar, line");
    expect(refused({ type: "bar" })).toContain('"series" is required');
    expect(refused({ type: "bar", series: [{ name: "a" }] })).toContain('needs "data"');
    expect(refused("nope")).toContain("JSON object");
  });

  it("keeps a gap a gap rather than calling it zero", () => {
    // A month with no reading is not a month that sold nothing, and a chart
    // that draws the second when it was told the first is inventing data.
    const chart = good({ type: "line", labels: ["a", "b", "c"], series: [{ data: [1, null, 3] }] });
    if (chart.kind !== "line") throw new Error("wrong kind");
    expect(chart.series[0]?.values).toEqual([1, null, 3]);
  });

  it("refuses more series than a reader can tell apart, and says what to do", () => {
    const many = Array.from({ length: 9 }, (_, at) => ({ name: `s${at}`, data: [1] }));
    expect(refused({ type: "bar", labels: ["a"], series: many })).toContain("Other");
  });

  it("refuses labels that do not line up with the data", () => {
    // Silently padding would slide every point one category to the left, which
    // is a chart that is confidently wrong rather than absent.
    expect(refused({ type: "bar", labels: ["a", "b"], series: [{ data: [1, 2, 3] }] })).toContain(
      "one label per point",
    );
  });

  it("numbers the categories when nobody named them", () => {
    const chart = good({ type: "bar", series: [{ data: [1, 2] }] });
    if (chart.kind !== "bar") throw new Error("wrong kind");
    expect(chart.labels).toEqual(["1", "2"]);
  });

  it("pads a short series rather than losing the whole figure", () => {
    const chart = good({
      type: "bar",
      labels: ["a", "b", "c"],
      series: [{ data: [1, 2, 3] }, { data: [4, 5] }],
    });
    if (chart.kind !== "bar") throw new Error("wrong kind");
    expect(chart.series[1]?.values).toEqual([4, 5, null]);
  });
});

describe("reading a pie", () => {
  it("takes one series and turns it into slices", () => {
    const chart = good({ type: "pie", labels: ["a", "b"], series: [{ data: [3, 1] }] });
    if (chart.kind !== "pie") throw new Error("wrong kind");
    expect(chart.slices).toEqual([
      { label: "a", value: 3 },
      { label: "b", value: 1 },
    ]);
  });

  it("sends several series to a stacked bar instead", () => {
    expect(refused({ type: "pie", series: [{ data: [1] }, { data: [2] }] })).toContain(
      "stacked bar",
    );
  });

  it("refuses a negative share, which a circle cannot show", () => {
    expect(refused({ type: "pie", series: [{ data: [1, -1] }] })).toContain("negative");
  });

  it("folds a long tail into Other rather than refusing", () => {
    // Past six wedges nobody is comparing them: they are reading the big ones
    // and the remainder. Folding gives them exactly that; refusing gives them
    // nothing.
    const chart = good({
      type: "pie",
      labels: ["a", "b", "c", "d", "e", "f", "g", "h"],
      series: [{ data: [10, 9, 8, 7, 6, 5, 4, 3] }],
    });
    if (chart.kind !== "pie") throw new Error("wrong kind");
    expect(chart.slices).toHaveLength(6);
    expect(chart.slices[5]).toEqual({ label: "Other", value: 5 + 4 + 3 });
  });

  it("drops slices worth nothing", () => {
    const chart = good({ type: "pie", labels: ["a", "b"], series: [{ data: [1, 0] }] });
    if (chart.kind !== "pie") throw new Error("wrong kind");
    expect(chart.slices).toHaveLength(1);
  });
});

describe("reading a scatter", () => {
  it("takes pairs", () => {
    const chart = good({
      type: "scatter",
      series: [
        {
          name: "runs",
          data: [
            [1, 2],
            [3, 4],
          ],
        },
      ],
    });
    if (chart.kind !== "scatter") throw new Error("wrong kind");
    expect(chart.series[0]?.points).toEqual([
      { x: 1, y: 2 },
      { x: 3, y: 4 },
    ]);
  });

  it("says what a pair looks like when it was given something else", () => {
    expect(refused({ type: "scatter", series: [{ data: [1, 2, 3] }] })).toContain("[1, 4.5]");
  });
});

describe("ticks", () => {
  it("steps by numbers a reader can do arithmetic with", () => {
    expect(niceTicks(0, 100)).toEqual([0, 20, 40, 60, 80, 100]);
    expect(niceTicks(0, 10)).toEqual([0, 2, 4, 6, 8, 10]);
  });

  it("reaches past the data rather than stopping under it", () => {
    // An axis whose top gridline is below the tallest bar leaves that bar
    // sticking out of its own frame, which reads as a chart that did not
    // finish drawing.
    for (const [low, high] of [
      [0, 1_240_000],
      [0, 97],
      [0, 3],
      [-40, 110],
    ] as const) {
      const ticks = niceTicks(low, high);
      expect(ticks[ticks.length - 1], `${low}..${high}`).toBeGreaterThanOrEqual(high);
      expect(ticks[0], `${low}..${high}`).toBeLessThanOrEqual(low);
    }
  });

  it("offers a quarter step, so a wide range is not three gridlines", () => {
    // Without 2.5 in the set, 1.24M steps by 500K and the whole chart gets
    // three lines to read against.
    expect(niceTicks(0, 1_240_000)).toEqual([0, 250_000, 500_000, 750_000, 1_000_000, 1_250_000]);
  });

  it("does not produce 0.30000000000000004", () => {
    // Floating point turns a tenth into seventeen digits, and an axis that
    // says so is an axis nobody trusts.
    for (const tick of niceTicks(0, 0.5)) {
      expect(String(tick).length).toBeLessThan(6);
    }
  });

  it("survives a series that never changes", () => {
    expect(() => niceTicks(5, 5)).not.toThrow();
  });
});

describe("formatting a value", () => {
  it("compacts what would otherwise eat the chart", () => {
    expect(
      formatValue({ prefix: "", unit: "", title: "", captionX: "", captionY: "" }, 1_250_000),
    ).toBe("1.25M");
    expect(
      formatValue({ prefix: "", unit: "", title: "", captionX: "", captionY: "" }, 12_500),
    ).toBe("12.5K");
  });

  it("does not write 4 as 4.00", () => {
    const plain = { prefix: "", unit: "", title: "", captionX: "", captionY: "" };
    expect(formatValue(plain, 4)).toBe("4");
    expect(formatValue(plain, 1234)).toBe("1,234");
  });

  it("wears the unit it was given", () => {
    expect(formatValue({ prefix: "$", unit: "", title: "", captionX: "", captionY: "" }, 12)).toBe(
      "$12",
    );
    expect(formatValue({ prefix: "", unit: "%", title: "", captionX: "", captionY: "" }, 12)).toBe(
      "12%",
    );
  });

  it("keeps the full number in the table, where the room is", () => {
    const chart = good({ type: "bar", labels: ["a"], series: [{ data: [1_250_000] }] });
    expect(chartTable(chart).rows[0]?.[1]).toBe("1,250,000");
  });
});

describe("laying a bar chart out", () => {
  const plot = layout(good(bars)) as CartesianPlot;

  it("anchors bars to zero, because their length is the quantity", () => {
    // A bar chart from 90 to 100 makes a two percent difference look like
    // everything there is.
    const tall = layout(good({ ...bars, series: [{ data: [90, 95, 100] }] })) as CartesianPlot;
    expect(tall.ticks[0]?.value).toBe(0);
  });

  it("lets a line float, because a line encodes change", () => {
    const line = layout(
      good({ type: "line", labels: ["a", "b"], series: [{ data: [90, 100] }] }),
    ) as CartesianPlot;
    expect(line.ticks[0]?.value).toBeGreaterThan(0);
  });

  it("never lets a bar fill its band", () => {
    // The air left over is what separates one category from the next, and it
    // does that job better than a stroke around each bar would.
    const band = plot.bands[0]?.size ?? 0;
    for (const bar of plot.bars) expect(bar.width).toBeLessThan(band);
  });

  it("caps how thick a bar gets however few there are", () => {
    const two = layout(good({ type: "bar", labels: ["a"], series: [{ data: [1] }] }));
    const [only] = (two as CartesianPlot).bars;
    expect(only?.width).toBeLessThanOrEqual(MARK.maxBarThickness);
  });

  it("puts surface between the segments of a stack", () => {
    const stack = layout(
      good({
        type: "bar",
        stacked: true,
        labels: ["a"],
        series: [{ data: [50] }, { data: [50] }],
      }),
    ) as CartesianPlot;
    const [lower, upper] = stack.bars;
    if (!lower || !upper) throw new Error("expected two segments");
    // The upper one ends short of where the lower one starts.
    expect(lower.y - (upper.y + upper.height)).toBeGreaterThanOrEqual(MARK.gap - 0.001);
  });

  it("writes the value on every bar of a single series and on none of a group", () => {
    // On one series the numbers are the point, and they are also what makes a
    // pale fill readable. On a group they are a wall of digits nobody reads.
    expect(plot.labels).toHaveLength(3);
    const grouped = layout(
      good({ ...bars, series: [{ data: [1, 2, 3] }, { data: [4, 5, 6] }] }),
    ) as CartesianPlot;
    expect(grouped.labels).toEqual([]);
  });

  it("drops whole category names rather than overlapping them", () => {
    // A label is never shrunk, rotated or clipped to fit: rotated axis text is
    // unreadable and a clipped one loses what told two categories apart.
    const many = layout(
      good({
        type: "bar",
        labels: Array.from({ length: 40 }, (_, at) => `week ${at + 1}`),
        series: [{ data: Array.from({ length: 40 }, () => 1) }],
      }),
    ) as CartesianPlot;
    expect(many.shown.length).toBeLessThan(40);
    expect(many.shown.length).toBeGreaterThan(0);
  });

  it("stops a stacked fill at the series under it, not at the axis", () => {
    // Every band drawn down to zero is every band drawn over the one before
    // it, which at a tenth opacity makes a stack whose colours are all
    // mixtures of each other.
    const stack = layout(
      good({
        type: "area",
        stacked: true,
        labels: ["a", "b"],
        series: [
          { name: "lower", data: [10, 10] },
          { name: "upper", data: [10, 10] },
        ],
      }),
    ) as CartesianPlot;
    const [lower, upper] = stack.strokes;
    if (!lower || !upper) throw new Error("expected two fills");

    // The lower one closes on the baseline. The upper one closes on the line
    // the lower one drew, and never reaches the axis at all.
    const floor = (stroke: { line: string; fill: string }) =>
      [...stroke.fill.slice(stroke.line.length).matchAll(/L[\d.]+ ([\d.]+)/g)].map((at) =>
        Number(at[1]),
      );
    const lowerLine = Number(lower.line.match(/M[\d.]+ ([\d.]+)/)?.[1]);

    expect(new Set(floor(lower))).toEqual(new Set([stack.baseline]));
    expect(new Set(floor(upper))).toEqual(new Set([lowerLine]));
  });

  it("labels a stacked series with its own value, not the running total", () => {
    // The readout and the table both say what this series was worth, and a
    // label saying something else is the chart contradicting itself two inches
    // lower down.
    const stack = layout(
      good({
        type: "area",
        stacked: true,
        labels: ["a"],
        series: [
          { name: "lower", data: [30] },
          { name: "upper", data: [12] },
        ],
      }),
    ) as CartesianPlot;
    expect(stack.labels.map((label) => label.text)).toEqual(["30", "12"]);
  });

  it("breaks a line at a gap instead of drawing through it", () => {
    const gapped = layout(
      good({ type: "line", labels: ["a", "b", "c"], series: [{ data: [1, null, 3] }] }),
    ) as CartesianPlot;
    expect(gapped.strokes).toHaveLength(2);
  });

  it("gives every category a pointer target the size of the whole band", () => {
    // A reader aims at a month, never at a two-pixel line.
    for (const band of plot.bands) expect(band.size).toBeGreaterThan(MARK.hitRadius);
  });
});

describe("laying a pie out", () => {
  it("says nothing beside a wedge too thin to hold a label", () => {
    // Two one-percent slivers put two labels in the same place, and the shot of
    // that reads "Partne Other 1%". The legend and the table already name them.
    const plot = layout(
      good({
        type: "pie",
        labels: ["big", "sliver"],
        series: [{ data: [990, 10] }],
      }),
    ) as RadialPlot;
    expect(plot.wedges[0]?.label).not.toBeNull();
    expect(plot.wedges[1]?.label).toBeNull();
  });

  it("puts surface between the wedges and a share on each", () => {
    const plot = layout(
      good({ type: "pie", labels: ["a", "b"], series: [{ data: [3, 1] }] }),
    ) as RadialPlot;
    expect(plot.wedges).toHaveLength(2);
    expect(plot.wedges[0]?.label?.text).toBe("a 75%");
    expect(plot.wedges[0]?.path).toMatch(/^M[\d.-]/);
  });

  it("draws a single slice as a whole circle rather than nothing", () => {
    // One wedge sweeping the full turn starts and ends on the same point, so
    // the arc collapses and a pie of one category draws an empty frame.
    const plot = layout(
      good({ type: "pie", labels: ["all"], series: [{ data: [5] }] }),
    ) as RadialPlot;
    const [only] = plot.wedges;
    expect(only?.path).toContain("A");
    expect(only?.label?.text).toBe("all 100%");
  });

  it("hollows a donut and leaves a pie solid", () => {
    const donut = layout(
      good({ type: "donut", labels: ["a", "b"], series: [{ data: [1, 1] }] }),
    ) as RadialPlot;
    const pie = layout(
      good({ type: "pie", labels: ["a", "b"], series: [{ data: [1, 1] }] }),
    ) as RadialPlot;
    // A solid wedge is drawn from the centre out; a hollow one never goes there.
    expect(pie.wedges[0]?.path.startsWith(`M${pie.center.x} ${pie.center.y}`)).toBe(true);
    expect(donut.wedges[0]?.path.startsWith(`M${donut.center.x} ${donut.center.y}`)).toBe(false);
  });
});

describe("a bar's outline", () => {
  it("is rounded where the data ends and square at the baseline", () => {
    // Rounded at both ends a bar floats, and every bar in the chart looks as
    // though it starts above the axis it is measured from.
    const path = barPath({
      series: 0,
      band: 0,
      x: 0,
      y: 0,
      width: 20,
      height: 100,
      round: "top",
      value: 1,
    });
    // Two corner arcs at the top, none at the bottom.
    expect(path.match(/A/g)).toHaveLength(2);
    expect(path).toContain("V100");
  });

  it("gives up the corners rather than turning a short bar into a lozenge", () => {
    const path = barPath({
      series: 0,
      band: 0,
      x: 0,
      y: 0,
      width: 20,
      height: 0.4,
      round: "top",
      value: 1,
    });
    expect(path).not.toContain("A");
  });
});

describe("the table every chart carries", () => {
  it("holds one row per category and one column per series", () => {
    const table = chartTable(
      good({ type: "bar", labels: ["a", "b"], series: [{ name: "x", data: [1, 2] }] }),
    );
    expect(table.head).toEqual(["", "x"]);
    expect(table.rows).toEqual([
      ["a", "1"],
      ["b", "2"],
    ]);
  });

  it("says a gap is a gap", () => {
    const table = chartTable(
      good({ type: "line", labels: ["a", "b"], series: [{ data: [1, null] }] }),
    );
    expect(table.rows[1]?.[1]).toBe("no reading");
  });

  it("refuses a series with nothing in it at all", () => {
    // Distinct from a gap: a series of nothing but gaps has no reading
    // anywhere, so it puts a name in the legend that points at no mark.
    expect(refused({ type: "line", labels: ["a"], series: [{ data: [null] }] })).toContain(
      "nothing to draw",
    );
  });
});
