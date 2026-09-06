import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../lib/ipc";
import type { SavedRepositoryCredential } from "../lib/types";
import { RepositoryToken, type TokenChoice } from "./RepositoryToken";

vi.mock("../lib/ipc", () => ({ api: { savedRepositoryCredentials: vi.fn() } }));
const saved = {
  id: "saved-id",
  remote: "https://forge.example/team/one.git",
  username: "engineer",
};
function Form({ remote = "https://forge.example/team/two.git" }: { remote?: string }) {
  const [choice, setChoice] = useState<TokenChoice>({ kind: "loading" });
  return (
    <RepositoryToken
      key={remote}
      remote={remote}
      choice={choice}
      onChange={setChoice}
      disabled={false}
    />
  );
}
beforeEach(() => vi.resetAllMocks());

it("reports a lookup failure and still allows a new token or existing Git access", async () => {
  vi.mocked(api.savedRepositoryCredentials).mockRejectedValue(new Error("Backend unavailable"));
  render(<Form />);
  expect((await screen.findByRole("alert")).textContent).toContain("Backend unavailable");
  expect(screen.getByLabelText("Access token")).toHaveProperty("value", "");
  fireEvent.change(screen.getByLabelText("Git credential"), { target: { value: "existing" } });
  expect(screen.queryByLabelText("Access token")).toBeNull();
});

it("ignores a late response from the previous server and clears a newly typed token", async () => {
  let resolve!: (value: SavedRepositoryCredential[]) => void;
  vi.mocked(api.savedRepositoryCredentials)
    .mockReturnValueOnce(
      new Promise((done) => {
        resolve = done;
      }),
    )
    .mockResolvedValue([]);
  const view = render(<Form />);
  view.rerender(<Form remote="https://other.example/repo.git" />);
  await screen.findByLabelText("Access token");
  await act(async () => resolve([saved]));
  expect(screen.queryByRole("option", { name: /Saved:/ })).toBeNull();
  fireEvent.change(screen.getByLabelText("Access token"), { target: { value: "new-secret" } });
  view.rerender(<Form remote="https://third.example/repo.git" />);
  expect(await screen.findByLabelText("Access token")).toHaveProperty("value", "");
});

it("defaults to the newest saved credential and offers a different token without revealing the saved value", async () => {
  vi.mocked(api.savedRepositoryCredentials).mockResolvedValue([saved]);
  render(<Form />);
  await waitFor(() =>
    expect(screen.getByLabelText("Git credential")).toHaveProperty("value", saved.id),
  );
  expect(screen.queryByLabelText("Access token")).toBeNull();
  fireEvent.change(screen.getByLabelText("Git credential"), { target: { value: "new" } });
  expect(screen.getByLabelText("Access token")).toHaveProperty("value", "");
  fireEvent.change(screen.getByLabelText("Access token"), { target: { value: "another-secret" } });
  fireEvent.change(screen.getByLabelText("Git credential"), { target: { value: saved.id } });
  fireEvent.change(screen.getByLabelText("Git credential"), { target: { value: "new" } });
  expect(screen.getByLabelText("Access token")).toHaveProperty("value", "");
});
