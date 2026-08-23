import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Plugin, PluginAccess, PluginOffer } from "../lib/types";
import { PluginList } from "./PluginList";

const pluginCatalogue = vi.fn<() => Promise<PluginOffer[]>>();
const groupPlugins = vi.fn<() => Promise<Plugin[]>>();
const connectPlugin = vi.fn<(groupId: string, kind: string) => Promise<Plugin>>();
const disconnectPlugin = vi.fn();
const setPluginAccess = vi.fn<(id: string, access: PluginAccess) => Promise<Plugin>>();
const openExternal = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    pluginCatalogue: () => pluginCatalogue(),
    groupPlugins: () => groupPlugins(),
    connectPlugin: (groupId: string, kind: string) => connectPlugin(groupId, kind),
    disconnectPlugin: (id: string) => disconnectPlugin(id),
    setPluginAccess: (id: string, access: PluginAccess) => setPluginAccess(id, access),
  },
  openExternal: (url: string) => openExternal(url),
}));

const GROUP = "00000000-0000-4000-8000-000000000001";

const OFFERS: PluginOffer[] = [
  {
    kind: "neon",
    name: "Neon",
    blurb: "Postgres databases.",
    docs: "https://neon.com/docs/ai/neon-mcp-server",
    endpoint: "https://mcp.neon.tech/mcp",
  },
  {
    kind: "stripe",
    name: "Stripe",
    blurb: "The live account.",
    docs: "https://docs.stripe.com/mcp",
    // Stripe's has no path, which is what the host line has to survive.
    endpoint: "https://mcp.stripe.com",
  },
];

function plugin(over: Partial<Plugin> = {}): Plugin {
  return {
    id: "p1",
    groupId: GROUP,
    kind: "neon",
    account: "",
    tools: ["run_sql", "create_branch"],
    access: { mode: "everyone" },
    signedIn: true,
    connectedAt: 0,
    ...over,
  };
}

/** Two agents, which is the smallest crew a plugin can be narrowed inside. */
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
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
  };
}

const CREW = [member("a1", "Revenue"), member("a2", "Scribe")];

describe("PluginList", () => {
  beforeEach(() => {
    pluginCatalogue.mockReset();
    groupPlugins.mockReset();
    connectPlugin.mockReset();
    disconnectPlugin.mockReset();
    setPluginAccess.mockReset();
    openExternal.mockReset();
    pluginCatalogue.mockResolvedValue(OFFERS);
    groupPlugins.mockResolvedValue([]);
  });

  it("names the host each sign-in would go to, before anything is clicked", async () => {
    // What an operator is agreeing to is which company gets their account. The
    // host is that; the path is noise beside it.
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("mcp.neon.tech")).toBeTruthy();
    expect(screen.getByText("mcp.stripe.com")).toBeTruthy();
  });

  it("connects one, and says it is waiting on a person while it does", async () => {
    // The flow finishes in a browser window, so the button has to explain a
    // wait that has nothing to do with the network.
    let finish: (value: Plugin) => void = () => {};
    connectPlugin.mockReturnValue(
      new Promise<Plugin>((resolve) => {
        finish = resolve;
      }),
    );

    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click((await screen.findAllByText("Connect"))[0]!);

    await waitFor(() => expect(connectPlugin).toHaveBeenCalledWith(GROUP, "neon"));
    expect(screen.getByText("Waiting for your browser…")).toBeTruthy();

    groupPlugins.mockResolvedValue([plugin()]);
    finish(plugin());
    expect(await screen.findByText(/2 tools/)).toBeTruthy();
  });

  it("offers no second connect while one is in flight", async () => {
    // Two loopback listeners and two browser windows, for one operator with
    // one pair of hands.
    connectPlugin.mockReturnValue(new Promise<Plugin>(() => {}));
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click((await screen.findAllByText("Connect"))[0]!);
    await waitFor(() => expect(connectPlugin).toHaveBeenCalled());

    for (const button of screen.getAllByRole("button")) {
      expect((button as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("says when a server asked for no sign-in, rather than claiming one", async () => {
    // Every server on the list asks for one today. This is what the row says
    // if one stops, because reporting it as signed in would be a claim about
    // the operator's account that is not true.
    groupPlugins.mockResolvedValue([plugin({ kind: "stripe", signedIn: false, tools: ["docs"] })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/asked for no sign-in/)).toBeTruthy();
  });

  it("disconnects one", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    disconnectPlugin.mockResolvedValue(undefined);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Disconnect"));
    await waitFor(() => expect(disconnectPlugin).toHaveBeenCalledWith("p1"));
  });

  it("opens the documentation in the shell, not in the webview", async () => {
    // A webview that navigates away from the app has no way back.
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click((await screen.findAllByText("What this can do"))[0]!);
    expect(openExternal).toHaveBeenCalledWith("https://neon.com/docs/ai/neon-mcp-server");
  });

  it("says a connected plugin is the whole crew's, until it is not", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/offered to every agent in this group/)).toBeTruthy();
    // Nobody is named while it is everyone's, because a tick list would be
    // claiming that the answer is those two agents rather than the crew.
    expect(screen.queryByRole("button", { name: "Revenue" })).toBe(null);
  });

  it("narrows one to chosen agents, and says plainly that nobody has it yet", async () => {
    // The state an operator is standing in for one click. It has to say what it
    // means: a plugin connected and callable by nobody looks identical to a
    // working one from the row above.
    const narrowed = plugin({ access: { mode: "chosen", agents: [] } });
    // What the panel reads back is what the store now says, which is the whole
    // reason nothing here keeps a draft.
    groupPlugins.mockResolvedValueOnce([plugin()]);
    groupPlugins.mockResolvedValue([narrowed]);
    setPluginAccess.mockResolvedValue(narrowed);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Only chosen agents"));

    await waitFor(() =>
      expect(setPluginAccess).toHaveBeenCalledWith("p1", { mode: "chosen", agents: [] }),
    );
    expect(await screen.findByText(/offered to nobody/)).toBeTruthy();
  });

  it("adds an agent to a narrowed plugin, and takes one back out", async () => {
    const both = plugin({ access: { mode: "chosen", agents: ["a1", "a2"] } });
    groupPlugins.mockResolvedValueOnce([plugin({ access: { mode: "chosen", agents: [] } })]);
    groupPlugins.mockResolvedValue([both]);
    setPluginAccess.mockResolvedValue(both);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByRole("button", { name: "Revenue" }));
    await waitFor(() =>
      expect(setPluginAccess).toHaveBeenCalledWith("p1", { mode: "chosen", agents: ["a1"] }),
    );

    // And the whole set every time, never a difference: the panel that sends
    // this is the one that knows who else was ticked, and a merge on the far
    // side would make unticking impossible to express.
    setPluginAccess.mockClear();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Scribe" }).getAttribute("aria-pressed")).toBe(
        "true",
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "Revenue" }));
    await waitFor(() =>
      expect(setPluginAccess).toHaveBeenCalledWith("p1", { mode: "chosen", agents: ["a2"] }),
    );
  });

  it("names who a narrowed plugin is for", async () => {
    groupPlugins.mockResolvedValue([plugin({ access: { mode: "chosen", agents: ["a1"] } })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/offered to Revenue/)).toBeTruthy();
    const revenue = screen.getByRole("button", { name: "Revenue" });
    expect(revenue.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Scribe" }).getAttribute("aria-pressed")).toBe(
      "false",
    );
  });

  it("asks nothing about who can use a plugin nobody has connected", async () => {
    // There is no sign-in to hand out, so the question is noise on four tiles.
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    await screen.findAllByText("Connect");
    expect(screen.queryByText("Only chosen agents")).toBe(null);
  });

  it("reports a refused sign-in instead of leaving the row looking connected", async () => {
    connectPlugin.mockRejectedValue(new Error("the sign-in was refused: access_denied"));
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click((await screen.findAllByText("Connect"))[0]!);
    expect(await screen.findByText(/access_denied/)).toBeTruthy();
    expect(screen.getAllByText("Connect").length).toBe(2);
  });
});
