/**
 * One of the cast, directed rather than fed agent signals.
 *
 * `AgentAvatar` decides a creature's mood from what the runtime says and can
 * aim a look up or down, which is everything an agent in a rail needs. A brand
 * needs two things it cannot do: look sideways at a peer or at the pointer, and
 * become another member of the cast mid-performance. So this is the same loop
 * in miniature, over the same painters (`CutArt`, `DrawnArt`), the same body,
 * moods, gaze smoother and knock, with a cue written by a director in place of
 * `moodFor`. Nothing is drawn here that the app does not already draw.
 */

import { type CSSProperties, useEffect, useRef } from "react";

import { CutArt } from "../../src/avatars/CutArt";
import { DrawnArt } from "../../src/avatars/DrawnArt";
import { gaitOf, join } from "../../src/avatars/clock";
import { prefersReducedMotion } from "../../src/lib/motion";
import { AIM, gazeAt, settle } from "../../src/avatars/eyes";
import { FORM, type Lump, type Point } from "../../src/avatars/form";
import type { Gesture, Paint } from "../../src/avatars/frame";
import { MOODS, MORPH, type Mood } from "../../src/avatars/moods";
import { SILHOUETTES } from "../../src/avatars/silhouette";

export type Look = Point | "pointer" | null;

/** What the director asks of one creature, right now. */
export interface Cue {
  mood: Mood;
  /** Where to look, in body radii. Null is the mood's own glances. */
  look: Look;
  /** A throw or a catch, and the scene time it began; a new time is a new one. */
  gesture: { kind: Exclude<Gesture, null>; at: number } | null;
  shown: boolean;
  /** Becoming another member of the cast: from one, to another, how far. */
  morph?: { from: Lump; to: Lump; u: number; colors: [string, string] };
}

export const REST_CUE: Cue = { mood: "idle", look: null, gesture: null, shown: true };

/** What a throw and a catch do to the mass, in body radii. `AgentAvatar`'s. */
const KNOCK = {
  send: { amp: 0.26, hz: 1.5, decay: 0.28, life: 0.9 },
  receive: { amp: 0.3, hz: 2.2, decay: 0.3, life: 0.8 },
};

/** How far a look at the pointer goes. The idle mood's own range, so no face does more than it already does. */
const REACH = { across: 0.38, up: AIM.up, down: AIM.down };

const pointer = { x: Number.NaN, y: Number.NaN };
if (typeof window !== "undefined") {
  addEventListener("pointermove", (e) => {
    pointer.x = e.clientX;
    pointer.y = e.clientY;
  });
  document.addEventListener("pointerleave", () => {
    pointer.x = Number.NaN;
  });
}

function toward(el: Element): Point | null {
  if (Number.isNaN(pointer.x)) return null;
  const box = el.getBoundingClientRect();
  const dx = pointer.x - (box.left + box.width / 2);
  const dy = pointer.y - (box.top + box.height / 2);
  const d = Math.hypot(dx, dy) || 1;
  /* A pointer on the face is looked at straight on; one a hand away gets the
     whole of the look. Past that the direction is all that changes. */
  const k = Math.min(1, d / (box.width * 2.5));
  return [
    (dx / d) * REACH.across * k,
    Math.max(-REACH.up, Math.min(REACH.down, (dy / d) * REACH.across * k)),
  ];
}

let morphs = 0;
const lerp = (a: number, b: number, u: number) => a + (b - a) * u;

/** A character partway to another: one outline function, and every number between. */
function between(key: string, a: Lump, b: Lump, u: number): Lump {
  const table = SILHOUETTES as Record<string, (angle: number) => number>;
  const from = table[a.form] as (angle: number) => number;
  const to = table[b.form] as (angle: number) => number;
  table[key] = (angle) => lerp(from(angle), to(angle), u);
  return {
    key: u < 0.5 ? a.key : b.key,
    label: "",
    form: key as Lump["form"],
    ax: lerp(a.ax, b.ax, u),
    ay: lerp(a.ay, b.ay, u),
    sig: [
      ...a.sig.map((l) => ({ ...l, amp: l.amp * (1 - u) })),
      ...b.sig.map((l) => ({ ...l, amp: l.amp * u })),
    ],
    eye: {
      spread: lerp(a.eye.spread, b.eye.spread, u),
      r: lerp(a.eye.r, b.eye.r, u),
      x: lerp(a.eye.x ?? 0, b.eye.x ?? 0, u),
      y: lerp(a.eye.y ?? 0, b.eye.y ?? 0, u),
    },
  };
}

function mix(a: string, b: string, u: number): string {
  const p = (h: string) => [1, 3, 5].map((i) => Number.parseInt(h.slice(i, i + 2), 16));
  const [x, y] = [p(a), p(b)];
  return `#${x.map((v, i) => Math.round(lerp(v, y[i] as number, u)).toString(16).padStart(2, "0")).join("")}`;
}

interface Cell {
  mood: Mood;
  from: Mood;
  at: number;
  gaze: Point;
  vel: Point;
  last: number;
  gesture: Gesture;
  gestureAt: number;
  gestureKey: number;
  aim: string;
  aimAt: number;
}

export function Actor({
  lump,
  color,
  px,
  cast,
  cue,
  seed,
}: {
  lump: Lump;
  color: string;
  /** Drawn size in CSS pixels; line weights are hinted against it. */
  px: number;
  cast: "cut" | "drawn";
  /** Read every frame, written by the director. Never a render. */
  cue: { current: Cue };
  seed: string;
}) {
  const box = useRef<HTMLSpanElement>(null);
  const art = useRef<Paint | null>(null);
  const morphKey = useRef(`morph-${++morphs}`);
  const cell = useRef<Cell>({
    mood: "idle",
    from: "idle",
    at: Number.NEGATIVE_INFINITY,
    gaze: [0, 0],
    vel: [0, 0],
    last: 0,
    gesture: null,
    gestureAt: 0,
    gestureKey: Number.NaN,
    aim: "",
    aimAt: Number.NEGATIVE_INFINITY,
  });
  const gait = gaitOf(seed);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const paint = (seconds: number, live: boolean) => {
      const want = cue.current;
      const c = cell.current;
      const t = seconds * gait.rate + gait.phase;

      el.style.opacity = want.shown ? "1" : "0";

      if (want.mood !== c.mood) {
        c.from = c.mood;
        c.mood = want.mood;
        c.at = seconds;
        el.dataset.mood = want.mood;
      }
      if (want.gesture && want.gesture.at !== c.gestureKey) {
        c.gestureKey = want.gesture.at;
        c.gesture = want.gesture.kind;
        c.gestureAt = seconds;
      }

      let aimed: Point | null = null;
      if (want.look === "pointer") aimed = toward(el);
      else if (want.look) aimed = want.look;
      const aim = want.look === "pointer" ? "pointer" : want.look ? want.look.join() : "";
      if (aim !== c.aim) {
        c.aim = aim;
        c.aimAt = seconds;
      }

      const since = seconds - c.at;
      const raw = live ? Math.min(1, since / MORPH) : 1;
      const u = raw * raw * (3 - 2 * raw);
      let target: Point;
      if (aimed) target = aimed;
      else {
        const a = gazeAt(t, MOODS[c.from].watch?.gaze);
        const b = gazeAt(t, MOODS[c.mood].watch?.gaze);
        target = [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u];
      }
      const dt = live ? Math.min(0.05, Math.max(0, seconds - c.last)) : 0;
      c.last = seconds;
      if (live) settle(c, target, dt);
      else c.gaze = [target[0], target[1]];
      const eyes: Point = [c.gaze[0], c.gaze[1]];
      const body: Point = [c.gaze[0], c.gaze[1]];

      /* Away from whoever the look is at: a throw recoils against itself and a
         catch is shoved back from the thrower. With nobody looked at, down. */
      if (c.gesture) {
        const knock = KNOCK[c.gesture];
        const age = seconds - c.gestureAt;
        if (age > knock.life) c.gesture = null;
        else if (live) {
          const wave =
            c.gesture === "send"
              ? Math.sin(age * knock.hz * Math.PI * 2)
              : Math.cos(age * knock.hz * Math.PI * 2);
          const push = knock.amp * wave * Math.exp(-age / knock.decay);
          const l = aimed ? Math.hypot(aimed[0], aimed[1]) : 0;
          const away: Point = aimed && l > 0.05 ? [-aimed[0] / l, -aimed[1] / l] : [0, 1];
          body[0] += away[0] * push;
          body[1] += away[1] * push;
          eyes[0] += away[0] * push * 0.35;
          eyes[1] += away[1] * push * 0.35;
        }
      }

      let drawn = lump;
      if (want.morph) {
        const m = want.morph;
        drawn = between(morphKey.current, m.from, m.to, m.u);
        el.style.setProperty("--accent", mix(m.colors[0], m.colors[1], m.u));
      } else {
        el.style.setProperty("--accent", color);
      }

      art.current?.({
        lump: drawn,
        t,
        dt,
        live,
        mood: c.mood,
        from: c.from,
        since: live ? since : Number.POSITIVE_INFINITY,
        /* Any aimed look quiets the mood's own saccades, in both casts. The
           value is not otherwise read for geometry. */
        look: aimed ? "down" : null,
        sinceLook: seconds - c.aimAt,
        gesture: c.gesture,
        sinceGesture: seconds - c.gestureAt,
        eyes,
        body,
        px,
      });
    };
    /* The shared clock stops under reduced motion and leaves every creature
       as it was last painted. An agent is repainted when its props change; a
       directed one is repainted when its cue does, still, on its own frame. */
    if (!prefersReducedMotion()) return join(el, paint);
    let frame = 0;
    let last = "";
    const still = () => {
      frame = requestAnimationFrame(still);
      const now = JSON.stringify(cue.current);
      if (now !== last) {
        last = now;
        paint(performance.now() / 1000, false);
      }
    };
    still();
    return () => cancelAnimationFrame(frame);
  }, [cue, gait.phase, gait.rate, lump, color, px]);

  const style = { width: px, height: px, "--accent": color } as CSSProperties;
  return (
    <span ref={box} className="avatar bm-actor" data-cast={cast} style={style}>
      <svg viewBox={`0 0 ${FORM.box} ${FORM.box}`} className="avatar__body" aria-hidden="true">
        {cast === "drawn" ? <DrawnArt paint={art} /> : <CutArt paint={art} />}
      </svg>
    </span>
  );
}
