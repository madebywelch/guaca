interface Props {
  name: string;
  given: boolean;
  busy: boolean;
  about: string;
  state?: string;
  onChange: (given: boolean) => void;
  expanded: boolean;
  onExpand: () => void;
}

export function Grant({ name, given, busy, about, state, onChange, expanded, onExpand }: Props) {
  return (
    <div className="grant">
      <h3 className="detail__title">
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
