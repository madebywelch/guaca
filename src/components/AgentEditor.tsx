import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { ACCENTS, CHARACTERS, suggestAccent, suggestCharacter } from "../avatars/catalog";
import { useDismiss } from "../lib/dismiss";
import { focusWriting } from "../lib/focus";
import { api } from "../lib/ipc";
import { onOpenRouter, providerFor } from "../lib/providers";
import { useStore } from "../lib/store";
import { type AgentCard, type AgentDraft, errorMessage } from "../lib/types";
import { GrantList } from "./GrantList";
import { Icon } from "./Icon";
import { ModelSuggestions } from "./ModelSuggestions";
import { SubscriptionModel } from "./ProviderFields";
import { Scrim } from "./Scrim";
import { SigninList } from "./SigninList";

interface Props {
  agent?: AgentCard;
  onClose: () => void;
}

const IME_KEY = 229;

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
    model: agent?.model ?? "",
    subscriptionModel: agent?.subscriptionModel ?? "",
    reasoningEffort: agent?.reasoningEffort ?? null,
    systemPrompt: agent?.systemPrompt ?? "",
    skills: agent?.skills ?? [],
  }));
  const [skillText, setSkillText] = useState(agent?.skills.join(", ") ?? "");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [more, setMore] = useState(agent !== undefined);
  const nameRef = useRef<HTMLInputElement>(null);
  const saving = useRef(false);
  const created = useRef(false);

  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  useDismiss(onClose);

  useEffect(
    () => () => {
      if (created.current) focusWriting();
    },
    [],
  );

  const patch = (fields: Partial<AgentDraft>) => setDraft((current) => ({ ...current, ...fields }));

  const skills = skillText
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

  const crew = groups.find((entry) => entry.id === draft.groupId) ?? groups[0];
  const provider = crew?.inference.provider ?? settings?.provider;
  const endpoint = crew?.inference.baseUrl || settings?.baseUrl || "";
  const providerName =
    provider === "chatgpt"
      ? "ChatGPT subscription"
      : provider === "claude"
        ? "Claude subscription"
        : providerFor(endpoint)?.name || endpoint || "App endpoint";
  const inheritedModel =
    provider === "chatgpt"
      ? crew?.inference.subscriptionModel || settings?.subscriptionModel || ""
      : crew?.inference.defaultModel || settings?.defaultModel || "";
  const inherit = `Use group default${inheritedModel ? ` · ${inheritedModel}` : ""}`;

  const save = async () => {
    if (saving.current || draft.name.trim().length === 0) return;
    saving.current = true;
    setBusy(true);
    setError(null);
    const payload: AgentDraft = { ...draft, skills };
    try {
      if (agent) {
        await api.updateAgent(agent.id, payload);
      } else {
        const made = await api.createAgent(payload);
        await refreshAgents();
        await select(made.id);
        created.current = true;
      }
      onClose();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };

  const submitOnReturn = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" || event.nativeEvent.isComposing) return;
    if (event.nativeEvent.keyCode === IME_KEY || event.target instanceof HTMLButtonElement) return;
    if (event.target !== nameRef.current && !event.metaKey && !event.ctrlKey) return;
    event.preventDefault();
    void save();
  };

  const colorGroup = useId();
  const characterGroup = useId();

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
    <Scrim>
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog dialog--form"
        role="dialog"
        aria-modal="true"
        aria-label={agent ? "Edit agent" : "New agent"}
        onKeyDown={submitOnReturn}
      >
        <div className="dialog__body">
          <div
            style={{
              display: "flex",
              gap: "0.9rem",
              alignItems: "center",
              marginBottom: "1rem",
            }}
          >
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
            <span className="field__label" title="Agents address each other by this name.">
              Name
            </span>
            <input
              className="input"
              ref={nameRef}
              value={draft.name}
              maxLength={48}
              placeholder="Manager"
              onChange={(event) => patch({ name: event.target.value })}
            />
          </label>

          <fieldset className="field field--choices">
            <legend className="field__label">Color</legend>
            <div className="swatches">
              {ACCENTS.map((accent) => (
                <label
                  key={accent.value}
                  className="swatch"
                  title={accent.name}
                  style={{ "--swatch": accent.value } as React.CSSProperties}
                >
                  <input
                    type="radio"
                    className="pick__input"
                    name={colorGroup}
                    aria-label={accent.name}
                    checked={draft.color.toLowerCase() === accent.value}
                    onChange={() => patch({ color: accent.value })}
                  />
                </label>
              ))}
            </div>
          </fieldset>

          <fieldset className="field field--choices">
            <legend className="field__label">Character</legend>
            <div className="picker">
              {CHARACTERS.map((entry) => (
                <label
                  key={entry.key}
                  className="picker__item"
                  title={entry.label}
                  style={{ "--accent": draft.color } as React.CSSProperties}
                >
                  <input
                    type="radio"
                    className="pick__input"
                    name={characterGroup}
                    aria-label={entry.label}
                    checked={draft.avatar === entry.key}
                    onChange={() => patch({ avatar: entry.key })}
                  />
                  <AgentAvatar
                    avatar={entry.key}
                    color={draft.color}
                    size="sm"
                    seed={entry.key}
                    plain
                  />
                </label>
              ))}
            </div>
          </fieldset>

          {groups.length > 1 && (
            <label className="field">
              <span
                className="field__label"
                title="Agents can only see and message others in the same group."
              >
                Group
              </span>
              <select
                className="input"
                value={draft.groupId ?? groups[0]?.id ?? ""}
                onChange={(event) => patch({ groupId: event.target.value })}
              >
                {groups.map((group) => (
                  <option key={group.id} value={group.id}>
                    {group.name}
                  </option>
                ))}
              </select>
            </label>
          )}

          <details
            className="disclosure agent-editor__more"
            open={more}
            onToggle={(event) => setMore(event.currentTarget.open)}
          >
            <summary>
              <Icon name="chevron-right" size={16} />
              Instructions, skills and model
            </summary>

            <div className="field">
              <span className="field__label">Provider · {providerName}</span>
              <span className="field__hint">Set in {crew?.name ?? "the group"}'s settings.</span>
            </div>

            {provider === "chatgpt" ? (
              <SubscriptionModel
                value={draft.subscriptionModel ?? ""}
                effort={draft.reasoningEffort ?? null}
                onEffortChange={(reasoningEffort) => patch({ reasoningEffort })}
                inheritedModel={inheritedModel}
                inheritedEffort={
                  crew?.inference.reasoningEffort ?? settings?.reasoningEffort ?? "auto"
                }
                effortInherit="Use group default"
                models={settings?.subscriptionModels ?? []}
                inherit={inherit}
                onChange={(subscriptionModel) => patch({ subscriptionModel })}
                hint="From your signed-in account."
              />
            ) : provider === "claude" ? (
              <div className="field">
                <span className="field__label">Model</span>
                <span
                  className="field__hint"
                  title="A per-agent model is not used with this provider. A saved one is kept for when you switch back."
                >
                  Claude controls which model runs.
                </span>
              </div>
            ) : (
              <>
                <label className="field">
                  <span className="field__label">Model</span>
                  <input
                    className="input"
                    value={draft.model}
                    placeholder={inherit}
                    title={`A model ID ${providerName} accepts. Blank uses the group default.`}
                    onChange={(event) => patch({ model: event.target.value })}
                  />
                </label>
                {draft.model && (
                  <div className="field">
                    <button
                      type="button"
                      className="btn btn--compact"
                      onClick={() => patch({ model: "" })}
                    >
                      Use group default
                    </button>
                  </div>
                )}
              </>
            )}

            <ModelSuggestions
              name={draft.name}
              skills={skills}
              instructions={draft.systemPrompt}
              model={draft.model}
              active={onOpenRouter(crew, settings)}
              onChoose={(model) => patch({ model })}
            />

            <label className="field">
              <span
                className="field__label"
                title="What other agents read when deciding who to ask. Comma separated."
              >
                Skills
              </span>
              <input
                className="input"
                value={skillText}
                placeholder="scheduling, delegation"
                onChange={(event) => setSkillText(event.target.value)}
              />
            </label>

            {agent && <SigninList agent={agent} />}

            {agent && <GrantList agent={agent} />}

            {agent && <WebSwitch agent={agent} />}

            {agent && <ErrandSwitch agent={agent} />}

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
          </details>
        </div>

        <div className="dialog__foot">
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
                    Delete {agent.name}, into the compost
                  </button>
                  <button
                    type="button"
                    className="btn btn--quiet"
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
                aria-busy={busy}
                onClick={() => void save()}
              >
                {agent ? "Save changes" : "Create agent"}
              </button>
            </div>
          </div>

          {agent && confirmDelete && (
            <p className="field__hint" style={{ marginTop: "0.6rem" }}>
              {agent.name} leaves the Groups sheet and stops receiving messages. What it already
              said stays readable in the other agents' channels, and the name becomes free to reuse.
            </p>
          )}
        </div>
      </div>
    </Scrim>
  );
}

function WebSwitch({ agent }: { agent: AgentCard }) {
  const [on, setOn] = useState(agent.readsWeb);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const change = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await api.setAgentWeb(agent.id, next);
      setOn(next);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <label
      className="field field--row"
      title={`${agent.name} searches the web through the provider this group uses, or the search key in Settings, and reads pages as text fetched by this host. It reads only: it cannot sign in, click or fill in a form, and it never fetches an address on this host's own network.`}
    >
      <input
        type="checkbox"
        checked={on}
        disabled={busy}
        onChange={(event) => void change(event.target.checked)}
      />
      <span>
        <span className="field__label">Web reading</span>
        <span className="field__hint">
          Searches the web and reads pages as text, with no browser or account.
        </span>
        {error && <span className="field__hint">{error}</span>}
      </span>
    </label>
  );
}

function ErrandSwitch({ agent }: { agent: AgentCard }) {
  const [on, setOn] = useState(agent.runsErrands);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const change = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await api.setAgentErrands(agent.id, next);
      setOn(next);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <label
      className="field field--row"
      title={`${agent.name} hands a piece of work to its own model, called again with one brief and its tools, up to three at once, and reads back what each found. An errand cannot message anyone, rewrite ${agent.name}'s memory or send errands of its own. Every errand spends model calls from the same conversation's budget.`}
    >
      <input
        type="checkbox"
        checked={on}
        disabled={busy}
        onChange={(event) => void change(event.target.checked)}
      />
      <span>
        <span className="field__label">Errands</span>
        <span className="field__hint">
          Up to three side tasks per turn, on {agent.name}'s own model, at extra cost.
        </span>
        {error && <span className="field__hint">{error}</span>}
      </span>
    </label>
  );
}
