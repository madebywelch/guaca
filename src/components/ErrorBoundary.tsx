import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
  info: string;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, info: "" };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Guaca failed to render", error, info);
    this.setState({ info: info.componentStack ?? "" });
  }

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;

    return (
      <div style={{ padding: "2rem", maxWidth: "48rem", margin: "0 auto", overflow: "auto" }}>
        <h1 className="dialog__title">Guaca could not draw this window</h1>
        <p className="dialog__lede">
          This is a bug in Guaca, not something you did. The details below say where it happened.
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
