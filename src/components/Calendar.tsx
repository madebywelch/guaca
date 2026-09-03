import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  busyDays,
  crewsWith,
  dayLabel,
  daysIn,
  isPast,
  monthOf,
  openingWindow,
  timeLabel,
  type Window,
} from "../lib/calendar";
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

/** What the editor holds while it is open. `null` is nothing being edited. */
interface Editing {
  /** The occasion being changed, or `null` for one being written. */
  id: OccasionId | null;
  groupId: GroupId;
  title: string;
  startsAt: string;
  minutes: string;
  place: string;
  detail: string;
}

/** How the date field is written, which is also how Rust reads it back. */
function fieldDate(occasion: Occasion): string {
  const when = new Date(occasion.startsAt);
  const pad = (n: number) => String(n).padStart(2, "0");
  const day = `${when.getFullYear()}-${pad(when.getMonth() + 1)}-${pad(when.getDate())}`;
  // A date with no time on it is what makes an occasion an all-day one, in
  // exactly one field. A second checkbox saying so is a second thing to keep in
  // step with the string beside it.
  return occasion.allDay ? day : `${day} ${pad(when.getHours())}:${pad(when.getMinutes())}`;
}

/** The month a window names, for the heading. */
function monthLabel(window: Window): string {
  return new Date(window.from).toLocaleDateString([], { month: "long", year: "numeric" });
}

/**
 * The workspace calendar: every crew's dates, in one place.
 *
 * Guaca's own, and not the Google plugin's view of the operator's real one.
 * What is on it is what an agent or the operator wrote down as coming, which is
 * the thing nothing else in the app held: an agent that learned a filing was
 * due had memory, a working note and a routine to choose from, and all three
 * are the wrong shape for "what is happening on Thursday".
 *
 * ## Every crew at once, filterable to one
 *
 * The default and the point. A calendar is per crew because the wall between
 * crews is real — one crew's agents cannot see or move another's dates — but
 * the operator is above that wall and stands in front of all of it. Filtering
 * to one crew is a chip, not a mode: the chips are drawn from the groups rather
 * than from the occasions, so a crew with an empty calendar is still something
 * you can pick, and picking it and finding nothing is an answer.
 *
 * ## A month at a time, as days
 *
 * Days rather than a list sorted by date, because a flat list reads as a feed
 * and nothing in it says where Tuesday ends. Only the days with something on
 * them are drawn: a month of empty rows is a month of scrolling, and the
 * heading already says which month is being looked at.
 *
 * A month rather than a rolling thirty days, because "show me October" has an
 * answer and "the next thirty days" slides under the operator every morning.
 * It opens on this month plus the rest of the next, which is what stops a
 * calendar opened on the 29th from being a calendar showing two days.
 *
 * ## An overlay, not a pane
 *
 * The same shape as the cafeteria and the compost, reached from the same
 * footer, because it is the same kind of thing: somewhere you go, look at the
 * whole workspace, and come back from. A pane would have to live inside a crew,
 * which is the one framing this surface exists to escape.
 */
export function Calendar({ onClose }: Props) {
  const groups = useStore((state) => state.groups);
  const agents = useStore((state) => state.agents);
  // Bumped whenever any crew's calendar moves, including by an agent mid-turn.
  // Without it the list on screen is the one that was true when the panel
  // opened, and the only way to find out otherwise is to close it and come back.
  const changed = useStore((state) => state.calendarVersion);
  const now = useNow(60_000);

  const [window, setWindow] = useState<Window>(() => openingWindow(Date.now()));
  const [crew, setCrew] = useState<GroupId | null>(null);
  const [occasions, setOccasions] = useState<Occasion[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [busy, setBusy] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  // The editor takes the keyboard as it opens, on the field that is always the
  // first thing typed. Keyed on whether one is open rather than on which
  // occasion it holds, so typing into the title does not pull focus back to the
  // start of it on every keystroke.
  const editorOpen = editing !== null;
  useEffect(() => {
    if (editorOpen) titleRef.current?.focus();
  }, [editorOpen]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // The editor first, so Escape backs out of what is on top rather than
      // closing the whole panel out from under a half-written occasion.
      if (event.key !== "Escape") return;
      if (editing) setEditing(null);
      else onClose();
    };
    globalThis.addEventListener("keydown", onKey);
    return () => globalThis.removeEventListener("keydown", onKey);
  }, [onClose, editing]);

  // Read unfiltered and filtered in the view. The crew chips carry a count
  // each, so the read has to cover every crew whichever one is picked; asking
  // Rust per crew would make the round trips the number of crews and still need
  // the whole set to draw the chips.
  const load = useCallback(async () => {
    try {
      setOccasions(await api.calendar(window.from, window.until));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setOccasions([]);
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

  /** Which crew a new occasion lands in when the operator has not picked one. */
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
      // Left open on a failure. The commonest one is a date Rust could not
      // read, and closing the editor would throw away everything typed beside
      // the one field that has to change.
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
              {/* The crew is drawn on every row, not only in the unfiltered
                  view. It is what an occasion belongs to and the reason one
                  crew's agents cannot touch another's, so a row that only says
                  it while you happen to be looking at everything is a row that
                  hides the thing that matters about it. */}
              {whose && <span className="occasion__crew">{whose.name}</span>}
              {occasion.place && <span>{occasion.place}</span>}
              {/* Who noticed it. Blank for the operator's own, which is the
                  ordinary case and needs no label saying so. */}
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
                className="btn btn--ghost"
                aria-label="Previous month"
                onClick={() => setWindow(monthOf(window.from, -1))}
              >
                ‹
              </button>
              <button
                type="button"
                className="btn btn--ghost"
                onClick={() => setWindow(openingWindow(Date.now()))}
              >
                Today
              </button>
              <button
                type="button"
                className="btn btn--ghost"
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

          {/* One row of crews, and "All" first because it is the default and
              the thing this surface is for. Drawn even with one crew, so the
              count on it is the answer to "is anything on this at all".

              A fieldset because it genuinely is one: a set of controls that
              only mean anything together, and the one thing a screen reader
              cannot get from the buttons themselves is what the set is for. The
              legend is offscreen because the chips already read as crew names
              and a visible "Which crew" above them would be a heading nobody
              needs. */}
          <fieldset className="calendar__crews">
            <legend className="offscreen">Which crew</legend>
            <button
              type="button"
              className="calendar__crew"
              aria-pressed={crew === null}
              onClick={() => setCrew(null)}
            >
              All crews
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
              Nothing on this calendar {crew === null ? "" : "for this crew "}in{" "}
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
                <span className="field__label">Crew</span>
                <select
                  className="input input--slim"
                  aria-label="Crew"
                  value={editing.groupId}
                  // Only on a new one. Moving an occasion between crews would
                  // move it out from under the agents that keep it, and there
                  // is no call that does it.
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
                className="btn btn--ghost"
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
          {/* Said once, at the bottom, because it is the same sentence about
              every row and the same sentence three times is wallpaper. It is
              also the one thing about this surface nobody would guess: the app
              has a Google Calendar plugin, and this is not it. */}
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
