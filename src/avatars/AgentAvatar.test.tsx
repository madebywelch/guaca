import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentAvatar } from "./AgentAvatar";
import { FORM } from "./form";

vi.mock("./clock", async (original) => ({
  ...(await original<typeof import("./clock")>()),
  join: () => () => {},
}));
vi.mock("../lib/motion", () => ({ prefersReducedMotion: () => true }));

afterEach(cleanup);

describe("paper relief", () => {
  it("keeps each creature's layers on its own outline when a neighbor changes", () => {
    const crew = (avatar: string) => (
      <>
        <AgentAvatar avatar={avatar} color="#7293aa" mood="working" />
        <AgentAvatar avatar="drop" color="#a2ada1" mood="pleased" />
      </>
    );
    const { container, rerender } = render(crew("orb"));
    const outlines = Array.from(container.querySelectorAll("defs > path"));
    expect(outlines).toHaveLength(2);
    expect(new Set(outlines.map((path) => path.id)).size).toBe(2);
    const before = outlines.map((path) => path.getAttribute("d"));
    expect(before.every((d) => d?.startsWith("M"))).toBe(true);

    rerender(crew("knot"));
    expect(outlines[0]?.getAttribute("d")).not.toBe(before[0]);
    expect(outlines[1]?.getAttribute("d")).toBe(before[1]);
    for (const path of outlines) {
      const layers = path.closest("svg")?.querySelectorAll("use");
      expect(layers).toHaveLength(3);
      for (const layer of layers ?? []) {
        expect(layer.getAttribute("href")).toBe(`#${path.id}`);
        expect(container.querySelector(`[id="${path.id}"]`)).toBe(path);
      }
    }
  });

  it("contains the relief without clipping a request for the operator", () => {
    const { container, rerender } = render(
      <AgentAvatar avatar="slab" color="#7293aa" mood="blocked" look="down" />,
    );
    const skin = container.querySelector(".avatar__skin");
    const clip = container.querySelector("clipPath");
    expect(skin?.getAttribute("clip-path")).toBe(`url(#${clip?.id})`);
    const circle = clip?.querySelector("circle");
    expect(circle?.getAttribute("cx")).toBe(String(FORM.center));
    expect(circle?.getAttribute("cy")).toBe(String(FORM.center));
    expect(circle?.getAttribute("r")).toBe(String(FORM.reach));
    expect(container.querySelector(".avatar__halo")).not.toBeNull();
    expect(skin?.querySelector(".avatar__mark")).toBeNull();

    rerender(<AgentAvatar avatar="slab" color="#7293aa" mood="paused" />);
    expect(container.querySelector(".avatar")?.getAttribute("data-mood")).toBe("paused");
    expect(container.querySelector(".avatar__halo")).toBeNull();
    expect(container.querySelector(".avatar__z")).not.toBeNull();
  });
});
