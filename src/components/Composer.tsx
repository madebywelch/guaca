import { Fragment, useEffect, useMemo, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { fileUrl, previewKind, readableSize } from "../lib/files";
import { api, onFileDrop } from "../lib/ipc";
import { applyMention, matchMentions, mentionAt, splitMentions } from "../lib/mentions";
import { useLiveAgents } from "../lib/store";
import { type Attachment, errorMessage } from "../lib/types";

interface Props {
  placeholder: string;
  disabled?: boolean;
  disabledReason?: string;
  onSend: (text: string, files: Attachment[]) => Promise<void>;
}

export function Composer({ placeholder, disabled, disabledReason, onSend }: Props) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [caret, setCaret] = useState(0);
  const [highlighted, setHighlighted] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [files, setFiles] = useState<Attachment[]>([]);
  /** What was dropped and could not be taken, in the words the runtime used. */
  const [refused, setRefused] = useState<string[]>([]);
  const [dragging, setDragging] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const mirror = useRef<HTMLDivElement>(null);

  // Dropping anywhere on the window attaches to whatever channel is open,
  // because the alternative is a small target the operator has to aim at while
  // holding a file.
  //
  // The file is taken into the store on the drop rather than on the send, so a
  // document too big to go is refused while they are still holding it and a
  // picture can be shown back to them before it goes.
  useEffect(() => {
    if (disabled) return;
    const stopping = onFileDrop({
      over: setDragging,
      dropped: (paths) => {
        void api
          .stageFiles(paths)
          .then((staged) => {
            // The same file twice is one attachment, by content rather than by
            // path: a second drop is a person making sure, and one document
            // saved in two places is still one document.
            setFiles((held) => [
              ...held,
              ...staged.attached.filter(
                (file) => !held.some((have) => have.digest === file.digest),
              ),
            ]);
            setRefused(staged.refused);
          })
          .catch((error) => setRefused([errorMessage(error)]));
      },
    });
    return () => {
      void stopping.then((stop) => stop());
    };
  }, [disabled]);

  const agents = useLiveAgents();
  const names = useMemo(() => agents.map((a) => a.name), [agents]);
  const query = dismissed ? null : mentionAt(text, caret);
  const matches = query ? matchMentions(names, query.term) : [];
  const showing = query !== null && matches.length > 0;
  const selected = matches.length > 0 ? Math.min(highlighted, matches.length - 1) : 0;

  // Grow with the content instead of scrolling a three-line box. Past the cap
  // it does scroll, which is why the layer under it is re-aligned here as well
  // as on a scroll event: typing at the bottom of a full box moves the view
  // without the operator having scrolled anything.
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
    setRefused([]);
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

  const hint = showing ? "↑↓ to choose · Enter to insert" : dragging ? "Drop to attach" : null;

  return (
    <div className="composer" data-dragging={dragging || undefined}>
      {files.length > 0 && (
        <ul className="composer__files" aria-label="Attached files">
          {files.map((file) => (
            <li key={file.digest} className="chip">
              {previewKind(file.mime) === "image" && (
                // The picture rather than its name: an operator who has dropped
                // three screenshots cannot tell them apart from
                // `Screenshot 2026-08-17 at 14.02.11.png` three times over.
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
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      {refused.length > 0 && (
        // Said where the attachments are, not as a banner over the app: the
        // operator is looking at what they just dropped, and the one that did
        // not arrive belongs beside the ones that did.
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

      <div className="composer__row">
        <div className="composer__field">
          {/* The draft, with its mentions drawn, under the box it belongs to.
              The operator's own characters are still the textarea's: this
              paints the pill behind them and nothing else, so the caret, a
              selection, undo and an input method are all left as they were.
              `aria-hidden` because it is the same text twice, and the copy a
              screen reader should read is the field. */}
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
            aria-expanded={showing}
            aria-controls={showing ? "mention-list" : undefined}
            aria-activedescendant={showing ? `mention-${selected}` : undefined}
            aria-autocomplete="list"
          />
        </div>
        <button
          type="button"
          className="composer__send"
          aria-label="Send"
          title="Send"
          onClick={() => void submit()}
          disabled={disabled || sending || (text.trim().length === 0 && files.length === 0)}
        >
          <span aria-hidden="true">↑</span>
        </button>
      </div>

      {/* Only when there is something to say. What used to live here
          permanently was three facts that announce themselves: the typeahead
          appears when you type @, the drop target says so while a file is over
          the window, and Enter sends in every chat box ever written. */}
      {hint && (
        <div className="composer__foot">
          <span className="hint">{hint}</span>
        </div>
      )}
    </div>
  );
}
