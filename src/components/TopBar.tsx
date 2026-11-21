import { IS_MAC } from "../lib/keybinds";
import { APP_SETTINGS, groupsLabel } from "../lib/menu";
import { useStore } from "../lib/store";
import { desktop } from "../lib/transport";
import { Icon } from "./Icon";
import { NewMenu } from "./NewMenu";

export interface WindowActionsProps {
  onSearch: () => void;
  onGroups: () => void;
  onSettings: () => void;
  onNewAgent: () => void;
  onNewGroup: () => void;
  onOpenCafeteria: () => void;
}

export const LIGHTS = desktop && IS_MAC;

export function WindowActions({
  onSearch,
  onGroups,
  onSettings,
  onNewAgent,
  onNewGroup,
  onOpenCafeteria,
}: WindowActionsProps) {
  const groups = useStore((state) => state.groups);
  return (
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
        aria-label={groupsLabel(groups.length)}
        title={groupsLabel(groups.length)}
        onClick={onGroups}
      >
        <Icon name="group" />
      </button>
      <NewMenu onNewAgent={onNewAgent} onNewGroup={onNewGroup} onOpenCafeteria={onOpenCafeteria} />
      <button
        type="button"
        className="topbar__button"
        aria-label={APP_SETTINGS}
        title={APP_SETTINGS}
        onClick={onSettings}
      >
        <Icon name="gear" />
      </button>
    </nav>
  );
}

export function TopBar({
  back,
  actions,
}: {
  back: { to: string; go: () => void } | null;
  actions: WindowActionsProps | null;
}) {
  if (!back && !actions && !LIGHTS) return null;
  return (
    <header className="topbar" data-lights={LIGHTS || undefined} data-tauri-drag-region>
      {back && (
        <button type="button" className="topbar__back" onClick={back.go}>
          <Icon name="chevron-left" />
          {back.to}
        </button>
      )}
      {actions && <WindowActions {...actions} />}
    </header>
  );
}
