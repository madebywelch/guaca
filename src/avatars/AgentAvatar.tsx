import { type CSSProperties, useContext, useEffect, useRef } from "react";

import { prefersReducedMotion } from "../lib/motion";
import type { Cast } from "../lib/prefs";
import type { LiveCall } from "../lib/trail";
import type { Activity, Lifecycle } from "../lib/types";
import { CutArt } from "./CutArt";
import { CastContext } from "./cast";
import { lookupCharacter } from "./catalog";
import { gaitOf, join, type Painter } from "./clock";
import { DrawnArt } from "./DrawnArt";
import { AIM, gazeAt, settle } from "./eyes";
import { FORM, type Point } from "./form";
import { AVATAR_PX, type Gesture, type Look, type Paint, type Size } from "./frame";
import { MOODS, MORPH, type Mood, moodFor } from "./moods";

export type { Gesture, Look } from "./frame";

interface Props {
  avatar: string;
  color: string;
  size?: Size;
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
  /** Overrides the operator's choice of cast. For a preview of the other one. */
  cast?: Cast;
  /** Where its own clock starts and how fast it runs. Pass the agent id. */
  seed?: string;
  look?: Look;
  /**
   * Where to look instead of wherever the mood would, in body radii. For a
   * creature that is directed rather than read off an agent: the pair in the
   * rail's corner, looking at each other or at the pointer. An aimed `look`
   * still outranks it, because the point of that look is who it is for.
   */
  gaze?: Point | null;
  gesture?: Gesture;
  /** A short shout shown in a bubble, e.g. "!" when a message is thrown. */
  says?: string | null;
  title?: string;
}

/** What a throw and a catch do to the mass, in body radii. */
const KNOCK = {
  send: { amp: 0.26, hz: 1.5, decay: 0.28, life: 0.9 },
  receive: { amp: 0.3, hz: 2.2, decay: 0.3, life: 0.8 },
};

/**
 * How far a directed gaze has to move to be a turn rather than a step. A turn
 * is blinked into, as a mood's own large saccade is; a gaze following the
 * pointer moves a little every frame, and blinking into each of those would
 * be a creature with its eyes shut.
 */
const TURN = 0.2;

/**
 * Which way a throw or a catch shoves the mass, as a unit vector: away from
 * whoever it is aimed at. An aimed look is up or down, so the shove is down or
 * up; a directed gaze can be any way; with nobody looked at, a parcel thrown
 * from above presses the creature down, which is the way an unexplained shove
 * reads best.
 */
export function knockAway(look: Look, gaze: Point | null | undefined): Point {
  if (look) return [0, look === "down" ? -1 : 1];
  const l = gaze ? Math.hypot(gaze[0], gaze[1]) : 0;
  if (gaze && l > 0.05) return [-gaze[0] / l, -gaze[1] / l];
  return [0, 1];
}

/** Everything one avatar remembers between frames. */
interface Cell {
  mood: Mood;
  from: Mood;
  /**
   * When the change to `mood` began, when a gesture did and when the aimed
   * look last changed, all on the shared clock rather than the creature's own:
   * how quickly a face reacts is not allowed to be a property of its id.
   */
  at: number;
  /** Where the look has got to, and how fast, in body radii. */
  gaze: Point;
  vel: Point;
  last: number;
  gesture: Gesture;
  gestureAt: number;
  look: Look;
  lookAt: number;
  /** The directed gaze a blink was last cued for. */
  cued: Point | null;
}

/**
 * An agent's character.
 *
 * The drawing is one of two casts, `CutArt` or `DrawnArt`, over the same body
 * (`form.ts`), the same moods (`moods.ts`) and the same gaze (`eyes.ts`). This
 * owns the one thing none of those can: what is true right now. Every frame it
 * decides the mood from the signals it was given, follows the gaze with the
 * mass, adds whatever a message landing did to it, and hands the result to the
 * cast, which writes its own attributes. Nothing here re-renders: a mood that
 * lasts two seconds and a message landing are both decided inside the loop, so
 * a rail of a dozen agents reacting to each other costs React nothing at all.
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
  cast,
  seed,
  look = null,
  gaze = null,
  gesture = null,
  says = null,
  title,
}: Props) {
  const character = lookupCharacter(avatar);
  const chosen = useContext(CastContext);
  const drawing = cast ?? chosen;

  const box = useRef<HTMLSpanElement>(null);
  const art = useRef<Paint | null>(null);
  const cell = useRef<Cell>({
    mood: "idle",
    from: "idle",
    at: Number.NEGATIVE_INFINITY,
    gaze: [0, 0],
    vel: [0, 0],
    last: 0,
    gesture: null,
    gestureAt: 0,
    look: null,
    lookAt: Number.NEGATIVE_INFINITY,
    cued: null,
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
    gaze,
    gesture,
    character,
    size,
  });

  /* Its own clock, so a crew of idle agents is not one animal breathing. Only
     the cycles below are on it; every age is measured on the shared seconds. */
  const gait = gaitOf(seed ?? avatar);

  const paint = useRef<Painter>(() => {});
  paint.current = (seconds, live) => {
    const now = props.current;
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
    if (now.look !== state.look) {
      state.look = now.look;
      state.lookAt = seconds;
    }
    const was = state.cued;
    const is = now.gaze;
    if (
      (was === null) !== (is === null) ||
      (was && is && Math.hypot(is[0] - was[0], is[1] - was[1]) > TURN)
    ) {
      state.cued = is;
      state.lookAt = seconds;
    }

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
      box.current?.setAttribute("data-mood", want);
    }

    /* Where it is asked to look. An aimed look at a peer outranks whatever the
       mood would have done, because the point of that look is who it is for. */
    const since = seconds - state.at;
    const raw = live ? Math.min(1, since / MORPH) : 1;
    const u = raw * raw * (3 - 2 * raw);
    let eyeGaze: Point;
    if (now.look) {
      eyeGaze = [0, now.look === "up" ? -AIM.up : AIM.down];
    } else if (now.gaze) {
      eyeGaze = [now.gaze[0], now.gaze[1]];
    } else if (!live) {
      eyeGaze = [0, 0];
    } else {
      const a = gazeAt(t, MOODS[state.from].watch?.gaze);
      const b = gazeAt(t, MOODS[state.mood].watch?.gaze);
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
           down. `knockAway` holds the rule. */
        const away = knockAway(now.look, now.gaze);
        const push = knock.amp * wave * Math.exp(-age / knock.decay);
        bodyGaze[0] += away[0] * push;
        bodyGaze[1] += away[1] * push;
        eyeGaze[0] += away[0] * push * 0.35;
        eyeGaze[1] += away[1] * push * 0.35;
      }
    }

    art.current?.({
      lump: now.character,
      t,
      dt,
      live,
      mood: state.mood,
      from: state.from,
      since: live ? since : Number.POSITIVE_INFINITY,
      look: now.look,
      sinceLook: seconds - state.lookAt,
      gesture: state.gesture,
      sinceGesture: seconds - state.gestureAt,
      eyes: eyeGaze,
      body: bodyGaze,
      px: AVATAR_PX[now.size],
    });
  };

  /* Joined once. The painter is stable, so nothing here is torn down and set up
     again on a render, and the observer is not asked the same question twice. */
  useEffect(() => {
    const el = box.current;
    return el ? join(el, (seconds, live) => paint.current(seconds, live)) : undefined;
  }, []);

  /* And painted again after every render, at the time the clock last reached,
     so a change of props (or of cast) is on screen whether or not anything is
     moving. */
  useEffect(() => {
    props.current = {
      activity,
      lifecycle,
      work,
      escalated,
      finishedAt,
      mood,
      look,
      gaze,
      gesture,
      character,
      size,
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
      data-cast={drawing}
      style={style}
      title={title ?? character.label}
      data-mood={cell.current.mood}
    >
      <svg viewBox={`0 0 ${FORM.box} ${FORM.box}`} className="avatar__body" aria-hidden="true">
        {drawing === "drawn" ? <DrawnArt paint={art} /> : <CutArt paint={art} />}
      </svg>
      {says && (
        <span className="avatar__says" aria-hidden="true">
          {says}
        </span>
      )}
    </span>
  );
}
