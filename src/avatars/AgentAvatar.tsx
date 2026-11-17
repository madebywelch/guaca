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
import { AVATAR_PX, type Gesture, KNOCK, type Look, type Paint, type Size } from "./frame";
import { MOODS, MORPH, type Mood, moodFor } from "./moods";

export type { Gesture, Look } from "./frame";

interface Props {
  avatar: string;
  color: string;
  size?: Size;
  activity?: Activity;
  lifecycle?: Lifecycle;
  work?: LiveCall[];
  escalated?: boolean;
  finishedAt?: number;
  mood?: Mood;
  cast?: Cast;
  seed?: string;
  look?: Look;
  gaze?: Point | null;
  gesture?: Gesture;
  says?: string | null;
  title?: string;
}

const TURN = 0.2;

export function knockAway(look: Look, gaze: Point | null | undefined): Point {
  if (look) return [0, look === "down" ? -1 : 1];
  const l = gaze ? Math.hypot(gaze[0], gaze[1]) : 0;
  if (gaze && l > 0.05) return [-gaze[0] / l, -gaze[1] / l];
  return [0, 1];
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
  look: Look;
  lookAt: number;
  cued: Point | null;
}

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

  const gait = gaitOf(seed ?? avatar);

  const paint = useRef<Painter>(() => {});
  paint.current = (seconds, live) => {
    const now = props.current;
    const state = cell.current;
    const t = seconds * gait.rate + gait.phase;

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

    const dt = live ? Math.min(0.05, Math.max(0, seconds - state.last)) : 0;
    state.last = seconds;
    if (live) settle(state, eyeGaze, dt);
    else {
      state.gaze = [eyeGaze[0], eyeGaze[1]];
      state.vel = [0, 0];
    }
    eyeGaze = [state.gaze[0], state.gaze[1]];
    const bodyGaze: Point = [state.gaze[0], state.gaze[1]];

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

  useEffect(() => {
    const el = box.current;
    return el ? join(el, (seconds, live) => paint.current(seconds, live)) : undefined;
  }, []);

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
