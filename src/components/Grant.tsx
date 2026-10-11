interface Props {
  /** The place, as the section is headed: Browser, Computer, Terminal. */
  name: string;
  given: boolean;
  busy: boolean;
  /** What pressing the switch would do, read before it is pressed. */
  about: string;
  /** What the thing itself is doing, when there is one to report. */
  state?: string;
  onChange: (given: boolean) => void;
  expanded: boolean;
  onExpand: () => void;
}

/**
 * The head of a place an agent can be given: its name, what it is doing, and
 * one switch that gives it and takes it back.
 *
 * A switch rather than a button each way. The pair it replaced said "Give one"
 * and "Take it back" without saying what, sat at opposite ends of the section,
 * and the terminal's took-it-back floated above the next heading, where it read
 * as belonging to the schedule. A switch beside the name says what it is about
 * and which way it is set in one glance.
 *
 * The disclosure opens an idle place's controls. A live picture remains in
 * view even when those controls are closed; the grant switch stays on the row
 * so an ungiven place occupies just that row.
 */
export function Grant({ name, given, busy, about, state, onChange, expanded, onExpand }: Props) {
  return (
    <div className="grant">
      <h3 className="grant__name">
        <button
          type="button"
          className="grant__disclosure"
          aria-expanded={expanded}
          onClick={onExpand}
        >
          {name}
        </button>
      </h3>
      {state && (
        <span className="screen__state" data-state={state}>
          {state}
        </span>
      )}
      <button
        type="button"
        role="switch"
        className="toggle grant__switch"
        aria-checked={given}
        aria-label={name}
        title={about}
        disabled={busy}
        onClick={() => onChange(!given)}
      >
        <span aria-hidden="true" className="toggle__track" />
      </button>
    </div>
  );
}
