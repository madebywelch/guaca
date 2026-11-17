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

export function Chart({ chart }: { chart: ChartValue }) {
  const plot = useMemo(() => layout(chart), [chart]);
  const names = useMemo(() => chartSeriesNames(chart), [chart]);
  const [hidden, setHidden] = useState<ReadonlySet<number>>(() => new Set());
  const [at, setAt] = useState<number | null>(null);
  const titleId = useId();

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
      {names.length > 1 && (
        <Legend
          chart={chart}
          names={names}
          hidden={hidden}
          onToggle={isRadial(chart) ? null : toggle}
        />
      )}
    </div>
  );
}

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

function Legend({
  chart,
  names,
  hidden,
  onToggle,
}: {
  chart: ChartValue;
  names: string[];
  hidden: ReadonlySet<number>;
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
            <span
              className={line ? "chart__key" : "chart__swatch"}
              data-mark={isScatter(chart) ? seriesMark(index) : undefined}
              style={{ background: seriesColor(index), color: seriesColor(index) }}
            />
            {name}
          </Entry>
        </li>
      ))}
    </ul>
  );
}

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
