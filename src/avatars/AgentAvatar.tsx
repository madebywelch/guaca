import type { CSSProperties } from "react";

import type { Activity, Lifecycle } from "../lib/types";
import { drawCharacter, lookupCharacter } from "./catalog";

/** Where the character is looking. Used to make a send visibly aimed at someone. */
export type Look = "up" | "down" | null;

/** A one-off reaction: winding up to throw, or being hit by something. */
export type Gesture = "send" | "receive" | null;

interface Props {
  avatar: string;
  color: string;
  size?: "xs" | "sm" | "md" | "lg";
  activity?: Activity;
  lifecycle?: Lifecycle;
  /** Desynchronizes idle motion. Pass the agent id so a crew never moves in unison. */
  seed?: string;
  look?: Look;
  gesture?: Gesture;
  /** A short shout shown in a bubble, e.g. "!" when a message is thrown. */
  says?: string | null;
  title?: string;
}

/** Deterministic 0..1 from a string. Only ever used for animation phase. */
function phase(seed: string): number {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  return (hash % 1000) / 1000;
}

/**
 * An agent's character.
 *
 * The drawing is the catalog's business; this owns behavior, layered in rough
 * order of loudness:
 *
 * - idle: a slow breath, and a glance around every several seconds
 * - thinking: a gentle wobble and quicker, wandering glances
 * - look: eyes and body aim up or down at whoever is being addressed
 * - gesture: a wind-up on send, a knock-back on receive
 */
export function AgentAvatar({
  avatar,
  color,
  size = "md",
  activity,
  lifecycle = "active",
  seed,
  look = null,
  gesture = null,
  says = null,
  title,
}: Props) {
  const character = lookupCharacter(avatar);
  const busy = activity?.state === "thinking";

  const style = {
    "--accent": color,
    // Negative delay starts each agent partway through its own cycle.
    "--phase": `${-(phase(seed ?? avatar) * 8).toFixed(2)}s`,
  } as CSSProperties;

  return (
    <span
      className={`avatar avatar--${size}`}
      style={style}
      data-busy={busy ? "true" : "false"}
      data-lifecycle={lifecycle}
      data-look={look ?? undefined}
      data-gesture={gesture ?? undefined}
      title={title ?? character.label}
    >
      <svg viewBox="0 0 64 64" className="avatar__body" aria-hidden="true">
        {drawCharacter(character)}
      </svg>
      {says && (
        <span className="avatar__says" aria-hidden="true">
          {says}
        </span>
      )}
    </span>
  );
}
