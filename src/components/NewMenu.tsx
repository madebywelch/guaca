import { useEffect, useLayoutEffect, useRef, useState } from "react";

interface Props {
  onNewAgent: () => void;
  onNewGroup: () => void;
}

/** Roughly what the menu measures. Only used to keep it inside the window. */
const MARGIN = 8;

/**
 * The one plus in the app: what you can make, from wherever you are.
 *
 * Both of these were permanent rows in the rail's footer, under the two places
 * you go. That put the rarest thing an operator does — making a crew — beside
 * the thing they do constantly, and it grew the footer by a row every time
 * something new became makeable. A plus is the one control that can absorb the
 * next one without the rail paying for it.
 *
 * It lives at the end of the channel header rather than in the rail because the
 * rail is a list of agents and this is not about any of them.
 */
export function NewMenu({ onNewAgent, onNewGroup }: Props) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState({ x: 0, y: 0 });

  // Measured after it is in the tree, like every other menu here: the button
  // sits at the right edge of a column whose width is the window's, so a menu
  // laid out from the button's left would hang off the side of the app.
  useLayoutEffect(() => {
    const button = buttonRef.current;
    const menu = menuRef.current;
    if (!open || !button || !menu) return;
    const anchor = button.getBoundingClientRect();
    const { width, height } = menu.getBoundingClientRect();
    setAt({
      // Right edges aligned, then pulled back inside the window if that is not
      // where it fits.
      x: Math.max(MARGIN, Math.min(anchor.right - width, window.innerWidth - width - MARGIN)),
      y: Math.min(anchor.bottom + 4, window.innerHeight - height - MARGIN),
    });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    // Named, so the same function is the one removed. An inline arrow in both
    // calls registers a listener nothing can take off again, and this one
    // outlives the menu it was closing.
    const close = () => setOpen(false);
    // Anything that moves the button closes the menu, or it would hang under
    // nothing. Same rule as the agent menu, and for the same reason.
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("keydown", onKey);
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
        className="pane__new"
        aria-label="Make something new"
        title="Make something new"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((was) => !was)}
      >
        +
      </button>

      {open && (
        <>
          {/* A real button, so dismissing by clicking away is reachable from
              the keyboard and announced. */}
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
            style={{ left: at.x, top: at.y }}
          >
            {item("New agent", "Somebody new in this workspace", onNewAgent)}
            {item("New group", "A crew that cannot see the others", onNewGroup)}
          </div>
        </>
      )}
    </>
  );
}
