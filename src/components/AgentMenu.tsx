import type { CSSProperties, FocusEvent } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

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
  /** Into the compost, where it waits thirty days. */
  onDelete: (agent: AgentCard) => void;
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
  onDelete,
  onMoveToGroup,
}: Props) {
  const { agent } = target;
  const elsewhere = groups.filter((group) => group.id !== agent.groupId);
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ x: target.x, y: target.y, origin: "top left" });
  /**
   * Which of the two destructive items is armed, and never both.
   *
   * Each is asked twice, and each second wording says what actually happens.
   * In the menu rather than in the pane behind it: a confirmation drawn where
   * the click did not happen is a confirmation the operator has to go and find,
   * and this menu opens from two places that draw nothing in common.
   *
   * Clearing said "Delete this history", which is accurate and is not the
   * question the operator is actually asking. An agent has two kinds of state
   * and this touches one of them: the channel is what its turns read as
   * conversation, and its memory is a separate file it wrote on purpose and
   * would have to write again. Not saying so is why this went unused by an
   * operator who needed it and would not risk finding out.
   *
   * One value rather than a flag each, because two flags is a state where both
   * are armed: opening one confirmation would leave a delete button under the
   * next click of somebody who came here to clear a history.
   */
  const [confirming, setConfirming] = useState<"history" | "delete" | null>(null);

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
        {item("Duplicate", () => onDuplicate(agent))}
        {/* Absent while there is one group, for the same reason the strip in
            the rail is: there is nowhere else to go. */}
        {elsewhere.length > 0 && (
          <MoveToGroup
            groups={elsewhere}
            onPick={(group) => {
              onMoveToGroup(agent, group);
              onClose();
            }}
          />
        )}
        <hr className="menu__rule" />
        {confirming === "history" ? (
          item("Delete history, keep memory", () => onClearHistory(agent), "danger")
        ) : (
          // The only kind of item that does not close the menu. Nothing has
          // been decided yet, and the next click is the one that matters.
          <button type="button" className="menu__item" onClick={() => setConfirming("history")}>
            Clear chat history…
          </button>
        )}
        {/* Asked twice, and the second wording says where it goes rather than
            that it is permanent, because it is not: this is the one destructive
            item in the menu that can be taken back, and an operator who thinks
            otherwise is one who will not press it when they should. What it
            does not say is thirty days. The panel it lands in draws the clock,
            and a number here would be a second place for that number to
            drift. */}
        {confirming === "delete" ? (
          item(`Delete ${agent.name}, into the compost`, () => onDelete(agent), "danger")
        ) : (
          <button type="button" className="menu__item" onClick={() => setConfirming("delete")}>
            Delete…
          </button>
        )}
      </div>
    </>
  );
}

/**
 * The crews this agent is not in, in a menu of their own to the side.
 *
 * A submenu rather than a row each, because the crews are the one part of this
 * menu whose length nothing bounds: everything else is a verb on the agent and
 * there are seven of them, while a workspace with eight crews pushed clearing a
 * history and deleting an agent off the bottom of the window. The items are
 * bare names because the row that opens them is the verb, and it is also the
 * submenu's label, so a screen reader reads the sentence either way.
 *
 * Hover and focus open it, which are the same gesture from the two devices that
 * can make it. Pointer and focus both leave through the wrapper rather than the
 * row, so crossing into the submenu is not leaving.
 */
function MoveToGroup({ groups, onPick }: { groups: Group[]; onPick: (group: Group) => void }) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const sub = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ side: "right", shift: 0, origin: "top left" });

  // Measured for the same reason the menu is, from the row it hangs off rather
  // than from itself: the menu it opens out of may already have been pulled
  // back from the right edge, and a submenu drawn past that edge is a list of
  // crews nothing can click.
  useLayoutEffect(() => {
    const node = sub.current;
    const row = wrap.current;
    if (!node || !row) return;
    const box = node.getBoundingClientRect();
    const from = row.getBoundingClientRect();
    const side = from.right + box.width + MARGIN <= window.innerWidth ? "right" : "left";
    // Never downward: the top of the submenu is the row that opened it, and the
    // only reason to move is a list that would otherwise run off the bottom.
    const shift = Math.min(0, window.innerHeight - MARGIN - (from.top + box.height));
    setAt({
      side,
      shift,
      origin: `${shift < 0 ? "bottom" : "top"} ${side === "right" ? "left" : "right"}`,
    });
  }, [open]);

  return (
    <div
      className="menu__nest"
      ref={wrap}
      // A box to hang the submenu off and nothing else: the row is the control
      // and the crews are the menu, and neither is this. The hover is on the
      // box rather than on the row because crossing from one into the other has
      // to not be a departure.
      role="none"
      onPointerEnter={() => setOpen(true)}
      onPointerLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)}
      onBlur={(event: FocusEvent<HTMLDivElement>) => {
        // React reports a blur for every step between two children of one box,
        // so where the focus went is what says whether it left at all.
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <button
        type="button"
        className="menu__item"
        aria-haspopup="menu"
        aria-expanded={open}
        // Opens rather than toggles. A click closing it leaves the pointer on
        // the row that opens it, which is a state nothing can get out of
        // without moving away and coming back.
        onClick={() => setOpen(true)}
      >
        Move to another group
        <span className="menu__more" aria-hidden="true">
          ▸
        </span>
      </button>
      {open && (
        <div
          className="menu__sub"
          ref={sub}
          role="menu"
          aria-label="Move to another group"
          data-side={at.side}
          style={{ top: at.shift, "--pop-origin": at.origin } as CSSProperties}
        >
          {groups.map((group) => (
            // Keyed on the crew, not on its name: two crews may be called the
            // same thing, and the row that moves an agent has to be the row for
            // the crew it names.
            <button
              key={group.id}
              type="button"
              className="menu__item"
              onClick={() => onPick(group)}
            >
              {group.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
