import { useEffect } from "react";
import { useDismiss } from "../lib/dismiss";
import { useShown } from "../lib/held";
import type { AgentId } from "../lib/types";
import { Desk } from "./Desk";
import { Toast } from "./Toast";

export function QuestionTray({
  selected,
  onSelect,
  onClose,
  onOpen,
}: {
  selected: string;
  onSelect: (id: string) => void;
  onClose: () => void;
  onOpen: (id: AgentId) => void;
}) {
  const { pending, stuck } = useShown();
  useDismiss(onClose);
  useEffect(() => {
    const all = [...pending, ...stuck];
    if (all.some((question) => question.id === selected)) return;
    if (all[0]) onSelect(all[0].id);
    else onClose();
  }, [pending, stuck, selected, onSelect, onClose]);

  return (
    <div className="scrim">
      <button
        type="button"
        className="scrim__close"
        aria-label="Close questions"
        onClick={onClose}
      />
      <div className="dialog question-tray" role="dialog" aria-modal="true" aria-label="Questions">
        <header className="question-tray__head">
          <h2 className="dialog__title">Questions</h2>
          <button type="button" className="btn btn--quiet btn--compact" onClick={onClose}>
            Done
          </button>
        </header>
        <Desk
          onOpenChannel={onOpen}
          title={null}
          queue={{ selected, onSelect, onLeave: onClose }}
        />
      </div>
      <Toast />
    </div>
  );
}
