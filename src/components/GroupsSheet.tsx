import { type ComponentProps, useEffect, useRef } from "react";

import { useDismiss } from "../lib/dismiss";
import { IS_MAC } from "../lib/keybinds";
import { groupsLabel } from "../lib/menu";
import { useStore } from "../lib/store";
import { desktop } from "../lib/transport";
import { Sidebar } from "./Sidebar";

type Props = ComponentProps<typeof Sidebar> & { onClose: () => void };

const LIGHTS = desktop && IS_MAC;

export function GroupsSheet({ onClose, onOpenChannel, ...rest }: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const title = useStore((state) => groupsLabel(state.groups.length));
  useDismiss(onClose);
  useEffect(() => {
    panel.current?.focus();
  }, []);
  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close groups" onClick={onClose} />
      <div
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-label="Groups and agents"
        tabIndex={-1}
        ref={panel}
      >
        <header className="sheet__head" data-lights={LIGHTS ? "" : undefined}>
          <h2 className="sheet__title">{title}</h2>
          <button type="button" className="btn btn--quiet btn--compact" onClick={onClose}>
            Done
          </button>
        </header>
        <div className="sheet__body">
          <Sidebar
            {...rest}
            onOpenChannel={() => {
              onOpenChannel?.();
              onClose();
            }}
          />
        </div>
      </div>
    </div>
  );
}
