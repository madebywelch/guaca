import { useEffect, useId, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { HIREABLE, toDraft } from "../lib/cafeteria";
import { api } from "../lib/ipc";
import { PLACE_LABELS } from "../lib/menu";
import {
  crewFor,
  FIRST_TASKS,
  type FirstTask,
  readFirstTask,
  saveCrewOffer,
  saveFirstTask,
} from "../lib/onboarding";
import { useStore } from "../lib/store";
import { BEFORE_RELOAD, desktop, isUnreachable } from "../lib/transport";
import { asSentence, errorMessage, type Group } from "../lib/types";
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
  const [unsaved, setUnsaved] = useState(false);
  const [checking, setChecking] = useState(false);
  const sending = useRef(false);
  const opening = useRef<HTMLButtonElement>(null);
  const pane = useRef<HTMLElement>(null);
  const briefHint = useId();
  const continueHint = useId();
  const groupHeading = useId();
  const settings = useStore((state) => state.settings);
  const agents = useStore((state) => state.agents);
  const refreshAgents = useStore((state) => state.refreshAgents);
  const select = useStore((state) => state.select);
  const task = FIRST_TASKS.find((entry) => entry.id === draft.task)!;
  const members = task.group.map((member) => ({
    ...member,
    preset: HIREABLE.find((entry) => entry.id === member.agent)!,
  }));
  const [lead, first, second] = members.map((member) => member.preset.name);
  const crew = crewFor(task.id);
  const later = crew?.group.map((member) => HIREABLE.find((entry) => entry.id === member.agent)!);
  const hired = agents.find(
    (agent) => agent.id === draft.agentId && agent.lifecycle !== "terminated",
  );
  const name = hired?.name ?? lead;

  useEffect(() => {
    if (pane.current) pane.current.scrollTop = 0;
    opening.current?.focus({ preventScroll: true });
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

  const fail = (cause: unknown) =>
    setError(
      isUnreachable(cause)
        ? "Guaca could not reach its host. Your brief is kept here. Check that the host is running, then try again."
        : asSentence(errorMessage(cause)),
    );

  useEffect(() => {
    if (error && !busy) opening.current?.focus({ preventScroll: true });
  }, [error, busy]);

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

  const finish = (current: FirstTask) => {
    setDraft({ ...current, sent: true });
    try {
      if (crewFor(current.task) && current.agentId)
        saveCrewOffer({ task: current.task, agentId: current.agentId, brief: current.brief });
      saveFirstTask(null);
      setStorageError(false);
      onDone();
    } catch {
      setStorageError(true);
    }
  };

  const open = async (agentId: string, sent?: FirstTask) => {
    await refreshAgents();
    await select(agentId);
    if (sent) finish(sent);
    else {
      setChecking(true);
      onInspect(true);
    }
  };

  const start = async () => {
    if (sending.current || !draft.brief.trim()) return;
    let next = { ...draft, key: draft.key ?? crypto.randomUUID() };
    setError("");
    setUnsaved(false);
    if (!keep(next)) return setUnsaved(true);
    sending.current = true;
    setBusy(true);
    onStarting();
    try {
      if (next.agentId && !hired)
        throw new Error(
          `${name} is no longer available. Choose another task below; your brief is kept here.`,
        );
      if (!next.agentId) {
        if (group.agentCount === 0 && group.name !== task.groupName)
          await api.updateGroup(group.id, { name: task.groupName });
        const created = await api.hireAgents(
          group.id,
          members.map((member) => toDraft(member.preset)),
          next.key,
        );
        if (created.length !== members.length)
          throw new Error("The host did not return the whole group. Try again.");
        const brief = created.reduce(
          (text, card, index) => text.replaceAll(members[index]!.preset.name, card.name),
          next.brief,
        );
        next = { ...next, agentId: created[0]!.id, brief };
        const saved = keep(next);
        const ids = new Set(created.map((agent) => agent.id));
        useStore.setState((state) => ({
          agents: [...state.agents.filter((agent) => !ids.has(agent.id)), ...created],
        }));
        if (!saved) return setUnsaved(true);
      }
      if (!next.sent) {
        await api.sendMessage(next.agentId!, next.brief.trim(), [], next.key);
        next = { ...next, sent: true };
        if (!keep(next)) return setUnsaved(true);
      }
      await open(next.agentId!, next);
    } catch (cause) {
      fail(cause);
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
            <button type="button" className="btn" onClick={() => finish(draft)}>
              Task was delivered
            </button>
          </div>
        </div>
      </>
    );

  return (
    <section ref={pane} className="onboarding" aria-label="Get started with Guaca" aria-busy={busy}>
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
              <h1>What would you like help with?</h1>
              <p>
                Pick a task and Guaca hires the agent suited to it, who comes back with a result.
                Once it has, you can bring in two colleagues for a deeper pass.
              </p>
            </div>
            <fieldset className="onboarding-options" aria-label="First task">
              {FIRST_TASKS.map((entry, index) => (
                <button
                  type="button"
                  className="onboarding-choice"
                  key={entry.id}
                  ref={index === 0 ? opening : undefined}
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
              <button type="button" className="btn btn--quiet" onClick={onBrowse}>
                {PLACE_LABELS.cafeteria}
              </button>
              <button type="button" className="btn btn--quiet" onClick={onCreate}>
                Create one agent
              </button>
            </div>
          </>
        ) : step === "provider" ? (
          <>
            <div className="onboarding-intro">
              <h1>Connect your AI</h1>
              <p>Choose what powers your agents. Your first brief is waiting for you.</p>
            </div>
            <OnboardingProvider
              onConnected={() => setStep("start")}
              onBack={() => setStep("task")}
            />
          </>
        ) : (
          <>
            <div className="onboarding-intro">
              <p className="onboarding-eyebrow">
                {later ? "One task. One agent first." : "One task. A group of three."}
              </p>
              <h1>Ready when you are.</h1>
              <p>
                Review your brief. {name} takes it,{" "}
                {later ? "works on it alone," : `decides what to hand ${first} and ${second},`} and
                comes back with a result, or with one question if it cannot go on without you.
              </p>
            </div>
            <section className="onboarding-group" aria-labelledby={groupHeading}>
              <h2 className="onboarding-eyebrow" id={groupHeading}>
                {later
                  ? draft.agentId
                    ? "Your agent"
                    : "Your agent for this task"
                  : draft.agentId
                    ? "Your group"
                    : "Your group for this task"}
              </h2>
              <ul>
                {members.map((member) => (
                  <li key={member.agent}>
                    <AgentAvatar
                      avatar={member.preset.avatar}
                      color={member.preset.color}
                      size="md"
                      seed={member.agent}
                    />
                    <strong>{member.preset.name}</strong>
                    <span>{member.role}</span>
                  </li>
                ))}
              </ul>
              {later && (
                <p className="onboarding-later">
                  Once its result is in, you can bring in {later[0]!.name} and {later[1]!.name} for
                  a deeper pass.
                </p>
              )}
            </section>
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
                  ? `${later ? `${name} is` : "Your group is"} already hired. If a connection failed, check the conversation before sending again.`
                  : draft.key
                    ? `Your previous start may have hired ${later ? name : "this group"}. Retry with the same brief to continue safely.`
                    : later
                      ? `Start task hires ${name} and sends your brief to it, using the provider you just connected. It answers alone, so your first result waits on nobody else.`
                      : `Start task hires these three and sends your brief to ${name}, using the provider you just connected. A group task uses more model calls than one agent would, at least five instead of one, and it stays inside your group's limits.`}
            </p>
            {desktop && (
              <p className="onboarding-notice">
                After you start, you can close this window: Guaca stays in the menu bar, and its
                icon opens a panel of what is waiting on you and who is working.
              </p>
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
                  onClick={() => void open(draft.agentId!).catch(fail)}
                >
                  Check conversation
                </button>
              )}
              <button
                ref={opening}
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
              {error ? (
                <p className="onboarding-notice" role="alert">
                  {error}
                </p>
              ) : (
                unsaved && (
                  <p className="onboarding-notice" role="status">
                    This device could not save your progress, so nothing more was started. The
                    message at the top of this page says how to fix it.
                  </p>
                )
              )}
            </div>
          </>
        )}
      </div>
    </section>
  );
}
