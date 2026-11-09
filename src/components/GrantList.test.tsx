import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AgentCard } from "../lib/types";
import { GrantList } from "./GrantList";

const api = vi.hoisted(() => ({ agentGrants: vi.fn(), revokeGrant: vi.fn() }));
vi.mock("../lib/ipc", () => ({ api }));

beforeEach(() => {
  vi.resetAllMocks();
  api.agentGrants.mockResolvedValue(["createAgent", "actOnBehalf"]);
});

it("blocks concurrent revocations and draws the returned grants", async () => {
  let finish!: (grants: string[]) => void;
  api.revokeGrant.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  render(<GrantList agent={{ id: "a1" } as AgentCard} />);
  const buttons = await screen.findAllByRole("button", { name: "Ask me again" });
  fireEvent.click(buttons[0]!);
  expect((screen.getByRole("button", { name: "Removing…" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  expect((buttons[1] as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(buttons[1]!);
  expect(api.revokeGrant).toHaveBeenCalledTimes(1);
  finish(["actOnBehalf"]);
  await waitFor(() =>
    expect(screen.queryByText("Adds agents to this group without asking.")).toBeNull(),
  );
  expect(screen.getByText("Acts outside Guaca in your name without asking.")).toBeTruthy();
});

it("keeps grants and shows revocation failures", async () => {
  api.revokeGrant.mockRejectedValue(new Error("offline"));
  render(<GrantList agent={{ id: "a1" } as AgentCard} />);
  fireEvent.click((await screen.findAllByRole("button", { name: "Ask me again" }))[0]!);
  expect(await screen.findByText("offline")).toBeTruthy();
  expect(screen.getAllByRole("button", { name: "Ask me again" })).toHaveLength(2);
});
