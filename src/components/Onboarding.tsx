import { useEffect, useId, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { HIREABLE, toDraft } from "../lib/cafeteria";
import { api } from "../lib/ipc";
import { FIRST_TASKS, type FirstTask, readFirstTask, saveFirstTask } from "../lib/onboarding";
import { useStore } from "../lib/store";
import { BEFORE_RELOAD, desktop } from "../lib/transport";
import { errorMessage, type Group } from "../lib/types";
import { OnboardingProvider } from "./OnboardingProvider";

interface Props {
  group: Group;
  onStarting: () => void;
  onDone: () => void;
  onInspect: (checking: boolean) => void;
  onBrowse: () => void;
  onCreate: () => void;
}

export function Onboarding({ group, onStarting, onDone, onInspect, onBrowse, onCreate }: Props) {
  const [draft, setDraft] = useState(readFirstTask);
  const [step, setStep] = useState<"task" | "provider" | "start">(
    draft.agentId || draft.key ? "start" : "task",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [storageError, setStorageError] = useState(false);
  const [checking, setChecking] = useState(false);
  const sending = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const briefHint = useId();
  const continueHint = useId();
  const settings = useStore((state) => state.settings);
  const agents = useStore((state) => state.agents);
  const refreshAgents = useStore((state) => state.refreshAgents);
  const select = useStore((state) => state.select);
  const task = FIRST_TASKS.find((entry) => entry.id === draft.task)!;
  const preset = HIREABLE.find((entry) => entry.id === task.agent)!;
  const hired = agents.find(
    (agent) => agent.id === draft.agentId && agent.lifecycle !== "terminated",
  );
  const name = hired?.name ?? preset.name;

  useEffect(() => {
    heading.current?.focus();
  }, [step, checking]);

  useEffect(() => {
    if (!storageError && !busy) return;
    const preserve = (event: Event) => {
      event.preventDefault();
      if (event.type === "beforeunload") (event as BeforeUnloadEvent).returnValue = "";
    };
    window.addEventListener("beforeunload", preserve);
    window.addEventListener(BEFORE_RELOAD, preserve);
    return () => {
      window.removeEventListener("beforeunload", preserve);
      window.removeEventListener(BEFORE_RELOAD, preserve);
    };
  }, [storageError, busy]);

  const keep = (next: FirstTask) => {
    setDraft(next);
    try {
      saveFirstTask(next);
      setStorageError(false);
      return true;
    } catch {
      setStorageError(true);
      return false;
    }
  };

  const finish = () => {
    setDraft((current) => ({ ...current, sent: true }));
    try {
      saveFirstTask(null);
      setStorageError(false);
      onDone();
    } catch {
      setStorageError(true);
    }
  };

  const open = async (agentId: string, sent = false) => {
    await refreshAgents();
    await select(agentId);
    if (sent) finish();
    else {
      setChecking(true);
      onInspect(true);
    }
  };

  const start = async () => {
    if (sending.current || !draft.brief.trim()) return;
    let next = { ...draft, key: draft.key ?? crypto.randomUUID() };
    if (!keep(next)) return;
    sending.current = true;
    setBusy(true);
    setError("");
    onStarting();
    try {
      if (next.agentId && !hired)
        throw new Error(
          "This agent is no longer available. Choose another task below; your brief is kept here.",
        );
      if (!next.agentId) {
        const created = await api.hireAgents(group.id, [toDraft(preset)], next.key);
        if (!created[0]) throw new Error("The host did not return an agent. Try again.");
        next = { ...next, agentId: created[0].id };
        const saved = keep(next);
        useStore.setState((state) => ({
          agents: [...state.agents.filter((agent) => agent.id !== created[0]!.id), created[0]!],
        }));
        if (!saved) return;
      }
      if (!next.sent) {
        await api.sendMessage(next.agentId!, next.brief.trim(), [], next.key);
        next = { ...next, sent: true };
        if (!keep(next)) return;
      }
      await open(next.agentId!, true);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };

  const warning = storageError && (
    <div className="banner banner--error" role="alert">
      <span>
        Your first-task progress could not be saved on this device. Keep this window open and copy
        your brief before leaving. Allow storage for this app or free space, then retry saving.
      </span>
      <button type="button" className="btn" disabled={busy} onClick={() => keep(draft)}>
        Retry saving
      </button>
    </div>
  );

  if (checking)
    return (
      <>
        {warning}
        <div className="banner onboarding-recovery" role="status">
          <span>
            Your first brief is still kept. Check whether it arrived below, then return to retry or
            confirm delivery.
          </span>
          <div className="onboarding-recovery__actions">
            <button
              type="button"
              className="btn"
              onClick={() => {
                setChecking(false);
                onInspect(false);
              }}
            >
              Return to first task
            </button>
            <button type="button" className="btn" onClick={finish}>
              Task was delivered
            </button>
          </div>
        </div>
      </>
    );

  return (
    <section className="onboarding" aria-label="Get started with Guaca" aria-busy={busy}>
      <div className="onboarding-content">
        {warning}
        <ol className="onboarding-steps" aria-label="Getting started">
          {(
            [
              ["task", "Your task"],
              ["provider", "Connect AI"],
              ["start", "Start"],
            ] as const
          ).map(([id, label], index) => (
            <li key={id} aria-current={step === id ? "step" : undefined}>
              <span>{index + 1}</span>
              {label}
            </li>
          ))}
        </ol>
        {step === "task" ? (
          <>
            <div className="onboarding-intro">
              <p className="onboarding-eyebrow">A little help to get going</p>
              <h1 ref={heading} tabIndex={-1}>
                What would you like help with?
              </h1>
              <p>Start with one task and one agent. You can build a group as you go.</p>
            </div>
            <fieldset className="onboarding-options" aria-label="First task">
              {FIRST_TASKS.map((entry) => (
                <button
                  type="button"
                  className="onboarding-choice"
                  key={entry.id}
                  aria-pressed={draft.task === entry.id}
                  onClick={() =>
                    keep({
                      ...draft,
                      task: entry.id,
                      brief:
                        !draft.brief || FIRST_TASKS.some((item) => item.brief === draft.brief)
                          ? entry.brief
                          : draft.brief,
                    })
                  }
                >
                  <strong>{entry.title}</strong>
                  <span>{entry.description}</span>
                </button>
              ))}
            </fieldset>
            <label className="field">
              <span className="field__label">Your first brief</span>
              <textarea
                className="input onboarding-brief"
                aria-label="Your first brief"
                aria-describedby={briefHint}
                rows={4}
                value={draft.brief}
                placeholder="What are you working on, and what would a useful result look like?"
                onChange={(event) => keep({ ...draft, brief: event.target.value })}
              />
              <span className="field__hint" id={briefHint}>
                Make it your own. Nothing is sent until you press Start task.
              </span>
            </label>
            <div className="onboarding-actions">
              <button
                type="button"
                className="btn btn--primary"
                aria-describedby={continueHint}
                disabled={!draft.brief.trim()}
                onClick={() => setStep("provider")}
              >
                Continue
              </button>
              {/* A button that will not press and says nothing is the first
                  thing this app does to a stranger. The reason sits here, on
                  every state of it, and is the button's own description so a
                  screen reader hears it too. */}
              <span id={continueHint}>
                {!draft.brief.trim()
                  ? "Pick one of the tasks above, or write your own brief, to continue."
                  : settings?.provider === "compatible" && !settings.apiKeySet
                    ? "Next: add an API key or connect ChatGPT."
                    : "No Guaca account needed."}
              </span>
            </div>
            <div className="onboarding-alternatives">
              <span>Prefer to set things up yourself?</span>
              <button type="button" className="btn btn--ghost" onClick={onBrowse}>
                Open the cafeteria
              </button>
              <button type="button" className="btn btn--ghost" onClick={onCreate}>
                Create one agent
              </button>
            </div>
          </>
        ) : step === "provider" ? (
          <>
            <div className="onboarding-intro">
              <h1 ref={heading} tabIndex={-1}>
                Connect your AI
              </h1>
              <p>Choose what powers your agent. Your first brief is waiting for you.</p>
            </div>
            <OnboardingProvider
              onConnected={() => setStep("start")}
              onBack={() => setStep("task")}
            />
          </>
        ) : (
          <>
            <div className="onboarding-intro">
              <p className="onboarding-eyebrow">One task. Someone to help.</p>
              <h1 ref={heading} tabIndex={-1}>
                Ready when you are.
              </h1>
              <p>Review your brief, then let {name} take the first step.</p>
            </div>
            <div className="onboarding-agent">
              <AgentAvatar
                avatar={hired?.avatar ?? preset.avatar}
                color={hired?.color ?? preset.color}
                size="lg"
                seed={task.agent}
              />
              <div>
                <span className="onboarding-eyebrow">
                  {draft.agentId ? "Your first agent" : "Recommended for this task"}
                </span>
                <h2>{name}</h2>
                <p>{preset.tagline}</p>
              </div>
            </div>
            <label className="field">
              <span className="field__label">Your first brief</span>
              <textarea
                className="input onboarding-brief"
                rows={5}
                aria-label="Your first brief"
                value={draft.brief}
                readOnly={busy || !!draft.key || !!draft.agentId}
                onChange={(event) => keep({ ...draft, brief: event.target.value })}
              />
            </label>
            <p className="onboarding-notice">
              {draft.sent
                ? "Your task was sent. Open the conversation to see how it is going."
                : draft.agentId
                  ? "Your agent is already created. If a connection failed, check the conversation before sending again."
                  : draft.key
                    ? "Your previous start may have created this agent. Retry with the same brief to continue safely."
                    : "Start task creates this agent and sends your brief using the provider you just connected. Other services can be connected later, when needed."}
            </p>
            {/* The last step, and the last moment to say where Guaca goes when
                the window does. A browser has no menu bar, so it is only said
                where there is one to look at. */}
            {desktop && (
              <p className="onboarding-notice">
                After you start, you can close this window: Guaca stays in the menu bar, and its
                icon opens a panel of what is waiting on you and who is working.
              </p>
            )}
            {error && (
              <div className="banner banner--error" role="alert">
                {error}
              </div>
            )}
            <div className="onboarding-actions">
              {!draft.agentId && !draft.key && (
                <button
                  type="button"
                  className="btn"
                  disabled={busy}
                  onClick={() => setStep("provider")}
                >
                  Back
                </button>
              )}
              {draft.agentId && !hired && (
                <button
                  type="button"
                  className="btn"
                  disabled={busy}
                  onClick={() => {
                    keep({ ...draft, agentId: null, sent: false, key: undefined });
                    setError("");
                    setStep("task");
                  }}
                >
                  Choose another task
                </button>
              )}
              {draft.agentId && hired && !draft.sent && (
                <button
                  type="button"
                  className="btn"
                  disabled={busy}
                  onClick={() =>
                    void open(draft.agentId!).catch((cause) => setError(errorMessage(cause)))
                  }
                >
                  Check conversation
                </button>
              )}
              <button
                type="button"
                className="btn btn--primary"
                disabled={busy || !draft.brief.trim() || (!!draft.agentId && !hired)}
                onClick={() => void start()}
              >
                {busy
                  ? "Starting your task…"
                  : draft.sent
                    ? "Open conversation"
                    : draft.agentId
                      ? "Send brief again"
                      : "Start task"}
              </button>
            </div>
          </>
        )}
      </div>
    </section>
  );
}
