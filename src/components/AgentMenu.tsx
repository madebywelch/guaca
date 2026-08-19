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
  onTogglePause: (agent: AgentCard) => void;
  onDuplicate: (agent: AgentCard) => void;
  onClearHistory: (agent: AgentCard) => void;
}

/** Roughly what the menu measures. Only used to keep it inside the window. */
const MARGIN = 8;

/**
 * Everything you can do to an agent, in the one place.
 *
 * Reached by right-clicking its row in the rail and by the button beside its
 * name, because those are two ways of asking the same question. Pausing and
 * clearing used to be permanent buttons over the transcript instead: three
 * controls the operator reads past on every message, two of which are used
 * once a week and one of which deletes history.
 *
 * Editing a profile lives behind two clicks on purpose: a name and a set of
 * instructions are written once and read often, so the cost of reaching them
 * belongs on the rare action rather than on every right-click.
 *
 * The model is written at the top rather than under the agent's name in the
 * header. It is what you come here to check when a reply looks wrong, and it
 * is nothing at all the rest of the time.
 */
export function AgentMenu({
  target,
  onClose,
  onEditProfile,
  onTogglePin,
  onTogglePause,
  onDuplicate,
  onClearHistory,
}: Props) {
  const { agent } = target;
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ x: target.x, y: target.y });
  /**
   * Clearing, asked twice.
   *
   * In the menu rather than in the pane behind it: a confirmation drawn where
   * the click did not happen is a confirmation the operator has to go and find,
   * and this menu opens from two places that draw nothing in common.
   */
  const [confirming, setConfirming] = useState(false);

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

  const item = (label: string, run: () => void, tone?: "danger") => (
    <button
      type="button"
      className="menu__item"
      data-tone={tone}
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
        <p className="menu__head">
          {agent.name}
          {agent.model && <span className="menu__model">{agent.model}</span>}
        </p>
        {item(agent.lifecycle === "paused" ? "Resume" : "Pause", () => onTogglePause(agent))}
        {item("Edit profile", () => onEditProfile(agent))}
        {item(agent.pinned ? "Unpin" : "Pin to top", () => onTogglePin(agent))}
        {item("Duplicate", () => onDuplicate(agent))}
        <hr className="menu__rule" />
        {confirming ? (
          item("Delete this history", () => onClearHistory(agent), "danger")
        ) : (
          // The only item that does not close the menu. Nothing has been
          // decided yet, and the next click is the one that matters.
          <button type="button" className="menu__item" onClick={() => setConfirming(true)}>
            Clear history…
          </button>
        )}
      </div>
    </>
  );
}
