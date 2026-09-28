import { useEffect, useReducer, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { ACCENTS } from "../avatars/catalog";
import type { Point } from "../avatars/form";
import { brandAt, toward } from "../lib/brand";
import { prefersReducedMotion } from "../lib/motion";

/**
 * The icon's two, in the icon's two colors: a pigment every agent can have,
 * and ink, which none can. The ink is a rail token because the icon's ink
 * body disappears against the dark rail, and paper is its opposite there.
 */
const SEATS = [
  { seed: "guaca-left", color: ACCENTS.find((a) => a.name === "Terracotta")?.value ?? "" },
  { seed: "guaca-right", color: "var(--rail-pair)" },
] as const;

/**
 * The corner of the rail: the icon's pair, and the name while it is worth
 * reading. `lib/brand.ts` decides what happens when; this renders at the beats
 * it names and at nothing in between, and turns a look at the pointer into a
 * direction, which only the page can do.
 *
 * The name stays in the document when it is folded away, so the rail is named
 * for a screen reader whether or not anybody is pointing at it.
 */
export function Brand({ waiting }: { waiting: boolean }) {
  const born = useRef(performance.now());
  const [, beat] = useReducer((n: number) => n + 1, 0);
  const [hoverAt, setHoverAt] = useState<number | null>(null);
  const [aims, setAims] = useState<[Point, Point]>([
    [0, 0],
    [0, 0],
  ]);
  const left = useRef<HTMLSpanElement>(null);
  const right = useRef<HTMLSpanElement>(null);
  const frame = useRef(0);

  const since = () => (performance.now() - born.current) / 1000;
  const t = since();
  const scene = brandAt(t, { hoverAt, waiting, still: prefersReducedMotion() });

  useEffect(() => {
    if (!Number.isFinite(scene.next)) return;
    const id = window.setTimeout(beat, Math.max(0, (scene.next - t) * 1000));
    return () => window.clearTimeout(id);
  });
  useEffect(() => () => cancelAnimationFrame(frame.current), []);

  /* Once a frame at most, and only while the pointer is over the corner. */
  const aim = (x: number, y: number) => {
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(() => {
      const at = (seat: HTMLSpanElement | null): Point =>
        seat ? toward(seat.getBoundingClientRect(), x, y) : [0, 0];
      setAims([at(left.current), at(right.current)]);
    });
  };

  const pair = [
    { cue: scene.left, ref: left, seat: SEATS[0], aim: aims[0] },
    { cue: scene.right, ref: right, seat: SEATS[1], aim: aims[1] },
  ];
  return (
    <span
      className="brand"
      data-named={scene.named ? "" : undefined}
      onPointerEnter={(e) => {
        setHoverAt(since());
        aim(e.clientX, e.clientY);
      }}
      onPointerMove={(e) => aim(e.clientX, e.clientY)}
      onPointerLeave={() => {
        cancelAnimationFrame(frame.current);
        setHoverAt(null);
      }}
    >
      <span className="brand__pair" aria-hidden="true">
        {pair.map(({ cue, ref, seat, aim: at }) => (
          <span
            key={seat.seed}
            ref={ref}
            className="brand__seat"
            data-shown={cue.shown ? "" : undefined}
          >
            <AgentAvatar
              avatar="slab"
              color={seat.color}
              size="xs"
              mood={cue.mood}
              gaze={cue.gaze === "pointer" ? at : cue.gaze}
              gesture={cue.gesture}
              seed={seat.seed}
              title=""
            />
          </span>
        ))}
      </span>
      <span className="brand__name">Guaca</span>
    </span>
  );
}
