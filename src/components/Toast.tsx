import { useHeld } from "../lib/held";
import { formatCombo } from "../lib/keybinds";
import { useStore } from "../lib/store";
import { Icon } from "./Icon";

const UNDO = formatCombo({ key: "z", mod: true });

export function Toast() {
  const notice = useHeld((s) => s.notice);
  const undo = useHeld((s) => s.undo);
  const dismiss = useHeld((s) => s.dismiss);
  const select = useStore((s) => s.select);
  if (!notice) return null;
  return (
    <div className="toast" role="status">
      <span className="toast__text">{notice.text}</span>
      {notice.kind === "held" ? (
        <button type="button" className="btn btn--quiet btn--compact" onClick={() => undo()}>
          <Icon name="undo" size={16} />
          Undo
          <span className="toast__key" aria-hidden="true">
            {UNDO}
          </span>
        </button>
      ) : (
        <button
          type="button"
          className="btn btn--quiet btn--compact"
          onClick={() => {
            dismiss();
            void select(notice.agentId);
          }}
        >
          Open the conversation
        </button>
      )}
    </div>
  );
}
