import { useId, useMemo, useState } from "react";

import {
  barPath,
  type CartesianPlot,
  type Chart as ChartValue,
  chartSeriesNames,
  chartSummary,
  chartTable,
  formatValue,
  isCartesian,
  isRadial,
  isScatter,
  layout,
  MARK,
  type Point,
  type RadialPlot,
  type ScatterPlot,
  WIDTH,
} from "../lib/chart";
import { seriesColor, seriesMark } from "../lib/palette";

/**
 * A chart, drawn.
 *
 * All the arithmetic happened in `lib/chart.ts`; this file turns coordinates
 * into elements and handles the two things a coordinate cannot: what the
 * pointer is on, and what the legend has switched off.
 *
 * Drawn at a fixed width and scaled by CSS rather than measured. jsdom does no
 * layout, so a chart that sized itself from a real element would be exercised
 * by no test in this repo; and a chart that waits to be measured is a jump on
 * every first paint. The trade is that type grows a little when a figure is
 * opened large, which is the behaviour a figure should have anyway.
 *
 * Three things carry every value, and that is deliberate. The marks carry the
 * shape, the direct labels and the readout carry the number, and the table
 * underneath carries all of them. A tooltip may never be the only way to find
 * out what something is worth: it is unreachable by touch, unreachable by a
 * screen reader, and gone from a screenshot.
 */
export function Chart({ chart }: { chart: ChartValue }) {
  const plot = useMemo(() => layout(chart), [chart]);
  const names = useMemo(() => chartSeriesNames(chart), [chart]);
  const [hidden, setHidden] = useState<ReadonlySet<number>>(() => new Set());
  const [at, setAt] = useState<number | null>(null);
  const titleId = useId();

  // A legend that could switch everything off is a legend that can empty the
  // chart, and an empty frame reads as a chart that broke.
  const toggle = (index: number) =>
    setHidden((was) => {
      const next = new Set(was);
      if (next.has(index)) next.delete(index);
      else if (next.size < names.length - 1) next.add(index);
      return next;
    });

  return (
    <div className="chart">
      {chart.title && (
        <h4 className="chart__title" id={titleId}>
          {chart.title}
        </h4>
      )}
      <div className="chart__frame">
        <svg
          className="chart__svg"
          viewBox={`0 0 ${WIDTH} ${plot.height}`}
          role="img"
          aria-label={chartSummary(chart)}
          onPointerLeave={() => setAt(null)}
        >
          <title>{chartSummary(chart)}</title>
          {plot.family === "cartesian" && (
            <Cartesian chart={chart} plot={plot} hidden={hidden} at={at} onAt={setAt} />
          )}
          {plot.family === "radial" && <Radial chart={chart} plot={plot} at={at} onAt={setAt} />}
          {plot.family === "scatter" && (
            <Scatter chart={chart} plot={plot} hidden={hidden} at={at} onAt={setAt} />
          )}
        </svg>
        <Readout chart={chart} plot={plot} hidden={hidden} at={at} names={names} />
      </div>
      {/* One series needs no legend: there is one colour, and the title above
          already says what is plotted. A box with a single swatch in it
          restates the title and costs a line. */}
      {names.length > 1 && (
        <Legend
          chart={chart}
          names={names}
          hidden={hidden}
          // A pie's legend is a key and nothing more. Its slices are shares of
          // one whole, so switching one off would leave the others claiming
          // percentages of a total that has not changed.
          onToggle={isRadial(chart) ? null : toggle}
        />
      )}
    </div>
  );
}

/** Bars, lines and areas, which share an axis and a set of category bands. */
function Cartesian({
  chart,
  plot,
  hidden,
  at,
  onAt,
}: {
  chart: ChartValue;
  plot: CartesianPlot;
  hidden: ReadonlySet<number>;
  at: number | null;
  onAt: (band: number | null) => void;
}) {
  const { frame, horizontal } = plot;
  const filled = chart.kind === "area";

  return (
    <>
      {/* Gridlines are solid hairlines a step off the surface. Dashed reads as
          a threshold or a projection when it is only a grid. */}
      {plot.ticks.map((tick) => (
        <line
          key={tick.value}
          className={tick.value === 0 ? "chart__axis" : "chart__grid"}
          x1={horizontal ? tick.at : frame.left}
          x2={horizontal ? tick.at : frame.left + frame.width}
          y1={horizontal ? frame.top : tick.at}
          y2={horizontal ? frame.top + frame.height : tick.at}
        />
      ))}

      {plot.ticks.map((tick) => (
        <text
          key={tick.value}
          className="chart__tick"
          x={horizontal ? tick.at : frame.left - 8}
          y={horizontal ? frame.top + frame.height + 16 : tick.at + 4}
          textAnchor={horizontal ? "middle" : "end"}
        >
          {tick.text}
        </text>
      ))}

      {plot.shown.map((index) => {
        const band = plot.bands[index];
        if (!band) return null;
        return (
          <text
            key={band.center}
            className="chart__name"
            x={horizontal ? frame.left - 8 : band.center}
            y={horizontal ? band.center + 4 : frame.top + frame.height + 16}
            textAnchor={horizontal ? "end" : "middle"}
          >
            {band.label}
          </text>
        );
      })}

      {plot.strokes.map((stroke) =>
        hidden.has(stroke.series) ? null : (
          <g key={`${stroke.series}:${stroke.line}`}>
            {filled && (
              <path
                d={stroke.fill}
                fill={seriesColor(stroke.series)}
                fillOpacity={0.1}
                stroke="none"
              />
            )}
            <path
              className="chart__line"
              d={stroke.line}
              stroke={seriesColor(stroke.series)}
              strokeWidth={MARK.lineWidth}
              fill="none"
            />
          </g>
        ),
      )}

      {plot.bars.map((bar) =>
        hidden.has(bar.series) ? null : (
          <path
            key={`${bar.series}:${bar.band}`}
            className="chart__bar"
            d={barPath(bar)}
            fill={seriesColor(bar.series)}
            data-lit={at === bar.band ? "" : undefined}
          />
        ),
      )}

      {/* Only where the pointer is. A dot on every point of a twelve-month line
          is a bead necklace, and the line already says where the readings are. */}
      {plot.dots.map((dot) =>
        hidden.has(dot.series) || at !== dot.band ? null : (
          <circle
            key={`${dot.series}:${dot.band}`}
            className="chart__dot"
            cx={dot.x}
            cy={dot.y}
            r={MARK.dotRadius}
            fill={seriesColor(dot.series)}
          />
        ),
      )}

      {plot.labels.map((label) => (
        <text
          key={`${label.x}:${label.y}:${label.text}`}
          className="chart__value"
          x={label.x}
          y={label.y}
          textAnchor={label.anchor}
        >
          {label.text}
        </text>
      ))}

      {/* The hit target is the whole band, not the marks in it: a reader aims
          at a month, never at a two-pixel line.

          Not focusable, and carrying no label. The `svg` above is `role="img"`
          with a sentence describing itself, which makes everything inside it
          invisible to a screen reader by definition, so a label here would be
          announced to nobody. Tab stops would be worse than useless: twelve
          invisible rectangles between the message above and the message below,
          for a readout the Figures table already holds in full and in a form
          somebody can actually read. The readout enhances; it never gates. */}
      {plot.bands.map((band) => (
        <rect
          key={band.center}
          className="chart__band"
          x={horizontal ? frame.left : band.from}
          y={horizontal ? band.from : frame.top}
          width={horizontal ? frame.width : band.size}
          height={horizontal ? band.size : frame.height}
          onPointerEnter={() => onAt(band.index)}
          onPointerMove={() => onAt(band.index)}
        />
      ))}
    </>
  );
}

/** Pies and donuts. */
function Radial({
  chart,
  plot,
  at,
  onAt,
}: {
  chart: ChartValue;
  plot: RadialPlot;
  at: number | null;
  onAt: (slice: number | null) => void;
}) {
  if (!isRadial(chart)) return null;
  return (
    <>
      {plot.wedges.map((wedge) => (
        <path
          key={wedge.slice}
          className="chart__wedge"
          d={wedge.path}
          fill={seriesColor(wedge.slice)}
          data-lit={at === wedge.slice ? "" : undefined}
          onPointerEnter={() => onAt(wedge.slice)}
        />
      ))}
      {plot.wedges.map(({ slice, label }) =>
        label === null ? null : (
          <text
            key={slice}
            className="chart__value"
            x={label.x}
            y={label.y}
            textAnchor={label.anchor}
          >
            {label.text}
          </text>
        ),
      )}
    </>
  );
}

/**
 * Clouds of points.
 *
 * Every series wears a shape as well as a colour. That is not decoration: in a
 * scatter any dot can end up beside any other, so the guarantee that holds for
 * neighbouring colours elsewhere does not hold here at all, and shape is the
 * channel that survives colourblindness, grey print and a screenshot.
 */
function Scatter({
  chart,
  plot,
  hidden,
  at,
  onAt,
}: {
  chart: ChartValue;
  plot: ScatterPlot;
  hidden: ReadonlySet<number>;
  at: number | null;
  onAt: (dot: number | null) => void;
}) {
  if (!isScatter(chart)) return null;
  const { frame } = plot;

  return (
    <>
      {plot.ticks.map((tick) => (
        <line
          key={`y${tick.value}`}
          className="chart__grid"
          x1={frame.left}
          x2={frame.left + frame.width}
          y1={tick.at}
          y2={tick.at}
        />
      ))}
      {plot.ticks.map((tick) => (
        <text
          key={`yt${tick.value}`}
          className="chart__tick"
          x={frame.left - 8}
          y={tick.at + 4}
          textAnchor="end"
        >
          {tick.text}
        </text>
      ))}
      {plot.across.map((tick) => (
        <text
          key={`xt${tick.value}`}
          className="chart__tick"
          x={tick.at}
          y={frame.top + frame.height + 16}
          textAnchor="middle"
        >
          {tick.text}
        </text>
      ))}

      {plot.dots.map((dot, index) =>
        hidden.has(dot.series) ? null : (
          <Mark
            key={`${dot.series}:${dot.band}`}
            dot={dot}
            lit={at === index}
            onEnter={() => onAt(index)}
          />
        ),
      )}
    </>
  );
}

/**
 * One point, with a target far bigger than itself.
 *
 * A nine-pixel dot is a pinpoint nobody hits, so the transparent circle over it
 * is what the pointer is actually aiming at. The ring in the surface colour is
 * what keeps two overlapping points readable as two.
 */
function Mark({ dot, lit, onEnter }: { dot: Point; lit: boolean; onEnter: () => void }) {
  const color = seriesColor(dot.series);
  const shape = seriesMark(dot.series);
  const r = MARK.dotRadius + 1;

  return (
    <g className="chart__mark" data-lit={lit ? "" : undefined} onPointerEnter={onEnter}>
      {shape === "circle" && <circle cx={dot.x} cy={dot.y} r={r} fill={color} />}
      {shape === "square" && (
        <rect x={dot.x - r} y={dot.y - r} width={r * 2} height={r * 2} fill={color} />
      )}
      {shape === "triangle" && (
        <path
          d={`M${dot.x} ${dot.y - r * 1.2}L${dot.x + r * 1.1} ${dot.y + r * 0.8}L${dot.x - r * 1.1} ${dot.y + r * 0.8}Z`}
          fill={color}
        />
      )}
      {shape === "diamond" && (
        <path
          d={`M${dot.x} ${dot.y - r * 1.3}L${dot.x + r * 1.3} ${dot.y}L${dot.x} ${dot.y + r * 1.3}L${dot.x - r * 1.3} ${dot.y}Z`}
          fill={color}
        />
      )}
      <circle className="chart__hit" cx={dot.x} cy={dot.y} r={MARK.hitRadius} />
    </g>
  );
}

/**
 * What is under the pointer, as words.
 *
 * Every series at that category, not just the one the pointer landed on: a
 * reader comparing two lines should not have to hit each of them in turn. The
 * value leads and the name follows, which is the legend's hierarchy inverted,
 * because by the time somebody is hovering they know which series they want
 * and are after the number.
 *
 * Positioned as a percentage of the drawing rather than in its coordinates,
 * since the drawing is scaled by CSS and this is not inside it.
 */
function Readout({
  chart,
  plot,
  hidden,
  at,
  names,
}: {
  chart: ChartValue;
  plot: ReturnType<typeof layout>;
  hidden: ReadonlySet<number>;
  at: number | null;
  names: string[];
}) {
  if (at === null) return null;

  if (plot.family === "radial") {
    if (!isRadial(chart)) return null;
    const slice = chart.slices[at];
    const wedge = plot.wedges[at];
    if (!slice || !wedge) return null;
    return (
      <div className="chart__readout" style={{ left: "50%", top: "8%" }} aria-hidden="true">
        <p className="chart__readout-head">{slice.label}</p>
        <p className="chart__readout-row">
          <span className="chart__readout-value">{formatValue(chart, slice.value, false)}</span>
          <span className="chart__readout-name">{Math.round(wedge.share * 100)}%</span>
        </p>
      </div>
    );
  }

  if (plot.family === "scatter") {
    if (!isScatter(chart)) return null;
    const dot = plot.dots[at];
    if (!dot) return null;
    const point = chart.series[dot.series]?.points[dot.band];
    return (
      <div
        className="chart__readout"
        style={{ left: `${(dot.x / WIDTH) * 100}%`, top: `${(dot.y / plot.height) * 100}%` }}
        aria-hidden="true"
      >
        <p className="chart__readout-head">{names[dot.series]}</p>
        <p className="chart__readout-row">
          <span className="chart__readout-value">
            {formatValue(chart, point?.x ?? 0, false)}, {formatValue(chart, dot.value, false)}
          </span>
        </p>
      </div>
    );
  }

  if (!isCartesian(chart) || plot.family !== "cartesian") return null;
  const band = plot.bands[at];
  if (!band) return null;

  return (
    <div
      className="chart__readout"
      style={
        plot.horizontal
          ? { left: "50%", top: `${(band.center / plot.height) * 100}%` }
          : { left: `${(band.center / WIDTH) * 100}%`, top: "6%" }
      }
      aria-hidden="true"
    >
      <p className="chart__readout-head">{band.label}</p>
      {chart.series.map((series, index) =>
        hidden.has(index) ? null : (
          <p className="chart__readout-row" key={series.name}>
            {/* A short stroke of the series colour, not a filled box: at this
                density a box is data-weight ink doing a label's job. */}
            <span className="chart__key" style={{ background: seriesColor(index) }} />
            <span className="chart__readout-value">
              {series.values[at] === null || series.values[at] === undefined
                ? "no reading"
                : formatValue(chart, series.values[at] as number, false)}
            </span>
            <span className="chart__readout-name">{series.name}</span>
          </p>
        ),
      )}
    </div>
  );
}

/**
 * Which series is which, and which are switched off.
 *
 * Always present past one series, because colour-matching alone is not an
 * identity channel a reader can rely on. Toggling never repaints what is left:
 * a colour belongs to a series, not to its position in what is currently shown,
 * and an operator who has learned that revenue is green is misled by a chart
 * that reassigns it when something else is hidden.
 */
function Legend({
  chart,
  names,
  hidden,
  onToggle,
}: {
  chart: ChartValue;
  names: string[];
  hidden: ReadonlySet<number>;
  /** `null` where switching a series off would misstate the rest. */
  onToggle: ((index: number) => void) | null;
}) {
  const line = chart.kind === "line" || chart.kind === "area";
  return (
    <ul className="chart__legend">
      {names.map((name, index) => (
        <li key={name}>
          <Entry
            className="chart__legend-item"
            pressed={onToggle ? !hidden.has(index) : undefined}
            onClick={onToggle ? () => onToggle(index) : undefined}
          >
            {/* The legend mirrors the mark: a rule for a line, a swatch for
                anything filled, a shape for a scatter. */}
            <span
              className={line ? "chart__key" : "chart__swatch"}
              data-mark={isScatter(chart) ? seriesMark(index) : undefined}
              // Both, because a triangle is drawn out of a border and a border
              // reads `currentColor`. One or the other leaves one shape grey.
              style={{ background: seriesColor(index), color: seriesColor(index) }}
            />
            {name}
          </Entry>
        </li>
      ))}
    </ul>
  );
}

/**
 * One legend row: a switch where there is something to switch, otherwise a key.
 *
 * A button that does nothing when pressed is worse than a label, so a legend
 * that cannot hide anything is not made of buttons.
 */
function Entry({
  className,
  pressed,
  onClick,
  children,
}: {
  className: string;
  pressed?: boolean;
  onClick?: () => void;
  children: React.ReactNode;
}) {
  if (!onClick) {
    return (
      <span className={className} data-static="">
        {children}
      </span>
    );
  }
  return (
    <button type="button" className={className} aria-pressed={pressed} onClick={onClick}>
      {children}
    </button>
  );
}

/**
 * The chart as its own numbers.
 *
 * Not a fallback. It is how a value is read by a screen reader, by an operator
 * who cannot separate two of the hues, and by anybody who wants the figure
 * rather than the shape. Folded away because a transcript is a conversation and
 * a table under every chart doubles its height; one press from open, because a
 * value behind a hover is a value some readers can never get to.
 */
export function ChartFigures({ chart }: { chart: ChartValue }) {
  const table = useMemo(() => chartTable(chart), [chart]);
  return (
    <details className="chart__table">
      <summary>Figures</summary>
      <div className="md__scroll">
        <table>
          <thead>
            <tr>
              {table.head.map((cell, at) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: two series can honestly carry the same name, and this grid is derived whole from an immutable chart rather than reordered
                <th key={at} scope="col">
                  {cell}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {table.rows.map((row, down) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: as above
              <tr key={down}>
                {row.map((cell, across) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: as above
                  <td key={across}>{cell}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </details>
  );
}
