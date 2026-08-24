import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Repository, RepositoryDraft } from "../lib/types";
import { RepositoryList } from "./RepositoryList";

const groupRepositories = vi.fn<(groupId: string) => Promise<Repository[]>>();
const createRepository = vi.fn<(draft: RepositoryDraft) => Promise<Repository>>();
const updateRepository = vi.fn();
const deleteRepository = vi.fn();
const setRepositoryAccess = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    groupRepositories: (groupId: string) => groupRepositories(groupId),
    createRepository: (draft: RepositoryDraft) => createRepository(draft),
    updateRepository: (id: string, name: string, note: string) => updateRepository(id, name, note),
    deleteRepository: (id: string) => deleteRepository(id),
    setRepositoryAccess: (id: string, agentId: string, allowed: boolean) =>
      setRepositoryAccess(id, agentId, allowed),
  },
}));

const GROUP = "00000000-0000-4000-8000-000000000001";

function member(id: string, name: string): AgentCard {
  return {
    id,
    groupId: GROUP,
    name,
    avatar: "avocado",
    color: "#7fb069",
    model: "",
    systemPrompt: "",
    skills: [],
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
  };
}

const CREW = [member("a1", "Ada"), member("a2", "Grace")];

function repository(over: Partial<Repository> = {}): Repository {
  return {
    id: "r1",
    groupId: GROUP,
    name: "guaca",
    path: "/Users/you/dev/guaca",
    note: "",
    reach: [],
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("RepositoryList", () => {
  beforeEach(() => {
    groupRepositories.mockReset();
    createRepository.mockReset();
    updateRepository.mockReset();
    deleteRepository.mockReset();
    setRepositoryAccess.mockReset();
    groupRepositories.mockResolvedValue([]);
  });

  it("offers no way to make every agent an engineer", async () => {
    // The refusal the whole panel is built on. Plugins have an "Every agent"
    // button because a crew's Linear account is usually the crew's; a working
    // tree is not, and an agent hired next week must not inherit one. If a
    // control like that ever appears here, it appears with this test failing.
    render(<RepositoryList groupId={GROUP} crew={CREW} />);
    groupRepositories.mockResolvedValue([repository()]);

    await screen.findByText("Link a repository");
    expect(screen.queryByText("Every agent")).toBeNull();
    expect(screen.queryByText(/engineer/i)).toBeNull();
    expect(screen.queryByText(/specialist/i)).toBeNull();
  });

  it("says a newly linked repository is handed to nobody", async () => {
    // Linking and handing out are two decisions. An operator who links one and
    // walks away has given their source to no agent, and the panel has to say
    // so rather than leaving an empty row that reads as unfinished loading.
    groupRepositories.mockResolvedValue([repository()]);
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("Handed to nobody yet.")).toBeTruthy();
  });

  it("hands one to a single agent, by name", async () => {
    groupRepositories.mockResolvedValue([repository()]);
    setRepositoryAccess.mockResolvedValue(repository({ reach: ["a1"] }));
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Ada"));

    // The change, not the list the panel believes in: a panel one tick behind
    // must not be able to revoke Grace while granting Ada.
    await waitFor(() => expect(setRepositoryAccess).toHaveBeenCalledWith("r1", "a1", true));
    expect(setRepositoryAccess).toHaveBeenCalledTimes(1);
  });

  it("takes one back from an agent that has it", async () => {
    groupRepositories.mockResolvedValue([repository({ reach: ["a1"] })]);
    setRepositoryAccess.mockResolvedValue(repository());
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    const ada = await screen.findByText("Ada");
    expect(ada.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(ada);

    await waitFor(() => expect(setRepositoryAccess).toHaveBeenCalledWith("r1", "a1", false));
  });

  it("sends the path as typed and lets the backend say whether it is a repository", async () => {
    // Nothing here guesses. Whether a directory exists and holds a git work
    // tree is a question only the disk can answer, and answering it in the
    // webview would be a second opinion that disagrees on somebody's machine.
    createRepository.mockResolvedValue(repository());
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Link a repository"));
    fireEvent.change(screen.getByPlaceholderText("/Users/you/dev/your-project"), {
      target: { value: "/Users/you/dev/guaca" },
    });
    fireEvent.click(screen.getByText("Link"));

    await waitFor(() =>
      expect(createRepository).toHaveBeenCalledWith({
        groupId: GROUP,
        name: "",
        path: "/Users/you/dev/guaca",
        note: "",
      }),
    );
  });

  it("shows the backend's refusal rather than a generic failure", async () => {
    // These refusals are the fix: they name the directory to link instead, or
    // the command to run in it. Swallowing them for a tidy message would cost
    // the operator the only useful sentence.
    createRepository.mockRejectedValue(
      new Error("`/Users/you/dev` is not a git repository. run `git init` in it first"),
    );
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Link a repository"));
    fireEvent.change(screen.getByPlaceholderText("/Users/you/dev/your-project"), {
      target: { value: "/Users/you/dev" },
    });
    fireEvent.click(screen.getByText("Link"));

    expect(await screen.findByText(/git init/)).toBeTruthy();
  });

  it("will not offer to edit the path, and says why where the boxes are", async () => {
    groupRepositories.mockResolvedValue([repository()]);
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Edit"));

    expect(screen.getByDisplayValue("guaca")).toBeTruthy();
    expect(screen.queryByDisplayValue("/Users/you/dev/guaca")).toBeNull();
    expect(screen.getByText(/path is not editable/)).toBeTruthy();
  });

  it("rewrites the line its agents read", async () => {
    groupRepositories.mockResolvedValue([repository()]);
    updateRepository.mockResolvedValue(repository({ note: "run ./scripts/ci.sh" }));
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Edit"));
    fireEvent.change(screen.getByPlaceholderText("run ./scripts/ci.sh before you finish"), {
      target: { value: "run ./scripts/ci.sh" },
    });
    fireEvent.click(screen.getByText("Save"));

    await waitFor(() =>
      expect(updateRepository).toHaveBeenCalledWith("r1", "guaca", "run ./scripts/ci.sh"),
    );
  });

  it("unlinks without claiming to delete anything", async () => {
    groupRepositories.mockResolvedValue([repository()]);
    deleteRepository.mockResolvedValue(undefined);
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    // The word matters: the button sits next to a path on the operator's own
    // disk, and "Delete" beside a path is a promise about their files.
    fireEvent.click(await screen.findByText("Unlink"));
    await waitFor(() => expect(deleteRepository).toHaveBeenCalledWith("r1"));
  });

  it("says a crew with no agents has nobody to hand one to", async () => {
    groupRepositories.mockResolvedValue([repository()]);
    render(<RepositoryList groupId={GROUP} crew={[]} />);

    expect(await screen.findByText("This group has no agents yet.")).toBeTruthy();
  });
});
