import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { AgentCard } from "../lib/types";

export interface MenuTarget {
  agent: AgentCard;
  x: number;
  y: number;
}

interface Props {
  target: MenuTarget;
  onClose: () => void;
  onEditProfile: (agent: AgentCard) => void;
  onTogglePin: (agent: AgentCard) => void;
  onDuplicate: (agent: AgentCard) => void;
}

/** Roughly what the menu measures. Only used to keep it inside the window. */
const MARGIN = 8;

/**
 * What you can do to an agent without opening it.
 *
 * Editing a profile lives behind two clicks on purpose: a name and a set of
 * instructions are written once and read often, so the cost of reaching them
 * belongs on the rare action rather than on every right-click. Pinning and
 * duplicating are one click each because they are the ones you actually do.
 */
export function AgentMenu({ target, onClose, onEditProfile, onTogglePin, onDuplicate }: Props) {
  const { agent } = target;
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ x: target.x, y: target.y });

  // Measured after it is in the tree rather than guessed: the menu is opened
  // by a click that can land anywhere, and one hanging off the bottom of the
  // window has items nothing can reach.
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const { width, height } = node.getBoundingClientRect();
    setAt({
      x: Math.min(target.x, window.innerWidth - width - MARGIN),
      y: Math.min(target.y, window.innerHeight - height - MARGIN),
    });
  }, [target.x, target.y]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    // Anything that moves what is under the menu closes it, or it would point
    // at a row that has scrolled away. The rail reorders itself as agents
    // talk, so this is not hypothetical.
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", onClose);
    window.addEventListener("scroll", onClose, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("scroll", onClose, true);
    };
  }, [onClose]);

  const item = (label: string, run: () => void) => (
    <button
      type="button"
      className="menu__item"
      onClick={() => {
        run();
        onClose();
      }}
    >
      {label}
    </button>
  );

  return (
    <>
      {/* A real button, so dismissing by clicking away is reachable from the
          keyboard and announced, rather than being an invisible div handler. */}
      <button
        type="button"
        className="menu__scrim"
        aria-label="Close menu"
        onClick={onClose}
        onContextMenu={(event) => {
          event.preventDefault();
          onClose();
        }}
      />
      <div
        className="menu"
        ref={ref}
        role="menu"
        aria-label={agent.name}
        style={{ left: at.x, top: at.y }}
      >
        <p className="menu__head">{agent.name}</p>
        {item("Edit profile", () => onEditProfile(agent))}
        {item(agent.pinned ? "Unpin" : "Pin to top", () => onTogglePin(agent))}
        {item("Duplicate", () => onDuplicate(agent))}
      </div>
    </>
  );
}
