import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AccountConnectors,
  AgentCard,
  HeaderPair,
  Plugin,
  PluginAccess,
  PluginOffer,
  PluginToolCard,
  ServerReport,
} from "../lib/types";
import { PluginList, pluginListReducer } from "./PluginList";

const pluginCatalog = vi.fn<() => Promise<PluginOffer[]>>();
const groupPlugins = vi.fn<() => Promise<Plugin[]>>();
const connectPlugin =
  vi.fn<(groupId: string, kind: string, connection?: string) => Promise<Plugin>>();
const disconnectPlugin = vi.fn();
const setPluginAccess = vi.fn<(id: string, access: PluginAccess) => Promise<Plugin>>();
const setPluginTool = vi.fn<(id: string, tool: string, access: PluginAccess) => Promise<Plugin>>();
const addPlugin =
  vi.fn<
    (
      groupId: string,
      name: string,
      url: string,
      key?: string,
      headers?: HeaderPair[],
    ) => Promise<Plugin>
  >();
const readdressPlugin =
  vi.fn<
    (
      groupId: string,
      id: string,
      url: string,
      key?: string,
      headers?: HeaderPair[],
    ) => Promise<Plugin>
  >();
const probeServer =
  vi.fn<(url: string, key?: string, headers?: HeaderPair[]) => Promise<ServerReport>>();
const checkPlugin = vi.fn<(id: string) => Promise<ServerReport>>();
const openExternal = vi.fn();
const accountStatus = vi.fn(async () => ({
  signedIn: true,
  email: "",
  origin: "https://broker.example",
  broker: false,
}));
const signInAccount = vi.fn(async () => ({
  signedIn: true,
  email: "",
  origin: "https://broker.example",
  broker: true,
}));

vi.mock("../lib/ipc", () => ({
  api: {
    pluginCatalog: () => pluginCatalog(),
    groupPlugins: () => groupPlugins(),
    accountConnectors: () => accountConnectors(),
    accountStatus: () => accountStatus(),
    signInAccount: () => signInAccount(),
    connectPlugin: (groupId: string, kind: string, connection?: string) =>
      connectPlugin(groupId, kind, connection),
    setPluginConnection: (groupId: string, kind: string, connection: string) =>
      setPluginConnection(groupId, kind, connection),
    disconnectPlugin: (id: string) => disconnectPlugin(id),
    setPluginAccess: (id: string, access: PluginAccess) => setPluginAccess(id, access),
    setPluginTool: (id: string, tool: string, access: PluginAccess) =>
      setPluginTool(id, tool, access),
    addPlugin: (groupId: string, name: string, url: string, key?: string, headers?: HeaderPair[]) =>
      addPlugin(groupId, name, url, key, headers),
    readdressPlugin: (
      groupId: string,
      id: string,
      url: string,
      key?: string,
      headers?: HeaderPair[],
    ) => readdressPlugin(groupId, id, url, key, headers),
    probeServer: (url: string, key?: string, headers?: HeaderPair[]) =>
      probeServer(url, key, headers),
    checkPlugin: (id: string) => checkPlugin(id),
  },
  openExternal: (url: string) => openExternal(url),
}));

const accountConnectors = vi.fn<() => Promise<AccountConnectors>>(async () => {
  throw new Error("not signed in");
});
const setPluginConnection = vi.fn<
  (groupId: string, kind: string, connection: string) => Promise<Plugin>
>(async () => plugin());

const GROUP = "00000000-0000-4000-8000-000000000001";

const OFFERS: PluginOffer[] = [
  {
    kind: "neon",
    name: "Neon",
    blurb: "Postgres databases.",
    docs: "https://neon.com/docs/ai/neon-mcp-server",
    endpoint: "https://mcp.neon.tech/mcp",
    accountBacked: false,
  },
  {
    kind: "stripe",
    name: "Stripe",
    blurb: "The live account.",
    docs: "https://docs.stripe.com/mcp",
    endpoint: "https://mcp.stripe.com",
    accountBacked: false,
  },
];

describe("plugin list load transitions", () => {
  const states = [
    { state: "loading" as const },
    { state: "failed" as const },
    { state: "ready" as const, offers: OFFERS, connected: [] },
  ];

  it.each(states)("accepts a successful load from $state", (state) => {
    expect(pluginListReducer(state, { type: "loaded", offers: OFFERS, connected: [] })).toEqual({
      state: "ready",
      offers: OFFERS,
      connected: [],
    });
  });

  it.each(states)("accepts a failed load from $state", (state) => {
    expect(pluginListReducer(state, { type: "failed" })).toEqual({ state: "failed" });
  });
});

function tool(name: string, access: PluginAccess = { mode: "everyone" }): PluginToolCard {
  return { name, description: `Runs ${name}.`, access };
}

const NOBODY: PluginAccess = { mode: "chosen", agents: [] };

function plugin(over: Partial<Plugin> = {}): Plugin {
  return {
    id: "p1",
    groupId: GROUP,
    kind: "neon",
    name: "Neon",
    endpoint: "https://mcp.neon.tech/mcp",
    custom: false,
    account: "",
    tools: [tool("run_sql"), tool("create_branch")],
    access: { mode: "everyone" },
    connection: "",
    headers: [],
    signedIn: true,
    connectedAt: 0,
    ...over,
  };
}

function member(id: string, name: string): AgentCard {
  return {
    id,
    groupId: GROUP,
    name,
    avatar: "avocado",
    color: "#7fb069",
    model: "",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
  };
}

const CREW = [member("a1", "Revenue"), member("a2", "Scribe")];

describe("PluginList", () => {
  beforeEach(() => {
    pluginCatalog.mockReset();
    groupPlugins.mockReset();
    connectPlugin.mockReset();
    disconnectPlugin.mockReset();
    setPluginAccess.mockReset();
    setPluginTool.mockReset();
    addPlugin.mockReset();
    readdressPlugin.mockReset();
    openExternal.mockReset();
    pluginCatalog.mockResolvedValue(OFFERS);
    groupPlugins.mockResolvedValue([]);
  });

  it("names the host each sign-in would go to, before anything is clicked", async () => {
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("mcp.neon.tech")).toBeTruthy();
    expect(screen.getByText("mcp.stripe.com")).toBeTruthy();
  });

  it("connects one, and says it is waiting on a person while it does", async () => {
    let finish: (value: Plugin) => void = () => {};
    connectPlugin.mockReturnValue(
      new Promise<Plugin>((resolve) => {
        finish = resolve;
      }),
    );

    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click((await screen.findAllByText("Connect"))[0]!);

    await waitFor(() => expect(connectPlugin).toHaveBeenCalledWith(GROUP, "neon", undefined));
    expect(screen.getByText("Waiting for your browser…")).toBeTruthy();

    groupPlugins.mockResolvedValue([plugin()]);
    finish(plugin());
    expect(await screen.findByText(/2 tools/)).toBeTruthy();
  });

  it("offers no second connect while one is in flight", async () => {
    connectPlugin.mockReturnValue(new Promise<Plugin>(() => {}));
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click((await screen.findAllByText("Connect"))[0]!);
    await waitFor(() => expect(connectPlugin).toHaveBeenCalled());

    for (const button of screen.getAllByRole("button")) {
      expect((button as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("says when a server asked for no sign-in, rather than claiming one", async () => {
    groupPlugins.mockResolvedValue([
      plugin({ kind: "stripe", signedIn: false, tools: [tool("docs")] }),
    ]);
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
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click((await screen.findAllByText("What this can do"))[0]!);
    expect(openExternal).toHaveBeenCalledWith("https://neon.com/docs/ai/neon-mcp-server");
  });

  it("says a connected plugin is the whole crew's, until it is not", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/offered to every agent in this group/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Revenue" })).toBe(null);
  });

  it("narrows one to chosen agents, and says plainly that nobody has it yet", async () => {
    const narrowed = plugin({ access: { mode: "chosen", agents: [] } });
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

  it("keeps the tool list shut until it is asked for", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("Show all 2")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Only chosen agents: run_sql" })).toBe(null);

    fireEvent.click(screen.getByText("Show all 2"));
    expect(screen.getByText("run_sql")).toBeTruthy();
    expect(screen.getByText("Runs run_sql.")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Every agent: run_sql" }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("says nothing per tool while a tool is the default", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 2"));
    expect(screen.queryByRole("button", { name: "Revenue: run_sql" })).toBe(null);
  });

  it("switches one tool off, and back on again", async () => {
    const off = plugin({ tools: [tool("run_sql", NOBODY), tool("create_branch")] });
    groupPlugins.mockResolvedValueOnce([plugin()]);
    groupPlugins.mockResolvedValue([off]);
    setPluginTool.mockResolvedValue(off);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 2"));
    fireEvent.click(screen.getByRole("button", { name: "Only chosen agents: run_sql" }));

    await waitFor(() => expect(setPluginTool).toHaveBeenCalledWith("p1", "run_sql", NOBODY));
    await waitFor(() =>
      expect(
        screen
          .getByRole("button", { name: "Only chosen agents: run_sql" })
          .getAttribute("aria-pressed"),
      ).toBe("true"),
    );
    expect(screen.getByText(/1 switched off/)).toBeTruthy();
    expect(screen.getByText(/nobody in this group can call it/)).toBeTruthy();

    setPluginTool.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Every agent: run_sql" }));
    await waitFor(() =>
      expect(setPluginTool).toHaveBeenCalledWith("p1", "run_sql", { mode: "everyone" }),
    );
  });

  it("gives two agents on one plugin different tools", async () => {
    const split = plugin({
      tools: [
        tool("run_sql", { mode: "chosen", agents: ["a1"] }),
        tool("create_branch", { mode: "chosen", agents: ["a2"] }),
      ],
    });
    groupPlugins.mockResolvedValue([split]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 2"));

    expect(screen.getByText("called by Revenue")).toBeTruthy();
    expect(screen.getByText("called by Scribe")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Revenue: run_sql" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      screen.getByRole("button", { name: "Scribe: run_sql" }).getAttribute("aria-pressed"),
    ).toBe("false");
    expect(screen.getByText(/2 for chosen agents/)).toBeTruthy();
  });

  it("adds an agent to a narrowed tool, and takes one back out", async () => {
    const both = plugin({
      tools: [tool("run_sql", { mode: "chosen", agents: ["a1", "a2"] }), tool("create_branch")],
    });
    groupPlugins.mockResolvedValueOnce([plugin({ tools: [tool("run_sql", NOBODY)] })]);
    groupPlugins.mockResolvedValue([both]);
    setPluginTool.mockResolvedValue(both);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 1"));
    fireEvent.click(screen.getByRole("button", { name: "Revenue: run_sql" }));
    await waitFor(() =>
      expect(setPluginTool).toHaveBeenCalledWith("p1", "run_sql", {
        mode: "chosen",
        agents: ["a1"],
      }),
    );

    setPluginTool.mockClear();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Scribe: run_sql" }).getAttribute("aria-pressed"),
      ).toBe("true"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Revenue: run_sql" }));
    await waitFor(() =>
      expect(setPluginTool).toHaveBeenCalledWith("p1", "run_sql", {
        mode: "chosen",
        agents: ["a2"],
      }),
    );
  });

  it("says when a tool names an agent the plugin itself does not reach", async () => {
    groupPlugins.mockResolvedValue([
      plugin({
        access: { mode: "chosen", agents: ["a1"] },
        tools: [tool("run_sql", { mode: "chosen", agents: ["a2"] })],
      }),
    ]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 1"));
    expect(screen.getByText(/nobody can call it: Scribe is not on this connector/)).toBeTruthy();
  });

  it("says when a connected plugin has nothing left switched on", async () => {
    groupPlugins.mockResolvedValue([
      plugin({ tools: [tool("run_sql", NOBODY), tool("create_branch", NOBODY)] }),
    ]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/all switched off/)).toBeTruthy();
  });

  it("asks nothing about tools of a plugin nobody has connected", async () => {
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    await screen.findAllByText("Connect");
    expect(screen.queryByText(/Show all/)).toBe(null);
  });

  it("asks nothing about who can use a plugin nobody has connected", async () => {
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

describe("choosing which account a crew uses", () => {
  const saying = (pattern: RegExp) => (_content: string, element: Element | null) =>
    element?.tagName === "P" && pattern.test(element.textContent ?? "");
  const GOOGLE_OFFER: PluginOffer = {
    kind: "google",
    name: "Google",
    blurb: "Your Gmail, Calendar and Drive.",
    docs: "https://guaca.bot/app",
    endpoint: "https://guaca.bot/mcp",
    accountBacked: true,
  };

  const two: AccountConnectors = {
    email: "robert@example.com",
    providers: [],
    connections: [
      { id: "acct_work", provider: "google", label: "work@example.com", capabilities: ["gmail"] },
      { id: "acct_home", provider: "google", label: "home@example.com", capabilities: ["drive"] },
    ],
  };

  const one: AccountConnectors = { ...two, connections: [two.connections[0]!] };

  beforeEach(() => {
    pluginCatalog.mockResolvedValue([GOOGLE_OFFER]);
  });

  it("keeps the newer account response when the previous load finishes last", async () => {
    let finishOld!: (account: AccountConnectors) => void;
    accountConnectors.mockReset();
    accountConnectors.mockReturnValueOnce(
      new Promise((resolve) => {
        finishOld = resolve;
      }),
    );
    accountConnectors.mockResolvedValueOnce({ ...one, email: "new@example.com" });
    groupPlugins.mockResolvedValue([]);
    connectPlugin.mockResolvedValue(plugin({ kind: "google" }));
    render(<PluginList groupId={GROUP} crew={CREW} />);

    await waitFor(() => expect(accountConnectors).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByText(saying(/from your Guaca account new@example.com/))).toBeTruthy();

    await act(async () => finishOld({ ...one, email: "old@example.com" }));
    expect(screen.getByText(saying(/from your Guaca account new@example.com/))).toBeTruthy();
    expect(screen.queryByText(saying(/old@example.com/))).toBeNull();
  });

  it("opens the configured account origin or a broker handoff", async () => {
    accountConnectors.mockResolvedValue({ ...two, connections: [] });
    accountStatus.mockResolvedValue({
      signedIn: true,
      email: "",
      origin: "https://broker.example",
      broker: false,
    });
    render(<PluginList groupId={GROUP} crew={CREW} />);
    const button = await screen.findByRole("button", { name: "Authorize a Google account" });
    fireEvent.click(button);
    await waitFor(() => expect(openExternal).toHaveBeenCalledWith("https://broker.example/app"));
    accountStatus.mockResolvedValue({
      signedIn: true,
      email: "",
      origin: "https://broker.example",
      broker: true,
    });
    fireEvent.click(button);
    await waitFor(() => expect(signInAccount).toHaveBeenCalled());
  });

  it("says which account it acts as, without an inert picker, when there is one", async () => {
    accountConnectors.mockResolvedValue(one);
    groupPlugins.mockResolvedValue([plugin({ kind: "google", connection: "acct_work" })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    await waitFor(() =>
      expect(screen.getByText(saying(/Acting as work@example.com/))).toBeTruthy(),
    );
    expect(screen.queryByLabelText(/^Account/)).toBeNull();
  });

  it("does not claim a group is acting as anyone before it is connected", async () => {
    accountConnectors.mockResolvedValue(one);
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    await waitFor(() => expect(screen.getByText(saying(/Will connect as/))).toBeTruthy());
    expect(screen.queryByText(saying(/Acting as/))).toBeNull();
  });

  it("names the Guaca account the identities came from", async () => {
    accountConnectors.mockResolvedValue(one);
    groupPlugins.mockResolvedValue([plugin({ kind: "google", connection: "acct_work" })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(
      await screen.findByText(saying(/from your Guaca account robert@example.com/)),
    ).toBeTruthy();
  });

  it("names it in the empty state too, which is where it matters most", async () => {
    accountConnectors.mockResolvedValue({ ...two, connections: [] });
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(
      await screen.findByText(
        saying(/No Google account is authorized on your Guaca account, robert@example.com/),
      ),
    ).toBeTruthy();
  });

  it("offers the picker before connecting, not only after", async () => {
    accountConnectors.mockResolvedValue(two);
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    const picker = (await screen.findByLabelText(/^Account/)) as HTMLSelectElement;
    expect([...picker.options].map((option) => option.value)).toEqual(["acct_work", "acct_home"]);
  });

  it("connects against the identity picked before the click", async () => {
    accountConnectors.mockResolvedValue(two);
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.change(await screen.findByLabelText(/^Account/), {
      target: { value: "acct_home" },
    });
    expect(setPluginConnection).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Connect"));
    await waitFor(() => expect(connectPlugin).toHaveBeenCalledWith(GROUP, "google", "acct_home"));
  });

  it("says so when the account has authorized nothing at that provider", async () => {
    accountConnectors.mockResolvedValue({ ...two, connections: [] });
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(saying(/No Google account is authorized/))).toBeTruthy();
    expect(screen.getByText("Authorize a Google account")).toBeTruthy();
  });

  it("offers a picker showing every authorized identity", async () => {
    accountConnectors.mockResolvedValue(two);
    groupPlugins.mockResolvedValue([plugin({ kind: "google", connection: "acct_work" })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    const picker = (await screen.findByLabelText(/^Account/)) as HTMLSelectElement;
    expect([...picker.options].map((option) => option.textContent)).toEqual([
      "work@example.com",
      "home@example.com",
    ]);
    expect(picker.value).toBe("acct_work");
  });

  it("moves the crew to the other account without disconnecting", async () => {
    accountConnectors.mockResolvedValue(two);
    groupPlugins.mockResolvedValue([plugin({ kind: "google", connection: "acct_work" })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    const picker = await screen.findByLabelText(/^Account/);
    fireEvent.change(picker, { target: { value: "acct_home" } });

    await waitFor(() =>
      expect(setPluginConnection).toHaveBeenCalledWith(GROUP, "google", "acct_home"),
    );
    expect(disconnectPlugin).not.toHaveBeenCalled();
  });

  it("connects a fresh crew against the first identity", async () => {
    accountConnectors.mockResolvedValue(two);
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Connect"));
    await waitFor(() => expect(connectPlugin).toHaveBeenCalledWith(GROUP, "google", "acct_work"));
  });

  it("draws the plugin list even when the account cannot be reached", async () => {
    accountConnectors.mockRejectedValue(new Error("not signed in"));
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("Google")).toBeTruthy();
    expect(screen.queryByLabelText(/^Account/)).toBeNull();
  });
});

describe("a server the operator added", () => {
  const added = (over: Partial<Plugin> = {}): Plugin =>
    plugin({
      id: "p2",
      kind: "home_assistant",
      name: "home_assistant",
      endpoint: "https://ha.example.com/mcp",
      custom: true,
      ...over,
    });

  beforeEach(() => {
    pluginCatalog.mockReset();
    groupPlugins.mockReset();
    addPlugin.mockReset();
    readdressPlugin.mockReset();
    disconnectPlugin.mockReset();
    probeServer.mockReset();
    checkPlugin.mockReset();
    pluginCatalog.mockResolvedValue(OFFERS);
    groupPlugins.mockResolvedValue([]);
  });

  it("does not replace the new group's list when the old load finishes late", async () => {
    let finish!: (plugins: Plugin[]) => void;
    groupPlugins.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    groupPlugins.mockResolvedValueOnce([added({ name: "new_group_server" })]);
    const view = render(<PluginList groupId={GROUP} crew={CREW} />);
    view.rerender(<PluginList groupId="other-group" crew={CREW} />);
    expect(await screen.findByText("new_group_server")).toBeTruthy();
    await act(async () => finish([added({ name: "old_group_server" })]));
    expect(screen.queryByText("old_group_server")).toBeNull();
    expect(screen.getByText("new_group_server")).toBeTruthy();
  });

  it("does not show a delayed address test in another group", async () => {
    let finish!: (report: ServerReport) => void;
    probeServer.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const view = render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://old.example/mcp" },
    });
    fireEvent.click(screen.getByText("Test"));
    view.rerender(<PluginList groupId="other-group" crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    await act(async () =>
      finish({
        endpoint: "https://old.example/mcp",
        transport: "HTTP",
        protocol: "",
        handshake: true,
        signin: "none",
        server: "old",
        tools: [],
        ms: 42,
      }),
    );
    expect(screen.queryByText(/answered in 42 ms/)).toBeNull();
  });

  it("keeps existing secret headers when reconnecting without editing them", async () => {
    groupPlugins.mockResolvedValue([added({ headers: ["x-api-key"] })]);
    readdressPlugin.mockResolvedValue(added());
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Change address"));
    fireEvent.click(screen.getByText("Save and reconnect"));
    await waitFor(() =>
      expect(readdressPlugin).toHaveBeenCalledWith(
        GROUP,
        "p2",
        "https://ha.example.com/mcp",
        undefined,
        undefined,
      ),
    );
  });

  it("draws it under the name its tools are called by, not the one that was typed", async () => {
    groupPlugins.mockResolvedValue([added()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("home_assistant")).toBeTruthy();
    expect(screen.getByText("ha.example.com")).toBeTruthy();
    expect(screen.getByText(/nobody has checked it/)).toBeTruthy();
  });

  it("adds one, and keeps what was typed when the address is refused", async () => {
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));

    fireEvent.change(screen.getByPlaceholderText("Home Assistant"), {
      target: { value: "Home Assistant" },
    });
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "http://ha.example.com/mcp" },
    });

    addPlugin.mockRejectedValue(new Error("that is not https"));
    fireEvent.click(screen.getByText("Add and connect"));
    expect(await screen.findByText(/not https/)).toBeTruthy();
    expect((screen.getByPlaceholderText("Home Assistant") as HTMLInputElement).value).toBe(
      "Home Assistant",
    );

    addPlugin.mockResolvedValue(added());
    groupPlugins.mockResolvedValue([added()]);
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://ha.example.com/mcp" },
    });
    fireEvent.click(screen.getByText("Add and connect"));

    await waitFor(() =>
      expect(addPlugin).toHaveBeenLastCalledWith(
        GROUP,
        "Home Assistant",
        "https://ha.example.com/mcp",
        undefined,
        [],
      ),
    );
    expect(await screen.findByText("home_assistant")).toBeTruthy();
    expect(screen.queryByPlaceholderText("Home Assistant")).toBeNull();
  });

  it("adds a program on the host, handed its environment and no key", async () => {
    const program = added({ name: "github", endpoint: "stdio:npx -y server-github" });
    addPlugin.mockResolvedValue(program);
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("Home Assistant"), {
      target: { value: "github" },
    });
    fireEvent.click(screen.getByRole("button", { name: "A program on the host" }));
    expect(screen.getByText("Key (optional)").closest("label")?.hidden).toBe(true);
    fireEvent.change(screen.getByPlaceholderText("npx -y @modelcontextprotocol/server-github"), {
      target: { value: "npx -y server-github" },
    });
    fireEvent.click(screen.getByText("Add a variable"));
    fireEvent.change(screen.getByLabelText("Variable 1 name"), {
      target: { value: "GITHUB_TOKEN" },
    });
    fireEvent.change(screen.getByLabelText("Variable 1 value"), { target: { value: "ghp_x" } });
    groupPlugins.mockResolvedValue([program]);
    fireEvent.click(screen.getByText("Add and connect"));
    await waitFor(() =>
      expect(addPlugin).toHaveBeenLastCalledWith(
        GROUP,
        "github",
        "stdio:npx -y server-github",
        undefined,
        [{ name: "GITHUB_TOKEN", value: "ghp_x" }],
      ),
    );
    expect(await screen.findByText("npx on the host")).toBeTruthy();
  });

  it("sends a pasted key only when there is one", async () => {
    addPlugin.mockResolvedValue(added());
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("Home Assistant"), {
      target: { value: "Vault" },
    });
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://vault.example.com/mcp" },
    });
    fireEvent.change(screen.getByLabelText(/^Key \(optional\)/), {
      target: { value: "abc123" },
    });
    fireEvent.click(screen.getByText("Add and connect"));

    await waitFor(() =>
      expect(addPlugin).toHaveBeenCalledWith(
        GROUP,
        "Vault",
        "https://vault.example.com/mcp",
        "abc123",
        [],
      ),
    );
  });

  it("moves one to a new address without disconnecting it", async () => {
    groupPlugins.mockResolvedValue([added()]);
    readdressPlugin.mockResolvedValue(added({ endpoint: "http://localhost:9000/mcp" }));
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Change address"));
    const address = screen.getByDisplayValue("https://ha.example.com/mcp");
    fireEvent.change(address, { target: { value: "http://localhost:9000/mcp" } });
    fireEvent.click(screen.getByText("Save and reconnect"));

    await waitFor(() =>
      expect(readdressPlugin).toHaveBeenCalledWith(
        GROUP,
        "p2",
        "http://localhost:9000/mcp",
        undefined,
        undefined,
      ),
    );
    expect(disconnectPlugin).not.toHaveBeenCalled();
  });

  it("sends the headers that were typed, and never sends a name back as a value", async () => {
    addPlugin.mockResolvedValue(added());
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));

    fireEvent.change(screen.getByPlaceholderText("Home Assistant"), {
      target: { value: "Vault" },
    });
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://vault.example.com/mcp" },
    });
    fireEvent.click(screen.getByText("Add a header"));
    fireEvent.change(screen.getByLabelText("Header 1 name"), { target: { value: "X-API-Key" } });
    fireEvent.change(screen.getByLabelText("Header 1 value"), { target: { value: "abc123" } });
    fireEvent.click(screen.getByText("Add and connect"));

    await waitFor(() =>
      expect(addPlugin).toHaveBeenCalledWith(
        GROUP,
        "Vault",
        "https://vault.example.com/mcp",
        undefined,
        [{ name: "X-API-Key", value: "abc123" }],
      ),
    );
  });

  it("keeps a half-typed header on the row it was typed on", async () => {
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.click(screen.getByText("Add a header"));
    fireEvent.click(screen.getByText("Add a header"));
    fireEvent.change(screen.getByLabelText("Header 1 name"), { target: { value: "first" } });
    fireEvent.change(screen.getByLabelText("Header 2 name"), { target: { value: "second" } });

    fireEvent.click(screen.getByLabelText("Remove header 1"));

    expect((screen.getByLabelText("Header 1 name") as HTMLInputElement).value).toBe("second");
  });

  it("hides a header's value and shows its name on the row", async () => {
    groupPlugins.mockResolvedValue([added({ headers: ["x-api-key", "cf-access-client-id"] })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/Sends x-api-key, cf-access-client-id/)).toBeTruthy();
  });

  it("leaves stored headers alone unless the operator replaces them", async () => {
    groupPlugins.mockResolvedValue([added({ headers: ["x-api-key"] })]);
    readdressPlugin.mockResolvedValue(added());
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Change address"));
    fireEvent.click(screen.getByText("Replace headers"));
    expect((screen.getByLabelText("Header 1 name") as HTMLInputElement).value).toBe("x-api-key");
    expect((screen.getByLabelText("Header 1 value") as HTMLInputElement).value).toBe("");

    fireEvent.change(screen.getByLabelText("Header 1 value"), { target: { value: "new" } });
    fireEvent.click(screen.getByText("Save and reconnect"));

    await waitFor(() =>
      expect(readdressPlugin).toHaveBeenCalledWith(
        GROUP,
        "p2",
        "https://ha.example.com/mcp",
        undefined,
        [{ name: "x-api-key", value: "new" }],
      ),
    );
  });

  it("tests an address before anything is connected, and says what it found", async () => {
    probeServer.mockResolvedValue({
      endpoint: "https://ha.example.com/mcp",
      transport: "HTTP+SSE (2024-11-05)",
      protocol: "2024-11-05",
      handshake: true,
      signin: "none",
      server: "Home Assistant",
      tools: ["turn_on"],
      ms: 42,
    });
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://ha.example.com/mcp" },
    });
    fireEvent.click(screen.getByText("Test"));

    expect(await screen.findByText(/HTTP\+SSE \(2024-11-05\)/)).toBeTruthy();
    expect(screen.getByText(/1 tool: turn_on/)).toBeTruthy();
    expect(addPlugin).not.toHaveBeenCalled();
  });

  it("keeps a refusal on screen rather than clearing the form it was about", async () => {
    probeServer.mockRejectedValue(new Error("could not reach https://ha.example.com/mcp"));
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://ha.example.com/mcp" },
    });
    fireEvent.click(screen.getByText("Test"));

    expect(await screen.findByText(/could not reach/)).toBeTruthy();
    expect((screen.getByPlaceholderText("https://example.com/mcp") as HTMLInputElement).value).toBe(
      "https://ha.example.com/mcp",
    );
  });

  it("says what to check where the eye is, and folds the raw error away", async () => {
    probeServer.mockRejectedValue({
      kind: "plugin",
      message:
        "Could not reach http://127.0.0.1:9/mcp. No connection was made: the name may not resolve, the port may be closed, or the server may have refused the encrypted connection. Check the address and its path, and that the server is running and reachable from here.",
      detail:
        "error sending request for url (http://127.0.0.1:9/mcp): client error (Connect): tcp connect error: Connection refused (os error 111)",
    });
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "http://127.0.0.1:9/mcp" },
    });
    fireEvent.click(screen.getByText("Test"));

    const says = await screen.findByText(/Check the address and its path/);
    const test = screen.getByRole("button", { name: "Test" });
    expect(says.compareDocumentPosition(test) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByText(/Connection refused/)).toBeTruthy();
    const detail = document.querySelector<HTMLDetailsElement>(".connector-failure__detail");
    expect(detail?.open).toBe(false);
    expect(detail?.querySelector("summary")?.textContent).toBe("The raw error");
  });

  it("tests a connected plugin against the sign-in it already has", async () => {
    groupPlugins.mockResolvedValue([added()]);
    checkPlugin.mockResolvedValue({
      endpoint: "https://ha.example.com/mcp",
      transport: "",
      protocol: "",
      handshake: false,
      signin: "refused",
      server: "",
      tools: [],
      ms: 31,
    });
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Test it"));

    await waitFor(() => expect(checkPlugin).toHaveBeenCalledWith("p2"));
    expect(await screen.findByText(/refused what you gave it/)).toBeTruthy();
    expect(readdressPlugin).not.toHaveBeenCalled();
  });

  it("draws a mark for a server named after something on Object's prototype", async () => {
    groupPlugins.mockResolvedValue([added({ kind: "constructor", name: "constructor" })]);
    const { container } = render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("constructor")).toBeTruthy();
    const marks = [...container.querySelectorAll(".mark__icon path")];
    expect(marks.every((path) => path.getAttribute("d"))).toBe(true);
  });

  it("offers no address control on a server Guaca ships", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/2 tools/)).toBeTruthy();
    expect(screen.queryByText("Change address")).toBeNull();
  });

  it("says what a test is doing, and promises no window it never opens", async () => {
    probeServer.mockReturnValue(new Promise<ServerReport>(() => {}));
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://ha.example.com/mcp" },
    });
    fireEvent.click(screen.getByText("Test"));

    expect(await screen.findByText(/Testing the address/)).toBeTruthy();
    expect(screen.queryByText(/browser window/)).toBeNull();
  });

  it("says what a disconnect is doing, and promises no window", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    disconnectPlugin.mockReturnValue(new Promise(() => {}));
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Disconnect"));
    expect(await screen.findByText(/Its tools leave this group/)).toBeTruthy();
    expect(screen.queryByText(/browser window/)).toBeNull();
  });

  it("clears what a test found when the address it was run against changes", async () => {
    probeServer.mockResolvedValue({
      endpoint: "https://ha.example.com/mcp",
      transport: "streamable HTTP",
      protocol: "2026-07-28",
      handshake: false,
      signin: "none",
      server: "Home Assistant",
      tools: ["turn_on"],
      ms: 42,
    });
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    const address = screen.getByPlaceholderText("https://example.com/mcp");
    fireEvent.change(address, { target: { value: "https://ha.example.com/mcp" } });
    fireEvent.click(screen.getByText("Test"));
    expect(await screen.findByText(/answered in 42 ms/)).toBeTruthy();

    fireEvent.change(address, { target: { value: "https://ha.example.com/other" } });
    expect(screen.queryByText(/answered in 42 ms/)).toBeNull();
    expect(addPlugin).not.toHaveBeenCalled();
  });

  it("clears a refusal when the key it was about changes", async () => {
    probeServer.mockRejectedValue({
      kind: "plugin",
      message:
        "Could not reach https://ha.example.com/mcp. No connection was made: the name may not resolve, the port may be closed, or the server may have refused the encrypted connection. Check the address and its path, and that the server is running and reachable from here.",
    });
    const { container } = render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://ha.example.com/mcp" },
    });
    fireEvent.click(screen.getByText("Test"));
    expect(await screen.findByText(/Could not reach/)).toBeTruthy();

    const key = container.querySelector<HTMLInputElement>('input[type="password"]');
    fireEvent.change(key!, { target: { value: "sk-new" } });
    expect(screen.queryByText(/Could not reach/)).toBeNull();
  });

  it("offers a tool to all agents without borrowing the default group's name", async () => {
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 2"));
    expect(screen.getByRole("button", { name: "Every agent: run_sql" }).textContent).toBe(
      "All agents",
    );
  });
});
