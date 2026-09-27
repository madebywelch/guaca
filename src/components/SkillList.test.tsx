import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { Skill, SkillDraft, SkillScope } from "../lib/types";
import { SkillList } from "./SkillList";

const listSkills = vi.fn<(scope: SkillScope) => Promise<Skill[]>>();
const readSkill = vi.fn<(scope: SkillScope, name: string) => Promise<Skill>>();
const saveSkill = vi.fn<(scope: SkillScope, draft: SkillDraft) => Promise<Skill>>();
const deleteSkill = vi.fn<(scope: SkillScope, name: string) => Promise<boolean>>();

vi.mock("../lib/ipc", () => ({
  api: {
    listSkills: (scope: SkillScope) => listSkills(scope),
    readSkill: (scope: SkillScope, name: string) => readSkill(scope, name),
    saveSkill: (scope: SkillScope, draft: SkillDraft) => saveSkill(scope, draft),
    deleteSkill: (scope: SkillScope, name: string) => deleteSkill(scope, name),
  },
}));

const GROUP = "00000000-0000-4000-8000-000000000001";
const CREW: SkillScope = { kind: "crew", groupId: GROUP };

function skill(name: string, scope: SkillScope, body = ""): Skill {
  return { name, description: `when ${name}`, scope, body, updatedAt: 1 };
}

describe("SkillList", () => {
  beforeEach(() => {
    for (const mock of [listSkills, readSkill, saveSkill, deleteSkill]) mock.mockReset();
    listSkills.mockImplementation(async (scope) =>
      scope.kind === "bundled" ? [skill("guaca", { kind: "bundled" })] : [],
    );
    useStore.setState({ skillsVersion: {} });
  });

  it("shows Guaca's own to the operator, read-only, and never inside a crew", async () => {
    const { unmount } = render(<SkillList scope={{ kind: "workspace" }} />);
    await screen.findByText("guaca");
    expect(screen.getByText("Guaca's own")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    readSkill.mockResolvedValue(skill("guaca", { kind: "bundled" }, "# Guaca\nThe manual."));
    fireEvent.click(screen.getByRole("button", { name: "Read" }));
    await screen.findByText(/The manual\./);
    unmount();
    listSkills.mockClear();

    render(<SkillList scope={CREW} />);
    await screen.findByText("This crew has no skills yet.");
    expect(screen.queryByText("guaca")).toBeNull();
    expect(listSkills.mock.calls.map(([scope]) => scope.kind)).toEqual(["crew"]);
  });

  it("writes a skill into the scope it was opened on", async () => {
    saveSkill.mockImplementation(async (scope, draft) => skill(draft.name, scope));
    render(<SkillList scope={CREW} />);
    fireEvent.click(await screen.findByRole("button", { name: "Write a skill" }));
    const save = screen.getByRole("button", { name: "Save skill" });
    expect(save.hasAttribute("disabled")).toBe(true);
    fireEvent.change(screen.getByLabelText(/^Name/), { target: { value: "deploy" } });
    fireEvent.change(screen.getByLabelText(/^When to use it/), {
      target: { value: "When deploying" },
    });
    fireEvent.change(screen.getByLabelText(/^Instructions/), { target: { value: "# Deploy" } });
    fireEvent.click(save);
    await waitFor(() => expect(saveSkill).toHaveBeenCalledTimes(1));
    expect(saveSkill).toHaveBeenCalledWith(CREW, {
      name: "deploy",
      description: "When deploying",
      body: "# Deploy",
    });
  });

  it("renames by sending the old name with the new one", async () => {
    listSkills.mockImplementation(async (scope) =>
      scope.kind === "crew" ? [skill("deploy", CREW)] : [],
    );
    readSkill.mockResolvedValue(skill("deploy", CREW, "# Deploy"));
    saveSkill.mockImplementation(async (scope, draft) => skill(draft.name, scope));
    render(<SkillList scope={CREW} />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    const name = await screen.findByLabelText(/^Name/);
    fireEvent.change(name, { target: { value: "deploy-site" } });
    fireEvent.click(screen.getByRole("button", { name: "Save skill" }));
    await waitFor(() => expect(saveSkill).toHaveBeenCalledTimes(1));
    expect(saveSkill.mock.calls[0]![1]).toMatchObject({ name: "deploy-site", previous: "deploy" });
  });

  it("redraws when an agent writes one mid-turn", async () => {
    render(<SkillList scope={CREW} />);
    await screen.findByText("This crew has no skills yet.");
    listSkills.mockImplementation(async (scope) =>
      scope.kind === "crew" ? [skill("release-notes", CREW)] : [],
    );
    act(() => useStore.getState().applyEvent({ type: "skillsChanged", scope: CREW }));
    await screen.findByText("release-notes");
  });

  it("says why a save was refused and keeps what was typed", async () => {
    saveSkill.mockRejectedValue({ kind: "validation", message: "`guaca` is Guaca's own skill" });
    render(<SkillList scope={{ kind: "workspace" }} />);
    fireEvent.click(await screen.findByRole("button", { name: "Write a skill" }));
    fireEvent.change(screen.getByLabelText(/^Name/), { target: { value: "guaca" } });
    fireEvent.change(screen.getByLabelText(/^When to use it/), { target: { value: "x" } });
    fireEvent.change(screen.getByLabelText(/^Instructions/), { target: { value: "y" } });
    fireEvent.click(screen.getByRole("button", { name: "Save skill" }));
    await screen.findByRole("alert");
    expect((screen.getByLabelText(/^Name/) as HTMLInputElement).value).toBe("guaca");
  });
});
