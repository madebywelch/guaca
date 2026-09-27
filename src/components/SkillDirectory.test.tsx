import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  DirectoryBoard,
  DirectoryListing,
  DirectoryPage,
  DirectorySkill,
  Skill,
  SkillScope,
} from "../lib/types";
import { SkillDirectory } from "./SkillDirectory";

const skillDirectory = vi.fn<(board: DirectoryBoard, page: number) => Promise<DirectoryPage>>();
const searchSkillDirectory = vi.fn<(query: string) => Promise<DirectoryListing[]>>();
const previewDirectorySkill = vi.fn<(id: string) => Promise<DirectorySkill>>();
const addDirectorySkill = vi.fn<(scope: SkillScope, id: string, hash: string) => Promise<Skill>>();
const openExternal = vi.fn<(url: string) => Promise<void>>();

vi.mock("../lib/ipc", () => ({
  api: {
    skillDirectory: (board: DirectoryBoard, page: number) => skillDirectory(board, page),
    searchSkillDirectory: (query: string) => searchSkillDirectory(query),
    previewDirectorySkill: (id: string) => previewDirectorySkill(id),
    addDirectorySkill: (scope: SkillScope, id: string, hash: string) =>
      addDirectorySkill(scope, id, hash),
  },
  openExternal: (url: string) => openExternal(url),
}));

function listing(id: string, installs: number, addable = true): DirectoryListing {
  const parts = id.split("/");
  const slug = parts.at(-1) ?? id;
  return {
    id,
    source: parts.slice(0, -1).join("/"),
    slug,
    name: slug,
    installs,
    url: `https://skills.sh/${id}`,
    addable,
  };
}

function opened(id: string, audits: DirectorySkill["audits"] = []): DirectorySkill {
  const name = id.split("/").pop() ?? id;
  return {
    id,
    name,
    description: `Use when ${name} fits.`,
    body: `# ${name}\n\nTHE INSTRUCTIONS`,
    files: ["reference.md"],
    hash: `hash-of-${name}`,
    url: `https://skills.sh/${id}`,
    audits,
  };
}

const WORKSPACE: SkillScope = { kind: "workspace" };

describe("SkillDirectory", () => {
  beforeEach(() => {
    for (const mock of [
      skillDirectory,
      searchSkillDirectory,
      previewDirectorySkill,
      addDirectorySkill,
      openExternal,
    ]) {
      mock.mockReset();
    }
    skillDirectory.mockImplementation(async (board, page) => ({
      skills:
        board === "allTime" && page === 0
          ? [
              listing("anthropics/skills/pdf", 201834),
              listing("open.feishu.cn/lark-doc", 90000, false),
            ]
          : board === "allTime"
            ? [listing("someone/skills/notes", 1)]
            : [listing(`someone/skills/${board}`, 5)],
      page,
      hasMore: board === "allTime" && page === 0,
    }));
    openExternal.mockResolvedValue();
  });

  it("says what went wrong when skills.sh cannot be reached, and draws no list", async () => {
    skillDirectory.mockRejectedValue({
      kind: "directory",
      message: "skills.sh could not be reached (the request failed). Check the connection",
    });
    render(<SkillDirectory scope={WORKSPACE} taken={new Set()} onAdded={() => {}} />);
    expect((await screen.findByRole("alert")).textContent).toContain("could not be reached");
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("will not add over a skill already here, and says what to do instead", async () => {
    previewDirectorySkill.mockResolvedValue(opened("anthropics/skills/pdf"));
    render(<SkillDirectory scope={WORKSPACE} taken={new Set(["pdf"])} onAdded={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Read" }));
    await screen.findByText(/THE INSTRUCTIONS/);
    expect(screen.getByRole("button", { name: "Add to workspace" }).hasAttribute("disabled")).toBe(
      true,
    );
    expect(screen.getByText(/already here\. Delete or rename it first/)).toBeTruthy();
    expect(addDirectorySkill).not.toHaveBeenCalled();
  });

  it("offers a skill published by its own site to be opened, not added", async () => {
    render(<SkillDirectory scope={WORKSPACE} taken={new Set()} onAdded={() => {}} />);
    const row = (await screen.findByText("lark-doc")).closest("li");
    if (!row) throw new Error("no row");
    expect(within(row).queryByRole("button", { name: "Read" })).toBeNull();
    fireEvent.click(within(row).getByRole("button", { name: "On skills.sh" }));
    expect(openExternal).toHaveBeenCalledWith("https://skills.sh/open.feishu.cn/lark-doc");
  });

  it("tells an audit nobody ran from one that could not be read", async () => {
    previewDirectorySkill.mockResolvedValueOnce(opened("anthropics/skills/pdf", null));
    render(<SkillDirectory scope={WORKSPACE} taken={new Set()} onAdded={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Read" }));
    await screen.findByText("Its security audits could not be read.");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));

    previewDirectorySkill.mockResolvedValueOnce(opened("anthropics/skills/pdf", []));
    fireEvent.click(screen.getByRole("button", { name: "Read" }));
    await screen.findByText("No security audit has run on this skill yet.");
  });

  it("reads before adding, and adds exactly what was read to the scope it was opened on", async () => {
    const crew: SkillScope = { kind: "crew", groupId: "00000000-0000-4000-8000-000000000001" };
    previewDirectorySkill.mockResolvedValue(
      opened("anthropics/skills/pdf", [
        { provider: "Snyk", verdict: "warn", summary: "Runs local scripts" },
      ]),
    );
    const written: Skill = {
      name: "pdf",
      description: "d",
      scope: crew,
      body: "",
      updatedAt: 1,
      files: ["reference.md"],
      origin: "https://skills.sh/anthropics/skills/pdf",
    };
    addDirectorySkill.mockResolvedValue(written);
    const onAdded = vi.fn();
    render(<SkillDirectory scope={crew} taken={new Set()} onAdded={onAdded} />);

    expect(await screen.findByText("201.8K installs")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Read" }));
    expect((await screen.findByText(/Runs local scripts/)).className).toBe("access__warn");
    expect(screen.getByText(/reference\.md\. Agents can read them; Guaca never runs them/));
    fireEvent.click(screen.getByRole("button", { name: "Add to this crew" }));
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith(written));
    expect(addDirectorySkill).toHaveBeenCalledWith(crew, "anthropics/skills/pdf", "hash-of-pdf");
    expect(screen.getByRole("button", { name: "Added" }).hasAttribute("disabled")).toBe(true);
  });

  it("pages through a ranking, switches rankings, and searches instead of ranking", async () => {
    render(<SkillDirectory scope={WORKSPACE} taken={new Set()} onAdded={() => {}} />);
    await screen.findByText("pdf");
    fireEvent.click(screen.getByRole("button", { name: "Show more" }));
    await screen.findByText("1 install");
    expect(screen.getByText("pdf")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Show more" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Hot" }));
    await screen.findByText("hot");
    expect(screen.queryByText("pdf")).toBeNull();

    searchSkillDirectory.mockResolvedValue([listing("someone/skills/found", 3)]);
    const box = screen.getByRole("searchbox", { name: "Search skills.sh" });
    fireEvent.change(box, { target: { value: "  forms " } });
    fireEvent.submit(box);
    await screen.findByText("found");
    expect(searchSkillDirectory).toHaveBeenCalledWith("forms");
    expect(screen.getByText("Skills matching “forms”.")).toBeTruthy();
    for (const board of ["Most installed", "Trending", "Hot"]) {
      expect(screen.getByRole("button", { name: board }).getAttribute("aria-pressed")).toBe(
        "false",
      );
    }
  });

  it("draws only the ranking asked for last when two answers cross", async () => {
    let answerFirst: (page: DirectoryPage) => void = () => {};
    skillDirectory.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answerFirst = resolve;
        }),
    );
    render(<SkillDirectory scope={WORKSPACE} taken={new Set()} onAdded={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Trending" }));
    await screen.findByText("trending");
    answerFirst({ skills: [listing("late/skills/stale", 9)], page: 0, hasMore: false });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText("stale")).toBeNull();
    expect(screen.getByText("trending")).toBeTruthy();
  });
});
