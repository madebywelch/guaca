/**
 * What an agent is doing, in the only vocabulary the drawing has.
 *
 * A mood is a way of deforming plus a pair of eyes plus what those eyes are
 * doing about looking. Adding one is a row in this table and nothing else: no
 * component learns about it, no stylesheet gains a rule, and `moodFor` is the
 * single place a runtime signal becomes an expression.
 *
 * The body amplitudes are small everywhere on purpose. What separates two moods
 * is the eyes; the body only breathes, leans and settles. `form.ts` has the
 * argument.
 */

import type { LiveCall } from "../lib/trail";
import type { Activity, Lifecycle } from "../lib/types";
import type { Eye, Watch } from "./eyes";
import type { Shape } from "./form";

export type Mood =
  | "idle"
  | "listening"
  | "thinking"
  | "working"
  | "frustrated"
  | "blocked"
  | "pleased"
  | "paused"
  | "stuck"
  | "surprised";

export interface Expression {
  shape: Shape;
  eye: Eye;
  watch?: Watch;
  /**
   * Drawn beside the head. Ink, except `bang`, which is amber because it is the
   * one state where a turn is parked on a person. Spend the amber anywhere else
   * and the rail stops meaning anything.
   */
  mark?: "dots" | "bang" | "z";
  /** Grey and faded: a creature that is not going to do anything. */
  dim?: boolean;
}

export const MOODS: Record<Mood, Expression> = {
  /* Still in the body and busy in the eyes. It had a breath and a wobble, and
     beside a face that blinks and looks about a body that also pulsed read as
     a second animation rather than as a creature at rest. What an idle
     creature does is look: mostly glances, and now and then a look the whole
     way to one side, which is the one thing that moves the body at all. */
  idle: {
    shape: {},
    eye: { w: 0.02, h: 2 },
    watch: { blink: true, gaze: { range: 0.4, hz: 0.32, cross: 0.26, far: 0.3 } },
  },

  listening: {
    shape: {
      aspect: [0.985, 1.03],
      knead: { amp: 0.025, hz: 0.5 },
      press: [{ th: 0.76, w: 0.5, amp: 0.035, beat: 2.4 }],
    },
    /* Raised: the dot lifted off its resting line is a pair of brows up. */
    eye: { w: 0.02, h: 2.45, sep: 0.3, dy: -0.5 },
    watch: {
      blink: true,
      gaze: { range: 0.07, hz: 0.5, cross: 0.3 },
      breath: { amp: 0.35, hz: 0.5 },
    },
  },

  /* One brow up and the other eye in a squint. The pair used to be a mirror
     here and read as mild; the two disagreeing is what turns mild into
     weighing something up. */
  thinking: {
    shape: { knead: { amp: 0.024, hz: 0.34 }, wob: [{ k: 3, amp: 0.014, spd: 1.2 }] },
    eye: { w: 0.4, h: 1.6, a: -12, skew: 0.42, lop: 0.4 },
    watch: {
      blink: true,
      gaze: { range: 0.22, hz: 0.68, cross: 0.22, bias: [-0.15, -0.14], far: 0.25 },
    },
    mark: "dots",
  },

  working: {
    shape: { knead: { amp: 0.075, hz: 1.1, sharp: true } },
    eye: { w: 1.3, h: 0.85, a: 20, dy: 0.25 },
    watch: { blink: true, gaze: { range: 0.14, hz: 2.2, cross: 0.1, bias: [0, 0.07] } },
  },

  frustrated: {
    shape: {
      aspect: [1.045, 0.965],
      knead: { amp: 0.03, hz: 2.4 },
      wob: [
        { k: 7, amp: 0.01, spd: 8 },
        { k: 4, amp: 0.008, spd: 5 },
      ],
    },
    /* The eye has become the brow: raised, thinned and tilted in until the
       stroke is nothing but a scowl. It glares off to one side now and then,
       which is where the body follows it. */
    eye: { w: 1.65, h: 0.78, a: 40, c: -0.1, dy: -0.5 },
    watch: {
      blink: "slow",
      gaze: { range: 0.36, hz: 1.6, cross: 0.1, far: 0.22, near: 0.12 },
      jitter: 0.065,
    },
  },

  /* The one mood that acts rather than holds a pose: it looks up at its own
     badge, narrows at it, and comes back to you. A fixed face over a pulsing
     body is what this replaced, and it read as a loading spinner. */
  blocked: {
    shape: { knead: { amp: 0.03, hz: 0.24 } },
    eye: { w: 0.02, h: 2.5, sep: 0.2 },
    watch: {
      blink: "slow",
      squint: { at: 0.3, w: 0.8, h: -0.62, a: 20, skew: 0.36, lop: 0.3 },
      gaze: {
        cross: 0.34,
        script: [
          [0, 0.02, 1.7],
          [0.24, -0.24, 1.2],
          [0.28, -0.29, 0.9],
          [0, 0.02, 1.5],
        ],
      },
    },
    mark: "bang",
  },

  pleased: {
    shape: { knead: { amp: 0.075, hz: 0.42, sharp: true } },
    eye: { w: 1.05, h: 0.8, c: -0.8, dy: -0.2 },
    watch: {
      blink: "slow",
      gaze: { range: 0.1, hz: 0.3, cross: 0.42, bias: [0, -0.05] },
      breath: { amp: 0.3, hz: 0.42 },
    },
  },

  paused: {
    shape: {
      aspect: [1.02, 0.965],
      sag: 1.1,
      spread: 0.07,
      knead: { amp: 0.025, hz: 0.13 },
      rise: 1,
    },
    eye: { w: 1.35, h: 0.5, c: 0.16 },
    watch: { blink: false, breath: { amp: 0.12, hz: 0.13 } },
    mark: "z",
    dim: true,
  },

  stuck: {
    shape: {
      aspect: [1.08, 0.915],
      sag: 2.1,
      spread: 0.13,
      rise: 1.9,
      knead: { amp: 0.045, hz: 0.1 },
      heave: { amp: 0.05, hz: 0.12 },
    },
    /* Worried rather than blank: inner ends up, one higher than the other,
       and the eyes darting where the body cannot go. What this replaced was a
       pair of small dots staring at nothing, which read as sleepy beside
       `paused` rather than as a creature that needs somebody. */
    eye: { w: 1.1, h: 0.7, a: -28, c: 0.14, dy: 0.35, sep: -0.15, skew: 0.24 },
    watch: {
      blink: "slow",
      gaze: { range: 0.14, hz: 1.3, cross: 0.1, bias: [0, 0.05] },
      jitter: 0.03,
    },
  },

  surprised: {
    shape: {
      aspect: [0.965, 1.055],
      knead: { amp: 0.04, hz: 1.9 },
      wob: [{ k: 5, amp: 0.018, spd: 6 }],
    },
    eye: { w: 0.02, h: 2.85, sep: 0.55 },
    watch: {
      blink: false,
      gaze: { range: 0.02, hz: 3, cross: 0.06 },
      breath: { amp: 0.3, hz: 1.6 },
    },
  },
};

/**
 * The marks drawn beside a head.
 *
 * Written as markup rather than as elements React keeps, because they change on
 * a mood change and nothing else, and re-rendering the component every time a
 * transient mood expires is the timer this design exists to avoid. The place
 * and the animation are two nested groups on purpose: a CSS `transform` beats a
 * `transform` attribute on the same element, so a mark that animated and
 * positioned itself on one node drew at the corner of the viewBox.
 */
export function markFor(mood: Mood): string {
  switch (MOODS[mood].mark) {
    case "dots":
      return `<g class="avatar__dots" fill="var(--eye)" opacity="0.7"><circle cx="48" cy="12" r="1.5"/><circle cx="52.6" cy="9.4" r="1.8"/><circle cx="57.4" cy="6.2" r="2.1"/></g>`;
    case "bang":
      return `<g transform="translate(52 11)"><circle class="avatar__halo" r="9" fill="none" stroke="var(--flesh)" stroke-width="2"/><circle r="7" fill="var(--flesh)"/><path d="M0 -3.6v4.4" stroke="#fff" stroke-width="2" stroke-linecap="round"/><circle cy="3.4" r="1.1" fill="#fff"/></g>`;
    case "z":
      return `<g transform="translate(45 15)"><g class="avatar__z"><path d="M-3.2 -3.2h6.4l-6.4 6.4h6.4" stroke="var(--eye)" stroke-width="1.9" fill="none" stroke-linecap="round" stroke-linejoin="round" opacity="0.6"/></g></g>`;
    default:
      return "";
  }
}

/** How long a finished turn keeps looking pleased about it. */
export const PLEASED_MS = 2600;
/** How long taking a message keeps looking surprised. */
export const STRUCK_MS = 900;

/**
 * Everything about an agent that can change its face.
 *
 * Kept as data rather than as five props on the component so the mapping is one
 * function a test can drive, and so a caller that only knows two of these does
 * not have to invent the rest.
 */
export interface Signals {
  activity?: Activity;
  lifecycle?: Lifecycle;
  /** This agent's live tool calls, which say whether work is in flight or going badly. */
  work?: LiveCall[];
  /** An escalation of this agent's is open on the desk. */
  escalated?: boolean;
  /** When its last reply landed, and when a message last hit it. */
  finishedAt?: number;
  struckAt?: number;
}

/**
 * One runtime signal becomes one expression, here and nowhere else.
 *
 * Read in order of what outranks what: being switched off beats everything,
 * then waiting on a person, then a reaction, then work, then having nothing to
 * do. `now` is passed rather than read so the two transient moods can be
 * decided inside the render loop without a timer or a re-render.
 */
export function moodFor(signals: Signals, now: number): Mood {
  if (signals.lifecycle && signals.lifecycle !== "active") return "paused";

  const activity = signals.activity?.state ?? "idle";
  if (activity === "paused") return "paused";
  if (activity === "awaitingApproval") return "blocked";

  if (signals.struckAt && now - signals.struckAt < STRUCK_MS) return "surprised";

  if (activity === "thinking") {
    const work = signals.work ?? [];
    /* A refusal or a failure on the last call back is the app's only honest
       "this is not going well": the runtime does not publish its retries. */
    const last = work[work.length - 1]?.done?.outcome.status;
    if (last === "refused" || last === "failed") return "frustrated";
    return work.some((call) => call.done === null) ? "working" : "thinking";
  }

  if (activity === "queued") return "listening";

  if (signals.finishedAt && now - signals.finishedAt < PLEASED_MS) return "pleased";
  if (signals.escalated) return "stuck";
  return "idle";
}
