import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentAvatar, knockAway } from "./AgentAvatar";
import { CastContext } from "./cast";
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
        <AgentAvatar avatar={avatar} color="#7293aa" mood="working" cast="cut" />
        <AgentAvatar avatar="drop" color="#a2ada1" mood="pleased" cast="cut" />
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
      <AgentAvatar avatar="slab" color="#7293aa" mood="blocked" look="down" cast="cut" />,
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

    rerender(<AgentAvatar avatar="slab" color="#7293aa" mood="paused" cast="cut" />);
    expect(container.querySelector(".avatar")?.getAttribute("data-mood")).toBe("paused");
    expect(container.querySelector(".avatar__halo")).toBeNull();
    expect(container.querySelector(".avatar__z")).not.toBeNull();
  });
});

describe("the two casts", () => {
  it("draws cut eyes: a white, a pupil and a lid line for each", () => {
    const { container } = render(
      <AgentAvatar avatar="orb" color="#7293aa" mood="idle" cast="cut" />,
    );
    expect(container.querySelector('[data-cast="cut"]')).not.toBeNull();
    expect(container.querySelectorAll(".avatar__white")).toHaveLength(2);
    expect(container.querySelectorAll(".avatar__pupil")).toHaveLength(2);
    for (const white of container.querySelectorAll(".avatar__white")) {
      expect(white.getAttribute("d")).toMatch(/^M/);
    }
    // The pupil is clipped to the white it sits in, and to nothing else.
    const pupil = container.querySelector(".avatar__pupil");
    const clip = pupil?.parentElement?.getAttribute("clip-path")?.match(/url\(#(.+)\)/)?.[1];
    const opening = container.querySelector(`[id="${clip}"] > path`);
    expect(opening?.getAttribute("d")).toBe(
      container.querySelector(".avatar__white")?.getAttribute("d"),
    );
  });

  it("hides the second eye of a one-eyed character rather than drawing it anywhere", () => {
    const { container } = render(
      <AgentAvatar avatar="cell" color="#7293aa" mood="idle" cast="cut" />,
    );
    const eyes = container.querySelectorAll(".avatar__eyes > g");
    expect(eyes).toHaveLength(2);
    expect(eyes[0]?.getAttribute("display")).toBeNull();
    expect(eyes[1]?.getAttribute("display")).toBe("none");
  });

  // The operator's choice is read by every creature on screen, so changing it
  // is one write and every face redraws; a preview can still ask for the other.
  it("follows the operator's choice, and lets a preview ask for the other one", () => {
    const crew = (cast: "cut" | "drawn") => (
      <CastContext.Provider value={cast}>
        <AgentAvatar avatar="orb" color="#7293aa" mood="thinking" />
        <AgentAvatar avatar="orb" color="#7293aa" mood="thinking" cast="cut" />
      </CastContext.Provider>
    );
    const { container, rerender } = render(crew("cut"));
    expect(container.querySelectorAll('[data-cast="cut"]')).toHaveLength(2);

    rerender(crew("drawn"));
    const [chosen, pinned] = Array.from(container.querySelectorAll(".avatar"));
    expect(chosen?.getAttribute("data-cast")).toBe("drawn");
    expect(pinned?.getAttribute("data-cast")).toBe("cut");
    // Drawn: a pigment, an edge and a face, all written on the first paint.
    expect(chosen?.querySelector(".avatar__edge:last-of-type")?.getAttribute("d")).toMatch(/^M/);
    expect(chosen?.querySelector(".avatar__brush path")?.getAttribute("d")).toMatch(/^M/);
    expect(chosen?.querySelector(".avatar__white")).toBeNull();
    // The thinking dots are the drawn cast's own, on twos, in the mark group.
    expect(chosen?.querySelector(".avatar__mark")?.innerHTML).toContain("circle");
  });

  // Outside the app there is no choice to read, and the default is the one an
  // operator who never opened Settings has.
  it("draws the drawn cast where nothing says otherwise", () => {
    const { container } = render(<AgentAvatar avatar="orb" color="#7293aa" />);
    expect(container.querySelector(".avatar")?.getAttribute("data-cast")).toBe("drawn");
  });

  // Nothing drawn outside the circle a crew is seated in, which the cut cast
  // gets from its relief's clip and the drawn one from its own.
  it("clips the drawn body to the reach", () => {
    const { container } = render(
      <AgentAvatar avatar="slab" color="#7293aa" mood="frustrated" cast="drawn" />,
    );
    const body = container.querySelector(".avatar__drawn");
    const id = body?.getAttribute("clip-path")?.match(/url\(#(.+)\)/)?.[1];
    const circle = container.querySelector(`[id="${id}"] circle`);
    expect(circle?.getAttribute("r")).toBe(String(FORM.reach));
  });
});

describe("a directed gaze", () => {
  const pupils = (container: HTMLElement) =>
    Array.from(container.querySelectorAll(".avatar__pupil")).map((p) =>
      Number(p.getAttribute("cx")),
    );

  // The pair in the rail's corner looks at each other, which is across, and
  // an aimed look only goes up or down.
  it("looks where it is told, across as well as up and down", () => {
    const at = (gaze: [number, number] | null) =>
      pupils(
        render(<AgentAvatar avatar="slab" color="#7293aa" mood="idle" gaze={gaze} cast="cut" />)
          .container,
      );
    const ahead = at(null);
    const right = at([0.36, 0]);
    const left = at([-0.36, 0]);
    expect(right).toHaveLength(2);
    for (const [i, x] of right.entries()) expect(x).toBeGreaterThan(ahead[i] as number);
    for (const [i, x] of left.entries()) expect(x).toBeLessThan(ahead[i] as number);
  });

  it("gives way to an aimed look, which is for somebody", () => {
    const aimed = pupils(
      render(<AgentAvatar avatar="slab" color="#7293aa" mood="idle" look="down" cast="cut" />)
        .container,
    );
    const both = pupils(
      render(
        <AgentAvatar
          avatar="slab"
          color="#7293aa"
          mood="idle"
          look="down"
          gaze={[0.36, 0]}
          cast="cut"
        />,
      ).container,
    );
    expect(aimed).toHaveLength(2);
    expect(both).toEqual(aimed);
  });
});

describe("the knock", () => {
  it("shoves exactly as it did for an aimed look, or for none", () => {
    expect(knockAway("down", null)).toEqual([0, -1]);
    expect(knockAway("up", null)).toEqual([0, 1]);
    expect(knockAway(null, null)).toEqual([0, 1]);
    expect(knockAway("down", [0.36, 0])).toEqual([0, -1]);
  });

  it("shoves away from whoever a directed gaze is on", () => {
    const [x, y] = knockAway(null, [-0.36, 0]);
    expect(x).toBeCloseTo(1);
    expect(y).toBeCloseTo(0);
    // A gaze held at the operator is barely a direction, and is not one.
    expect(knockAway(null, [0, 0.02])).toEqual([0, 1]);
  });
});
