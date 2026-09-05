import { type CSSProperties, useEffect, useRef } from "react";

import { prefersReducedMotion } from "../lib/motion";
import type { LiveCall } from "../lib/trail";
import type { Activity, Lifecycle } from "../lib/types";
import { lookupCharacter } from "./catalog";
import { gaitOf, join, type Painter } from "./clock";
import { AIM, aimedEye, blendEyes, type Drawn, eyePath, eyesAt, gazeAt, settle } from "./eyes";
import { blend, bodyPoints, FORM, outline, type Point } from "./form";
import { MOODS, type Mood, markFor, moodFor } from "./moods";
import { Skin } from "./Skin";

/** Where the character is looking. Used to make a send visibly aimed at someone. */
export type Look = "up" | "down" | null;

/** A one-off reaction: winding up to throw, or being hit by something. */
export type Gesture = "send" | "receive" | null;

interface Props {
  avatar: string;
  color: string;
  size?: "xs" | "sm" | "md" | "lg";
  /** What the runtime says it is doing. `moods.ts` turns these into a face. */
  activity?: Activity;
  lifecycle?: Lifecycle;
  /** Its live tool calls, which say whether work is in flight or going badly. */
  work?: LiveCall[];
  /** An escalation of its own is open on the desk. */
  escalated?: boolean;
  /** When its last reply landed. Worth a couple of seconds of looking pleased. */
  finishedAt?: number;
  /** Overrides everything above. For a preview, where there is no agent to read. */
  mood?: Mood;
  /** Where its own clock starts and how fast it runs. Pass the agent id. */
  seed?: string;
  look?: Look;
  gesture?: Gesture;
  /** A short shout shown in a bubble, e.g. "!" when a message is thrown. */
  says?: string | null;
  title?: string;
}

/** How long one mood takes to become another. */
const MORPH = 0.6;

/** What a throw and a catch do to the mass, in body radii. */
const KNOCK = {
  send: { amp: 0.26, hz: 1.5, decay: 0.28, life: 0.9 },
  receive: { amp: 0.3, hz: 2.2, decay: 0.3, life: 0.8 },
};

/** Everything one avatar remembers between frames. */
interface Cell {
  mood: Mood;
  from: Mood;
  /**
   * When the change to `mood` began, and when a gesture did, both on the shared
   * clock rather than the creature's own: how quickly a face reacts is not
   * allowed to be a property of its id.
   */
  at: number;
  /** Where the look has got to, and how fast, in body radii. */
  gaze: Point;
  vel: Point;
  last: number;
  gesture: Gesture;
  gestureAt: number;
  /** Which mood's mark the group is currently holding. */
  marked: Mood | null;
}

/**
 * An agent's character.
 *
 * The drawing is `form.ts` and `eyes.ts`; the table of expressions is
 * `moods.ts`. This owns the one thing neither of those can: what is true right
 * now. Every frame it decides the mood from the signals it was given, moulds
 * the last mood into it, follows the gaze with the mass, and writes three
 * attributes. Nothing here re-renders: a mood that lasts two seconds and a
 * message landing are both decided inside the loop, so a rail of a dozen agents
 * reacting to each other costs React nothing at all.
 */
export function AgentAvatar({
  avatar,
  color,
  size = "md",
  activity,
  lifecycle = "active",
  work,
  escalated,
  finishedAt,
  mood,
  seed,
  look = null,
  gesture = null,
  says = null,
  title,
}: Props) {
  const character = lookupCharacter(avatar);

  const box = useRef<HTMLSpanElement>(null);
  const skin = useRef<SVGPathElement>(null);
  const eyes = useRef<SVGGElement>(null);
  const mark = useRef<SVGGElement>(null);
  const cell = useRef<Cell>({
    mood: "idle",
    from: "idle",
    at: -MORPH,
    gaze: [0, 0],
    vel: [0, 0],
    last: 0,
    gesture: null,
    gestureAt: 0,
    marked: null,
  });

  /* Read by the painter rather than closed over, so the loop never holds a
     stale render's props and never has to be re-registered to see new ones. */
  const props = useRef({
    activity,
    lifecycle,
    work,
    escalated,
    finishedAt,
    mood,
    look,
    gesture,
    character,
  });

  /* Its own clock, so a crew of idle agents is not one animal breathing. Only
     the cycles below are on it; every age is measured on the shared seconds. */
  const gait = gaitOf(seed ?? avatar);

  const paint = useRef<Painter>(() => {});
  paint.current = (seconds, live) => {
    const now = props.current;
    const lump = now.character;
    const body = skin.current;
    const face = eyes.current;
    if (!body || !face) return;

    const state = cell.current;
    const t = seconds * gait.rate + gait.phase;

    /* A catch is what makes an agent look surprised, and it is the only
       transient the component raises for itself. */
    if (now.gesture !== state.gesture) {
      state.gesture = now.gesture;
      state.gestureAt = seconds;
    }
    const struckAt =
      state.gesture === "receive" ? Date.now() - (seconds - state.gestureAt) * 1000 : undefined;

    const want =
      now.mood ??
      moodFor(
        {
          activity: now.activity,
          lifecycle: now.lifecycle,
          work: now.work,
          escalated: now.escalated,
          finishedAt: now.finishedAt,
          struckAt,
        },
        Date.now(),
      );
    if (want !== state.mood) {
      state.from = state.mood;
      state.mood = want;
      state.at = seconds;
    }
    /* Both of these are structure rather than geometry, so they are written on
       a change of mood and not on every frame. */
    if (state.marked !== state.mood) {
      state.marked = state.mood;
      if (mark.current) mark.current.innerHTML = markFor(state.mood);
      box.current?.setAttribute("data-mood", state.mood);
    }

    const to = MOODS[state.mood];
    const from = MOODS[state.from];
    const raw = live ? Math.min(1, (seconds - state.at) / MORPH) : 1;
    const u = raw * raw * (3 - 2 * raw);

    /* Where it is asked to look. An aimed look at a peer outranks whatever the
       mood would have done, because the point of that look is who it is for. */
    let eyeGaze: Point;
    if (now.look) {
      eyeGaze = [0, now.look === "up" ? -AIM.up : AIM.down];
    } else if (!live) {
      eyeGaze = [0, 0];
    } else {
      const a = gazeAt(t, from.watch?.gaze);
      const b = gazeAt(t, to.watch?.gaze);
      eyeGaze = [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u];
    }

    /* One look, smoothed once, read by the eyes and the body at the same
       instant, so the body is pulled as the eyes go rather than after they
       went. `eyes.ts` owns the smoother, and the outline is bounded whatever
       it is handed. */
    const dt = live ? Math.min(0.05, Math.max(0, seconds - state.last)) : 0;
    state.last = seconds;
    if (live) settle(state, eyeGaze, dt);
    else {
      state.gaze = [eyeGaze[0], eyeGaze[1]];
      state.vel = [0, 0];
    }
    eyeGaze = [state.gaze[0], state.gaze[1]];
    const bodyGaze: Point = [state.gaze[0], state.gaze[1]];

    /* A throw and a catch are displacements, not transforms: the creature is
       pulled away and comes back, and its outline is what shows it. */
    if (state.gesture) {
      const knock = KNOCK[state.gesture];
      const age = seconds - state.gestureAt;
      if (age > knock.life) {
        state.gesture = null;
      } else if (live) {
        const wave =
          state.gesture === "send"
            ? Math.sin(age * knock.hz * Math.PI * 2)
            : Math.cos(age * knock.hz * Math.PI * 2);
        /* Away from the peer, which is the one direction both gestures need:
           a parcel thrown from above presses the creature down and a throw
           recoils against itself. The look is what says where the peer is, so
           an exchange whose other end is not drawn in the rail falls back to
           down, which is the way an unexplained shove reads best. */
        const away = now.look === "down" ? -1 : 1;
        const push = away * knock.amp * wave * Math.exp(-age / knock.decay);
        bodyGaze[1] += push;
        eyeGaze[1] += push * 0.35;
      }
    }

    const shape = bodyPoints(lump, to.shape, t, bodyGaze);
    const pts =
      u >= 1 ? shape.pts : blend(bodyPoints(lump, from.shape, t, bodyGaze).pts, shape.pts, u);
    body.setAttribute("d", outline(pts));

    const blended = u >= 1 ? to.eye : blendEyes(from.eye, to.eye, u);
    /* An aimed look is moulded in on top of the mood rather than replacing it,
       so a creature that is thinking still looks like it is thinking while it
       watches a message go. `eyes.ts` has the argument for why the offset alone
       was not enough to read as looking anywhere. */
    const eye = now.look ? aimedEye(blended, now.look) : blended;
    const watch = u > 0.5 ? to.watch : from.watch;
    write(face, eyesAt(lump, eye, watch, t, live, eyeGaze));
  };

  /* Joined once. The painter is stable, so nothing here is torn down and set up
     again on a render, and the observer is not asked the same question twice. */
  useEffect(() => {
    const el = box.current;
    return el ? join(el, (seconds, live) => paint.current(seconds, live)) : undefined;
  }, []);

  /* And painted again after every render, at the time the clock last reached,
     so a change of props is on screen whether or not anything is moving. */
  useEffect(() => {
    props.current = {
      activity,
      lifecycle,
      work,
      escalated,
      finishedAt,
      mood,
      look,
      gesture,
      character,
    };
    paint.current(cell.current.last, !prefersReducedMotion());
  });

  /* The phase is handed to CSS as well, because the marks beside a head loop
     there and would otherwise run on the document's clock, which every creature
     shares. `styles.css` explains what that looks like. */
  const style = { "--accent": color, "--gait": `${gait.phase.toFixed(2)}s` } as CSSProperties;

  return (
    <span
      ref={box}
      className={`avatar avatar--${size}`}
      style={style}
      title={title ?? character.label}
    >
      <svg viewBox={`0 0 ${FORM.box} ${FORM.box}`} className="avatar__body" aria-hidden="true">
        <Skin pathRef={skin} />
        <g
          ref={eyes}
          className="avatar__eyes"
          fill="none"
          stroke="var(--eye)"
          strokeLinecap="round"
        >
          <path />
          <path />
        </g>
        <g ref={mark} className="avatar__mark" />
      </svg>
      {says && (
        <span className="avatar__says" aria-hidden="true">
          {says}
        </span>
      )}
    </span>
  );
}

/** Two path elements, made once and written to every frame. */
function write(group: SVGGElement, drawn: Drawn[]) {
  const paths = group.children;
  for (let i = 0; i < paths.length; i++) {
    const path = paths[i] as SVGPathElement;
    const eye = drawn[i];
    if (!eye) {
      path.setAttribute("d", "");
      continue;
    }
    path.setAttribute("d", eyePath(eye));
    path.setAttribute("stroke-width", eye.h.toFixed(2));
  }
}
