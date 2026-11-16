import { readFileSync } from "node:fs";
import { join } from "node:path";

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const host = vi.hoisted(() => ({ desktop: true, mac: true }));

vi.mock("../lib/transport", async (original) => ({
  ...(await original<typeof import("../lib/transport")>()),
  get desktop() {
    return host.desktop;
  },
}));

vi.mock("../lib/keybinds", async (original) => ({
  ...(await original<typeof import("../lib/keybinds")>()),
  get IS_MAC() {
    return host.mac;
  },
}));

const css = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

/** Where the sheet's head starts, as a rule in the stylesheet says it. */
function paddingLeft(selector: string): string | undefined {
  const escaped = selector.replace(/[.[\]]/g, "\\$&");
  const body = css.match(new RegExp(`^${escaped} \\{\\n([\\s\\S]*?)^\\}`, "m"))?.[1];
  return body?.match(/padding-left:\s*([^;]+);/)?.[1];
}

/** Read once at import, so each host needs its own copy of the module. */
async function sheetOn(desktop: boolean, mac: boolean) {
  host.desktop = desktop;
  host.mac = mac;
  vi.resetModules();
  const { GroupsSheet } = await import("./GroupsSheet");
  const noop = () => {};
  render(
    <GroupsSheet
      onClose={noop}
      onEditAgent={noop}
      onEditGroup={noop}
      onOpenCafeteria={noop}
      onOpenCalendar={noop}
      onOpenArtifacts={noop}
      onOpenSettings={noop}
      onOpenSearch={noop}
      onNewAgent={noop}
      onNewGroup={noop}
      onOpenMenu={noop}
    />,
  );
  return screen.getByRole("dialog", { name: "Groups and agents" }).querySelector("header")!;
}

describe("GroupsSheet", () => {
  afterEach(cleanup);

  it("keeps its head clear of a Mac's window buttons, as far as the top row does", async () => {
    // The sheet opens at the window's left edge, which on a Mac is where the
    // close, minimize and zoom buttons float over whatever is drawn there.
    const head = await sheetOn(true, true);
    expect(head.hasAttribute("data-lights")).toBe(true);
    expect(paddingLeft(".sheet__head[data-lights]")).toBeDefined();
    expect(paddingLeft(".sheet__head[data-lights]")).toBe(paddingLeft(".topbar[data-lights]"));
  });

  it("starts its head at the edge where there are no window buttons", async () => {
    expect((await sheetOn(false, true)).hasAttribute("data-lights")).toBe(false);
    cleanup();
    expect((await sheetOn(true, false)).hasAttribute("data-lights")).toBe(false);
  });
});
