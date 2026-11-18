import { useState } from "react";

import { isFault } from "../lib/chart";
import type { Figure as FigureValue } from "../lib/figure";
import { Chart, ChartFigures } from "./Chart";
import { HtmlArtifact } from "./HtmlArtifact";

export function Figure({ figure, source }: { figure: FigureValue; source: string }) {
  const [showing, setShowing] = useState(false);

  if (figure.kind === "pending") {
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
        {(fault || showing) && <pre className="md__pre figure__source">{source}</pre>}
      </div>
      <div className="figure__foot">
        {fault ? (
          <p className="figure__fault">{fault}</p>
        ) : (
          <>
            {drawn && <ChartFigures chart={drawn} />}
            <button
              type="button"
              className="btn btn--quiet btn--compact"
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
