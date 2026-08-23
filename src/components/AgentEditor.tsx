import { useEffect, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import {
  ACCENTS,
  CHARACTER_GROUPS,
  CHARACTERS,
  suggestAccent,
  suggestCharacter,
} from "../avatars/catalog";
import { api } from "../lib/ipc";
import { onOpenRouter } from "../lib/providers";
import { useStore } from "../lib/store";
import { type AgentCard, type AgentDraft, errorMessage } from "../lib/types";
import { GrantList } from "./GrantList";
import { ModelSuggestions } from "./ModelSuggestions";
import { SigninList } from "./SigninList";

interface Props {
  /** Absent when creating. */
  agent?: AgentCard;
  onClose: () => void;
}

const PROMPT_PLACEHOLDER =
  "What this agent is for, and how it should behave. For example: You coordinate the other agents. Delegate rather than doing work yourself, and keep replies to two sentences.";

export function AgentEditor({ agent, onClose }: Props) {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const settings = useStore((s) => s.settings);
  const select = useStore((s) => s.select);
  const refreshAgents = useStore((s) => s.refreshAgents);

  const [draft, setDraft] = useState<AgentDraft>(() => ({
    name: agent?.name ?? "",
    groupId: agent?.groupId,
    avatar: agent?.avatar ?? suggestCharacter(agents.map((a) => a.avatar)),
    color: agent?.color ?? suggestAccent(agents.map((a) => a.color)),
    model: agent?.model ?? settings?.defaultModel ?? "",
    systemPrompt: agent?.systemPrompt ?? "",
    skills: agent?.skills ?? [],
  }));
  const [skillText, setSkillText] = useState(agent?.skills.join(", ") ?? "");
  const [notes, setNotes] = useState("");
  const [notesLoaded, setNotesLoaded] = useState(!agent);
  const [notesDirty, setNotesDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);

  // Focus moves into the dialog on open. Done explicitly rather than with
  // autoFocus so the timing is under our control.
  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  // An agent's memory lives on disk rather than on the card, so it is fetched
  // separately and only written back when the operator actually edited it.
  // Saving it unconditionally would clobber anything the agent wrote while this
  // dialog was open.
  useEffect(() => {
    if (!agent) return;
    let cancelled = false;
    void api
      .agentNotes(agent.id)
      .then((content) => {
        if (cancelled) return;
        setNotes(content);
        setNotesLoaded(true);
      })
      .catch(() => setNotesLoaded(true));
    return () => {
      cancelled = true;
    };
  }, [agent]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const patch = (fields: Partial<AgentDraft>) => setDraft((current) => ({ ...current, ...fields }));

  // The field is one string and a card holds a list, so the split is what both
  // saving and the model suggestions read the skills through.
  const skills = skillText
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

  // Which crew's settings decide how this agent's turns are paid for. The
  // select below defaults to the first group, so an agent that has not been
  // given one yet is already in it. Named for the crew rather than the group
  // because the character picker below binds `group` to something else.
  const crew = groups.find((entry) => entry.id === draft.groupId) ?? groups[0];

  const save = async () => {
    setBusy(true);
    setError(null);
    const payload: AgentDraft = { ...draft, skills };
    try {
      if (agent) {
        await api.updateAgent(agent.id, payload);
        if (notesDirty) await api.setAgentNotes(agent.id, notes);
      } else {
        const created = await api.createAgent(payload);
        await refreshAgents();
        await select(created.id);
      }
      onClose();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!agent) return;
    setBusy(true);
    try {
      await api.deleteAgent(agent.id);
      onClose();
    } catch (caught) {
      setError(errorMessage(caught));
      setBusy(false);
    }
  };

  return (
    <div className="scrim">
      {/* A real button, so dismissing by clicking away is reachable from the
          keyboard and announced, rather than being an invisible div handler. */}
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-label={agent ? "Edit agent" : "New agent"}
      >
        <div style={{ display: "flex", gap: "0.9rem", alignItems: "center", marginBottom: "1rem" }}>
          <AgentAvatar avatar={draft.avatar} color={draft.color} size="lg" seed={agent?.id} />
          <div>
            <h2 className="dialog__title">{agent ? draft.name || agent.name : "New agent"}</h2>
            <p className="dialog__lede" style={{ margin: 0 }}>
              {agent
                ? `Version ${agent.version} · other agents see the name and skills, never the instructions.`
                : "Other agents see the name and skills. The instructions stay private."}
            </p>
          </div>
        </div>

        <label className="field">
          <span className="field__label">Name</span>
          <input
            className="input input--mono"
            ref={nameRef}
            value={draft.name}
            maxLength={48}
            placeholder="Manager"
            onChange={(event) => patch({ name: event.target.value })}
          />
          <span className="field__hint">
            Agents address each other by this name, so make it short and unambiguous.
          </span>
        </label>

        <div className="field">
          <span className="field__label">Colour</span>
          <div style={{ display: "flex", gap: "0.35rem", flexWrap: "wrap" }}>
            {ACCENTS.map((accent) => (
              <button
                key={accent.value}
                type="button"
                className="swatch"
                title={accent.name}
                aria-label={accent.name}
                aria-pressed={draft.color.toLowerCase() === accent.value}
                style={{ "--swatch": accent.value } as React.CSSProperties}
                onClick={() => patch({ color: accent.value })}
              />
            ))}
          </div>
        </div>

        <div className="field">
          <span className="field__label">Character</span>
          {CHARACTER_GROUPS.map((group) => (
            <div key={group} style={{ marginBottom: "0.55rem" }}>
              <span className="hint" style={{ display: "block", marginBottom: "0.25rem" }}>
                {group}
              </span>
              <div className="picker">
                {CHARACTERS.filter((entry) => entry.group === group).map((entry) => (
                  <button
                    key={entry.key}
                    type="button"
                    className="picker__item"
                    title={entry.label}
                    aria-label={entry.label}
                    aria-pressed={draft.avatar === entry.key}
                    style={{ "--accent": draft.color } as React.CSSProperties}
                    onClick={() => patch({ avatar: entry.key })}
                  >
                    <AgentAvatar
                      avatar={entry.key}
                      color={draft.color}
                      size="sm"
                      seed={entry.key}
                    />
                  </button>
                ))}
              </div>
            </div>
          ))}
        </div>

        {/* Only shown once a second group exists. With one group there is no
            choice to make, and the field would be asking about a boundary the
            operator has not drawn yet. */}
        {groups.length > 1 && (
          <label className="field">
            <span className="field__label">Group</span>
            <select
              className="input input--mono"
              value={draft.groupId ?? groups[0]?.id ?? ""}
              onChange={(event) => patch({ groupId: event.target.value })}
            >
              {groups.map((group) => (
                <option key={group.id} value={group.id}>
                  {group.name}
                </option>
              ))}
            </select>
            <span className="field__hint">
              Agents can only see and message others in the same group.
            </span>
          </label>
        )}

        <label className="field">
          <span className="field__label">Model</span>
          <input
            className="input input--mono"
            value={draft.model}
            placeholder={settings?.defaultModel || "anthropic/claude-sonnet-4.5"}
            onChange={(event) => patch({ model: event.target.value })}
          />
          <span className="field__hint">
            Any model slug your endpoint accepts. Agents can each use a different one.
          </span>
        </label>

        <ModelSuggestions
          name={draft.name}
          skills={skills}
          instructions={draft.systemPrompt}
          model={draft.model}
          active={onOpenRouter(crew, settings)}
          onChoose={(model) => patch({ model })}
        />

        <label className="field">
          <span className="field__label">Skills</span>
          <input
            className="input"
            value={skillText}
            placeholder="scheduling, delegation"
            onChange={(event) => setSkillText(event.target.value)}
          />
          <span className="field__hint">
            Comma separated. This is what other agents read when deciding who to ask.
          </span>
        </label>

        {agent && (
          <label className="field">
            <span className="field__label">Memory</span>
            <textarea
              className="textarea input--mono"
              value={notesLoaded ? notes : "Loading…"}
              rows={6}
              disabled={!notesLoaded}
              placeholder="Empty. The agent writes here when it learns something durable."
              onChange={(event) => {
                setNotes(event.target.value);
                setNotesDirty(true);
              }}
            />
            <span className="field__hint">
              The agent's own memory, shown to it every turn and rewritten by it with{" "}
              <code>update_notes</code>: ask it to remember something, or to update its memory, and
              this is the file it writes. Seed a persona here if you like. Stored as markdown in the
              workspace folder beside the database.
            </span>
          </label>
        )}

        {agent && <SigninList agent={agent} />}

        {agent && <GrantList agent={agent} />}

        <label className="field">
          <span className="field__label">Instructions</span>
          <textarea
            className="textarea"
            value={draft.systemPrompt}
            rows={5}
            placeholder={PROMPT_PLACEHOLDER}
            onChange={(event) => patch({ systemPrompt: event.target.value })}
          />
        </label>

        {error && (
          <div className="banner banner--error" style={{ margin: "0 0 0.9rem" }}>
            <span>{error}</span>
          </div>
        )}

        <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
          {agent &&
            (confirmDelete ? (
              <>
                <button
                  type="button"
                  className="btn btn--danger"
                  disabled={busy}
                  onClick={() => void remove()}
                >
                  Delete {agent.name}
                </button>
                <button
                  type="button"
                  className="btn btn--ghost"
                  onClick={() => setConfirmDelete(false)}
                >
                  Keep
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn--danger"
                onClick={() => setConfirmDelete(true)}
              >
                Delete
              </button>
            ))}

          <div style={{ marginLeft: "auto", display: "flex", gap: "0.5rem" }}>
            <button type="button" className="btn" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className="btn btn--primary"
              disabled={busy || draft.name.trim().length === 0}
              onClick={() => void save()}
            >
              {agent ? "Save changes" : "Create agent"}
            </button>
          </div>
        </div>

        {agent && confirmDelete && (
          <p className="field__hint" style={{ marginTop: "0.6rem" }}>
            {agent.name} leaves the sidebar and stops receiving messages. What it already said stays
            readable in the other agents' channels, and the name becomes free to reuse.
          </p>
        )}
      </div>
    </div>
  );
}
