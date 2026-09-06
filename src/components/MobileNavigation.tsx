import { useStore } from "../lib/store";

interface Props {
  onChats: () => void;
  onSearch: () => void;
  onSettings: () => void;
}

/** The destinations needed while away from the desktop, within thumb reach. */
export function MobileNavigation({ onChats, onSearch, onSettings }: Props) {
  const pending = useStore((s) => s.pending);
  const stuck = useStore((s) => s.stuck);
  const decisions = useStore((s) => s.decisions);
  const forYou = useStore((s) => s.forYou);
  const showForYou = useStore((s) => s.showForYou);
  const count =
    pending.length +
    stuck.length +
    decisions.filter(
      (item) => item.status === "pending" || (item.status === "answered" && item.interrupted),
    ).length;
  return (
    <nav className="mobile-nav" aria-label="Workspace navigation">
      <button type="button" aria-current={!forYou ? "page" : undefined} onClick={onChats}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M5 4h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H9l-6 4V6a2 2 0 0 1 2-2Z" />
        </svg>
        <span>Chats</span>
      </button>
      <button
        type="button"
        onClick={() => showForYou(true)}
        aria-label={count ? `For you, ${count} needs attention` : "For you"}
      >
        <span className="mobile-nav__icon">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 4h16v16H4zM8 9h8M8 14h5" />
          </svg>
          {count > 0 && (
            <span className="mobile-nav__badge" aria-hidden="true">
              {count}
            </span>
          )}
        </span>
        <span>For you</span>
      </button>
      <button type="button" onClick={onSearch}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="10" cy="10" r="6" />
          <path d="m15 15 6 6" />
        </svg>
        <span>Search</span>
      </button>
      <button type="button" onClick={onSettings}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M4 6h16M4 12h16M4 18h16M8 3v6M16 9v6M10 15v6" />
        </svg>
        <span>Settings</span>
      </button>
    </nav>
  );
}
