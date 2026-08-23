import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Plugin, PluginOffer } from "../lib/types";
import { PluginList } from "./PluginList";

const pluginCatalogue = vi.fn<() => Promise<PluginOffer[]>>();
const groupPlugins = vi.fn<() => Promise<Plugin[]>>();
const connectPlugin = vi.fn<(groupId: string, kind: string) => Promise<Plugin>>();
const disconnectPlugin = vi.fn();
const openExternal = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    pluginCatalogue: () => pluginCatalogue(),
    groupPlugins: () => groupPlugins(),
    connectPlugin: (groupId: string, kind: string) => connectPlugin(groupId, kind),
    disconnectPlugin: (id: string) => disconnectPlugin(id),
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
    signedIn: true,
    connectedAt: 0,
    ...over,
  };
}

describe("PluginList", () => {
  beforeEach(() => {
    pluginCatalogue.mockReset();
    groupPlugins.mockReset();
    connectPlugin.mockReset();
    disconnectPlugin.mockReset();
    openExternal.mockReset();
    pluginCatalogue.mockResolvedValue(OFFERS);
    groupPlugins.mockResolvedValue([]);
  });

  it("names the host each sign-in would go to, before anything is clicked", async () => {
    // What an operator is agreeing to is which company gets their account. The
    // host is that; the path is noise beside it.
    render(<PluginList groupId={GROUP} />);

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

    render(<PluginList groupId={GROUP} />);
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
    render(<PluginList groupId={GROUP} />);

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
    render(<PluginList groupId={GROUP} />);

    expect(await screen.findByText(/asked for no sign-in/)).toBeTruthy();
  });

  it("disconnects one", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    disconnectPlugin.mockResolvedValue(undefined);
    render(<PluginList groupId={GROUP} />);

    fireEvent.click(await screen.findByText("Disconnect"));
    await waitFor(() => expect(disconnectPlugin).toHaveBeenCalledWith("p1"));
  });

  it("opens the documentation in the shell, not in the webview", async () => {
    // A webview that navigates away from the app has no way back.
    render(<PluginList groupId={GROUP} />);

    fireEvent.click((await screen.findAllByText("What this can do"))[0]!);
    expect(openExternal).toHaveBeenCalledWith("https://neon.com/docs/ai/neon-mcp-server");
  });

  it("reports a refused sign-in instead of leaving the row looking connected", async () => {
    connectPlugin.mockRejectedValue(new Error("the sign-in was refused: access_denied"));
    render(<PluginList groupId={GROUP} />);

    fireEvent.click((await screen.findAllByText("Connect"))[0]!);
    expect(await screen.findByText(/access_denied/)).toBeTruthy();
    expect(screen.getAllByText("Connect").length).toBe(2);
  });
});
