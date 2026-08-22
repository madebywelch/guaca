/**
 * The five bounds a conversation runs inside, as they are written for a person.
 *
 * Here rather than in the dialog that first drew them, because they are now set
 * in two places: app-wide in Settings, and per crew in a group. Two copies of
 * this list would drift, and what drifts is the sentence explaining a number
 * whose consequence is money.
 *
 * The ranges are the ones `GuardLimits::sanitized` clamps to. They are advisory
 * here — a typed or pasted number sails past a `max` on an input — and the
 * runtime clamps whatever arrives, which is why both dialogs read their limits
 * back after saving rather than leaving what was typed on screen.
 */

import type { GuardLimits } from "./types";

export interface LimitField {
  key: keyof GuardLimits;
  label: string;
  hint: string;
  min: number;
  max: number;
}

export const LIMITS: LimitField[] = [
  {
    key: "maxStepsPerRun",
    label: "Model calls per conversation",
    hint: "The hard ceiling on spend. One conversation is your message plus everything it sets off.",
    min: 1,
    max: 500,
  },
  {
    key: "maxToolRounds",
    label: "Tool calls per turn",
    hint: "How many times an agent can act and look again within one turn. Working a browser is a loop of read, click, read again, so this needs room.",
    min: 1,
    max: 100,
  },
  {
    key: "maxHops",
    label: "Relay depth",
    hint: "How far a message can travel from you. A relays to B relays to C is two hops.",
    min: 1,
    max: 16,
  },
  {
    key: "maxSendsPerPair",
    label: "Messages between any two agents",
    hint: "Stops two agents from talking to each other indefinitely.",
    min: 1,
    max: 50,
  },
  {
    key: "maxFanoutPerCall",
    label: "Recipients per send",
    hint: "How many agents one message can go to at once.",
    min: 1,
    max: 64,
  },
];
