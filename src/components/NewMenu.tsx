import type { CSSProperties } from "react";
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
 * next one without the rail paying for it, which is why it is a menu.
 *
 * It sits at the top of the rail, beside the wordmark. It spent a while at the
 * end of the channel header instead, on the grounds that the rail is a list of
 * agents and this is about none of them, and that turned out to be the wrong
 * half of the argument: an agent is a row in the rail and a group is a heading
 * in it, so the rail is exactly where somebody looks to add one. It was also
 * the one place the plus could not be drawn when the workspace is empty, since
 * there is no channel open to hang a header on, which is the state where making
 * an agent is the only thing left to do.
 */
export function NewMenu({ onNewAgent, onNewGroup }: Props) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState({ x: 0, y: 0, origin: "top left" });

  // Measured after it is in the tree, like every other menu here. It opens to
  // the right because the button is near the left edge of the window: laid out
  // from the button's right edge, a menu wider than the rail would be clamped
  // to the window and end up beside the plus rather than under it.
  useLayoutEffect(() => {
    const button = buttonRef.current;
    const menu = menuRef.current;
    if (!open || !button || !menu) return;
    const anchor = button.getBoundingClientRect();
    const { width, height } = menu.getBoundingClientRect();
    // Left edges aligned, then pulled back inside the window if that is not
    // where it fits.
    const wanted = { x: Math.max(MARGIN, anchor.left), y: anchor.bottom + 4 };
    const x = Math.min(wanted.x, window.innerWidth - width - MARGIN);
    const y = Math.min(wanted.y, window.innerHeight - height - MARGIN);
    setAt({
      x,
      y,
      // The corner the menu grew out of, which is the corner nearest the plus.
      // Compared against where it asked to go rather than against the button:
      // a menu pulled back off a window edge is no longer under its button, and
      // growing it from the corner it wanted slides it across the screen on the
      // way in.
      origin: `${y < wanted.y ? "bottom" : "top"} ${x < wanted.x ? "right" : "left"}`,
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
        className="rail__new"
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
            style={{ left: at.x, top: at.y, "--pop-origin": at.origin } as CSSProperties}
          >
            {item("New agent", "Somebody new in this workspace", onNewAgent)}
            {item("New group", "A crew that cannot see the others", onNewGroup)}
          </div>
        </>
      )}
    </>
  );
}
