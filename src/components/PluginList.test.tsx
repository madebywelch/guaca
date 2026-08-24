import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
import { PluginList } from "./PluginList";

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

vi.mock("../lib/ipc", () => ({
  api: {
    pluginCatalog: () => pluginCatalog(),
    groupPlugins: () => groupPlugins(),
    accountConnectors: () => accountConnectors(),
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

/**
 * The account's authorized identities.
 *
 * Rejecting by default, which is what an install with no Guaca account does.
 * The panel has to draw anyway: the account is optional and a plugin list that
 * waited on it would be blank for everybody who never signed in.
 */
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
    // Stripe's has no path, which is what the host line has to survive.
    endpoint: "https://mcp.stripe.com",
    accountBacked: false,
  },
];

/** One tool as the server published it, for whoever the plugin is for. */
function tool(name: string, access: PluginAccess = { mode: "everyone" }): PluginToolCard {
  return { name, description: `Runs ${name}.`, access };
}

/** The one state the old two-way switch could express: off for the crew. */
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
    hasComputer: false,
    hasBrowser: false,
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

    // No identity named: Neon signs in per group, and an account with none
    // connects against the account's default.
    await waitFor(() => expect(connectPlugin).toHaveBeenCalledWith(GROUP, "neon", undefined));
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

  it("keeps the tool list shut until it is asked for", async () => {
    // A crew with three plugins connected has sixty of these between the
    // operator and the button they came here for. The count on the line above
    // is what says whether opening one is worth it.
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("Show all 2")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Only chosen agents: run_sql" })).toBe(null);

    fireEvent.click(screen.getByText("Show all 2"));
    expect(screen.getByText("run_sql")).toBeTruthy();
    // The vendor's own sentence, because `execute` and `create_refund` are not
    // decisions anybody can make off the name alone.
    expect(screen.getByText("Runs run_sql.")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Every agent: run_sql" }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("says nothing per tool while a tool is the default", async () => {
    // Forty rows each repeating "offered to every agent in this group" is the
    // default said forty times, and it buries the one row that is not it.
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 2"));
    expect(screen.queryByRole("button", { name: "Revenue: run_sql" })).toBe(null);
  });

  it("switches one tool off, and back on again", async () => {
    // Narrowing to nobody is what off is now, and it is still one click. The
    // agents underneath are what the same click opens up.
    const off = plugin({ tools: [tool("run_sql", NOBODY), tool("create_branch")] });
    groupPlugins.mockResolvedValueOnce([plugin()]);
    groupPlugins.mockResolvedValue([off]);
    setPluginTool.mockResolvedValue(off);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 2"));
    fireEvent.click(screen.getByRole("button", { name: "Only chosen agents: run_sql" }));

    // The tool by name and the answer it should have, never a toggle: two
    // panels open on one group cannot swap a decision between them.
    await waitFor(() => expect(setPluginTool).toHaveBeenCalledWith("p1", "run_sql", NOBODY));
    // And the row draws what came back, not what was clicked.
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
    // The thing the crew-wide switch could not say. One sign-in, one inbox,
    // and the agent that triages it reads while the agent that answers it
    // sends.
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

    // And the whole set every time, never a difference, for the reason the
    // plugin above it sends the whole set: a merge on the far side would make
    // unticking impossible to express.
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
    // The two controls are set in either order, so this is a state an operator
    // passes through rather than a mistake. Drawing the tool as Scribe's when
    // Scribe cannot spend the sign-in would name an agent that gets refused,
    // which is the one thing a permission panel must not do.
    groupPlugins.mockResolvedValue([
      plugin({
        access: { mode: "chosen", agents: ["a1"] },
        tools: [tool("run_sql", { mode: "chosen", agents: ["a2"] })],
      }),
    ]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    fireEvent.click(await screen.findByText("Show all 1"));
    expect(screen.getByText(/nobody can call it: Scribe is not on this plugin/)).toBeTruthy();
  });

  it("says when a connected plugin has nothing left switched on", async () => {
    // The same state the crew-of-nobody row says out loud, one axis over: a
    // plugin that is signed in and can call nothing looks identical to a
    // working one until something says so.
    groupPlugins.mockResolvedValue([
      plugin({ tools: [tool("run_sql", NOBODY), tool("create_branch", NOBODY)] }),
    ]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/all switched off/)).toBeTruthy();
  });

  it("asks nothing about tools of a plugin nobody has connected", async () => {
    // There is nothing to switch off, and the question is noise on four tiles.
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    await screen.findAllByText("Connect");
    expect(screen.queryByText(/Show all/)).toBe(null);
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

/**
 * Two Google accounts on one Guaca account.
 *
 * The case this whole column exists for: a work Google and a personal one are
 * two grants, and two crews have to be able to use one each. Before this, both
 * reached whichever the service returned first.
 */
describe("choosing which account a crew uses", () => {
  /**
   * Matches against an element's whole text content.
   *
   * The default matcher reads only an element's direct text-node children, and
   * these sentences are built from several JSX interpolations, so it sees each
   * fragment separately and matches none of them.
   */
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

  it("says which account it acts as, without an inert picker, when there is one", async () => {
    // A select with a single option cannot do anything, but a crew acting as
    // somebody's mail must never leave them guessing whose.
    accountConnectors.mockResolvedValue(one);
    groupPlugins.mockResolvedValue([plugin({ kind: "google", connection: "acct_work" })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    await waitFor(() =>
      expect(screen.getByText(saying(/Acting as work@example.com/))).toBeTruthy(),
    );
    expect(screen.queryByLabelText(/^Account/)).toBeNull();
  });

  it("does not claim a group is acting as anyone before it is connected", async () => {
    // A group with nothing connected is not acting as anybody, and saying it is
    // reads as a grant that exists and does not.
    accountConnectors.mockResolvedValue(one);
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    await waitFor(() => expect(screen.getByText(saying(/Will connect as/))).toBeTruthy());
    expect(screen.queryByText(saying(/Acting as/))).toBeNull();
  });

  it("names the Guaca account the identities came from", async () => {
    // The failure that costs the most time is a machine signed in to the
    // account that does not hold the grant the operator is looking at in a
    // browser. Nothing else on this panel can tell those two apart.
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
    // The whole complaint: Connect took the first account silently, so a crew
    // could end up on the wrong mailbox with nothing on screen saying so.
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
    // Nothing is written until Connect: there is no row to write to yet.
    expect(setPluginConnection).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Connect"));
    await waitFor(() => expect(connectPlugin).toHaveBeenCalledWith(GROUP, "google", "acct_home"));
  });

  it("says so when the account has authorized nothing at that provider", async () => {
    // A hidden picker is indistinguishable from a broken feature.
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
    // Reconnecting would replace the row and lose the per-tool switches the
    // operator set, which is why this is its own command.
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
    // The account is optional. A panel that waited on guaca.bot would be blank
    // for everybody who never signed in.
    accountConnectors.mockRejectedValue(new Error("not signed in"));
    groupPlugins.mockResolvedValue([]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("Google")).toBeTruthy();
    expect(screen.queryByLabelText(/^Account/)).toBeNull();
  });
});

/**
 * A server nobody vouched for.
 *
 * The catalog rows above it are drawn from an offer, and these are drawn from
 * the row itself, because there is no offer: what it is called and where it is
 * come back from Rust with the plugin. Everything else on the row — who can use
 * it, which of its tools are whose — is the same code and is tested above.
 */
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

  it("draws it under the name its tools are called by, not the one that was typed", async () => {
    // The row shows what came back rather than predicting it, because the rule
    // that turns one into the other lives in Rust and a second copy of it here
    // would be a second place for it to be wrong.
    groupPlugins.mockResolvedValue([added()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("home_assistant")).toBeTruthy();
    expect(screen.getByText("ha.example.com")).toBeTruthy();
    // And it says out loud that nobody checked it, which is the whole of the
    // difference between this row and the six above it.
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

    // Refused: a crew's grant would cross the network in the open.
    addPlugin.mockRejectedValue(new Error("that is not https"));
    fireEvent.click(screen.getByText("Add and connect"));
    expect(await screen.findByText(/not https/)).toBeTruthy();
    // The form is still there, still filled in. A refusal that made the
    // operator retype the URL costs more than the mistake did.
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
    // Cleared only once it worked, and the row it produced is what is drawn.
    expect(await screen.findByText("home_assistant")).toBeTruthy();
    expect(screen.queryByPlaceholderText("Home Assistant")).toBeNull();
  });

  it("sends a pasted key only when there is one", async () => {
    // Empty means "ask the server what it wants", which is what a vendor's
    // server gets. Sending an empty string would be a key that authorizes
    // nothing, presented as though it were one.
    addPlugin.mockResolvedValue(added());
    render(<PluginList groupId={GROUP} crew={CREW} />);
    fireEvent.click(await screen.findByText("Add a server"));
    fireEvent.change(screen.getByPlaceholderText("Home Assistant"), {
      target: { value: "Vault" },
    });
    fireEvent.change(screen.getByPlaceholderText("https://example.com/mcp"), {
      target: { value: "https://vault.example.com/mcp" },
    });
    // Anchored: the label wraps the hint under it, so the accessible name is
    // the whole paragraph and an exact match would never find the box.
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
    // A local server changing port is the common case, and Disconnect plus Add
    // would take the per-tool switches with it.
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
        // Untouched, which is what keeps the stored set: a value cannot be read
        // back, so a reconnection that meant "replace" would drop it.
        undefined,
      ),
    );
    expect(disconnectPlugin).not.toHaveBeenCalled();
  });

  it("sends the headers that were typed, and never sends a name back as a value", async () => {
    // The third thing a server somebody runs can want, and the one nothing can
    // discover: an `X-API-Key` because that is where its framework looks.
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
    // Keyed by position instead, removing the first row hands its DOM node —
    // and whatever is in it — to the second, which is exactly when the operator
    // is looking at two boxes that both say something they did not type.
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
    // Both halves. The value is a credential and the panel must not be a place
    // to read one back; the name is what an operator debugging their own
    // server needs, because "is x-api-key on the request" is the question.
    groupPlugins.mockResolvedValue([added({ headers: ["x-api-key", "cf-access-client-id"] })]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/Sends x-api-key, cf-access-client-id/)).toBeTruthy();
  });

  it("leaves stored headers alone unless the operator replaces them", async () => {
    // A value cannot be read back, so a reconnection that meant "replace" would
    // drop it: the same rule a group's API key has. Replacing is a thing the
    // operator does on purpose, and the box starts with the names and no
    // values, because that is exactly what is knowable.
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
    // The whole point of the button: an operator who has to press Add to find
    // out what is wrong connects the same server four times.
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

    // The transport is the field that costs the most to guess at: a refusal on
    // the current one and a working event stream are the same sentence and
    // opposite instructions.
    expect(await screen.findByText(/HTTP\+SSE \(2024-11-05\)/)).toBeTruthy();
    expect(screen.getByText(/1 tool: turn_on/)).toBeTruthy();
    // And it connected nothing.
    expect(addPlugin).not.toHaveBeenCalled();
  });

  it("keeps a refusal on screen rather than clearing the form it was about", async () => {
    // `run` reloads the panel and drops what it was told; a test has to keep
    // its answer on a failure, because a server that refused is what somebody
    // pressed this to find out.
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

  it("tests a connected plugin against the sign-in it already has", async () => {
    // The question a crew notices as an agent reporting a tool it cannot call,
    // and the only other way to ask it is to connect again, which replaces the
    // tool list and opens a browser.
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
    // A name here is an operator's word, and the prototype chain answers to
    // several of them. A plain lookup with a fallback finds a truthy value that
    // is not a brand for `constructor`, so the fallback never fires and the row
    // draws an empty square, which reads as a broken row rather than an added
    // server.
    groupPlugins.mockResolvedValue([added({ kind: "constructor", name: "constructor" })]);
    const { container } = render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText("constructor")).toBeTruthy();
    const marks = [...container.querySelectorAll(".mark__icon path")];
    expect(marks.every((path) => path.getAttribute("d"))).toBe(true);
  });

  it("offers no address control on a server Guaca ships", async () => {
    // Where a vendor's server lives is not an operator setting: it is a
    // decision the build makes, and a box implying otherwise is a way to point
    // a crew's sign-in somewhere nobody chose.
    groupPlugins.mockResolvedValue([plugin()]);
    render(<PluginList groupId={GROUP} crew={CREW} />);

    expect(await screen.findByText(/2 tools/)).toBeTruthy();
    expect(screen.queryByText("Change address")).toBeNull();
  });
});
