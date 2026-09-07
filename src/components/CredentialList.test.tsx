import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Connector, ConnectorDraft } from "../lib/types";
import { CredentialList } from "./CredentialList";

const groupConnectors = vi.fn<(groupId: string) => Promise<Connector[]>>();
const createConnector = vi.fn<(draft: ConnectorDraft) => Promise<Connector>>();
const deleteConnector = vi.fn();
const updateConnector = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    groupConnectors: (groupId: string) => groupConnectors(groupId),
    createConnector: (draft: ConnectorDraft) => createConnector(draft),
    updateConnector: (...args: unknown[]) => updateConnector(...args),
    deleteConnector: (id: string) => deleteConnector(id),
  },
}));

const GROUP = "00000000-0000-4000-8000-000000000001";

function connector(over: Partial<Connector> = {}): Connector {
  return {
    id: "c1",
    groupId: GROUP,
    service: "GitHub",
    account: "",
    envVar: "GITHUB_TOKEN",
    note: "",
    secretSet: true,
    agents: [],
    secretHint: "...ter2",
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

/** Opens the form, which is behind a button now that there are no tiles. */
async function openForm() {
  fireEvent.click(await screen.findByText("Add a secret"));
}

describe("CredentialList", () => {
  beforeEach(() => {
    groupConnectors.mockReset();
    createConnector.mockReset();
    deleteConnector.mockReset();
    updateConnector.mockReset();
    groupConnectors.mockResolvedValue([]);
  });

  it("offers no brands, only the escape hatch", async () => {
    // The twelve-tile grid is gone: what a crew reaches is a plugin, and this
    // is what is left for a service that has none. A tile here would be an
    // offer Guaca cannot keep, since it knows nothing about the service beyond
    // the variable name it suggested.
    render(<CredentialList groupId={GROUP} />);

    expect(await screen.findByText("Add a secret")).toBeTruthy();
    expect(screen.queryByText("GitHub")).toBeNull();
    expect(screen.queryByText("Something else")).toBeNull();
  });

  it("asks what it is for, the variable, and the token", async () => {
    createConnector.mockResolvedValue(connector());
    const { container } = render(<CredentialList groupId={GROUP} />);
    await openForm();

    const inputs = [...container.querySelectorAll("input")];
    expect(inputs.length).toBe(3);

    fireEvent.change(screen.getByPlaceholderText("what it is for"), {
      target: { value: "Fly" },
    });
    fireEvent.change(screen.getByPlaceholderText("MY_API_KEY"), {
      target: { value: "FLY_API_TOKEN" },
    });
    fireEvent.change(screen.getByPlaceholderText("Fly token"), { target: { value: "fo_live" } });
    fireEvent.click(screen.getByText("Add"));

    await waitFor(() => expect(createConnector).toHaveBeenCalled());
    expect(createConnector.mock.calls[0]?.[0]).toMatchObject({
      groupId: GROUP,
      service: "Fly",
      envVar: "FLY_API_TOKEN",
      secret: "fo_live",
    });
  });

  it("will not add one without all three", async () => {
    render(<CredentialList groupId={GROUP} />);
    await openForm();

    const add = () => screen.getByText("Add") as HTMLButtonElement;
    expect(add().disabled).toBe(true);

    fireEvent.change(screen.getByPlaceholderText("what it is for"), { target: { value: "Fly" } });
    expect(add().disabled).toBe(true);

    fireEvent.change(screen.getByPlaceholderText("MY_API_KEY"), {
      target: { value: "FLY_API_TOKEN" },
    });
    // Still not: a variable stored empty reads to the agent as a revoked token
    // rather than as unfinished setup.
    expect(add().disabled).toBe(true);

    fireEvent.change(screen.getByPlaceholderText("Fly token"), { target: { value: "x" } });
    expect(add().disabled).toBe(false);
  });

  it("shows what is held without ever showing a value", async () => {
    groupConnectors.mockResolvedValue([connector()]);
    render(<CredentialList groupId={GROUP} />);

    expect(await screen.findByText("GitHub")).toBeTruthy();
    expect(screen.getByText("$GITHUB_TOKEN")).toBeTruthy();
    expect(screen.getByText("Value saved")).toBeTruthy();
    expect(screen.queryByText("...ter2")).toBeNull();
  });

  it("says when a credential has no value, because it will hand over an empty variable", async () => {
    groupConnectors.mockResolvedValue([connector({ secretSet: false, secretHint: "" })]);
    render(<CredentialList groupId={GROUP} />);

    expect(await screen.findByText("no value set")).toBeTruthy();
  });

  it("forgets one", async () => {
    groupConnectors.mockResolvedValue([connector()]);
    deleteConnector.mockResolvedValue(undefined);
    render(<CredentialList groupId={GROUP} />);

    fireEvent.click(await screen.findByText("Forget"));
    await waitFor(() => expect(deleteConnector).toHaveBeenCalledWith("c1"));
  });
  it("defaults to nobody and saves only the selected recipients", async () => {
    render(
      <CredentialList
        groupId={GROUP}
        crew={[
          { id: "deployer", name: "Deployer" },
          { id: "writer", name: "Writer" },
        ]}
      />,
    );
    await openForm();
    expect(screen.getByRole("button", { name: "Deployer" }).getAttribute("aria-pressed")).toBe(
      "false",
    );
    fireEvent.change(screen.getByLabelText("Service"), { target: { value: "Cloudflare" } });
    fireEvent.change(screen.getByLabelText("Environment variable"), {
      target: { value: "CLOUDFLARE_API_TOKEN" },
    });
    fireEvent.change(screen.getByLabelText("Secret value"), {
      target: { value: "private-fixture" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Deployer" }));
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(createConnector).toHaveBeenCalledWith(
        expect.objectContaining({ agents: ["deployer"] }),
      ),
    );
    await waitFor(() => expect(screen.queryByLabelText("Secret value")).toBeNull());
  });

  it("rotates and revokes together without reading back the value", async () => {
    groupConnectors.mockResolvedValue([connector({ agents: ["deployer"] })]);
    render(<CredentialList groupId={GROUP} crew={[{ id: "deployer", name: "Deployer" }]} />);
    fireEvent.click(await screen.findByRole("button", { name: "Manage" }));
    fireEvent.click(screen.getByRole("button", { name: "Deployer" }));
    fireEvent.change(screen.getByLabelText("Replace value (optional)"), {
      target: { value: "replacement-fixture" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(updateConnector).toHaveBeenCalledWith("c1", [], "replacement-fixture"),
    );
    await waitFor(() => expect(screen.queryByLabelText("Replace value (optional)")).toBeNull());
  });

  it("keeps a failed update visible and does not claim access was changed", async () => {
    groupConnectors.mockResolvedValue([connector()]);
    updateConnector.mockRejectedValue(new Error("Backend unavailable"));
    render(<CredentialList groupId={GROUP} />);
    fireEvent.click(await screen.findByRole("button", { name: "Manage" }));
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    expect((await screen.findByRole("alert")).textContent).toContain("Backend unavailable");
    expect(updateConnector).toHaveBeenCalledWith("c1", [], null);
    expect(screen.getByRole("button", { name: "Save changes" })).toBeTruthy();
  });
});
