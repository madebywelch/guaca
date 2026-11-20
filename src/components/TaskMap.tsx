import { type KeyboardEvent, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";
import { focusWriting } from "../lib/focus";
import { useShown } from "../lib/held";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type TaskMap as MapData, type TaskBranch, taskMaps } from "../lib/taskMap";
import { clockLabel } from "../lib/time";
import type { Lookups } from "../lib/transcript";
import { RECONNECTED } from "../lib/transport";
import {
  type AgentId,
  type Approval,
  type Envelope,
  plainText,
  type RunId,
  type TaskRecord,
} from "../lib/types";
import { FileCard } from "./FileCard";
import { Markdown } from "./Markdown";
import "./TaskMap.css";

const DIRECT_BRANCHES = 3;
const RECENT_TASKS = 20;
const PREVIEW_LENGTH = 140;
const SUMMARY_LENGTH = 64;

type TaskRead =
  | { kind: "loaded"; record: TaskRecord | null }
  | { kind: "retry"; record: TaskRecord | null }
  | { kind: "oversized"; record: TaskRecord | null }
  | { kind: "unsupported"; record: null };

function preview(text: string, limit: number): string {
  const line = text.trim().split(/\s+/);
  let kept = "";
  for (const word of line) {
    const next = kept ? `${kept} ${word}` : word;
    if (next.length > limit) return `${kept}…`;
    kept = next;
  }
  return kept;
}

export function useTaskMaps(
  channel: AgentId,
  local: readonly Envelope[] | undefined,
  runs: string | null,
) {
  const { pending } = useShown();
  const ownedRuns = useStore((state) =>
    state.running
      .filter((run) => run.agentId === channel)
      .map((run) => run.runId)
      .join(" "),
  );
  const [records, setRecords] = useState<
    ReadonlyMap<RunId, TaskRead & { channel: AgentId; running: boolean }>
  >(() => new Map());
  const localRuns = [
    ...new Set(
      [...(local ?? [])]
        .reverse()
        .flatMap((message) =>
          message.parts.some((part) => part.type === "toolCall" && part.name === "send_message")
            ? [message.runId]
            : [],
        ),
    ),
  ]
    .slice(0, RECENT_TASKS)
    .join(" ");
  const pulse = useStore((state) => state.pulses.at(-1)?.id);
  const [attempt, setAttempt] = useState(0);
  const retry = () => setAttempt((value) => value + 1);
  const latest = useRef(records);
  useLayoutEffect(() => {
    latest.current = records;
  }, [records]);
  useEffect(() => {
    const reconnect = () => setAttempt((value) => value + 1);
    window.addEventListener(RECONNECTED, reconnect);
    return () => window.removeEventListener(RECONNECTED, reconnect);
  }, []);

  useEffect(() => {
    if (runs === null || !api.taskRecord) return;
    const active = new Set(ownedRuns.split(" ").filter(Boolean));
    const requested = new Set([...active, ...localRuns.split(" ").filter(Boolean)]);
    let current = true;
    for (const run of requested) {
      const running = active.has(run);
      const prior = latest.current.get(run);
      if (
        prior?.channel === channel &&
        (prior.kind === "unsupported" ||
          prior.kind === "oversized" ||
          (!running && !prior.running && prior.kind === "loaded"))
      )
        continue;
      const update = (read: TaskRead) => {
        if (!current) return;
        setRecords((old) =>
          new Map(
            [...old].filter(([id, value]) => requested.has(id) && value.channel === channel),
          ).set(run, { channel, ...read, running }),
        );
      };
      void api.taskRecord(channel, run).then(
        (record) => update({ kind: "loaded", record }),
        (error: unknown) => {
          const kind =
            typeof error === "object" && error !== null && "kind" in error ? error.kind : null;
          if (kind === "unknownCommand") update({ kind: "unsupported", record: null });
          else
            update({
              kind: kind === "validation" ? "oversized" : "retry",
              record: prior?.channel === channel ? prior.record : null,
            });
        },
      );
    }
    return () => {
      current = false;
    };
  }, [channel, ownedRuns, localRuns, runs, pulse, attempt]);

  const candidates = new Set(
    typeof api.taskRecord === "function"
      ? `${ownedRuns} ${localRuns}`.split(" ").filter(Boolean)
      : [],
  );
  const active = new Set(ownedRuns.split(" "));
  const delegated = new Set(localRuns.split(" ").filter(Boolean));
  const recorded = new Map<RunId, TaskRecord>();
  const results = new Map<RunId, TaskRecord["resultId"]>();
  const unavailable = new Map<RunId, "retry" | "oversized">();
  const checking = new Set<RunId>();
  for (const run of candidates) {
    const read = records.get(run);
    if (!read || read.channel !== channel) {
      if (delegated.has(run)) results.set(run, null);
      continue;
    }
    if (read.kind === "retry" || read.kind === "oversized") {
      if (delegated.has(run)) {
        unavailable.set(run, read.kind);
        results.set(run, null);
      }
    }
    if (read.running && !active.has(run)) checking.add(run);
    if (read.record) recorded.set(run, read.record);
  }
  const maps =
    runs === null
      ? []
      : taskMaps(
          channel,
          [...recorded.values()].flatMap((task) => task.messages),
          pending,
          new Set(runs.split(" ")),
        );
  for (const map of maps) {
    const read = records.get(map.runId);
    if (read?.kind === "retry" || read?.kind === "oversized") {
      unavailable.set(map.runId, read.kind);
      results.set(map.runId, null);
    } else {
      results.set(map.runId, recorded.get(map.runId)?.resultId ?? null);
    }
  }
  return { maps, results, unavailable, checking, retry };
}

function firstQuestion(map: MapData): Approval | undefined {
  return map.branches[0]?.questions[0];
}

export function ActiveTaskMaps({ maps, lookups }: { maps: MapData[]; lookups: Lookups }) {
  const [folded, setFolded] = useState<ReadonlySet<RunId>>(() => new Set());
  const [selected, setSelected] = useState<RunId | null>(null);
  const ordered = [...maps].sort((a, b) => {
    const x = firstQuestion(a);
    const y = firstQuestion(b);
    if (x && y)
      return (x.lapsesAt ?? Infinity) - (y.lapsesAt ?? Infinity) || x.createdAt - y.createdAt;
    return Number(!!y) - Number(!!x);
  });
  if (!ordered.length) return null;
  const opened =
    ordered.find((map) => map.runId === selected && !folded.has(map.runId))?.runId ??
    ordered.find((map) => !folded.has(map.runId) && firstQuestion(map))?.runId ??
    (ordered.length === 1 && !folded.has(ordered[0]!.runId) ? ordered[0]!.runId : null);
  return (
    <section className="task-map-stack" aria-label="Tasks in progress">
      {ordered.map((map) => (
        <TaskMapCard
          key={map.runId}
          map={map}
          lookups={lookups}
          open={opened === map.runId}
          onToggle={() => {
            setFolded((old) => {
              const next = new Set(old);
              if (opened === map.runId) next.add(map.runId);
              else next.delete(map.runId);
              return next;
            });
            setSelected(opened === map.runId ? null : map.runId);
          }}
        />
      ))}
    </section>
  );
}

export function TaskMapCard({
  map,
  lookups,
  open,
  onToggle,
  delivered = false,
  checking = false,
  unavailable,
}: {
  map: MapData;
  lookups: Lookups;
  open: boolean;
  onToggle: () => void;
  delivered?: boolean;
  checking?: boolean;
  unavailable?: "retry" | "oversized";
}) {
  const [detail, setDetail] = useState<
    "colleagues" | "brief" | { name: string; message: Envelope }
  >();
  const [fitted, setFitted] = useState(DIRECT_BRANCHES);
  const first = useRef<HTMLDivElement>(null);
  const graph = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const dialog = useRef<HTMLDivElement>(null);
  const closeDetail = () => setDetail(undefined);
  useDismiss(closeDetail, open && detail !== undefined);
  useLayoutEffect(() => {
    if (detail) dialog.current?.focus();
  }, [detail]);
  const select = useStore((state) => state.select);
  const answer = useStore((state) => state.answerQuestion);
  const lead = map.brief.to.kind === "agent" ? lookups.byId(map.brief.to.id) : undefined;
  const text = plainText(map.brief);
  const visibleBrief = preview(text, PREVIEW_LENGTH);
  const count = map.branches.length;
  const waiting = map.branches.filter((branch) => branch.questions.length).length;
  const answered = map.branches.filter((branch) => branch.reply).length;
  const hidden = Math.max(0, count - fitted);
  useLayoutEffect(() => {
    const node = graph.current;
    const branches = list.current;
    if (!open || !map.inFlight || !node || !branches) return;
    const measure = () => {
      if (!node.getClientRects().length) return;
      const candidates = Array.from(branches.querySelectorAll<HTMLElement>(":scope > li"));
      const fitFromStart = () => {
        const bottom =
          node.getBoundingClientRect().bottom - parseFloat(getComputedStyle(node).paddingBottom);
        const firstHidden = candidates.findIndex(
          (branch) => branch.getBoundingClientRect().bottom > bottom + 0.5,
        );
        return firstHidden === -1 ? candidates.length : firstHidden;
      };
      for (const branch of candidates) branch.hidden = false;
      node.removeAttribute("data-compact");
      let fit = fitFromStart();
      if (fit === 0) {
        node.setAttribute("data-compact", "true");
        fit = fitFromStart();
      }
      candidates.forEach((branch, index) => {
        branch.hidden = index >= fit;
      });
      setFitted(fit);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    observer.observe(branches);
    const pane = node.closest(".pane__body");
    if (pane) observer.observe(pane);
    return () => observer.disconnect();
  }, [open, map]);
  const onKeyDown = (event: KeyboardEvent) => {
    if (
      event.nativeEvent.isComposing ||
      event.metaKey ||
      event.ctrlKey ||
      event.altKey ||
      event.target instanceof HTMLInputElement ||
      detail !== undefined ||
      first.current?.closest("li")?.hidden
    )
      return;
    const question = firstQuestion(map);
    if (question?.request.kind !== "question") return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (first.current?.contains(document.activeElement)) focusWriting();
      else first.current?.focus();
    } else if (first.current?.contains(document.activeElement) && /^[1-9]$/.test(event.key)) {
      const option = question.request.options[Number(event.key) - 1];
      if (option !== undefined) {
        event.preventDefault();
        void answer(question.id, option);
      }
    }
  };
  const name = (id: AgentId) => lookups.byId(id)?.name ?? "Deleted agent";
  const participant = (id: AgentId) => {
    const agent = lookups.byId(id);
    return (
      <button
        type="button"
        className="task-map__person"
        disabled={!agent}
        onClick={() => void select(id)}
      >
        {agent && <AgentAvatar avatar={agent.avatar} color={agent.color} size="xs" seed={id} />}
        {name(id)}
      </button>
    );
  };
  const renderBranch = (branch: TaskBranch, index?: number) => (
    <li
      key={branch.agentId}
      className="task-map__node"
      data-question={branch.questions.length > 0 || undefined}
    >
      <div className="task-map__branch-head">
        {participant(branch.agentId)}
        <span>
          {branch.questions.length
            ? "Question for you"
            : branch.reply
              ? "Sent a message"
              : map.inFlight
                ? "Working"
                : "Did not answer"}
        </span>
      </div>
      {!branch.questions.length && !branch.reply && (
        <p className="task-map__excerpt">{preview(plainText(branch.asked), PREVIEW_LENGTH)}</p>
      )}
      {branch.questions.map((request, questionIndex) => (
        <BranchQuestion
          key={request.id}
          question={request}
          first={index === 0 && questionIndex === 0 ? first : undefined}
        />
      ))}
      {branch.reply && (
        <div className="task-map__reply">
          {!branch.questions.length && (
            <p className="task-map__excerpt">{preview(plainText(branch.reply), PREVIEW_LENGTH)}</p>
          )}
          <button
            type="button"
            className="btn btn--compact"
            aria-haspopup="dialog"
            onClick={() => {
              if (branch.reply) setDetail({ name: name(branch.agentId), message: branch.reply });
            }}
          >
            Open {name(branch.agentId)}’s message
          </button>
        </div>
      )}
      {branch.delegatedTo.map((id) => (
        <div key={id} className="task-map__nested">
          Asked {participant(id)}
        </div>
      ))}
    </li>
  );
  const detailTitle =
    detail === "colleagues"
      ? "More colleagues"
      : detail === "brief"
        ? "Your brief"
        : detail
          ? `Message from ${detail.name}`
          : "";
  return (
    <section
      className="task-map"
      aria-label="Task map"
      onKeyDown={onKeyDown}
      data-in-flight={map.inFlight || undefined}
    >
      <button type="button" className="task-map__summary" aria-expanded={open} onClick={onToggle}>
        <strong>
          {map.inFlight || checking || unavailable
            ? "Task map"
            : delivered
              ? "Hand-off complete"
              : "Hand-off ended"}
        </strong>
        <span className="task-map__subject">{preview(text, SUMMARY_LENGTH)}</span>
        <span>
          {unavailable
            ? "Map could not refresh"
            : checking
              ? "Checking for the result"
              : map.inFlight
                ? waiting
                  ? `${waiting} waiting for you`
                  : `Started ${clockLabel(map.brief.createdAt)}`
                : `${answered} of ${count} answered`}
        </span>
        <span>{open ? "Hide map" : "Show map"}</span>
      </button>
      {open && (
        <>
          <div className="task-map__graph" ref={graph}>
            <div className="task-map__root">
              <div className="task-map__node">
                <span>Your brief</span>
                <p>{visibleBrief}</p>
                {text.length > PREVIEW_LENGTH && (
                  <button
                    type="button"
                    className="btn btn--compact"
                    aria-haspopup="dialog"
                    onClick={() => setDetail("brief")}
                  >
                    Read brief
                  </button>
                )}
              </div>
              <div className="task-map__node">
                <span>Leads the task</span>
                {lead && participant(lead.id)}
              </div>
            </div>
            <ul className="task-map__branches" ref={list}>
              {map.branches.slice(0, DIRECT_BRANCHES).map(renderBranch)}
            </ul>
          </div>
          <div className="task-map__return">
            <span>
              {unavailable
                ? "Showing the last recorded map."
                : checking
                  ? "Checking for the result."
                  : map.inFlight
                    ? "The result will arrive in this conversation."
                    : delivered
                      ? "The result is in this conversation."
                      : "No result was delivered."}
            </span>
            {(map.inFlight || hidden > 0) && (
              <button
                type="button"
                className="btn btn--compact"
                aria-haspopup="dialog"
                data-empty={hidden === 0 || undefined}
                onClick={() => setDetail("colleagues")}
              >
                Show {hidden} more {hidden === 1 ? "colleague" : "colleagues"}
              </button>
            )}
          </div>
          {detail &&
            createPortal(
              <div className="scrim">
                <button
                  type="button"
                  className="scrim__close"
                  aria-label="Close dialog"
                  onClick={closeDetail}
                />
                <div
                  className="dialog dialog--form task-map__detail"
                  role="dialog"
                  aria-modal="true"
                  aria-label={detailTitle}
                  ref={dialog}
                  tabIndex={-1}
                >
                  <div className="dialog__body">
                    <h2 className="dialog__title">{detailTitle}</h2>
                    {detail === "colleagues" ? (
                      <ul className="task-map__branches">
                        {map.branches.slice(fitted).map((branch) => renderBranch(branch))}
                      </ul>
                    ) : detail === "brief" ? (
                      <p>{text}</p>
                    ) : (
                      <>
                        <Markdown>{plainText(detail.message)}</Markdown>
                        {detail.message.parts.flatMap((part) =>
                          part.type === "file" ? [<FileCard file={part} key={part.digest} />] : [],
                        )}
                      </>
                    )}
                  </div>
                  <div className="dialog__foot">
                    <button type="button" className="btn" onClick={closeDetail}>
                      Close
                    </button>
                  </div>
                </div>
              </div>,
              document.body,
            )}
        </>
      )}
    </section>
  );
}

function BranchQuestion({
  question,
  first,
}: {
  question: Approval;
  first?: React.RefObject<HTMLDivElement | null>;
}) {
  const answer = useStore((state) => state.answerQuestion);
  const [written, setWritten] = useState("");
  const [typing, setTyping] = useState(false);
  const [sending, setSending] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useLayoutEffect(() => {
    if (typing) input.current?.focus();
  }, [typing]);
  const send = (text: string) => {
    setSending(true);
    void answer(question.id, text).finally(() => setSending(false));
  };
  if (question.request.kind !== "question") return null;
  return (
    <div ref={first} tabIndex={-1} className="task-map__question" data-question={question.id}>
      <p>{question.summary}</p>
      {question.lapsesAt !== null && <span>Waits until {clockLabel(question.lapsesAt)}</span>}
      <div className="task-map__actions">
        {!typing &&
          question.request.options.map((option) => (
            <button
              key={option}
              className="btn btn--compact"
              type="button"
              disabled={sending}
              onClick={() => send(option)}
            >
              {option}
            </button>
          ))}
        {question.request.options.length > 0 && !typing ? (
          <button className="btn btn--compact" type="button" onClick={() => setTyping(true)}>
            Type an answer
          </button>
        ) : (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (written.trim()) send(written.trim());
            }}
          >
            <input
              ref={input}
              aria-label="Type an answer"
              placeholder="Type an answer"
              className="ask__field"
              value={written}
              disabled={sending}
              onChange={(event) => setWritten(event.target.value)}
            />
            <button
              className="btn btn--compact"
              type="submit"
              disabled={sending || !written.trim()}
            >
              Send answer
            </button>
          </form>
        )}
      </div>
    </div>
  );
}
