import { useEffect, useMemo, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { HIREABLE, type Hireable, pick, STARTER_CREW, STATIONS, toDraft } from "../lib/cafeteria";
import { useDismiss } from "../lib/dismiss";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { errorMessage, type GroupId } from "../lib/types";

interface Props {
  onClose: () => void;
}

export function Cafeteria({ onClose }: Props) {
  const groups = useStore((s) => s.groups);
  const agents = useStore((s) => s.agents);
  const select = useStore((s) => s.select);
  const refreshAgents = useStore((s) => s.refreshAgents);
  const railGroup = useStore((s) => s.railGroup);
  const selected = useStore((s) => s.selected);

  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [groupId, setGroupId] = useState<GroupId | "">(() => {
    const open = selected !== null ? agents.find((a) => a.id === selected)?.groupId : undefined;
    return railGroup ?? open ?? groups[0]?.id ?? "";
  });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  useDismiss(onClose);

  useEffect(() => {
    if (groups.length > 0 && !groups.some((group) => group.id === groupId)) {
      setGroupId(groups[0]!.id);
    }
  }, [groups, groupId]);

  const group = groups.find((entry) => entry.id === groupId);

  const employed = useMemo(() => {
    const here = agents.filter((a) => a.groupId === groupId && a.lifecycle !== "terminated");
    return new Set(here.map((a) => a.name.toLowerCase()));
  }, [agents, groupId]);

  const toggle = (id: string) =>
    setPicked((current) => {
      const next = new Set(current);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const hire = async () => {
    if (!group || picked.size === 0) return;
    setBusy(true);
    setError(null);
    try {
      const hired = await api.hireAgents(group.id, pick(picked).map(toDraft));
      await refreshAgents();
      if (hired[0]) await select(hired[0].id);
      onClose();
    } catch (caught) {
      setError(errorMessage(caught));
      setBusy(false);
    }
  };

  const card = (preset: Hireable) => {
    const chosen = picked.has(preset.id);
    const already = employed.has(preset.name.toLowerCase());
    return (
      <button
        key={preset.id}
        type="button"
        className="hire"
        aria-pressed={chosen}
        style={{ "--accent": preset.color } as React.CSSProperties}
        onClick={() => toggle(preset.id)}
      >
        <span className="hire__face">
          <AgentAvatar avatar={preset.avatar} color={preset.color} size="md" seed={preset.id} />
        </span>
        <span className="hire__body">
          <span className="hire__name">
            {preset.name}
            {already && (
              <span
                className="hire__already"
                title={`${group?.name ?? "This group"} already has one`}
              >
                on staff
              </span>
            )}
          </span>
          <span className="hire__tagline">{preset.tagline}</span>
          <span className="hire__skills">
            {preset.skills.map((skill) => (
              <span key={skill} className="hire__skill">
                {skill}
              </span>
            ))}
          </span>
        </span>
      </button>
    );
  };

  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog dialog--cafeteria"
        role="dialog"
        aria-modal="true"
        aria-label="Cafeteria"
        tabIndex={-1}
        ref={panelRef}
      >
        <div className="cafeteria__head">
          <div>
            <h2 className="dialog__title">Cafeteria</h2>
            <p className="dialog__lede" style={{ margin: 0 }}>
              Agents that are already set up. Hire as many as you like: each one arrives as an
              ordinary agent you can rename, rewrite or delete.
            </p>
          </div>

          {groups.length > 1 && (
            <label className="field" style={{ margin: 0, minWidth: "11rem" }}>
              <span className="field__label">Hire into</span>
              <select
                className="input input--mono"
                value={groupId}
                onChange={(event) => setGroupId(event.target.value)}
              >
                {groups.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.name}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>

        <div className="cafeteria__stations">
          {STATIONS.map((station) => (
            <section key={station} className="cafeteria__station">
              <h3 className="cafeteria__station-name">{station}</h3>
              <div className="cafeteria__grid">
                {HIREABLE.filter((preset) => preset.station === station).map(card)}
              </div>
            </section>
          ))}
        </div>

        {error && (
          <div className="banner banner--error" style={{ margin: "0 1.35rem" }}>
            <span>{error}</span>
          </div>
        )}

        <div className="cafeteria__foot">
          {employed.size === 0 && (
            <button
              type="button"
              className="btn btn--ghost"
              onClick={() => setPicked(new Set(STARTER_CREW))}
            >
              Pick a starter group
            </button>
          )}
          <button
            type="button"
            className="btn btn--ghost"
            disabled={picked.size === 0}
            onClick={() => setPicked(new Set())}
          >
            Clear
          </button>

          <span className="cafeteria__spacer" />

          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--primary"
            disabled={busy || picked.size === 0 || !group}
            onClick={() => void hire()}
          >
            {busy
              ? "Hiring…"
              : picked.size === 0
                ? "Nobody picked"
                : `Hire ${picked.size} into ${group?.name ?? "…"}`}
          </button>
        </div>
      </div>
    </div>
  );
}
