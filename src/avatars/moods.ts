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
  mark?: "dots" | "bang" | "z";
  dim?: boolean;
}

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
    eye: { ...REST, open: -0.04, tilt: -4, arch: 0.4, low: 0, pupil: 0.6, size: 1.06 },
    watch: { blink: true, gaze: { range: 0.05, hz: 0.5, cross: 0.28 } },
  },

  thinking: {
    shape: { knead: { amp: 0.024, hz: 0.34 }, wob: [{ k: 3, amp: 0.014, spd: 1.2 }] },
    eye: { ...REST, open: 0.3, tilt: -6, arch: 0.18, low: 0.16, pupil: 0.46, skew: 0.3 },
    watch: {
      blink: true,
      gaze: { range: 0.2, hz: 0.55, cross: 0.2, bias: [-0.22, -0.2], far: 0.25 },
    },
    mark: "dots",
  },

  working: {
    shape: { knead: { amp: 0.075, hz: 1.1, sharp: true } },
    eye: { ...REST, open: 0.42, arch: 0.1, low: 0.26, pupil: 0.46 },
    watch: { blink: true, gaze: { cross: 0.07, script: READING } },
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

export const MORPH = 0.6;

export const PLEASED_MS = 2600;
export const STRUCK_MS = 900;

export interface Signals {
  activity?: Activity;
  lifecycle?: Lifecycle;
  work?: LiveCall[];
  escalated?: boolean;
  finishedAt?: number;
  struckAt?: number;
}

export function moodFor(signals: Signals, now: number): Mood {
  if (signals.lifecycle && signals.lifecycle !== "active") return "paused";

  const activity = signals.activity?.state ?? "idle";
  if (activity === "paused") return "paused";
  if (activity === "awaitingApproval") return "blocked";

  if (signals.struckAt && now - signals.struckAt < STRUCK_MS) return "surprised";

  if (activity === "thinking") {
    const work = signals.work ?? [];
    const last = work[work.length - 1]?.done?.outcome.status;
    if (last === "refused" || last === "failed") return "frustrated";
    return work.some((call) => call.done === null) ? "working" : "thinking";
  }

  if (activity === "queued") return "listening";

  if (signals.finishedAt && now - signals.finishedAt < PLEASED_MS) return "pleased";
  if (signals.escalated) return "stuck";
  return "idle";
}
