import { useEffect, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { onFileDrop } from "../lib/ipc";
import { applyMention, matchMentions, mentionAt } from "../lib/mentions";
import { useLiveAgents } from "../lib/store";

interface Props {
  placeholder: string;
  disabled?: boolean;
  disabledReason?: string;
  onSend: (text: string, files: string[]) => Promise<void>;
}

/** The last segment of a path, which is what a person calls the file. */
function basename(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

export function Composer({ placeholder, disabled, disabledReason, onSend }: Props) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [caret, setCaret] = useState(0);
  const [highlighted, setHighlighted] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [files, setFiles] = useState<string[]>([]);
  const [dragging, setDragging] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);

  // Dropping anywhere on the window attaches to whatever channel is open,
  // because the alternative is a small target the operator has to aim at while
  // holding a file.
  useEffect(() => {
    if (disabled) return;
    const stopping = onFileDrop({
      over: setDragging,
      dropped: (paths) =>
        // Same file twice is one attachment: a second drop of the same
        // document is a person making sure, not a request to send it twice.
        setFiles((held) => [...held, ...paths.filter((p) => !held.includes(p))]),
    });
    return () => {
      void stopping.then((stop) => stop());
    };
  }, [disabled]);

  const agents = useLiveAgents();
  const query = dismissed ? null : mentionAt(text, caret);
  const matches = query
    ? matchMentions(
        agents.map((a) => a.name),
        query.term,
      )
    : [];
  const showing = query !== null && matches.length > 0;
  const selected = matches.length > 0 ? Math.min(highlighted, matches.length - 1) : 0;

  // Grow with the content instead of scrolling a three-line box.
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${node.scrollHeight}px`;
  }, [text]);

  const choose = (name: string) => {
    if (!query) return;
    const next = applyMention(text, query, name);
    setText(next.text);
    setDismissed(false);
    // The caret has to move after React has written the new value, or the
    // browser puts it back at the end of the textarea.
    requestAnimationFrame(() => {
      const node = ref.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(next.caret, next.caret);
      setCaret(next.caret);
    });
  };

  const submit = async () => {
    const body = text.trim();
    // A file on its own is a message. "Read this" with nothing typed is how
    // people actually hand over a document.
    if ((!body && files.length === 0) || sending || disabled) return;
    setSending(true);
    // Clear optimistically: the message is echoed back from the runtime, and
    // leaving the draft in place makes it look like nothing happened.
    setText("");
    setFiles([]);
    try {
      await onSend(body, files);
    } catch {
      setText(body);
      setFiles(files);
    } finally {
      setSending(false);
      ref.current?.focus();
    }
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
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

    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void submit();
    }
  };

  const track = (event: { currentTarget: HTMLTextAreaElement }) => {
    setCaret(event.currentTarget.selectionStart ?? 0);
  };

  return (
    <div className="composer" data-dragging={dragging || undefined}>
      {files.length > 0 && (
        <ul className="composer__files" aria-label="Attached files">
          {files.map((path) => (
            <li key={path} className="chip">
              <span className="chip__name">{basename(path)}</span>
              <button
                type="button"
                className="chip__remove"
                aria-label={`Remove ${basename(path)}`}
                onClick={() => setFiles((held) => held.filter((p) => p !== path))}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      {showing && (
        // Focus stays in the textarea and `aria-activedescendant` names the
        // highlighted option: the standard combobox arrangement, and the reason
        // the options are not themselves focusable.
        <div className="mentions" role="listbox" id="mention-list" aria-label="Agents">
          {matches.map((name, index) => {
            const agent = agents.find((a) => a.name === name);
            const active = index === selected;
            return (
              <div
                key={name}
                id={`mention-${index}`}
                className="mentions__item"
                role="option"
                // Programmatically focusable but out of the tab order, which is
                // what a listbox option driven by aria-activedescendant wants.
                tabIndex={-1}
                aria-selected={active}
                data-active={active}
                onMouseEnter={() => setHighlighted(index)}
                // `mousedown` fires before the textarea loses focus, so the
                // menu is not torn down before the click lands.
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

      <textarea
        ref={ref}
        className="composer__input"
        rows={1}
        value={text}
        placeholder={disabled ? (disabledReason ?? placeholder) : placeholder}
        disabled={disabled}
        onChange={(event) => {
          setText(event.target.value);
          setCaret(event.target.selectionStart ?? 0);
          setDismissed(false);
          setHighlighted(0);
        }}
        onKeyUp={track}
        onClick={track}
        onKeyDown={onKeyDown}
        role="combobox"
        aria-expanded={showing}
        aria-controls={showing ? "mention-list" : undefined}
        aria-activedescendant={showing ? `mention-${selected}` : undefined}
        aria-autocomplete="list"
      />
      <div className="composer__foot">
        <span className="hint">
          {showing
            ? "↑↓ to choose · Enter to insert"
            : dragging
              ? "Drop to attach"
              : "Enter to send · @ to tag an agent · drop a file to attach it"}
        </span>
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => void submit()}
          disabled={disabled || sending || (text.trim().length === 0 && files.length === 0)}
        >
          Send
        </button>
      </div>
    </div>
  );
}
