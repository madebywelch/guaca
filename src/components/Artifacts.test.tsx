import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AgentCard,
  Artifact,
  ArtifactDetail,
  ArtifactRead,
  Settings,
  WidgetWidth,
} from "../lib/types";
import { aGroup, DEFAULT_GROUP } from "../test-fixtures";

/**
 * The Artifacts dialog and the card that opens it, over a mocked store.
 *
 * The sentences in the history are `lib/artifacts`'s and are tested there.
 * What is checked here is the wiring nothing else covers: which crew's list is
 * asked for, which version's page, what a restore and a hand-over send, and
 * that deleting asks first.
 */

const artifacts = vi.fn<(groupId?: string | null) => Promise<Artifact[]>>(async () => []);
const artifactDetail = vi.fn<(id: string) => Promise<ArtifactDetail>>();
const artifactPage = vi.fn<(id: string, version?: number | null) => Promise<string>>(
  async () => "<p>page</p>",
);
const restoreArtifact = vi.fn<(id: string, version: number) => Promise<Artifact>>();
const handArtifact = vi.fn<(id: string, agentId: string) => Promise<Artifact>>();
const deleteArtifact = vi.fn<(id: string) => Promise<void>>(async () => {});
const artifactData = vi.fn<(id: string, version?: number | null) => Promise<ArtifactRead[]>>(
  async () => [],
);
const allowArtifactSources = vi.fn<(id: string) => Promise<Artifact>>();
const sendMessage = vi.fn<(agentId: string, text: string) => Promise<string>>(async () => "run");
const pinArtifact = vi.fn<(id: string, width: WidgetWidth) => Promise<Settings>>();
const unpinArtifact = vi.fn<(id: string) => Promise<Settings>>();

vi.mock("../lib/ipc", () => ({
  api: {
    artifacts: (groupId?: string | null) => artifacts(groupId),
    artifactDetail: (id: string) => artifactDetail(id),
    artifactPage: (id: string, version?: number | null) => artifactPage(id, version),
    restoreArtifact: (id: string, version: number) => restoreArtifact(id, version),
    handArtifact: (id: string, agentId: string) => handArtifact(id, agentId),
    deleteArtifact: (id: string) => deleteArtifact(id),
    artifactData: (id: string, version?: number | null) => artifactData(id, version),
    allowArtifactSources: (id: string) => allowArtifactSources(id),
    sendMessage: (agentId: string, text: string) => sendMessage(agentId, text),
    pinArtifact: (id: string, width: WidgetWidth) => pinArtifact(id, width),
    unpinArtifact: (id: string) => unpinArtifact(id),
    frameArtifact: async () => ({ port: 1, id: "digest", ticket: null }),
  },
}));

const { ArtifactCard, Artifacts } = await import("./Artifacts");
const { useStore } = await import("../lib/store");

const OTHER_CREW = "00000000-0000-4000-8000-000000000002";

function agent(id: string, name: string, groupId = DEFAULT_GROUP): AgentCard {
  return {
    id,
    groupId,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    hasTerminal: false,
    harness: "pi",
    gate: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
  };
}

function kept(over: Partial<Artifact> = {}): Artifact {
  return {
    id: "artifact-1",
    groupId: DEFAULT_GROUP,
    owner: { id: "rae", name: "Rae", gone: false },
    title: "Pipeline by stage",
    version: 2,
    editedBy: { kind: "agent", id: "milo", name: "Milo" },
    createdAt: 0,
    updatedAt: 0,
    sources: [],
    sourcesAllowed: false,
    condensed: false,
    ...over,
  };
}

const detail: ArtifactDetail = {
  artifact: kept(),
  log: [
    {
      seq: 1,
      at: 0,
      change: "created",
      by: { kind: "agent", id: "rae", name: "Rae" },
      version: 1,
      owner: "Rae",
      note: "First cut",
    },
    {
      seq: 2,
      at: 1,
      change: "edited",
      by: { kind: "agent", id: "milo", name: "Milo" },
      version: 2,
      owner: "Rae",
      note: "Added a Q4 column",
    },
  ],
};

const onClose = vi.fn();

function open(railGroup: string | null, list: Artifact[] = [kept()]) {
  artifacts.mockResolvedValue(list);
  useStore.setState({
    railGroup,
    artifactsOpen: { id: null },
    artifactsVersion: 0,
    activity: {},
    agents: [agent("rae", "Rae"), agent("milo", "Milo"), agent("juno", "Juno", OTHER_CREW)],
    groups: [aGroup({ name: "Sales" }), aGroup({ id: OTHER_CREW, name: "Ops" })],
    settings: { widgets: [] } as unknown as Settings,
  });
  return render(<Artifacts onClose={onClose} />);
}

beforeEach(() => {
  vi.clearAllMocks();
  artifactDetail.mockResolvedValue(detail);
  artifactPage.mockResolvedValue("<p>page</p>");
});

describe("which artifacts it lists", () => {
  it("lists only the crew the rail is inside, and does not repeat the crew on every row", async () => {
    open(DEFAULT_GROUP);
    expect(await screen.findByText("Pipeline by stage")).toBeTruthy();
    expect(artifacts).toHaveBeenCalledWith(DEFAULT_GROUP);
    expect(screen.getByText(/^Owner Rae · v2 · /)).toBeTruthy();
  });

  it("lists every crew's while the rail shows every crew, and says whose each is", async () => {
    open(null);
    expect(await screen.findByText("Pipeline by stage")).toBeTruthy();
    expect(artifacts).toHaveBeenCalledWith(null);
    expect(screen.getByText(/^Sales · Owner Rae · v2 · /)).toBeTruthy();
    expect(screen.getByText("Every crew")).toBeTruthy();
  });

  it("says an owner who left is still the owner, and that they left", async () => {
    open(DEFAULT_GROUP, [kept({ owner: { id: "rae", name: "Rae", gone: true } })]);
    expect(await screen.findByText(/Owner Rae, left the crew/)).toBeTruthy();
  });
});

describe("the status bar, from the page", () => {
  async function openOne(artifact: Artifact) {
    artifactDetail.mockResolvedValue({ ...detail, artifact });
    open(DEFAULT_GROUP, [artifact]);
    fireEvent.click(await screen.findByRole("button", { name: /Pipeline by stage/ }));
    await waitFor(() => expect(artifactPage).toHaveBeenCalled());
  }

  it("puts a page with a condensed view on the bar at a width, and takes it off", async () => {
    await openOne(kept({ condensed: true }));
    const bar = await screen.findByRole("combobox", { name: "Status bar" });
    expect((bar as HTMLSelectElement).value).toBe("");
    const pinned = {
      widgets: [
        { artifactId: "artifact-1", width: "wide", everyMinutes: 5, addedBy: "the operator" },
      ],
    } as unknown as Settings;
    pinArtifact.mockResolvedValue(pinned);
    fireEvent.change(bar, { target: { value: "wide" } });
    await waitFor(() => expect(pinArtifact).toHaveBeenCalledWith("artifact-1", "wide"));
    await waitFor(() => expect((bar as HTMLSelectElement).value).toBe("wide"));

    unpinArtifact.mockResolvedValue({ widgets: [] } as unknown as Settings);
    fireEvent.change(bar, { target: { value: "" } });
    await waitFor(() => expect(unpinArtifact).toHaveBeenCalledWith("artifact-1"));
  });

  it("offers nothing for a page with no condensed view", async () => {
    await openOne(kept());
    expect(screen.queryByRole("combobox", { name: "Status bar" })).toBeNull();
  });

  it("can only take a pinned page off once its version has no condensed view", async () => {
    // A restore can put back a version from before the strip existed. The pin
    // is still the operator's to remove; it cannot be resized onto nothing.
    await openOne(kept());
    act(() =>
      useStore.setState({
        settings: {
          widgets: [{ artifactId: "artifact-1", width: "narrow", everyMinutes: 5, addedBy: "Rae" }],
        } as unknown as Settings,
      }),
    );
    const bar = await screen.findByRole("combobox", { name: "Status bar" });
    const options = [...bar.querySelectorAll("option")];
    expect(options.map((option) => option.disabled)).toEqual([false, true, true]);
  });
});

describe("one artifact", () => {
  async function openOne() {
    open(DEFAULT_GROUP);
    fireEvent.click(await screen.findByRole("button", { name: /Pipeline by stage/ }));
    await waitFor(() => expect(artifactPage).toHaveBeenCalledWith("artifact-1", 2));
  }

  async function openHistory() {
    fireEvent.click(await screen.findByRole("button", { name: "History" }));
    await screen.findByText(/Milo edited it/);
  }

  it("opens on the current page, with the history kept out of the way", async () => {
    // The page is what the operator came for. The history is there for working
    // out what happened, one click away, and takes no room until then.
    await openOne();
    expect(artifactDetail).toHaveBeenCalledWith("artifact-1");
    expect(screen.queryByText(/Milo edited it/)).toBeNull();
    expect(screen.queryByRole("combobox", { name: "Owner" })).toBeNull();
    expect(screen.queryByText("Put this version back")).toBeNull();
  });

  it("is the reading view, with no second view to open the page into", async () => {
    // The dialog framed the page the way a message does, capped and with an
    // Open over it. Pressed, that opened a second modal inside this one,
    // trapped in the dialog's box by the transform its arrival leaves on it:
    // a strip of the page, a window's height too short, scrolling sideways.
    await openOne();
    const frame = await screen.findByTitle("Pipeline by stage");
    expect(screen.queryByRole("button", { name: "Open" })).toBeNull();
    expect((frame as HTMLIFrameElement).style.height).toBe("");
  });

  it("opens an earlier version from the history and puts it back as a new one", async () => {
    await openOne();
    await openHistory();
    fireEvent.click(screen.getByRole("button", { name: "v1" }));
    await waitFor(() => expect(artifactPage).toHaveBeenCalledWith("artifact-1", 1));
    restoreArtifact.mockResolvedValue(kept({ version: 3 }));
    fireEvent.click(await screen.findByRole("button", { name: "Put this version back" }));
    await waitFor(() => expect(restoreArtifact).toHaveBeenCalledWith("artifact-1", 1));
  });

  it("hands ownership only to the artifact's own crew", async () => {
    await openOne();
    await openHistory();
    const owner = screen.getByRole("combobox", { name: "Owner" });
    const names = [...owner.querySelectorAll("option")].map((option) => option.textContent);
    expect(names).toEqual(["Rae", "Milo"]);
    handArtifact.mockResolvedValue(kept({ owner: { id: "milo", name: "Milo", gone: false } }));
    fireEvent.change(owner, { target: { value: "milo" } });
    await waitFor(() => expect(handArtifact).toHaveBeenCalledWith("artifact-1", "milo"));
  });

  it("backs out of the history before it backs out of the artifact", async () => {
    await openOne();
    await openHistory();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByText(/Milo edited it/)).toBeNull());
    expect(useStore.getState().artifactsOpen).toEqual({ id: "artifact-1" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("asks before deleting an artifact and its history", async () => {
    await openOne();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(deleteArtifact).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Delete it and its history" }));
    await waitFor(() => expect(deleteArtifact).toHaveBeenCalledWith("artifact-1"));
  });
});

describe("a page that reads live data", () => {
  const board = kept({
    sources: [{ name: "issues", tool: "linear__list_issues", arguments: { team: "ENG" } }],
  });

  it("asks for its reads in full, and allows exactly those", async () => {
    // A permission, so every call and its arguments are on screen. The
    // arguments are the part the page can never change.
    artifactDetail.mockResolvedValue({ ...detail, artifact: board });
    allowArtifactSources.mockResolvedValue({ ...board, sourcesAllowed: true });
    open(DEFAULT_GROUP, [board]);
    fireEvent.click(await screen.findByRole("button", { name: /Pipeline by stage/ }));
    expect(await screen.findByText('linear · list_issues {"team":"ENG"}')).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Allow these reads" }));
    await waitFor(() => expect(allowArtifactSources).toHaveBeenCalledWith("artifact-1"));
  });

  it("reads the version on screen, and reads again on Refresh", async () => {
    const allowed = { ...board, sourcesAllowed: true };
    artifactDetail.mockResolvedValue({ ...detail, artifact: allowed });
    open(DEFAULT_GROUP, [allowed]);
    fireEvent.click(await screen.findByRole("button", { name: /Pipeline by stage/ }));
    await waitFor(() => expect(artifactData).toHaveBeenCalledWith("artifact-1", 2));
    expect(screen.queryByText("Allow these reads")).toBeNull();
    const before = artifactData.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(artifactData.mock.calls.length).toBe(before + 1));
  });
});

describe("a click on a kept page", () => {
  /** Posts what a page's `guaca.send` posts, from the page's own window. */
  async function pageSends(value: unknown) {
    const frame = (await screen.findByTitle("Pipeline by stage")) as HTMLIFrameElement;
    act(() => {
      window.dispatchEvent(
        new MessageEvent("message", {
          source: frame.contentWindow,
          data: { guaca: "artifact-send", value: JSON.stringify(value) },
        }),
      );
    });
    return frame;
  }

  async function openOne() {
    open(DEFAULT_GROUP);
    fireEvent.click(await screen.findByRole("button", { name: /Pipeline by stage/ }));
    await screen.findByTitle("Pipeline by stage");
  }

  it("reaches the owner as the operator's message, naming the page", async () => {
    await openOne();
    const frame = await screen.findByTitle("Pipeline by stage");
    act(() => frame.focus());
    await pageSends({ move: "Hooli", to: "Won" });
    await waitFor(() => expect(sendMessage).toHaveBeenCalled());
    const [to, text] = sendMessage.mock.calls[0]!;
    expect(to).toBe("rae");
    expect(text).toContain('"Pipeline by stage" (artifact artifact-1, version 2)');
    expect(text).toContain('{"move":"Hooli","to":"Won"}');
    expect(await screen.findByText(/Sent to Rae/)).toBeTruthy();
  });

  it("sends nothing the operator did not click", async () => {
    // Focus is somewhere else: this is a page sending by itself.
    await openOne();
    await pageSends({ move: "Hooli" });
    expect(await screen.findByText(/without a click, so nothing was sent/)).toBeTruthy();
    expect(sendMessage).not.toHaveBeenCalled();
  });

  it("sends one at a time, and not while the owner is still working", async () => {
    await openOne();
    useStore.setState({ activity: { rae: { state: "thinking" } } });
    const frame = await screen.findByTitle("Pipeline by stage");
    act(() => frame.focus());
    await pageSends({ move: "Hooli" });
    expect(await screen.findByText(/Rae is still working/)).toBeTruthy();
    expect(sendMessage).not.toHaveBeenCalled();
  });
});

describe("the card a turn leaves", () => {
  it("opens the dialog on the artifact it names", () => {
    useStore.setState({ artifactsOpen: null });
    render(<ArtifactCard made={{ id: "artifact-9", version: 4, title: "Hiring plan" }} />);
    fireEvent.click(screen.getByRole("button", { name: /Hiring plan/ }));
    expect(useStore.getState().artifactsOpen).toEqual({ id: "artifact-9" });
    expect(screen.getByText("Artifact, version 4")).toBeTruthy();
  });
});
