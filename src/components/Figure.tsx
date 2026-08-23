import { useState } from "react";

import { isFault } from "../lib/chart";
import type { Figure as FigureValue } from "../lib/figure";
import { Chart, ChartFigures } from "./Chart";
import { HtmlArtifact } from "./HtmlArtifact";

/**
 * A fenced block that turned out to be worth drawing.
 *
 * One card whatever is inside it, so a chart and a page an agent wrote sit in
 * a transcript the same way a file does: a bounded thing with a hairline round
 * it, not a wall of markup in the middle of a sentence.
 *
 * The source is always one press away, and that is not a debugging affordance.
 * A figure drawn from a model's own JSON is a claim about numbers, and an
 * operator who wants to check the chart against what was actually written has
 * nowhere else to look. It is also the only thing that can be copied out.
 */
export function Figure({ figure, source }: { figure: FigureValue; source: string }) {
  const [showing, setShowing] = useState(false);

  if (figure.kind === "pending") {
    // Still arriving. Named as such rather than left blank, because a gap that
    // appears mid-reply and then fills in reads as the message having broken.
    return (
      <div className="figure figure--waiting">
        <p className="hint">Drawing…</p>
      </div>
    );
  }

  const fault = figure.kind === "chart" && isFault(figure.read) ? figure.read.why : null;
  const drawn = figure.kind === "chart" && !isFault(figure.read) ? figure.read.chart : null;

  return (
    <div className="figure">
      <div className="figure__body">
        {drawn && <Chart chart={drawn} />}
        {figure.kind === "html" && <HtmlArtifact html={figure.html} title="Page" />}
        {/* A refused chart shows what was asked for. The operator needs to see
            the request, and the agent that wrote it needs to be told what to
            change; a box saying "invalid chart" is neither of those. */}
        {(fault || showing) && <pre className="md__pre figure__source">{source}</pre>}
      </div>
      <div className="figure__foot">
        {fault ? (
          <p className="figure__fault">{fault}</p>
        ) : (
          <>
            {/* The chart's own numbers and the spec that drew them, side by
                side: they are the two ways of checking a figure and they belong
                in the same strip rather than as two stray rows under it. */}
            {drawn && <ChartFigures chart={drawn} />}
            <button
              type="button"
              className="btn btn--ghost btn--small"
              aria-expanded={showing}
              onClick={() => setShowing((was) => !was)}
            >
              {showing ? "Hide source" : "Source"}
            </button>
          </>
        )}
      </div>
    </div>
  );
}
