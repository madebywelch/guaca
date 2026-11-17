import type { Lump, Point } from "./form";
import type { Mood } from "./moods";

export type Look = "up" | "down" | null;

export type Gesture = "send" | "receive" | null;

export const KNOCK: Record<
  Exclude<Gesture, null>,
  { amp: number; hz: number; decay: number; life: number }
> = {
  send: { amp: 0.26, hz: 1.5, decay: 0.28, life: 0.9 },
  receive: { amp: 0.3, hz: 2.2, decay: 0.3, life: 0.8 },
};

export interface Frame {
  lump: Lump;
  t: number;
  dt: number;
  live: boolean;
  mood: Mood;
  from: Mood;
  since: number;
  look: Look;
  sinceLook: number;
  gesture: Gesture;
  sinceGesture: number;
  eyes: Point;
  body: Point;
  px: number;
}

export type Size = "xs" | "sm" | "md" | "lg";

export const AVATAR_PX: Record<Size, number> = { xs: 24, sm: 32, md: 44, lg: 68 };

export type Paint = (frame: Frame) => void;
