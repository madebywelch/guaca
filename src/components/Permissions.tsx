import { useEffect, useRef, useState } from "react";
import { api } from "../lib/ipc";
import { nameFor } from "../lib/plugins";
import { useStore } from "../lib/store";
import {
  errorMessage,
  type PermissionClass,
  type PermissionCondition,
  type PermissionField,
  type PermissionPredicate,
  type PermissionRule,
  type PermissionRuleDraft,
} from "../lib/types";

const CLASSES: Record<PermissionClass, string> = {
  actOnBehalf: "Acting in your name",
  createAgent: "Adding an agent",
  changeSettings: "Changing settings",
  actOnSite: "Pressing or typing on a site",
};
const FIELDS: Record<PermissionClass, PermissionField[]> = {
  actOnBehalf: ["recipients", "domain", "amount"],
  createAgent: ["agentName"],
  changeSettings: ["settingKeys"],
  actOnSite: ["domain"],
};
const FIELD_LABELS: Record<PermissionField, string> = {
  recipients: "Recipient",
  domain: "Domain",
  amount: "Amount",
  agentName: "Agent name",
  settingKeys: "Setting key",
};
const OPS: Record<PermissionField, PermissionPredicate["op"][]> = {
  recipients: ["is", "oneOf", "under", "outsideDomain"],
  domain: ["is", "oneOf", "under", "outsideDomain"],
  amount: ["atMost", "greaterThan"],
  agentName: ["is", "oneOf"],
  settingKeys: ["is", "oneOf"],
};
const OP_LABELS: Record<PermissionPredicate["op"], string> = {
  is: "is",
  oneOf: "is one of",
  under: "is at or under domain",
  outsideDomain: "is outside domain",
  atMost: "is at most",
  greaterThan: "is greater than",
};
const predicate = (op: PermissionPredicate["op"]): PermissionPredicate => {
  if (op === "oneOf") return { op, values: [] };
  if (op === "atMost" || op === "greaterThan") return { op, value: "", currency: "" };
  return { op, value: "" };
};
const toolLabel = (value: string, connector?: string) => {
  const [kind, ...parts] = value.split("__");
  const raw = parts.join("__");
  if (!kind || !raw) return value;
  const name = raw.startsWith(`${kind}_`) ? raw.slice(kind.length + 1) : raw;
  return `${connector ?? nameFor(kind)} › ${name.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase())}`;
};
const empty = (groupId: string | null, effect: "allow" | "block"): PermissionRuleDraft => ({
  groupId,
  effect,
  class: "actOnBehalf",
  tool: null,
  agents: [],
  conditions: [],
});
const describeCondition = (
  { field, predicate: p }: PermissionCondition,
  effect: "allow" | "block",
) => {
  const subject =
    field === "recipients"
      ? `${effect === "allow" ? "every" : "any"} recipient`
      : field === "settingKeys"
        ? "a setting key"
        : `the ${FIELD_LABELS[field].toLowerCase()}`;
  const value = (p.op === "oneOf" ? p.values.join(", ") : p.value) || "…";
  const comparison = {
    is: "is",
    oneOf: "is one of",
    under: "is at",
    outsideDomain: "is outside",
    atMost: "is at most",
    greaterThan: "is over",
  }[p.op];
  return `${subject} ${comparison} ${value}${p.op === "atMost" || p.op === "greaterThan" ? ` ${p.currency || "…"}` : ""}`;
};

export function Permissions({ groupId }: { groupId: string | null }) {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const availableAgents = agents.filter((agent) => groupId === null || agent.groupId === groupId);
  const groupIds = groupId === null ? groups.map((group) => group.id).join(",") : groupId;
  const [tools, setTools] = useState<{ value: string; label: string }[]>([]);
  const [toolError, setToolError] = useState<string | null>(null);
  const [rules, setRules] = useState<PermissionRule[] | null>(null);
  const [draft, setDraft] = useState<PermissionRuleDraft | null>(null);
  const [conditionIds, setConditionIds] = useState<string[]>([]);
  const [editing, setEditing] = useState<PermissionRule | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const editorRef = useRef<HTMLDivElement>(null);
  const editingRule = draft !== null;
  useEffect(() => {
    if (editingRule) editorRef.current?.scrollIntoView?.({ block: "start" });
  }, [editingRule]);
  const refresh = async () => setRules(await api.listPermissionRules(groupId));
  useEffect(() => {
    let active = true;
    void api
      .listPermissionRules(groupId)
      .then((found) => {
        if (active) setRules(found);
      })
      .catch((caught) => {
        if (active) setError(errorMessage(caught));
      });
    return () => {
      active = false;
    };
  }, [groupId]);
  useEffect(() => {
    let active = true;
    const ids = groupIds ? groupIds.split(",") : [];
    void Promise.all(ids.map((id) => api.groupPlugins(id)))
      .then((sets) => {
        if (!active) return;
        const found = new Map<string, string>();
        for (const plugin of sets.flat())
          for (const tool of plugin.tools) {
            const value = `${plugin.kind}__${tool.name}`;
            found.set(value, toolLabel(value, plugin.name));
          }
        setTools(
          [...found]
            .map(([value, label]) => ({ value, label }))
            .sort((a, b) => a.label.localeCompare(b.label)),
        );
        setToolError(null);
      })
      .catch((caught) => {
        if (active) setToolError(errorMessage(caught));
      });
    return () => {
      active = false;
    };
  }, [groupIds]);
  const save = async () => {
    if (!draft) return;
    for (const { predicate: p } of draft.conditions) {
      if (
        p.op === "oneOf"
          ? p.values.length === 0 || p.values.length > 20 || p.values.some((value) => !value.trim())
          : !p.value.trim()
      ) {
        setError("Enter a value for every condition, with at most 20 values in a list.");
        return;
      }
      if (
        (p.op === "atMost" || p.op === "greaterThan") &&
        (!/^\d+(\.\d+)?$/.test(p.value) || !/^[a-zA-Z]{3}$/.test(p.currency))
      ) {
        setError("Enter a nonnegative amount and a three-letter currency.");
        return;
      }
    }
    if (draft.agents.length > 100) {
      setError("Choose at most 100 agents.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.savePermissionRule(
        {
          ...draft,
          conditions: draft.conditions.map((c) =>
            c.predicate.op === "oneOf"
              ? {
                  ...c,
                  predicate: {
                    ...c.predicate,
                    values: c.predicate.values.map((value) => value.trim()),
                  },
                }
              : c,
          ),
        },
        editing?.id ?? null,
        editing?.version ?? null,
      );
      await refresh();
      setDraft(null);
      setEditing(null);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };
  const remove = async (rule: PermissionRule) => {
    if (!window.confirm(`Remove this rule?\n\n${rule.wrote}`)) return;
    setBusy(true);
    setError(null);
    try {
      await api.deletePermissionRule(rule.id, rule.groupId, rule.version);
      await refresh();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };
  const updateCondition = (index: number, condition: PermissionCondition) => {
    if (draft)
      setDraft({
        ...draft,
        conditions: draft.conditions.map((item, i) => (i === index ? condition : item)),
      });
  };
  return (
    <>
      <h3 className="settings__title">Permissions</h3>
      <p className="settings__lede">Rules answer some requests for you, and a block always wins.</p>
      <details>
        <summary>How rules match</summary>
        <p className="field__hint">
          Rules match facts the agent declares in its request. At a domain includes its subdomains.
          When a block needs a fact that is missing, you are asked. Connector calls do not
          automatically ask for permission, and these rules do not gate them. Rules do not grant
          access to a connector or a browser. Actions after reading a page or screenshot cannot be
          allowed by a rule.
        </p>
      </details>
      {error && (
        <div className="banner banner--error" role="alert">
          {error}
        </div>
      )}
      {rules === null && !error && <p>Loading permissions…</p>}
      {rules && rules.length === 0 && (
        <p className="field__hint">
          No rules yet. Requests follow their usual permission settings.
        </p>
      )}
      {rules &&
        !draft &&
        (["allow", "block"] as const).map((effect) => (
          <section className="permissions__list" key={effect}>
            <div className="permissions__head">
              <h4>{effect === "allow" ? "Acts without asking" : "Never"}</h4>
              <button
                type="button"
                className="btn btn--ghost btn--small"
                disabled={busy || draft !== null}
                onClick={() => {
                  setEditing(null);
                  setDraft(empty(groupId, effect));
                }}
              >
                Add rule
              </button>
            </div>
            {rules
              .filter((rule) => rule.effect === effect)
              .sort((a, b) => Number(a.groupId !== null) - Number(b.groupId !== null))
              .map((rule) => (
                <div className="permissions__row" key={rule.id}>
                  <div>
                    <strong>{rule.wrote}</strong>
                    <div className="field__hint">
                      {rule.groupId === null ? "Every group" : "This group"} ·{" "}
                      {rule.agents.length
                        ? rule.agents
                            .map(
                              (id) =>
                                agents.find((agent) => agent.id === id)?.name ??
                                "Unavailable agent",
                            )
                            .join(", ")
                        : "Everyone"}{" "}
                      · {new Date(rule.createdAt).toLocaleDateString()}
                    </div>
                  </div>
                  {rule.groupId === groupId && (
                    <div className="permissions__actions">
                      <button
                        type="button"
                        className="btn btn--ghost btn--small"
                        disabled={busy || draft !== null}
                        onClick={() => {
                          const {
                            id: _id,
                            version: _version,
                            wrote: _wrote,
                            createdAt: _createdAt,
                            ...value
                          } = rule;
                          setEditing(rule);
                          setConditionIds(value.conditions.map(() => crypto.randomUUID()));
                          setDraft(value);
                        }}
                      >
                        Edit
                      </button>
                      <button
                        type="button"
                        className="btn btn--ghost btn--small"
                        disabled={busy || draft !== null}
                        onClick={() => void remove(rule)}
                      >
                        Remove
                      </button>
                    </div>
                  )}
                </div>
              ))}
          </section>
        ))}
      {draft && (
        <div className="permissions__editor" ref={editorRef}>
          <h4>{editing ? "Edit rule" : "New rule"}</h4>
          <label className="field">
            <span className="field__label">Verdict</span>
            <select
              className="input"
              value={draft.effect}
              onChange={(e) => {
                const effect = e.target.value as "allow" | "block";
                const reset =
                  effect === "allow" &&
                  (draft.class === "changeSettings" || draft.class === "actOnSite");
                setDraft({
                  ...draft,
                  effect,
                  ...(reset ? { class: "actOnBehalf", conditions: [], tool: null } : {}),
                });
              }}
            >
              <option value="allow">Allow</option>
              <option value="block">Never</option>
            </select>
          </label>
          <label className="field">
            <span className="field__label">What</span>
            <select
              className="input"
              value={draft.class}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  class: e.target.value as PermissionClass,
                  conditions: [],
                  tool: null,
                })
              }
            >
              {Object.entries(CLASSES)
                .filter(
                  ([key]) =>
                    draft.effect === "block" || (key !== "changeSettings" && key !== "actOnSite"),
                )
                .map(([key, label]) => (
                  <option value={key} key={key}>
                    {label}
                  </option>
                ))}
            </select>
          </label>
          {draft.class === "actOnBehalf" && (
            <label className="field">
              <span className="field__label">Connector tool (optional)</span>
              <select
                className="input"
                value={draft.tool ?? ""}
                onChange={(e) => setDraft({ ...draft, tool: e.target.value || null })}
              >
                <option value="">Any tool</option>
                {draft.tool && !tools.some((tool) => tool.value === draft.tool) && (
                  <option value={draft.tool}>
                    {toolLabel(draft.tool)} (not currently connected)
                  </option>
                )}
                {tools.map((tool) => (
                  <option key={tool.value} value={tool.value}>
                    {tool.label}
                  </option>
                ))}
              </select>
              {toolError && (
                <span className="field__error">Could not load connector tools: {toolError}</span>
              )}
            </label>
          )}
          {availableAgents.length > 0 && (
            <fieldset className="permissions__agents">
              <legend className="field__label">Who</legend>
              <span className="field__hint">
                No selection means everyone {groupId ? "in this group" : "in every group"}.
              </span>
              {availableAgents.map((agent) => (
                <label key={agent.id}>
                  <input
                    type="checkbox"
                    checked={draft.agents.includes(agent.id)}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        agents: e.target.checked
                          ? [...draft.agents, agent.id]
                          : draft.agents.filter((id) => id !== agent.id),
                      })
                    }
                  />{" "}
                  {agent.name}
                  {groupId === null
                    ? ` · ${groups.find((group) => group.id === agent.groupId)?.name ?? "Unavailable group"}`
                    : ""}
                </label>
              ))}
            </fieldset>
          )}
          <div className="permissions__head">
            <span className="field__label">When</span>
            <button
              type="button"
              className="btn btn--ghost btn--small"
              disabled={draft.conditions.length >= 20}
              onClick={() => {
                const field = FIELDS[draft.class][0]!;
                setConditionIds([...conditionIds, crypto.randomUUID()]);
                setDraft({
                  ...draft,
                  conditions: [
                    ...draft.conditions,
                    { field, predicate: predicate(OPS[field][0]!) },
                  ],
                });
              }}
            >
              Add condition
            </button>
          </div>
          {!draft.conditions.length && (
            <p className="field__hint">Any time this action is requested.</p>
          )}
          {draft.conditions.map((condition, index) => (
            <div className="permissions__condition" key={conditionIds[index]}>
              <label className="field">
                <span className="field__label">Fact</span>
                <select
                  className="input"
                  value={condition.field}
                  onChange={(e) => {
                    const field = e.target.value as PermissionField;
                    updateCondition(index, { field, predicate: predicate(OPS[field][0]!) });
                  }}
                >
                  {FIELDS[draft.class].map((field) => (
                    <option key={field} value={field}>
                      {FIELD_LABELS[field]}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span className="field__label">Comparison</span>
                <select
                  className="input"
                  value={condition.predicate.op}
                  onChange={(e) =>
                    updateCondition(index, {
                      ...condition,
                      predicate: predicate(e.target.value as PermissionPredicate["op"]),
                    })
                  }
                >
                  {OPS[condition.field].map((op) => (
                    <option key={op} value={op}>
                      {OP_LABELS[op]}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span className="field__label">
                  {condition.predicate.op === "oneOf" ? "Values, separated by commas" : "Value"}
                </span>
                <input
                  className="input"
                  value={
                    condition.predicate.op === "oneOf"
                      ? condition.predicate.values.join(",")
                      : condition.predicate.value
                  }
                  onChange={(e) => {
                    const p = condition.predicate;
                    updateCondition(index, {
                      ...condition,
                      predicate:
                        p.op === "oneOf"
                          ? { ...p, values: e.target.value.split(",") }
                          : { ...p, value: e.target.value },
                    });
                  }}
                />
              </label>
              {(condition.predicate.op === "atMost" ||
                condition.predicate.op === "greaterThan") && (
                <label className="field">
                  <span className="field__label">Currency</span>
                  <input
                    className="input"
                    value={condition.predicate.currency}
                    onChange={(e) =>
                      updateCondition(index, {
                        ...condition,
                        predicate: {
                          ...condition.predicate,
                          currency: e.target.value,
                        } as PermissionPredicate,
                      })
                    }
                    placeholder="USD"
                  />
                </label>
              )}
              <button
                type="button"
                className="btn btn--ghost btn--small"
                onClick={() => {
                  setConditionIds(conditionIds.filter((_, i) => i !== index));
                  setDraft({
                    ...draft,
                    conditions: draft.conditions.filter((_, i) => i !== index),
                  });
                }}
              >
                Remove condition
              </button>
            </div>
          ))}
          <p className="field__hint">
            Preview: {draft.effect === "allow" ? "Allow" : "Never"}{" "}
            {draft.class === "actOnBehalf"
              ? draft.effect === "allow"
                ? "acting in your name"
                : "act in your name"
              : draft.class === "createAgent"
                ? draft.effect === "allow"
                  ? "adding an agent"
                  : "add an agent"
                : draft.class === "changeSettings"
                  ? "change settings"
                  : "press or type on a site"}
            {draft.tool
              ? ` with ${tools.find((tool) => tool.value === draft.tool)?.label ?? toolLabel(draft.tool)}`
              : ""}
            {draft.agents.length ? " for selected agents" : ""}
            {draft.conditions.length
              ? ` when ${draft.conditions.map((condition) => describeCondition(condition, draft.effect)).join(" and ")}`
              : ""}
            . Applies to{" "}
            {draft.agents.length
              ? draft.agents
                  .map((id) => agents.find((agent) => agent.id === id)?.name ?? "Unavailable agent")
                  .join(", ")
              : `everyone ${groupId ? "in this group" : "in every group"}`}
            .
          </p>
          <div className="permissions__actions">
            <button
              type="button"
              className="btn btn--primary"
              disabled={busy}
              onClick={() => void save()}
            >
              {busy ? "Saving…" : "Save rule"}
            </button>
            <button
              type="button"
              className="btn btn--ghost"
              disabled={busy}
              onClick={() => {
                setDraft(null);
                setEditing(null);
              }}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </>
  );
}
