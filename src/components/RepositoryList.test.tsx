import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AgentCard,
  Bench,
  Gate,
  Harness,
  HarnessOnMachine,
  Repository,
  RepositoryDraft,
} from "../lib/types";
import { RepositoryList } from "./RepositoryList";

const groupRepositories = vi.fn<(groupId: string) => Promise<Repository[]>>();
const createRepository = vi.fn<(draft: RepositoryDraft) => Promise<Repository>>();
const updateRepository = vi.fn();
const deleteRepository = vi.fn();
const setAgentRepository = vi.fn();
const codingHarnesses = vi.fn<() => Promise<HarnessOnMachine[]>>();

vi.mock("../lib/ipc", () => ({
  api: {
    groupRepositories: (groupId: string) => groupRepositories(groupId),
    createRepository: (draft: RepositoryDraft) => createRepository(draft),
    updateRepository: (
      id: string,
      name: string,
      note: string,
      harness: Harness,
      gate: Gate,
      bench: Bench,
    ) => updateRepository(id, name, note, harness, gate, bench),
    deleteRepository: (id: string) => deleteRepository(id),
    setAgentRepository: vi.fn(),
    codingHarnesses: () => codingHarnesses(),
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
    browserConsent: "open",
    repositoryId: null,
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
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
    harness: "pi",
    gate: "open",
    bench: "own",
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
    codingHarnesses.mockReset();
    groupRepositories.mockResolvedValue([]);
    codingHarnesses.mockResolvedValue([
      {
        harness: "pi",
        installed: true,
        version: "0.9.0",
        bridged: false,
        install: "npm install -g pi",
      },
      {
        harness: "claude",
        installed: true,
        version: "2.1.247 (Claude Code)",
        bridged: true,
        install: "npm install -g @anthropic-ai/claude-code",
      },
    ]);
  });

  it("offers no way to make every agent an engineer", async () => {
    // The refusal the whole feature is built on. Plugins have an "Every agent"
    // button because a crew's Linear account is usually the crew's; a working
    // tree is not, and an agent hired next week must not inherit one. If a
    // control like that ever appears, it appears with this test failing.
    render(<RepositoryList groupId={GROUP} crew={CREW} />);
    groupRepositories.mockResolvedValue([repository()]);

    await screen.findByText("Link a repository");
    expect(screen.queryByText("Every agent")).toBeNull();
    expect(screen.queryByText(/engineer/i)).toBeNull();
    expect(screen.queryByText(/specialist/i)).toBeNull();
  });

  it("says a newly linked repository is worked in by nobody", async () => {
    // Linking and handing out are two decisions. An operator who links one and
    // walks away has given their source to no agent, and the panel has to say
    // so rather than leaving an empty row that reads as unfinished loading.
    groupRepositories.mockResolvedValue([repository()]);
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/Worked in by nobody yet\./)).toBeTruthy();
  });

  it("names who works in one without offering to change it here", async () => {
    // The read stays, because auditing is a real question and answering it
    // should not mean opening six agents. The control does not, because the
    // operator asking "what can Ada work on" is on Ada's panel, not this one.
    groupRepositories.mockResolvedValue([repository()]);
    render(
      <RepositoryList groupId={GROUP} crew={[{ ...CREW[0]!, repositoryId: "r1" }, CREW[1]!]} />,
    );

    expect(await screen.findByText(/Worked in by Ada/)).toBeTruthy();
    expect(screen.queryByText("Grace")).toBeNull();
    expect(setAgentRepository).not.toHaveBeenCalled();
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
        harness: "pi",
        gate: "open",
        bench: "own",
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
      expect(updateRepository).toHaveBeenCalledWith(
        "r1",
        "guaca",
        "run ./scripts/ci.sh",
        "pi",
        "open",
        "own",
      ),
    );
  });

  it("changes which program writes the code on the click, not on a later Save", async () => {
    // The day this exists for: one plan is spent, and the operator's way out is
    // the other program rather than a setting on the one that stopped paying.
    //
    // On the click, because a `.choice` means that everywhere else in this app.
    // Staged, it sits under a Save button an operator has every reason to press
    // before they reach it, and the change is lost with nothing saying so.
    groupRepositories.mockResolvedValue([repository({ note: "never touch migrations" })]);
    updateRepository.mockResolvedValue(repository({ harness: "claude" }));
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Edit"));
    fireEvent.click(screen.getByRole("button", { name: "Coding harness: Claude Code" }));

    await waitFor(() =>
      expect(updateRepository).toHaveBeenCalledWith(
        "r1",
        "guaca",
        "never touch migrations",
        "claude",
        "open",
        "own",
      ),
    );
  });

  it("does not save a half-typed rename along with the harness", async () => {
    // The click is a decision about the program, and nothing else. A name the
    // operator is in the middle of typing is not something they asked to store,
    // and Save is still the gesture that stores it.
    groupRepositories.mockResolvedValue([repository()]);
    updateRepository.mockResolvedValue(repository({ harness: "claude" }));
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Edit"));
    fireEvent.change(screen.getByDisplayValue("guaca"), { target: { value: "guac" } });
    fireEvent.click(screen.getByRole("button", { name: "Coding harness: Claude Code" }));

    await waitFor(() =>
      expect(updateRepository).toHaveBeenCalledWith("r1", "guaca", "", "claude", "open", "own"),
    );
  });

  it("says which program a repository runs without opening it", async () => {
    // Visible on the row, because the question "which of these is on the plan
    // that still works" is asked about the list rather than about one row.
    groupRepositories.mockResolvedValue([repository({ harness: "claude" })]);
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/written by Claude Code/)).toBeTruthy();
  });

  it("links a repository with the harness that was chosen", async () => {
    groupRepositories.mockResolvedValue([]);
    createRepository.mockResolvedValue(repository({ harness: "claude" }));
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Link a repository"));
    fireEvent.change(screen.getByPlaceholderText("/Users/you/dev/your-project"), {
      target: { value: "/Users/you/dev/guaca" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Coding harness: Claude Code" }));
    fireEvent.click(screen.getByText("Link"));

    await waitFor(() =>
      expect(createRepository).toHaveBeenCalledWith({
        groupId: GROUP,
        name: "",
        path: "/Users/you/dev/guaca",
        note: "",
        harness: "claude",
        gate: "open",
        bench: "own",
      }),
    );
  });

  it("saves the gate on the stored name and note, not on a half-typed rename", async () => {
    // The same rule the harness above follows. A click on a tick is not the
    // gesture that saves a rename somebody is in the middle of typing.
    groupRepositories.mockResolvedValue([repository()]);
    updateRepository.mockResolvedValue(repository({ gate: "askBeforePushing" }));
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Edit"));
    fireEvent.change(screen.getByPlaceholderText("what you call it"), {
      target: { value: "half-typed" },
    });
    fireEvent.click(screen.getByRole("checkbox"));

    await waitFor(() =>
      expect(updateRepository).toHaveBeenCalledWith(
        "r1",
        "guaca",
        "",
        "pi",
        "askBeforePushing",
        "own",
      ),
    );
  });

  it("does not offer the gate on a harness that cannot be reached while it works", async () => {
    // A control that silently does nothing is worse than one that is not
    // offered, and the hint has to say which harness cannot take it.
    codingHarnesses.mockResolvedValue([
      {
        harness: "pi",
        installed: true,
        version: "0.9.0",
        bridged: false,
        install: "npm install -g pi",
      },
      {
        harness: "claude",
        installed: true,
        version: "2.1.247 (Claude Code)",
        bridged: true,
        install: "npm install -g @anthropic-ai/claude-code",
      },
    ]);
    groupRepositories.mockResolvedValue([repository()]);
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Edit"));
    await waitFor(() =>
      expect((screen.getByRole("checkbox") as HTMLInputElement).disabled).toBe(true),
    );
    expect(screen.getByText(/pi cannot be reached while it works/)).toBeTruthy();
  });

  it("offers a harness that is not installed, disabled, with the command that installs it", async () => {
    // Not hidden. The state this control exists for is a plan that has just run
    // out, and an absent option reads as a thing the app cannot do. Not enabled
    // either: the only symptom of storing it would be a coding job that never
    // starts, reported to an agent forty minutes later.
    codingHarnesses.mockResolvedValue([
      {
        harness: "pi",
        installed: true,
        version: "0.9.0",
        bridged: false,
        install: "npm install -g pi",
      },
      {
        harness: "claude",
        installed: false,
        version: "",
        bridged: false,
        install: "npm install -g @anthropic-ai/claude-code",
      },
    ]);
    groupRepositories.mockResolvedValue([repository()]);
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Edit"));

    await waitFor(() =>
      expect(
        screen
          .getByRole("button", { name: "Coding harness: Claude Code" })
          .hasAttribute("disabled"),
      ).toBe(true),
    );
    expect(screen.getByText("npm install -g @anthropic-ai/claude-code")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Coding harness: pi" }).hasAttribute("disabled"),
    ).toBe(false);
  });

  it("disables neither when the machine could not be asked", async () => {
    // A check that could not run must not refuse to save the thing the operator
    // can see working in their own terminal. A job's own refusal already names
    // the install command.
    codingHarnesses.mockRejectedValue(new Error("no"));
    groupRepositories.mockResolvedValue([repository()]);
    render(<RepositoryList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Edit"));

    await waitFor(() =>
      expect(
        screen
          .getByRole("button", { name: "Coding harness: Claude Code" })
          .hasAttribute("disabled"),
      ).toBe(false),
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
});
