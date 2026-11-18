import type { CSSProperties } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useDismiss } from "../lib/dismiss";
import { Icon } from "./Icon";

interface Props {
  onNewAgent: () => void;
  onNewGroup: () => void;
  onOpenCafeteria: () => void;
}

const MARGIN = 8;

export function NewMenu({ onNewAgent, onNewGroup, onOpenCafeteria }: Props) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState({ x: 0, y: 0, origin: "top left" });

  useLayoutEffect(() => {
    const button = buttonRef.current;
    const menu = menuRef.current;
    if (!open || !button || !menu) return;
    const anchor = button.getBoundingClientRect();
    const { width, height } = menu.getBoundingClientRect();
    const wanted = { x: Math.max(MARGIN, anchor.left), y: anchor.bottom + 4 };
    const x = Math.min(wanted.x, window.innerWidth - width - MARGIN);
    const y = Math.min(wanted.y, window.innerHeight - height - MARGIN);
    setAt({
      x,
      y,
      origin: `${y < wanted.y ? "bottom" : "top"} ${x < wanted.x ? "right" : "left"}`,
    });
  }, [open]);

  useDismiss(() => setOpen(false), open);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [open]);

  const item = (label: string, detail: string, run: () => void) => (
    <button
      type="button"
      className="menu__item"
      role="menuitem"
      onClick={() => {
        setOpen(false);
        run();
      }}
    >
      {label}
      <span className="menu__detail">{detail}</span>
    </button>
  );

  return (
    <>
      <button
        type="button"
        ref={buttonRef}
        className="rail__new"
        aria-label="Make something new"
        title="Make something new"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((was) => !was)}
      >
        <Icon name="plus" />
      </button>

      {open && (
        <>
          <button
            type="button"
            className="menu__scrim"
            aria-label="Close menu"
            onClick={() => setOpen(false)}
          />
          <div
            className="menu"
            ref={menuRef}
            role="menu"
            aria-label="Make something new"
            style={{ left: at.x, top: at.y, "--pop-origin": at.origin } as CSSProperties}
          >
            {item("New agent", "Somebody new in this group", onNewAgent)}
            {item("New group", "A group that cannot see the others", onNewGroup)}
            {item("Hire from the cafeteria", "Agents who are already set up", onOpenCafeteria)}
          </div>
        </>
      )}
    </>
  );
}
