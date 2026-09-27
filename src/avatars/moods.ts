/**
 * What an agent is doing, in the only vocabulary the drawing has.
 *
 * A mood is a way of deforming plus a pair of eyes plus what those eyes are
 * doing about looking. Adding one is a row in this table and nothing else: no
 * component learns about it, no stylesheet gains a rule, and `moodFor` is the
 * single place a runtime signal becomes an expression.
 *
 * The body amplitudes are small everywhere on purpose. What separates two moods
 * is the eyes, and in the eyes it is the lids; the body only breathes, leans
 * and settles. `form.ts` has the argument.
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

/** An eye at rest: open, level, the lid just over the top of the pupil. */
const REST: Eye = {
  open: 0.16,
  tilt: 0,
  arch: 0.3,
  low: 0.05,
  smile: 0,
  pupil: 0.5,
  size: 1,
  skew: 0,
};

/* Working reads. Short steps to the right along a line, one long return, and
   the next line a little lower: the one saccade pattern everybody recognizes
   without knowing they do. */
const READING: [number, number, number][] = [
  [-0.2, 0.12, 0.42],
  [-0.08, 0.12, 0.3],
  [0.04, 0.12, 0.34],
  [0.16, 0.13, 0.4],
  [-0.2, 0.16, 0.46],
  [-0.06, 0.16, 0.28],
  [0.08, 0.16, 0.36],
  [0.18, 0.17, 0.52],
];

export const MOODS: Record<Mood, Expression> = {
  /* Still in the body and busy in the eyes. It had a breath and a wobble, and
     beside a face that blinks and looks about a body that also pulsed read as
     a second animation rather than as a creature at rest. What an idle
     creature does is look: mostly glances, which the pupil takes on its own,
     and now and then a look the whole way to one side, which is the one thing
     that moves the body at all. */
  idle: {
    shape: {},
    eye: REST,
    watch: { blink: true, gaze: { range: 0.4, hz: 0.32, cross: 0.22, far: 0.3 } },
  },

  listening: {
    shape: {
      aspect: [0.985, 1.03],
      knead: { amp: 0.025, hz: 0.5 },
      press: [{ th: 0.76, w: 0.5, amp: 0.035, beat: 2.4 }],
    },
    /* Lids pulled back and the pupils opened: interest, held on you. */
    eye: { ...REST, open: -0.04, tilt: -4, arch: 0.4, low: 0, pupil: 0.6, size: 1.06 },
    watch: { blink: true, gaze: { range: 0.05, hz: 0.5, cross: 0.28 } },
  },

  /* One lid lower than the other and the look up and away. The pair used to
     be a mirror here and read as mild; the two disagreeing is what turns mild
     into weighing something up. */
  thinking: {
    shape: { knead: { amp: 0.024, hz: 0.34 }, wob: [{ k: 3, amp: 0.014, spd: 1.2 }] },
    eye: { ...REST, open: 0.3, tilt: -6, arch: 0.18, low: 0.16, pupil: 0.46, skew: 0.3 },
    watch: {
      blink: true,
      gaze: { range: 0.2, hz: 0.55, cross: 0.2, bias: [-0.22, -0.2], far: 0.25 },
    },
    mark: "dots",
  },

  /* Level lids half down, reading. Not tilted: a tilt is an opinion, and a
     creature at work has not got one yet. */
  working: {
    shape: { knead: { amp: 0.075, hz: 1.1, sharp: true } },
    eye: { ...REST, open: 0.42, arch: 0.1, low: 0.26, pupil: 0.46 },
    watch: { blink: true, gaze: { cross: 0.07, script: READING } },
  },

  /* Inner ends down hard and the lower lids up under them: a glare, off to
     one side now and then, which is where the body follows it. */
  frustrated: {
    shape: {
      aspect: [1.045, 0.965],
      knead: { amp: 0.03, hz: 2.4 },
      wob: [
        { k: 7, amp: 0.01, spd: 8 },
        { k: 4, amp: 0.008, spd: 5 },
      ],
    },
    eye: {
      ...REST,
      open: 0.4,
      tilt: 30,
      arch: -0.06,
      low: 0.32,
      smile: -0.12,
      pupil: 0.36,
      size: 0.97,
    },
    watch: {
      blink: "slow",
      gaze: { range: 0.34, hz: 1.3, cross: 0.09, far: 0.25, near: 0.12 },
      jitter: 0.05,
    },
  },

  /* The one mood that acts rather than holds a pose: it looks up at its own
     badge, narrows one eye at it, and comes back to you. A fixed face over a
     pulsing body is what this replaced, and it read as a loading spinner. */
  blocked: {
    shape: { knead: { amp: 0.03, hz: 0.24 } },
    eye: { ...REST, open: 0.06, tilt: -10, low: 0.04, size: 1.03 },
    watch: {
      blink: "slow",
      squint: { at: 0.28, open: 0.32, tilt: 22, skew: 0.34, low: 0.18 },
      gaze: {
        cross: 0.3,
        script: [
          [0, 0.02, 1.7],
          [0.24, -0.26, 1.2],
          [0.28, -0.3, 0.9],
          [0, 0.02, 1.5],
        ],
      },
    },
    mark: "bang",
  },

  /* The lower lids pushed up and bowed: the white is a crescent over them,
     which is a smile with no mouth in it. */
  pleased: {
    shape: { knead: { amp: 0.075, hz: 0.42, sharp: true } },
    eye: {
      ...REST,
      open: 0.24,
      tilt: -3,
      arch: 0.26,
      low: 0.86,
      smile: 1.05,
      pupil: 0.52,
      size: 1.04,
    },
    watch: { blink: "slow", gaze: { range: 0.08, hz: 0.3, cross: 0.4, bias: [0, -0.06] } },
  },

  paused: {
    shape: {
      aspect: [1.02, 0.965],
      sag: 1.1,
      spread: 0.07,
      knead: { amp: 0.025, hz: 0.13 },
      rise: 1,
    },
    /* Shut onto a lower lid that sags in the middle: asleep, not blank. */
    eye: { ...REST, open: 1, low: 0.4, smile: -0.32 },
    watch: { blink: false },
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
    /* Worried rather than blank: inner ends up, one lid lower than the other,
       and the eyes darting where the body cannot go. What this replaced was a
       pair of small dots staring at nothing, which read as sleepy beside
       `paused` rather than as a creature that needs somebody. */
    eye: {
      ...REST,
      open: 0.26,
      tilt: -24,
      arch: 0.12,
      low: 0.12,
      smile: -0.08,
      pupil: 0.4,
      skew: 0.12,
    },
    watch: {
      blink: "slow",
      gaze: { range: 0.16, hz: 1.2, cross: 0.08, bias: [0, 0.08] },
      jitter: 0.02,
    },
  },

  /* Lids retracted clear of the ball and the pupils pinned: shock. */
  surprised: {
    shape: {
      aspect: [0.965, 1.055],
      knead: { amp: 0.04, hz: 1.9 },
      wob: [{ k: 5, amp: 0.018, spd: 6 }],
    },
    eye: { ...REST, open: -0.12, tilt: -3, arch: 0.5, low: -0.08, pupil: 0.27, size: 1.2 },
    watch: { blink: false, gaze: { range: 0.015, hz: 3, cross: 0.05 } },
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
      return `<g transform="translate(52 11)"><circle class="avatar__halo" r="9" fill="none" stroke="var(--attention-fill)" stroke-width="2"/><circle r="7" fill="var(--attention-fill)"/><path d="M0 -3.6v4.4" stroke="var(--on-attention-fill)" stroke-width="2" stroke-linecap="round"/><circle cy="3.4" r="1.1" fill="var(--on-attention-fill)"/></g>`;
    case "z":
      return `<g transform="translate(45 15)"><g class="avatar__z"><path d="M-3.2 -3.2h6.4l-6.4 6.4h6.4" stroke="var(--eye)" stroke-width="1.9" fill="none" stroke-linecap="round" stroke-linejoin="round" opacity="0.6"/></g></g>`;
    default:
      return "";
  }
}

/**
 * How long one mood takes to become another, in seconds. The cut cast morphs
 * over it; the drawn cast gets there in five drawings instead, and only the
 * gaze, which both share, is blended across it.
 */
export const MORPH = 0.6;

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
