import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Connector, ConnectorDraft } from "../lib/types";
import { CredentialList } from "./CredentialList";

const groupConnectors = vi.fn<(groupId: string) => Promise<Connector[]>>();
const createConnector = vi.fn<(draft: ConnectorDraft) => Promise<Connector>>();
const deleteConnector = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    groupConnectors: (groupId: string) => groupConnectors(groupId),
    createConnector: (draft: ConnectorDraft) => createConnector(draft),
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
    secretHint: "...ter2",
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("CredentialList", () => {
  beforeEach(() => {
    groupConnectors.mockReset();
    createConnector.mockReset();
    deleteConnector.mockReset();
    groupConnectors.mockResolvedValue([]);
  });

  it("asks for a service and a token, and nothing else", async () => {
    // Adding one used to be five empty boxes. Four of them were questions the
    // operator had to invent an answer for; only the token is something they
    // actually hold.
    createConnector.mockResolvedValue(connector());
    const { container } = render(<CredentialList groupId={GROUP} />);

    fireEvent.click(await screen.findByText("GitHub"));

    // Exactly one field, and it is the token.
    const inputs = [...container.querySelectorAll("input")];
    expect(inputs.length).toBe(1);
    expect(inputs[0]?.getAttribute("placeholder")).toBe("GitHub token");
    expect(inputs[0]?.getAttribute("type")).toBe("password");

    fireEvent.change(screen.getByPlaceholderText("GitHub token"), {
      target: { value: "ghp_x" },
    });
    fireEvent.click(screen.getByText("Add"));

    await waitFor(() => expect(createConnector).toHaveBeenCalled());
    // The variable a GitHub token belongs in is not a preference.
    expect(createConnector.mock.calls[0]?.[0]).toMatchObject({
      groupId: GROUP,
      service: "GitHub",
      envVar: "GITHUB_TOKEN",
      secret: "ghp_x",
    });
  });

  it("says where to get the token for the service that was picked", async () => {
    render(<CredentialList groupId={GROUP} />);
    fireEvent.click(await screen.findByText("Cloudflare"));
    expect(screen.getByText(/dash\.cloudflare\.com/)).toBeTruthy();
  });

  it("does not offer a service the group already holds", async () => {
    groupConnectors.mockResolvedValue([connector()]);
    render(<CredentialList groupId={GROUP} />);

    // The one in the list is the credential itself, not an offer to add it.
    await screen.findByText("$GITHUB_TOKEN");
    expect(screen.getAllByText("GitHub").length).toBe(1);
  });

  it("still takes a service nobody listed", async () => {
    createConnector.mockResolvedValue(connector());
    render(<CredentialList groupId={GROUP} />);

    fireEvent.click(await screen.findByText("Something else"));
    fireEvent.change(screen.getByPlaceholderText("what it is for"), {
      target: { value: "Internal" },
    });
    fireEvent.change(screen.getByPlaceholderText("MY_API_KEY"), {
      target: { value: "INTERNAL_TOKEN" },
    });
    fireEvent.change(screen.getByPlaceholderText("Internal token"), {
      target: { value: "tok" },
    });
    fireEvent.click(screen.getByText("Add"));

    await waitFor(() => expect(createConnector).toHaveBeenCalled());
    expect(createConnector.mock.calls[0]?.[0]).toMatchObject({
      service: "Internal",
      envVar: "INTERNAL_TOKEN",
      secret: "tok",
    });
  });

  it("will not send a credential without its value", async () => {
    render(<CredentialList groupId={GROUP} />);
    fireEvent.click(await screen.findByText("GitHub"));

    // There is no edit path, so an empty one could never be completed.
    expect((screen.getByText("Add") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByPlaceholderText("GitHub token"), { target: { value: "x" } });
    expect((screen.getByText("Add") as HTMLButtonElement).disabled).toBe(false);
  });

  it("never renders a value, only that one is set", async () => {
    groupConnectors.mockResolvedValue([connector()]);
    const { container } = render(<CredentialList groupId={GROUP} />);

    expect(await screen.findByText("...ter2")).toBeTruthy();
    expect(container.textContent).not.toContain("ghp_");
  });

  it("shows a credential nobody finished setting up as broken", async () => {
    groupConnectors.mockResolvedValue([connector({ secretSet: false, secretHint: "" })]);
    render(<CredentialList groupId={GROUP} />);
    expect(await screen.findByText("no value set")).toBeTruthy();
  });

  it("points at signing in on a computer for anything with a login page", async () => {
    render(<CredentialList groupId={GROUP} />);
    expect(await screen.findByText(/sign in on an agent's computer/)).toBeTruthy();
  });

  it("gives every service its own colour, so the grid can be scanned", async () => {
    // The name says which one it is; the colour is what the eye uses to find
    // it. A grid of identical buttons has neither.
    const { container } = render(<CredentialList groupId={GROUP} />);
    await screen.findByText("GitHub");

    // The "Something else" tile is deliberately unbranded, so it has no colour.
    const marks = [...container.querySelectorAll(".service .mark:not(.mark--other)")];
    expect(marks.length).toBeGreaterThan(8);

    const colours = new Set(
      marks.map((mark) => (mark as HTMLElement).style.getPropertyValue("--mark")),
    );
    expect(colours.size).toBeGreaterThan(6);
    expect(colours.has("")).toBe(false);
  });

  it("draws the real brand mark where there is one, and an initial where there is not", async () => {
    // A logo approximated by eye is a wrong logo, so the ones that exist are
    // real path data and the ones that do not degrade to a letter rather than
    // to something invented.
    const { container } = render(<CredentialList groupId={GROUP} />);
    await screen.findByText("GitHub");

    const github = container.querySelector(".service .mark svg path");
    expect(github?.getAttribute("d")?.length).toBeGreaterThan(200);

    // OpenAI has no published icon in the set, so it keeps its initial.
    const openai = [...container.querySelectorAll(".service")].find((tile) =>
      tile.textContent?.includes("OpenAI"),
    );
    expect(openai?.querySelector("svg")).toBeNull();
    expect(openai?.querySelector(".mark")?.textContent).toBe("O");
  });

  it("no longer offers Anthropic, which is what the app talks to, not a tool", async () => {
    render(<CredentialList groupId={GROUP} />);
    await screen.findByText("GitHub");
    expect(screen.queryByText("Anthropic")).toBeNull();
  });
});
