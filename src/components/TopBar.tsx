import { IS_MAC } from "../lib/keybinds";
import { useStore } from "../lib/store";
import { useNow } from "../lib/time";
import { desktop } from "../lib/transport";
import { Icon } from "./Icon";
import { NewMenu } from "./NewMenu";

interface Props {
  open: string | null;
  onHome: () => void;
  onSearch: () => void;
  onGroups: () => void;
  onSettings: () => void;
  onNewAgent: () => void;
  onNewGroup: () => void;
  onOpenCafeteria: () => void;
}

const LIGHTS = desktop && IS_MAC;

export function TopBar({
  open,
  onHome,
  onSearch,
  onGroups,
  onSettings,
  onNewAgent,
  onNewGroup,
  onOpenCafeteria,
}: Props) {
  const now = useNow(30_000);
  const groups = useStore((state) => state.groups);
  const day = new Date(now).toLocaleDateString([], {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
  const short = new Date(now).toLocaleDateString([], {
    month: "short",
    day: "numeric",
  });
  return (
    <header className="topbar" data-lights={LIGHTS || undefined} data-tauri-drag-region>
      {open ? (
        <button type="button" className="topbar__back" onClick={onHome}>
          <Icon name="chevron-left" />
          Today
        </button>
      ) : (
        <p className="topbar__day" data-tauri-drag-region>
          <span className="topbar__day-long">{day}</span>
          <span className="topbar__day-short">{short}</span>
        </p>
      )}
      <nav className="topbar__places" aria-label="Window">
        <button
          type="button"
          className="topbar__button"
          aria-label="Search"
          title="Search"
          onClick={onSearch}
        >
          <Icon name="search" />
        </button>
        <button
          type="button"
          className="topbar__button"
          aria-label={groups.length > 1 ? "Groups and agents" : "Agents"}
          title={groups.length > 1 ? "Groups and agents" : "Agents"}
          onClick={onGroups}
        >
          <Icon name="people" />
        </button>
        <NewMenu
          onNewAgent={onNewAgent}
          onNewGroup={onNewGroup}
          onOpenCafeteria={onOpenCafeteria}
        />
        <button
          type="button"
          className="topbar__button"
          aria-label="App settings"
          title="App settings"
          onClick={onSettings}
        >
          <Icon name="settings" />
        </button>
      </nav>
    </header>
  );
}
