import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { relativeTime } from "../lib/time";
import {
  errorMessage,
  type Skill,
  type SkillDraft,
  type SkillScope,
  skillScopeKey,
} from "../lib/types";
import { SkillDirectory } from "./SkillDirectory";

interface Props {
  scope: SkillScope;
}

const BLANK: SkillDraft = { name: "", description: "", body: "" };

/**
 * One scope's skills, and the editor for them.
 *
 * The operator's pane also lists Guaca's own, read-only, because the question
 * an operator arrives with is often "what does an agent know about this app",
 * and the answer is a document they should be able to read.
 */
export function SkillList({ scope }: Props) {
  const key = skillScopeKey(scope);
  const version = useStore((state) => state.skillsVersion[key] ?? 0);
  const [skills, setSkills] = useState<Skill[] | null>(null);
  const [bundled, setBundled] = useState<Skill[]>([]);
  const [draft, setDraft] = useState<SkillDraft | null>(null);
  const [reading, setReading] = useState<Skill | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [browsing, setBrowsing] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const crew = scope.kind === "crew";

  // A scope is a new object on every render and the same scope by its key, so
  // the list is read through a ref and reloaded when the key or its counter moves.
  const scopeRef = useRef(scope);
  scopeRef.current = scope;
  const load = useCallback(async () => {
    const at = scopeRef.current;
    setSkills(await api.listSkills(at));
    if (at.kind !== "crew") setBundled(await api.listSkills({ kind: "bundled" }));
  }, []);

  // Again whenever this scope changes, which an agent can do mid-turn.
  useEffect(() => {
    void load().catch((caught) => setError(errorMessage(caught)));
  }, [load, key, version]);

  useEffect(() => {
    if (draft && !draft.previous) nameRef.current?.focus();
  }, [draft]);

  const run = async (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await action();
      setDraft(null);
      await load();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  const open = async (skill: Skill, edit: boolean) => {
    setError(null);
    try {
      const full = await api.readSkill(skill.scope, skill.name);
      if (edit) {
        setReading(null);
        setDraft({
          name: full.name,
          description: full.description,
          body: full.body,
          previous: full.name,
        });
      } else {
        setReading(reading?.name === full.name ? null : full);
      }
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };

  const editor = draft && (
    <div className="access__item">
      <label className="field">
        <span className="field__label">Name</span>
        <input
          className="input input--mono"
          placeholder="deploy-site"
          ref={nameRef}
          value={draft.name}
          onChange={(event) => setDraft({ ...draft, name: event.target.value })}
        />
        <span className="field__hint">Lowercase words joined by dashes.</span>
      </label>
      <label className="field">
        <span className="field__label">When to use it</span>
        <input
          className="input"
          placeholder="When deploying the marketing site"
          value={draft.description}
          onChange={(event) => setDraft({ ...draft, description: event.target.value })}
        />
        <span className="field__hint">
          One line. It is all an agent sees until it decides to read the rest.
        </span>
      </label>
      <label className="field">
        <span className="field__label">Instructions</span>
        <textarea
          className="textarea textarea--code"
          rows={12}
          value={draft.body}
          onChange={(event) => setDraft({ ...draft, body: event.target.value })}
        />
      </label>
      <div className="access__row">
        <button
          type="button"
          className="btn btn--small btn--primary"
          disabled={busy || !draft.name.trim() || !draft.description.trim() || !draft.body.trim()}
          onClick={() => void run(() => api.saveSkill(scope, draft))}
        >
          Save skill
        </button>
        <button
          type="button"
          className="btn btn--small btn--ghost"
          disabled={busy}
          onClick={() => setDraft(null)}
        >
          Cancel
        </button>
      </div>
    </div>
  );

  return (
    <div className="access">
      <p className="field__hint">
        {crew
          ? "Instructions this crew's agents load when a task fits. Its agents can write these too. They also read your workspace skills and Guaca's own."
          : "Instructions every crew's agents load when a task fits. An agent sees each skill's name and when to use it, and reads the rest when it needs to."}
      </p>
      {skills === null && !error && <p className="field__hint">Loading skills…</p>}
      {skills?.length === 0 && !draft && (
        <p className="field__hint">{crew ? "This crew has no skills yet." : "No skills yet."}</p>
      )}
      {[...(skills ?? []), ...bundled].map((skill) => {
        const readOnly = skill.scope.kind === "bundled";
        const showing = reading?.name === skill.name && reading.scope.kind === skill.scope.kind;
        return (
          <div className="access__item" key={`${skill.scope.kind}:${skill.name}`}>
            <div className="access__row">
              <strong className="access__name">{skill.name}</strong>
              <span className="access__where">
                {readOnly
                  ? "Guaca's own"
                  : `${skill.origin ? "from skills.sh, " : ""}edited ${relativeTime(skill.updatedAt, Date.now())}`}
              </span>
              <button
                type="button"
                className="btn btn--small"
                disabled={busy || draft !== null}
                onClick={() => void open(skill, !readOnly)}
              >
                {readOnly ? (showing ? "Close" : "Read") : "Edit"}
              </button>
              {!readOnly && (
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy || draft !== null}
                  onClick={() => void run(() => api.deleteSkill(scope, skill.name))}
                >
                  Delete
                </button>
              )}
            </div>
            <p className="field__hint">{skill.description}</p>
            {showing && <pre className="skill__body">{reading.body}</pre>}
            {draft?.previous === skill.name && skill.scope.kind !== "bundled" && editor}
            {draft?.previous === skill.name && skill.files.length > 0 && (
              <p className="field__hint">
                Saving changes the instructions only. It also carries {skill.files.join(", ")},
                which stay as they are.
              </p>
            )}
          </div>
        );
      })}

      {draft && !draft.previous && editor}
      {browsing && (
        <SkillDirectory
          scope={scope}
          taken={new Set((skills ?? []).map((skill) => skill.name))}
          onAdded={() => void load().catch((caught) => setError(errorMessage(caught)))}
        />
      )}
      {!draft && (
        <div className="access__row">
          <button
            type="button"
            className="btn btn--small"
            disabled={busy || skills === null}
            onClick={() => setDraft({ ...BLANK })}
          >
            Write a skill
          </button>
          <button
            type="button"
            className="btn btn--small btn--ghost"
            aria-expanded={browsing}
            onClick={() => setBrowsing(!browsing)}
          >
            {browsing ? "Close skills.sh" : "Browse skills.sh"}
          </button>
        </div>
      )}

      {error && (
        <div className="banner banner--error" role="alert">
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
