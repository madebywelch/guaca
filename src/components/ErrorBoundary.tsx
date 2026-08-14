import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
  info: string;
}

/**
 * Last line of defence against a blank window.
 *
 * When a render or a layout effect throws, React unmounts the entire tree. The
 * window is left painted with the body background and nothing else: no error,
 * no clue, and reloading changes nothing because it fails again. Showing the
 * message is the difference between a five-minute fix and an hour of guessing.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, info: "" };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Guac failed to render", error, info);
    this.setState({ info: info.componentStack ?? "" });
  }

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;

    return (
      <div style={{ padding: "2rem", maxWidth: "48rem", margin: "0 auto", overflow: "auto" }}>
        <h1 className="dialog__title">Guac could not draw this window</h1>
        <p className="dialog__lede">
          This is a bug in Guac, not something you did. The details below say where it happened.
        </p>
        <pre
          className="chip chip--error"
          style={{ display: "block", whiteSpace: "pre-wrap", padding: "0.75rem", margin: 0 }}
        >
          {error.message}
        </pre>
        {info && (
          <details style={{ marginTop: "1rem" }}>
            <summary className="hint" style={{ cursor: "pointer" }}>
              Component stack
            </summary>
            <pre className="hint" style={{ whiteSpace: "pre-wrap", overflowX: "auto" }}>
              {info}
            </pre>
          </details>
        )}
        <button
          type="button"
          className="btn btn--primary"
          style={{ marginTop: "1rem" }}
          onClick={() => window.location.reload()}
        >
          Reload
        </button>
      </div>
    );
  }
}
