import type { CSSProperties, FocusEvent } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useDismiss } from "../lib/dismiss";
import { ownModel } from "../lib/providers";
import type { AgentCard, Group, Provider } from "../lib/types";
import { Icon } from "./Icon";

export interface MenuTarget {
  agent: AgentCard;
  x: number;
  y: number;
}

interface Props {
  target: MenuTarget;
  groups: Group[];
  provider: Provider | undefined;
  onClose: () => void;
  onEditProfile: (agent: AgentCard) => void;
  onTogglePin: (agent: AgentCard) => void;
  onTogglePause: (agent: AgentCard) => void;
  onDuplicate: (agent: AgentCard) => void;
  onClearHistory: (agent: AgentCard) => void;
  onDelete: (agent: AgentCard) => void;
  onMoveToGroup: (agent: AgentCard, group: Group) => void;
}

const MARGIN = 8;

export function AgentMenu({
  target,
  groups,
  provider,
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
  const model = ownModel(agent, provider);
  const elsewhere = groups.filter((group) => group.id !== agent.groupId);
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ x: target.x, y: target.y, origin: "top left" });
  const [confirming, setConfirming] = useState<"history" | "delete" | null>(null);

  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const { width, height } = node.getBoundingClientRect();
    const x = Math.min(target.x, window.innerWidth - width - MARGIN);
    const y = Math.min(target.y, window.innerHeight - height - MARGIN);
    setAt({
      x,
      y,
      origin: `${y < target.y ? "bottom" : "top"} ${x < target.x ? "right" : "left"}`,
    });
  }, [target.x, target.y]);

  useDismiss(onClose);

  useEffect(() => {
    window.addEventListener("resize", onClose);
    window.addEventListener("scroll", onClose, true);
    return () => {
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
          {model && <span className="menu__model">{model}</span>}
        </p>
        {item(agent.lifecycle === "paused" ? "Resume" : "Pause", () => onTogglePause(agent))}
        {item("Edit profile", () => onEditProfile(agent))}
        {item(agent.pinned ? "Unpin" : "Pin to top", () => onTogglePin(agent))}
        {item("Duplicate", () => onDuplicate(agent))}
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
          <button type="button" className="menu__item" onClick={() => setConfirming("history")}>
            Clear chat history…
          </button>
        )}
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

function MoveToGroup({ groups, onPick }: { groups: Group[]; onPick: (group: Group) => void }) {
  const [open, setOpen] = useState(false);
  useDismiss(() => setOpen(false), open);
  const wrap = useRef<HTMLDivElement>(null);
  const sub = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ side: "right", shift: 0, origin: "top left" });

  useLayoutEffect(() => {
    const node = sub.current;
    const row = wrap.current;
    if (!node || !row) return;
    const box = node.getBoundingClientRect();
    const from = row.getBoundingClientRect();
    const side = from.right + box.width + MARGIN <= window.innerWidth ? "right" : "left";
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
      role="none"
      onPointerEnter={() => setOpen(true)}
      onPointerLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)}
      onBlur={(event: FocusEvent<HTMLDivElement>) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <button
        type="button"
        className="menu__item"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(true)}
      >
        Move to another group
        <span className="menu__more" aria-hidden="true">
          <Icon name="chevron-right" size={16} />
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
