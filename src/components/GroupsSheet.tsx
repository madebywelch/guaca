import { type ComponentProps, useEffect, useRef } from "react";

import { useDismiss } from "../lib/dismiss";
import { useStore } from "../lib/store";
import { Sidebar } from "./Sidebar";

type Props = ComponentProps<typeof Sidebar> & { onClose: () => void };

/**
 * Every group and every agent in it, as the rail drew them, in a sheet.
 *
 * The rail used to stand beside every conversation. The timeline at the top
 * now says who did what and when, which is what the rail was mostly read for,
 * and a column of names beside it would say the same thing twice. What the
 * rail also did, and nothing else does, is arrange: groups and the agents in
 * them, the order, pins, moving an agent between groups, each group's
 * settings, making one. That is all here unchanged, one click away, and opening
 * someone from it closes it.
 */
export function GroupsSheet({ onClose, onOpenChannel, ...rest }: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const many = useStore((state) => state.groups.length > 1);
  const title = many ? "Groups and agents" : "Agents";
  useDismiss(onClose);
  useEffect(() => {
    panel.current?.focus();
  }, []);
  return (
    <div className="scrim sheet-scrim">
      <button type="button" className="scrim__close" aria-label="Close groups" onClick={onClose} />
      <div
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-label="Groups and agents"
        tabIndex={-1}
        ref={panel}
      >
        <header className="sheet__head">
          <h2 className="sheet__title">{title}</h2>
          <button type="button" className="btn btn--ghost btn--small" onClick={onClose}>
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
