import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { errorMessage, type ToolSummary } from "../lib/types";

/**
 * Guaca's own tools, read from the definitions agents are sent.
 *
 * A list rather than switches. Which agent may use which tool is decided by
 * what it was given (a computer, a browser, a repository) and, for a
 * connector's tools, under Connectors; this is where an operator reads what
 * the functions are, in the words the agent reads them.
 */
export function ToolList() {
  const [tools, setTools] = useState<ToolSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api
      .builtinTools()
      .then((found) => {
        if (live) setTools(found);
      })
      .catch((caught) => {
        if (live) setError(errorMessage(caught));
      });
    return () => {
      live = false;
    };
  }, []);

  if (error)
    return (
      <p className="field__error" role="alert">
        {error}
      </p>
    );
  if (!tools) return <p className="field__hint">Loading tools…</p>;
  return (
    <div className="access">
      {tools.map((tool) => (
        <div className="access__item" key={tool.name}>
          <div className="access__row">
            <strong className="access__name">
              <code>{tool.name}</code>
            </strong>
            <span className="access__where">{tool.needs ? `needs ${tool.needs}` : ""}</span>
          </div>
          <p className="field__hint">{tool.summary}</p>
        </div>
      ))}
    </div>
  );
}
