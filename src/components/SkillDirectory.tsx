import { type FormEvent, useCallback, useEffect, useRef, useState } from "react";

import { api, openExternal } from "../lib/ipc";
import {
  type DirectoryBoard,
  type DirectoryListing,
  type DirectorySkill,
  errorMessage,
  type Skill,
  type SkillAudit,
  type SkillScope,
} from "../lib/types";

interface Props {
  scope: SkillScope;
  /** Names already in this scope. An add under one of them is refused. */
  taken: ReadonlySet<string>;
  onAdded: (skill: Skill) => void;
}

const BOARDS: { id: DirectoryBoard; label: string }[] = [
  { id: "allTime", label: "Most installed" },
  { id: "trending", label: "Trending" },
  { id: "hot", label: "Hot" },
];

const COUNT = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });

/** "201.8K installs", and "1 install" for the one skill somebody tried once. */
function installs(count: number): string {
  return `${COUNT.format(count)} ${count === 1 ? "install" : "installs"}`;
}

function verdictClass(audit: SkillAudit): string {
  if (audit.verdict === "pass") return "access__ok";
  if (audit.verdict === "unknown") return "field__hint";
  return "access__warn";
}

/**
 * skills.sh, ranked, searched and read before anything is added.
 *
 * Nothing is fetched until the operator opens this, and nothing is added
 * without being opened first: the add sends back the hash of what was read,
 * and Rust refuses it if the skill changed in between. A skill is
 * instructions a crew will follow, so what the operator reads is the whole
 * `SKILL.md`, the files it carries and what skills.sh's auditors said, in
 * that order.
 */
export function SkillDirectory({ scope, taken, onAdded }: Props) {
  const [board, setBoard] = useState<DirectoryBoard>("allTime");
  const [query, setQuery] = useState("");
  const [searched, setSearched] = useState<string | null>(null);
  const [listings, setListings] = useState<DirectoryListing[] | null>(null);
  const [page, setPage] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [opened, setOpened] = useState<DirectorySkill | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const [added, setAdded] = useState<ReadonlySet<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Only the newest request draws. Switching boards twice quickly would
  // otherwise let the first answer land on top of the second.
  const latest = useRef(0);

  const show = useCallback(async (load: (current: () => boolean) => Promise<void>) => {
    const ticket = ++latest.current;
    const current = () => ticket === latest.current;
    setBusy(true);
    setError(null);
    try {
      await load(current);
    } catch (caught) {
      if (current()) setError(errorMessage(caught));
    } finally {
      if (current()) setBusy(false);
    }
  }, []);

  useEffect(() => {
    if (searched !== null) return;
    setListings(null);
    setOpened(null);
    void show(async (current) => {
      const first = await api.skillDirectory(board, 0);
      if (!current()) return;
      setListings(first.skills);
      setPage(0);
      setHasMore(first.hasMore);
    });
  }, [board, searched, show]);

  const more = () =>
    void show(async (current) => {
      const next = await api.skillDirectory(board, page + 1);
      if (!current()) return;
      setListings((shown) => [...(shown ?? []), ...next.skills]);
      setPage(next.page);
      setHasMore(next.hasMore);
    });

  const search = (event: FormEvent) => {
    event.preventDefault();
    const words = query.trim();
    if (!words) {
      setSearched(null);
      return;
    }
    setOpened(null);
    void show(async (current) => {
      const found = await api.searchSkillDirectory(words);
      if (!current()) return;
      setSearched(words);
      setListings(found);
      setHasMore(false);
    });
  };

  const open = async (listing: DirectoryListing) => {
    if (opened?.id === listing.id) {
      setOpened(null);
      return;
    }
    setOpening(listing.id);
    setError(null);
    try {
      setOpened(await api.previewDirectorySkill(listing.id));
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setOpening(null);
    }
  };

  const add = async (skill: DirectorySkill) => {
    setBusy(true);
    setError(null);
    try {
      const written = await api.addDirectorySkill(scope, skill.id, skill.hash);
      setAdded((before) => new Set(before).add(skill.id));
      onAdded(written);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  const reading = (skill: DirectorySkill) => {
    const here = added.has(skill.id);
    const clash = !here && taken.has(skill.name);
    return (
      <div className="directory__reading">
        <p className="field__hint">{skill.description}</p>
        {skill.audits === null ? (
          <p className="field__hint">Its security audits could not be read.</p>
        ) : skill.audits.length === 0 ? (
          <p className="field__hint">No security audit has run on this skill yet.</p>
        ) : (
          <ul className="directory__audits">
            {skill.audits.map((audit) => (
              <li key={audit.provider} className={verdictClass(audit)}>
                <strong>{audit.provider}</strong>: {audit.verdict}. {audit.summary}
              </li>
            ))}
          </ul>
        )}
        {skill.files.length > 0 && (
          <p className="field__hint">
            Carries {skill.files.length} {skill.files.length === 1 ? "file" : "files"} beside its
            instructions: {skill.files.join(", ")}. Agents can read them; Guaca never runs them.
          </p>
        )}
        <div className="access__row">
          <button
            type="button"
            className="btn btn--small btn--primary"
            disabled={busy || here || clash}
            onClick={() => void add(skill)}
          >
            {here ? "Added" : scope.kind === "crew" ? "Add to this crew" : "Add to workspace"}
          </button>
          <button
            type="button"
            className="btn btn--small btn--ghost"
            onClick={() => void openExternal(skill.url)}
          >
            Open on skills.sh
          </button>
          {clash && (
            <span className="field__hint">
              A skill called {skill.name} is already here. Delete or rename it first.
            </span>
          )}
        </div>
        <pre className="skill__body">{skill.body}</pre>
      </div>
    );
  };

  return (
    <div className="access__item directory">
      <div className="access__row">
        <div className="choices">
          {BOARDS.map((option) => (
            <button
              key={option.id}
              type="button"
              className="choice choice--tight"
              aria-pressed={searched === null && option.id === board}
              onClick={() => {
                setQuery("");
                setSearched(null);
                setBoard(option.id);
              }}
            >
              {option.label}
            </button>
          ))}
        </div>
        <form className="directory__search" onSubmit={search}>
          <input
            className="input"
            type="search"
            aria-label="Search skills.sh"
            placeholder="Search skills.sh"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </form>
      </div>
      <p className="field__hint">
        {searched === null
          ? "Ranked by installs across every agent that uses skills.sh. Open one to read it before adding it."
          : `Skills matching “${searched}”.`}
      </p>

      {listings === null && busy && <p className="field__hint">Loading skills.sh…</p>}
      {listings?.length === 0 && <p className="field__hint">Nothing on skills.sh matches that.</p>}
      {listings && listings.length > 0 && (
        <ol className="directory__list">
          {listings.map((listing) => (
            <li key={listing.id} className="directory__entry">
              <div className="access__row">
                <strong className="access__name">{listing.name}</strong>
                <span className="access__where">{listing.source}</span>
                <span className="access__when">{installs(listing.installs)}</span>
                {listing.addable ? (
                  <button
                    type="button"
                    className="btn btn--small"
                    disabled={opening !== null}
                    onClick={() => void open(listing)}
                  >
                    {opened?.id === listing.id
                      ? "Close"
                      : opening === listing.id
                        ? "Opening…"
                        : "Read"}
                  </button>
                ) : (
                  <button
                    type="button"
                    className="btn btn--small btn--ghost"
                    title="Published by its own site, which Guaca cannot add from yet"
                    onClick={() => void openExternal(listing.url)}
                  >
                    On skills.sh
                  </button>
                )}
              </div>
              {opened?.id === listing.id && reading(opened)}
            </li>
          ))}
        </ol>
      )}
      {hasMore && searched === null && (
        <button type="button" className="btn btn--small" disabled={busy} onClick={more}>
          Show more
        </button>
      )}

      {error && (
        <div className="banner banner--error" role="alert">
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
