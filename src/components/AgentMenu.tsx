import type { CSSProperties } from "react";
import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";

import type { AgentCard, Group } from "../lib/types";

export interface MenuTarget {
  agent: AgentCard;
  x: number;
  y: number;
}

interface Props {
  target: MenuTarget;
  /** Every group, so the ones this agent is not in can be offered. */
  groups: Group[];
  onClose: () => void;
  onEditProfile: (agent: AgentCard) => void;
  onTogglePin: (agent: AgentCard) => void;
  onTogglePause: (agent: AgentCard) => void;
  onDuplicate: (agent: AgentCard) => void;
  onClearHistory: (agent: AgentCard) => void;
  /** One row up or down: the drag, without a mouse. */
  onNudge: (agent: AgentCard, delta: -1 | 1) => void;
  onMoveToGroup: (agent: AgentCard, group: Group) => void;
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
  groups,
  onClose,
  onEditProfile,
  onTogglePin,
  onTogglePause,
  onDuplicate,
  onClearHistory,
  onNudge,
  onMoveToGroup,
}: Props) {
  const { agent } = target;
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ x: target.x, y: target.y, origin: "top left" });
  /**
   * Clearing, asked twice, and the second wording says what survives.
   *
   * In the menu rather than in the pane behind it: a confirmation drawn where
   * the click did not happen is a confirmation the operator has to go and find,
   * and this menu opens from two places that draw nothing in common.
   *
   * It said "Delete this history", which is accurate and is not the question
   * the operator is actually asking. An agent has two kinds of state and this
   * touches one of them: the channel is what its turns read as conversation,
   * and its memory is a separate file it wrote on purpose and would have to
   * write again. Not saying so is why this went unused by an operator who
   * needed it and would not risk finding out.
   */
  const [confirming, setConfirming] = useState(false);

  // Measured after it is in the tree rather than guessed: the menu is opened
  // by a click that can land anywhere, and one hanging off the bottom of the
  // window has items nothing can reach.
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const { width, height } = node.getBoundingClientRect();
    const x = Math.min(target.x, window.innerWidth - width - MARGIN);
    const y = Math.min(target.y, window.innerHeight - height - MARGIN);
    setAt({
      x,
      y,
      // The corner nearest the click, read off where the menu actually landed.
      // Clamped away from an edge it is no longer under the pointer, and an
      // origin taken from the click would slide it across the screen.
      origin: `${y < target.y ? "bottom" : "top"} ${x < target.x ? "right" : "left"}`,
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
        style={{ left: at.x, top: at.y, "--pop-origin": at.origin } as CSSProperties}
      >
        <p className="menu__head">
          {agent.name}
          {agent.model && <span className="menu__model">{agent.model}</span>}
        </p>
        {item(agent.lifecycle === "paused" ? "Resume" : "Pause", () => onTogglePause(agent))}
        {item("Edit profile", () => onEditProfile(agent))}
        {item(agent.pinned ? "Unpin" : "Pin to top", () => onTogglePin(agent))}
        {/* The same two moves a drag makes, reachable without one. A rail that
            can only be arranged by dragging cannot be arranged from a keyboard
            at all, and this menu is already where everything else about an
            agent lives. */}
        {item("Move up", () => onNudge(agent, -1))}
        {item("Move down", () => onNudge(agent, 1))}
        {item("Duplicate", () => onDuplicate(agent))}
        {/* Absent while there is one group, for the same reason the strip in
            the rail is: there is nowhere else to go. */}
        {groups
          .filter((group) => group.id !== agent.groupId)
          .map((group) => (
            // Keyed on the crew, not on its name: two crews may be called the
            // same thing, and the row that moves an agent has to be the row
            // for the crew it names.
            <Fragment key={group.id}>
              {item(`Move to ${group.name}`, () => onMoveToGroup(agent, group))}
            </Fragment>
          ))}
        <hr className="menu__rule" />
        {confirming ? (
          item("Delete history, keep memory", () => onClearHistory(agent), "danger")
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
