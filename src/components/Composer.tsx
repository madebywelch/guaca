import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import {
  type DraftAction,
  readDraftState,
  reduceDraft,
  saveDraft,
  saveDraftState,
} from "../lib/draft";
import { fileUrl, previewKind, readableSize } from "../lib/files";
import { focusWriting } from "../lib/focus";
import { api, onFileDrop } from "../lib/ipc";
import { FIND_KEY } from "../lib/keybinds";
import { applyMention, matchMentions, mentionAt, splitMentions } from "../lib/mentions";
import { useLiveAgents, useStore } from "../lib/store";
import { BEFORE_RELOAD, workspaceOrigin } from "../lib/transport";
import { type Attachment, errorMessage, type GroupId, type Staged } from "../lib/types";
import { Icon } from "./Icon";
import "./Composer.css";

const owners = new Map<string, ReturnType<typeof createOwner>>();

function createOwner(key: string, persistent: boolean) {
  let state = persistent
    ? readDraftState(key)
    : { draft: { text: "", files: [] }, delivery: { kind: "idle" as const }, staging: 0, held: [] };
  const listeners = new Set<() => void>();
  let saved = true;
  const owner = {
    get: () => state,
    preserve: () => saveDraftState(key, state),
    subscribe: (listener: () => void) => {
      owners.set(key, owner);
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
        if (!listeners.size && saved && state.delivery.kind !== "sending" && !state.staging)
          owners.delete(key);
      };
    },
    dispatch: (action: DraftAction, required = false) => {
      const next = reduceDraft(state, action);
      try {
        if (persistent) saveDraftState(key, next);
        saved = true;
      } catch (error) {
        saved = false;
        if (required) throw error;
      }
      state = next;
      listeners.forEach((listener) => {
        listener();
      });
      if (!listeners.size && saved && state.delivery.kind !== "sending" && !state.staging)
        owners.delete(key);
    },
  };
  return owner;
}

function ownerFor(key: string, persistent: boolean) {
  let owner = owners.get(key);
  if (!owner) {
    owner = createOwner(key, persistent);
    owners.set(key, owner);
  }
  return owner;
}

if (typeof window !== "undefined") {
  window.addEventListener(BEFORE_RELOAD, (event) => {
    try {
      for (const owner of owners.values()) {
        if (owner.get().delivery.kind === "sending" || owner.get().staging) {
          event.preventDefault();
          return;
        }
        owner.preserve();
      }
    } catch {
      event.preventDefault();
    }
  });
}

interface Props {
  placeholder: string;
  model?: string;
  draftKey?: string;
  recipient?: { id: string; name: string };
  onRecipientChange?: (id: string) => void;
  waiting?: { asker: string; answer: () => void };
  group: GroupId | null;
  disabled?: boolean;
  disabledReason?: string;
  onSend: (text: string, files: Attachment[], key: string) => Promise<unknown>;
}

export function Composer({
  placeholder,
  model,
  group,
  disabled,
  disabledReason,
  onSend,
  draftKey,
  recipient,
  onRecipientChange,
  waiting,
}: Props) {
  const storageKey = `guaca.draft:${workspaceOrigin()}:${draftKey ?? group ?? "composer"}`;
  const owner = useMemo(() => ownerFor(storageKey, !!draftKey), [storageKey, draftKey]);
  const state = useSyncExternalStore(owner.subscribe, owner.get);
  const { text, files } = state.draft;
  const sending = state.delivery.kind === "sending";
  const stagingCount = state.staging;
  const setText = (value: string) => owner.dispatch({ type: "text", text: value });
  const setFiles = (value: Attachment[] | ((held: Attachment[]) => Attachment[])) =>
    owner.dispatch({
      type: "files",
      files: typeof value === "function" ? value(owner.get().draft.files) : value,
    });
  const [caret, setCaret] = useState(0);
  const writingTo = recipient?.id ?? draftKey;
  useEffect(() => {
    if (writingTo !== undefined) focusWriting();
  }, [writingTo]);
  const [highlighted, setHighlighted] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [refused, setRefused] = useState<string[]>([]);
  const [dragging, setDragging] = useState(false);
  useEffect(() => {
    const preserve = (event: Event) => {
      try {
        if (
          [...owners.values()].some(
            (entry) => entry.get().delivery.kind === "sending" || entry.get().staging,
          )
        )
          throw new Error("A message is being sent. Wait for it to finish before reloading.");
        for (const entry of owners.values()) entry.preserve();
      } catch (error) {
        event.preventDefault();
        setRefused([errorMessage(error)]);
      }
    };
    window.addEventListener(BEFORE_RELOAD, preserve);
    return () => window.removeEventListener(BEFORE_RELOAD, preserve);
  }, [storageKey, owner, sending, stagingCount]);

  const ref = useRef<HTMLTextAreaElement>(null);
  const mirror = useRef<HTMLDivElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const localFiles = useStore((s) => s.capabilities.localFiles);

  const take = useCallback(
    (staging: Promise<Staged>) => {
      owner.dispatch({ type: "stage" });
      void staging
        .then((staged) => {
          owner.dispatch({ type: "staged", files: staged.attached });
          setRefused(staged.refused);
        })
        .catch((error) => {
          owner.dispatch({ type: "staged", files: [] });
          setRefused([errorMessage(error)]);
        });
    },
    [owner],
  );

  useEffect(() => {
    if (disabled) return;
    const stopping = onFileDrop({ over: setDragging, dropped: take });
    return () => {
      void stopping.then((stop) => stop());
    };
  }, [disabled, take]);

  const live = useLiveAgents();
  const groups = useStore((s) => s.groups);
  const crew = useMemo(
    () => (group ? live.filter((agent) => agent.groupId === group) : []),
    [live, group],
  );
  const names = useMemo(() => crew.map((a) => a.name), [crew]);
  const query = dismissed ? null : mentionAt(text, caret);
  const matches = query ? matchMentions(names, query.term) : [];
  const showing = query !== null && matches.length > 0;
  const selected = matches.length > 0 ? Math.min(highlighted, matches.length - 1) : 0;

  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${node.scrollHeight}px`;
    if (mirror.current) mirror.current.scrollTop = node.scrollTop;
  }, [text]);

  const choose = (name: string) => {
    if (!query) return;
    const next = applyMention(text, query, name);
    setText(next.text);
    setDismissed(false);
    requestAnimationFrame(() => {
      const node = ref.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(next.caret, next.caret);
      setCaret(next.caret);
    });
  };

  const supportKey = `guaca.keyed-send:${workspaceOrigin()}`;
  const supportsRetry = () => {
    try {
      return sessionStorage.getItem(supportKey) === "yes";
    } catch {
      return false;
    }
  };
  const send = async (payload: { text: string; files: Attachment[] }, key: string) => {
    setRefused([]);
    try {
      const result = await onSend(payload.text.trim(), payload.files, key);
      try {
        if (
          result &&
          typeof result === "object" &&
          "messageId" in result &&
          "runId" in result &&
          typeof result.messageId === "string" &&
          typeof result.runId === "string"
        )
          sessionStorage.setItem(supportKey, "yes");
        else sessionStorage.removeItem(supportKey);
      } catch {}
      owner.dispatch({ type: "success" });
    } catch {
      owner.dispatch({ type: "unknown" });
    } finally {
      ref.current?.focus();
    }
  };

  const submit = async () => {
    const current = owner.get();
    const { text, files } = current.draft;
    const body = text.trim();
    if (
      (!body && files.length === 0) ||
      current.delivery.kind !== "idle" ||
      current.staging ||
      disabled
    )
      return;
    try {
      const key = Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) =>
        byte.toString(16).padStart(2, "0"),
      ).join("");
      owner.dispatch({ type: "send", key }, true);
      void send({ text, files }, key);
    } catch (error) {
      setRefused([errorMessage(error)]);
      return;
    }
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (showing) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setHighlighted((current) => (current + 1) % matches.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setHighlighted((current) => (current - 1 + matches.length) % matches.length);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        choose(matches[selected]!);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setDismissed(true);
        return;
      }
    }

    if (event.key === "Tab" && !event.shiftKey && waiting && !text && files.length === 0) {
      event.preventDefault();
      waiting.answer();
      return;
    }

    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void submit();
    }
  };

  const track = (event: { currentTarget: HTMLTextAreaElement }) => {
    setCaret(event.currentTarget.selectionStart ?? 0);
  };

  const hint = showing ? "↑↓ to choose · Enter to insert" : dragging ? "Drop to attach" : null;

  return (
    <div className="composer" data-dragging={dragging || undefined}>
      {recipient && onRecipientChange && (
        <label className="composer__recipient">
          <span>To</span>
          <select
            aria-label="Recipient"
            value={recipient.id}
            disabled={state.delivery.kind !== "idle" || stagingCount > 0 || state.held.length > 0}
            onChange={(event) => {
              const id = event.target.value;
              if (id === recipient.id) return;
              const destination = `guaca.draft:${workspaceOrigin()}:${id}`;
              try {
                sessionStorage.getItem(destination);
                const held = owners.get(destination)?.get() ?? readDraftState(destination);
                if (held.staging) {
                  setRefused([
                    "That conversation has a file being attached. Wait for it to finish before moving this draft.",
                  ]);
                  return;
                }
                if (
                  held.draft.text ||
                  held.draft.files.length ||
                  held.delivery.kind !== "idle" ||
                  held.held.length
                ) {
                  setRefused([
                    "That conversation already has a saved draft. Open it before moving this draft.",
                  ]);
                  return;
                }
                saveDraft(destination, { text, files });
                sessionStorage.removeItem(storageKey);
                onRecipientChange(id);
              } catch (error) {
                setRefused([errorMessage(error)]);
              }
            }}
          >
            {!live.some((agent) => agent.id === recipient.id) && (
              <option value={recipient.id}>{recipient.name}</option>
            )}
            {[...new Set(live.map((agent) => agent.groupId))].map((groupId) => {
              const name = groups.find((entry) => entry.id === groupId)?.name ?? "Independent";
              return (
                <optgroup key={groupId ?? "independent"} label={name}>
                  {live
                    .filter((agent) => agent.groupId === groupId)
                    .map((agent) => (
                      <option key={agent.id} value={agent.id}>
                        {agent.name}
                      </option>
                    ))}
                </optgroup>
              );
            })}
          </select>
        </label>
      )}
      {files.length > 0 && (
        <ul className="composer__files" aria-label="Attached files">
          {files.map((file) => (
            <li key={file.digest} className="chip">
              {previewKind(file.mime) === "image" && (
                <img className="chip__thumb" src={fileUrl(file)} alt="" />
              )}
              <span className="chip__name">{file.name}</span>
              <span className="chip__size">{readableSize(file.bytes)}</span>
              <button
                type="button"
                className="chip__remove"
                aria-label={`Remove ${file.name}`}
                onClick={() =>
                  setFiles((held) => held.filter((have) => have.digest !== file.digest))
                }
              >
                <Icon name="close" size={16} />
              </button>
            </li>
          ))}
        </ul>
      )}
      {state.delivery.kind === "unknown" && (
        <div role="alert">
          <p>
            {state.delivery.key && supportsRetry()
              ? "Guaca could not confirm this was delivered. Sending again will not create a duplicate."
              : "Delivery could not be confirmed. Check the conversation before sending again."}
          </p>
          {(state.draft.text !== state.delivery.payload.text ||
            JSON.stringify(state.draft.files) !== JSON.stringify(state.delivery.payload.files)) && (
            <>
              <p>{state.delivery.payload.text}</p>
              {state.delivery.payload.files.map((file) => (
                <span key={file.digest}>{file.name}</span>
              ))}
            </>
          )}
          <button
            type="button"
            className="btn btn--compact"
            disabled={disabled || stagingCount > 0}
            onClick={() => {
              const delivery = owner.get().delivery;
              if (delivery.kind === "unknown" && delivery.key && supportsRetry()) {
                try {
                  owner.dispatch({ type: "retry" }, true);
                  void send(delivery.payload, delivery.key);
                } catch (error) {
                  setRefused([errorMessage(error)]);
                }
              } else owner.dispatch({ type: "dismiss" });
            }}
          >
            {state.delivery.key && supportsRetry() ? "Send again" : "Conversation checked"}
          </button>
        </div>
      )}
      {state.held.map((draft, index) => {
        const value = JSON.stringify(draft);
        const key = `${value}:${state.held.slice(0, index).filter((item) => JSON.stringify(item) === value).length}`;
        return (
          <div key={key}>
            <p>
              Saved draft:{" "}
              {draft.text ||
                `${draft.files.length} attached file${draft.files.length === 1 ? "" : "s"}`}
            </p>
            <button
              type="button"
              className="btn btn--compact"
              aria-label={`Switch to saved draft ${index + 1}`}
              disabled={state.delivery.kind !== "idle" || stagingCount > 0}
              onClick={() => owner.dispatch({ type: "switch", index })}
            >
              Switch drafts
            </button>
          </div>
        );
      })}
      {refused.length > 0 && (
        <ul className="composer__refused" aria-label="Files that could not be attached">
          {refused.map((why) => (
            <li key={why} className="chip chip--error">
              <span className="chip__text">{why}</span>
              <button
                type="button"
                className="chip__remove"
                aria-label={`Dismiss: ${why}`}
                onClick={() => setRefused((held) => held.filter((line) => line !== why))}
              >
                <Icon name="close" size={16} />
              </button>
            </li>
          ))}
        </ul>
      )}
      {showing && (
        <div className="mentions" role="listbox" id="mention-list" aria-label="Agents">
          {matches.map((name, index) => {
            const agent = crew.find((a) => a.name === name);
            const active = index === selected;
            return (
              <div
                key={name}
                id={`mention-${index}`}
                className="mentions__item"
                role="option"
                tabIndex={-1}
                aria-selected={active}
                data-active={active}
                onMouseEnter={() => setHighlighted(index)}
                onMouseDown={(event) => {
                  event.preventDefault();
                  choose(name);
                }}
              >
                <AgentAvatar
                  avatar={agent?.avatar ?? "plain"}
                  color={agent?.color ?? "#c7d96b"}
                  size="xs"
                  seed={agent?.id}
                />
                <span className="mentions__name">{name}</span>
                {agent?.skills.length ? (
                  <span className="mentions__skills">{agent.skills.join(", ")}</span>
                ) : null}
              </div>
            );
          })}
        </div>
      )}

      <div className="composer__row">
        <div className="composer__field">
          <div className="composer__mirror" aria-hidden="true" ref={mirror}>
            {splitMentions(text, names).map((run) =>
              run.kind === "mention" ? (
                <span key={run.at} className="mention" data-mention={run.name}>
                  {run.text}
                </span>
              ) : (
                <Fragment key={run.at}>{run.text}</Fragment>
              ),
            )}
          </div>
          <textarea
            ref={ref}
            className="composer__input"
            rows={1}
            value={text}
            placeholder={disabled ? (disabledReason ?? placeholder) : placeholder}
            disabled={disabled}
            data-field=""
            onChange={(event) => {
              setText(event.target.value);
              setCaret(event.target.selectionStart ?? 0);
              setDismissed(false);
              setHighlighted(0);
            }}
            onKeyUp={track}
            onClick={track}
            onKeyDown={onKeyDown}
            onScroll={(event) => {
              if (mirror.current) mirror.current.scrollTop = event.currentTarget.scrollTop;
            }}
            role="combobox"
            aria-label={recipient ? `Write to ${recipient.name}` : placeholder}
            aria-expanded={showing}
            aria-controls={showing ? "mention-list" : undefined}
            aria-activedescendant={showing ? `mention-${selected}` : undefined}
            aria-autocomplete="list"
          />
        </div>
        {!localFiles && (
          <>
            <input
              ref={picker}
              type="file"
              multiple
              hidden
              onChange={(event) => {
                const chosen = Array.from(event.target.files ?? []);
                event.target.value = "";
                if (chosen.length > 0) take(api.stageUploads(chosen));
              }}
            />
            <button
              type="button"
              className="composer__attach"
              aria-label="Attach a file"
              title="Attach a file"
              disabled={disabled || sending}
              onClick={() => picker.current?.click()}
            >
              <Icon name="attach" />
            </button>
          </>
        )}
        <button
          type="button"
          className="composer__send"
          aria-label="Send"
          aria-busy={sending}
          title="Send"
          onClick={() => void submit()}
          disabled={
            disabled ||
            state.delivery.kind !== "idle" ||
            stagingCount > 0 ||
            (text.trim().length === 0 && files.length === 0)
          }
        >
          <span>Send</span>
          <Icon name="arrow-up" size={16} />
        </button>
      </div>

      <div className="composer__foot">
        <span className="hint">
          {hint ?? (
            <>
              {waiting && (
                <>
                  <b>Tab</b> to answer {waiting.asker} ·{" "}
                </>
              )}
              {model && `${model} · `}
              <b>@</b> to name an agent
            </>
          )}
        </span>
        <span className="composer__keys">{FIND_KEY} search</span>
      </div>
    </div>
  );
}
