import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  busyDays,
  crewsWith,
  dayLabel,
  daysIn,
  fieldDate,
  isPast,
  monthOf,
  openingWindow,
  timeLabel,
  type Window,
} from "../lib/calendar";
import { useDismiss } from "../lib/dismiss";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { useNow } from "../lib/time";
import {
  errorMessage,
  type GroupId,
  type Occasion,
  type OccasionDraft,
  type OccasionId,
} from "../lib/types";

interface Props {
  onClose: () => void;
}

const EMPTY_OCCASIONS: Occasion[] = [];

interface Editing {
  id: OccasionId | null;
  groupId: GroupId;
  title: string;
  startsAt: string;
  minutes: string;
  place: string;
  detail: string;
}

function monthLabel(window: Window): string {
  return new Date(window.from).toLocaleDateString([], { month: "long", year: "numeric" });
}

export function Calendar({ onClose }: Props) {
  const groups = useStore((state) => state.groups);
  const agents = useStore((state) => state.agents);
  const changed = useStore((state) => state.calendarVersion);
  const now = useNow(60_000);

  const [window, setWindow] = useState<Window>(() => openingWindow(Date.now()));
  const [crew, setCrew] = useState<GroupId | null>(null);
  const [read, setRead] = useState<
    { status: "loading" } | { status: "loaded"; occasions: Occasion[] } | { status: "failed" }
  >({ status: "loading" });
  const occasions =
    read.status === "loading" ? null : read.status === "loaded" ? read.occasions : EMPTY_OCCASIONS;
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [busy, setBusy] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  const editorOpen = editing !== null;
  useEffect(() => {
    if (editorOpen) titleRef.current?.focus();
  }, [editorOpen]);

  useDismiss(onClose);
  useDismiss(() => setEditing(null), editorOpen);

  const load = useCallback(async () => {
    try {
      setRead({ status: "loaded", occasions: await api.calendar(window.from, window.until) });
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setRead({ status: "failed" });
    }
  }, [window]);

  useEffect(() => {
    void load();
  }, [load, changed]);

  const crews = useMemo(() => crewsWith(groups, occasions ?? []), [groups, occasions]);
  const shown = useMemo(
    () => (crew === null ? (occasions ?? []) : (occasions ?? []).filter((o) => o.groupId === crew)),
    [occasions, crew],
  );
  const days = useMemo(() => busyDays(daysIn(shown, window)), [shown, window]);

  const defaultCrew = crew ?? groups[0]?.id;

  const save = async () => {
    if (!editing) return;
    setBusy(true);
    setError(null);
    const draft: OccasionDraft = {
      groupId: editing.groupId,
      title: editing.title,
      detail: editing.detail,
      place: editing.place,
      startsAt: editing.startsAt,
      minutes: editing.minutes.trim() === "" ? null : Number(editing.minutes),
    };
    try {
      if (editing.id) await api.updateOccasion(editing.id, draft);
      else await api.createOccasion(draft);
      setEditing(null);
      await load();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (id: OccasionId) => {
    setBusy(true);
    setError(null);
    try {
      await api.deleteOccasion(id);
      setEditing(null);
      await load();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  const open = (occasion: Occasion) =>
    setEditing({
      id: occasion.id,
      groupId: occasion.groupId,
      title: occasion.title,
      startsAt: fieldDate(occasion),
      minutes: occasion.minutes === null ? "" : String(occasion.minutes),
      place: occasion.place,
      detail: occasion.detail,
    });

  const blank = () => {
    if (!defaultCrew) return;
    const day = new Date(Math.max(window.from, Date.now()));
    day.setHours(9, 0, 0, 0);
    const pad = (n: number) => String(n).padStart(2, "0");
    setEditing({
      id: null,
      groupId: defaultCrew,
      title: "",
      startsAt: `${day.getFullYear()}-${pad(day.getMonth() + 1)}-${pad(day.getDate())} 09:00`,
      minutes: "",
      place: "",
      detail: "",
    });
  };

  const row = (occasion: Occasion) => {
    const whose = groups.find((group) => group.id === occasion.groupId);
    const author = agents.find((agent) => agent.id === occasion.agentId);
    return (
      <li key={occasion.id}>
        <button
          type="button"
          className="occasion"
          data-past={isPast(occasion, now) ? "true" : undefined}
          onClick={() => open(occasion)}
          title={occasion.detail || occasion.title}
        >
          <span className="occasion__when">{timeLabel(occasion)}</span>
          <span className="occasion__body">
            <span className="occasion__title">{occasion.title}</span>
            <span className="occasion__meta">
              {whose && <span className="occasion__crew">{whose.name}</span>}
              {occasion.place && <span>{occasion.place}</span>}
              {author && <span>{author.name}</span>}
            </span>
          </span>
        </button>
      </li>
    );
  };

  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog dialog--calendar"
        role="dialog"
        aria-modal="true"
        aria-label="Calendar"
        tabIndex={-1}
        ref={panelRef}
      >
        <div className="calendar__head">
          <div className="calendar__title-row">
            <h2 className="dialog__title">{monthLabel(window)}</h2>
            <div className="calendar__nav">
              <button
                type="button"
                className="btn btn--quiet"
                aria-label="Previous month"
                onClick={() => setWindow(monthOf(window.from, -1))}
              >
                ‹
              </button>
              <button
                type="button"
                className="btn btn--quiet"
                onClick={() => setWindow(openingWindow(Date.now()))}
              >
                Today
              </button>
              <button
                type="button"
                className="btn btn--quiet"
                aria-label="Next month"
                onClick={() => setWindow(monthOf(window.from, 1))}
              >
                ›
              </button>
              <button
                type="button"
                className="btn btn--primary"
                disabled={!defaultCrew}
                onClick={blank}
              >
                Add
              </button>
            </div>
          </div>

          <fieldset className="calendar__crews">
            <legend className="offscreen">Which group</legend>
            <button
              type="button"
              className="calendar__crew"
              aria-pressed={crew === null}
              onClick={() => setCrew(null)}
            >
              All groups
              <span className="calendar__count">{occasions?.length ?? 0}</span>
            </button>
            {crews.map((one) => (
              <button
                key={one.id}
                type="button"
                className="calendar__crew"
                aria-pressed={crew === one.id}
                onClick={() => setCrew(one.id)}
              >
                {one.name}
                <span className="calendar__count">{one.count}</span>
              </button>
            ))}
          </fieldset>
        </div>

        <div className="calendar__body">
          {occasions === null ? (
            <p className="routines__note">Loading…</p>
          ) : days.length === 0 ? (
            <p className="routines__note">
              Nothing on this calendar {crew === null ? "" : "for this group "}in{" "}
              {monthLabel(window)}. This is Guaca's own calendar, not your Google one: agents put
              what they learn about here, and so can you.
            </p>
          ) : (
            days.map((day) => (
              <section key={day.at} className="calendar__day">
                <h3 className="calendar__date">{dayLabel(day.at, now)}</h3>
                <ul className="calendar__list">{day.occasions.map(row)}</ul>
              </section>
            ))
          )}
        </div>

        {editing && (
          <div className="calendar__editor">
            <label className="field">
              <span className="field__label">What is happening</span>
              <input
                className="input"
                aria-label="What is happening"
                ref={titleRef}
                value={editing.title}
                placeholder="Board call"
                onChange={(event) => setEditing({ ...editing, title: event.target.value })}
              />
            </label>

            <div className="calendar__pair">
              <label className="field">
                <span className="field__label">When</span>
                <input
                  className="input input--slim"
                  aria-label="When"
                  value={editing.startsAt}
                  placeholder="2026-09-14 15:00"
                  onChange={(event) => setEditing({ ...editing, startsAt: event.target.value })}
                />
                <span className="field__hint">
                  Local time. A date on its own is a whole day, which is what a deadline is.
                </span>
              </label>

              <label className="field">
                <span className="field__label">Minutes</span>
                <input
                  className="input input--slim"
                  aria-label="Minutes"
                  inputMode="numeric"
                  value={editing.minutes}
                  placeholder="60"
                  onChange={(event) => setEditing({ ...editing, minutes: event.target.value })}
                />
                <span className="field__hint">Leave blank for something with no length.</span>
              </label>
            </div>

            <div className="calendar__pair">
              <label className="field">
                <span className="field__label">Group</span>
                <select
                  className="input input--slim"
                  aria-label="Group"
                  value={editing.groupId}
                  disabled={editing.id !== null}
                  onChange={(event) =>
                    setEditing({ ...editing, groupId: event.target.value as GroupId })
                  }
                >
                  {groups.map((group) => (
                    <option key={group.id} value={group.id}>
                      {group.name}
                    </option>
                  ))}
                </select>
              </label>

              <label className="field">
                <span className="field__label">Where</span>
                <input
                  className="input input--slim"
                  aria-label="Where"
                  value={editing.place}
                  placeholder="Zoom"
                  onChange={(event) => setEditing({ ...editing, place: event.target.value })}
                />
              </label>
            </div>

            <label className="field">
              <span className="field__label">Notes</span>
              <textarea
                className="textarea"
                aria-label="Notes"
                rows={3}
                value={editing.detail}
                placeholder="What you need to walk in prepared"
                onChange={(event) => setEditing({ ...editing, detail: event.target.value })}
              />
            </label>

            <div className="calendar__actions">
              {editing.id && (
                <button
                  type="button"
                  className="btn btn--danger"
                  disabled={busy}
                  onClick={() => void remove(editing.id as OccasionId)}
                >
                  Cancel it
                </button>
              )}
              <span style={{ flex: 1 }} />
              <button
                type="button"
                className="btn btn--quiet"
                disabled={busy}
                onClick={() => setEditing(null)}
              >
                Back
              </button>
              <button
                type="button"
                className="btn btn--primary"
                disabled={busy || editing.title.trim() === ""}
                onClick={() => void save()}
              >
                {busy ? "Saving…" : "Save"}
              </button>
            </div>
          </div>
        )}

        {error && (
          <div className="banner banner--error" style={{ margin: "0 1.35rem" }}>
            <span>{error}</span>
          </div>
        )}

        <div className="calendar__foot">
          <p className="calendar__note">
            Guaca's own calendar. Agents keep it as they learn things; nothing here books a meeting,
            invites anyone or leaves this machine.
          </p>
          <button type="button" className="btn" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
