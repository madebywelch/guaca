import type { Lump, Point } from "./form";
import type { Mood } from "./moods";

export type Look = "up" | "down" | null;

export type Gesture = "send" | "receive" | null;

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
