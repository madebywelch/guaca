import { useEffect, useState } from "react";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import {
  exportLegacyGroup,
  type GroupArchive,
  legacyGroups,
  parseGroupFile,
  type Reconnect,
  saveGroup,
} from "../lib/transfer";
import { desktop } from "../lib/transport";
import { errorMessage, type Group, type GroupId } from "../lib/types";

function Connections({ items }: { items: Reconnect[] }) {
  if (!items.length) return null;
  return (
    <div className="field">
      <span className="field__label">Reconnect on this host</span>
      <ul>
        {[...new Map(items.map((item) => [JSON.stringify(item), item])).entries()].map(
          ([key, item]) => (
            <li key={key}>
              <strong>{item.name}</strong> ({item.kind})
              {item.agents.length > 0 && `: assign to ${item.agents.join(", ")}`}
              <div className="field__hint">
                {Object.entries(item.details)
                  .filter(([, v]) => typeof v === "string" && v)
                  .map(([k, v]) => `${k}: ${v}`)
                  .join(" · ")}
              </div>
            </li>
          ),
        )}
      </ul>
    </div>
  );
}

export function GroupTransfer({ group }: { group?: Group }) {
  const [archive, setArchive] = useState<GroupArchive | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [connections, setConnections] = useState<Reconnect[]>([]);
  useEffect(() => {
    let disposed = false;
    if (group)
      void api
        .groupReconnect(group.id)
        .then((items) => {
          if (!disposed) setConnections(items);
        })
        .catch(() => {});
    return () => {
      disposed = true;
    };
  }, [group]);
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      await action();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="host-choice">
      <h3>Move a group</h3>
      <p className="field__hint">
        Export agents, instructions, memory, conversations, attachments, calendar entries and
        routines. Sign-ins and repository working files stay on the original host. Exports include
        conversation content; keep them somewhere private.
      </p>
      {group && (
        <button
          className="btn btn--small"
          type="button"
          disabled={busy}
          onClick={() =>
            void run(async () => setMessage(await saveGroup(await api.exportGroup(group.id))))
          }
        >
          Export {group.name}
        </button>
      )}
      <Connections items={connections} />
      <label className="field">
        <span className="field__label">Import a group file</span>
        <input
          type="file"
          accept=".json,.guaca.json"
          disabled={busy}
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (!file) return;
            void run(async () => {
              setArchive(null);
              if (file.size > 64 * 1024 * 1024) throw new Error("Group files must be under 64 MB.");
              const parsed = parseGroupFile(await file.text());
              setArchive(parsed);
              setName(String(parsed.tables.groups?.[0]?.name ?? "Imported group"));
            });
          }}
        />
      </label>
      {archive && (
        <>
          <label className="field">
            <span className="field__label">Imported group name</span>
            <input
              className="input"
              value={name}
              disabled={busy}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <p>
            {archive.tables.agents?.length ?? 0} agents · {archive.tables.messages?.length ?? 0}{" "}
            messages · {archive.tables.routines?.length ?? 0} routines
          </p>
          <p className="field__hint">
            This creates a separate group. Routines arrive paused. Reconnect providers,
            repositories, plugins and agent computers before resuming work. The original group stays
            in place.
          </p>
          <Connections items={archive.reconnect} />
          <button
            className="btn btn--primary"
            type="button"
            disabled={busy || !name.trim()}
            onClick={() =>
              void run(async () => {
                const imported = await api.importGroup(archive, name.trim());
                setArchive(null);
                setConnections(archive.reconnect);
                await useStore.getState().refreshAgents();
                setMessage(
                  `Imported ${imported.name}. Its routines are paused. Open the group to reconnect services and resume work.`,
                );
              })
            }
          >
            {busy ? "Importing…" : "Import as a new group"}
          </button>
        </>
      )}
      {message && (
        <p className="banner banner--ok" role="status">
          {message}
        </p>
      )}
      {error && (
        <p className="banner banner--error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

export function LegacyGroups() {
  const [groups, setGroups] = useState<{ id: GroupId; name: string }[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  useEffect(() => {
    if (desktop)
      void legacyGroups()
        .then(setGroups)
        .catch((cause) => setMessage(errorMessage(cause)));
  }, []);
  if (!desktop || (!groups.length && !message)) return null;
  return (
    <details className="field">
      <summary>Groups from the previous desktop version</summary>
      <p className="field__hint">
        Your original workspace is still on this Mac. Quit the old Guaca before exporting so its
        work and memories stop changing. This app reads the old workspace without starting its
        agents or changing its database.
      </p>
      {groups.map((group) => (
        <button
          key={group.id}
          className="btn btn--small"
          type="button"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setMessage("");
            void exportLegacyGroup(group.id)
              .then(saveGroup)
              .then(setMessage)
              .catch((cause) => setMessage(errorMessage(cause)))
              .finally(() => setBusy(false));
          }}
        >
          Export {group.name}
        </button>
      ))}
      {message && <p role="status">{message}</p>}
    </details>
  );
}
