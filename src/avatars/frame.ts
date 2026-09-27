/**
 * What the loop in `AgentAvatar` decides about one creature, once a frame, and
 * hands to whichever cast is drawing it.
 *
 * The two casts share everything that is a decision: which mood, since when,
 * where the creature is looking and how hard a message just hit it. What they
 * do not share is anything about how that is put on screen, which is why this
 * is the whole of what crosses between them.
 */

import type { Lump, Point } from "./form";
import type { Mood } from "./moods";

/** Where the character is looking. Used to make a send visibly aimed at someone. */
export type Look = "up" | "down" | null;

/** A one-off reaction: winding up to throw, or being hit by something. */
export type Gesture = "send" | "receive" | null;

export interface Frame {
  lump: Lump;
  /** The creature's own seconds. Only cycles run on these. */
  t: number;
  /** Seconds since the last frame, already bounded. */
  dt: number;
  /** False is reduced motion: the right drawing, held. */
  live: boolean;
  mood: Mood;
  from: Mood;
  /** Seconds since `from` began becoming `mood`, on the shared clock. */
  since: number;
  look: Look;
  /** Seconds since the aimed look last changed. Infinite if it never has. */
  sinceLook: number;
  gesture: Gesture;
  /** Seconds since the gesture began. */
  sinceGesture: number;
  /** The look, smoothed, as the eyes take it and as the mass does, in body radii. */
  eyes: Point;
  body: Point;
  /** What the creature is drawn at, in CSS pixels at 100%. What line weights are hinted against. */
  px: number;
}

export type Size = "xs" | "sm" | "md" | "lg";

/**
 * What each size is drawn at, in CSS pixels at 100%. `styles.css` sets the
 * sizes; this copy is what line weights are hinted against, and
 * `styles.test.ts` holds the two to each other.
 */
export const AVATAR_PX: Record<Size, number> = { xs: 24, sm: 32, md: 44, lg: 68 };

/** A cast's painter: writes one frame into the elements it owns. */
export type Paint = (frame: Frame) => void;
