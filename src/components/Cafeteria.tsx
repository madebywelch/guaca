import { useEffect, useMemo, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { HIREABLE, type Hireable, pick, STARTER_CREW, STATIONS, toDraft } from "../lib/cafeteria";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { errorMessage, type GroupId } from "../lib/types";

interface Props {
  onClose: () => void;
}

/**
 * Agents that are already set up, and the group they get hired into.
 *
 * Browsing and picking, then one action, then back to the workspace. That shape
 * is a dialog rather than a place, which is why this is not a channel: there is
 * nothing here to come back to, no state worth addressing, and nothing that
 * changes while you are not looking at it.
 *
 * Everything is a copy. A hired agent has no link back to the preset it came
 * from, so editing it later is editing an ordinary agent and this file cannot
 * become something the database has to agree with.
 */
export function Cafeteria({ onClose }: Props) {
  const groups = useStore((s) => s.groups);
  const agents = useStore((s) => s.agents);
  const select = useStore((s) => s.select);
  const refreshAgents = useStore((s) => s.refreshAgents);
  const railGroup = useStore((s) => s.railGroup);
  const selected = useStore((s) => s.selected);

  const [picked, setPicked] = useState<Set<string>>(new Set());
  /**
   * The crew the operator is standing in, which is the one they came to hire
   * into.
   *
   * Read once, when the dialog opens. Defaulting to the first group instead
   * pointed everything below at somebody else's crew: an operator who had just
   * emptied a group was told the preset they were about to hire was already
   * "on staff", offered no starter crew for a room with nobody in it, and
   * would have hired into a group they were not looking at. Three symptoms,
   * one wrong id.
   *
   * The rail's focus is the answer when there is one, because that is the crew
   * the rail is inside. With the rail in the overview the open channel is what
   * says where the operator is, and `select` has already followed that agent
   * into its crew. Neither, on a workspace with no channel open, falls back to
   * the first group as before.
   */
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

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // A group deleted in another window would leave the picker holding an id
  // nothing matches, and the hire would be refused by a command the operator
  // never saw make a choice.
  useEffect(() => {
    if (groups.length > 0 && !groups.some((group) => group.id === groupId)) {
      setGroupId(groups[0]!.id);
    }
  }, [groups, groupId]);

  const group = groups.find((entry) => entry.id === groupId);

  /**
   * Who is already in the group being hired into.
   *
   * Shown on the card rather than enforced, because hiring a second Researcher
   * is a thing operators do on purpose. The runtime settles the name; this is
   * only so nobody is surprised by what it settles on.
   */
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
      // Opening the first hire is what makes the button feel like it did
      // something: the rail is ordered by who spoke last, and nobody hired has
      // spoken, so a new crew lands at the bottom of a busy workspace.
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

          {/* Only asked once there is a choice to make. With one group the
              boundary has not been drawn yet, and the button below names the
              destination in every case anyway. */}
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
          {/* Only offered to a group with nobody in it. A crew of four is a
              suggestion for an empty room, and says nothing useful to an
              operator who already has thirty agents. */}
          {employed.size === 0 && (
            <button
              type="button"
              className="btn btn--ghost"
              onClick={() => setPicked(new Set(STARTER_CREW))}
            >
              Pick a starter crew
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
