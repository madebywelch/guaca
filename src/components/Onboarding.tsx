import { useEffect, useId, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { HIREABLE, toDraft } from "../lib/cafeteria";
import { api } from "../lib/ipc";
import { FIRST_TASKS, type FirstTask, readFirstTask, saveFirstTask } from "../lib/onboarding";
import { useStore } from "../lib/store";
import { errorMessage, type Group } from "../lib/types";
import { OnboardingProvider } from "./OnboardingProvider";

interface Props {
  group: Group;
  onStarting: () => void;
  onDone: () => void;
  onBrowse: () => void;
  onCreate: () => void;
}

export function Onboarding({ group, onStarting, onDone, onBrowse, onCreate }: Props) {
  const [draft, setDraft] = useState(readFirstTask);
  const [step, setStep] = useState<"task" | "provider" | "start">(draft.agentId ? "start" : "task");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const sending = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const briefHint = useId();
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
  }, [step]);

  const keep = (next: FirstTask) => {
    saveFirstTask(next);
    setDraft(next);
  };

  const open = async (agentId: string, sent = false) => {
    await refreshAgents();
    await select(agentId);
    if (sent) saveFirstTask(null);
    onDone();
  };

  const start = async () => {
    if (sending.current || !draft.brief.trim()) return;
    sending.current = true;
    setBusy(true);
    setError("");
    onStarting();
    let next = draft;
    try {
      if (next.agentId && !hired)
        throw new Error(
          "This agent is no longer available. Choose another task below; your brief is kept here.",
        );
      if (!next.agentId) {
        const created = await api.hireAgents(group.id, [toDraft(preset)]);
        if (!created[0]) throw new Error("The host did not return an agent. Try again.");
        next = { ...next, agentId: created[0].id };
        keep(next);
        useStore.setState((state) => ({
          agents: [...state.agents.filter((agent) => agent.id !== created[0]!.id), created[0]!],
        }));
      }
      if (!next.sent) {
        await api.sendMessage(next.agentId!, next.brief.trim());
        next = { ...next, sent: true };
        keep(next);
      }
      await open(next.agentId!, true);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };

  return (
    <section className="onboarding" aria-label="Get started with Guaca" aria-busy={busy}>
      <div className="onboarding-content">
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
              <p>Start with one task and one agent. You can build a crew as you go.</p>
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
                disabled={!draft.brief.trim()}
                onClick={() => setStep("provider")}
              >
                Continue
              </button>
              <span>
                {settings?.provider === "compatible" && !settings.apiKeySet
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
                disabled={busy || !!draft.agentId}
                onChange={(event) => keep({ ...draft, brief: event.target.value })}
              />
            </label>
            <p className="onboarding-notice">
              {draft.sent
                ? "Your task was sent. Open the conversation to see how it is going."
                : draft.agentId
                  ? "Your agent is already created. If a connection failed, check the conversation before sending again."
                  : "Start task creates this agent and sends your brief using the provider you just connected. Other services can be connected later, when needed."}
            </p>
            {error && (
              <div className="banner banner--error" role="alert">
                {error}
              </div>
            )}
            <div className="onboarding-actions">
              {!draft.agentId && (
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
                    keep({ ...draft, agentId: null, sent: false });
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
