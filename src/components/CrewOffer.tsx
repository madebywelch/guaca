import { useId, useState } from "react";
import { HIREABLE, toDraft } from "../lib/cafeteria";
import { api } from "../lib/ipc";
import { crewFor, FIRST_TASKS, named, readCrewOffer, saveCrewOffer } from "../lib/onboarding";
import { useStore } from "../lib/store";
import { isUnreachable } from "../lib/transport";
import { type AgentCard, asSentence, errorMessage } from "../lib/types";

export function CrewOffer({ agent }: { agent: AgentCard }) {
  const [offer, setOffer] = useState(readCrewOffer);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const said = useId();
  const refreshAgents = useStore((state) => state.refreshAgents);
  const select = useStore((state) => state.select);
  const crew = offer?.agentId === agent.id ? crewFor(offer.task) : null;
  if (!offer || !crew || !agent.groupId || agent.lifecycle === "terminated") return null;
  const groupId = agent.groupId;
  const solo = HIREABLE.find(
    (preset) => preset.id === FIRST_TASKS.find((task) => task.id === offer.task)!.group[0].agent,
  )!;
  const presets = crew.group.map(
    (member) => HIREABLE.find((preset) => preset.id === member.agent)!,
  );
  const text = named(crew.offer, new Map([[solo.name, agent.name]]));

  const dismiss = () => {
    setOffer(null);
    try {
      saveCrewOffer(null);
    } catch {}
  };

  const bring = async () => {
    if (busy) return;
    const next = { ...offer, key: offer.key ?? crypto.randomUUID() };
    setError("");
    try {
      saveCrewOffer(next);
    } catch {
      return setError(
        "This device could not save your progress, so nobody was hired. Allow storage for this app, then try again.",
      );
    }
    setOffer(next);
    setBusy(true);
    try {
      const created = await api.hireAgents(groupId, presets.map(toDraft), next.key);
      if (created.length !== presets.length)
        throw new Error("The host did not return both colleagues. Try again.");
      const names = new Map([
        [solo.name, agent.name],
        ...presets.map((preset, index): [string, string] => [preset.name, created[index]!.name]),
      ]);
      const brief = named(crew.brief, names).replace("{brief}", () => next.brief);
      await api.sendMessage(created[0]!.id, brief, [], next.key);
      const ids = new Set(created.map((card) => card.id));
      useStore.setState((state) => ({
        agents: [...state.agents.filter((card) => !ids.has(card.id)), ...created],
      }));
      dismiss();
      await refreshAgents();
      await select(created[0]!.id);
    } catch (cause) {
      setError(
        isUnreachable(cause)
          ? "Guaca could not reach its host. Check that the host is running, then try again."
          : asSentence(errorMessage(cause)),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="crew-offer" aria-label="A deeper pass" aria-busy={busy}>
      <p className="crew-offer__text" id={said}>
        {text}
      </p>
      <div className="crew-offer__actions">
        <button
          type="button"
          className="btn btn--compact"
          aria-describedby={said}
          disabled={busy}
          onClick={() => void bring()}
        >
          {busy ? "Bringing them in…" : "Bring them in"}
        </button>
        <button
          type="button"
          className="btn btn--quiet btn--compact"
          disabled={busy}
          onClick={dismiss}
        >
          Not now
        </button>
      </div>
      {error && (
        <p className="crew-offer__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
