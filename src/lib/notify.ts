/**
 * Showing what the Host decided to say, in exactly one window.
 *
 * Nothing here decides whether to interrupt anyone. The Host does that once,
 * for every window, from its attention settings and from where the operator
 * is (`domain/outbox.rs`), and offers each delivery to every window it has.
 * This file is what a window does with an offer: claim it, and show it only if
 * the claim won. Two windows open on one Host are two offers and one banner,
 * because `claimDelivery` answers true once.
 *
 * On a Mac a banner counts as shown only once macOS has taken it. A delivery
 * macOS refuses after the claim is still in For you and its channel, but the
 * Host has already counted it as shown and offers it to no other window and no
 * push. Showing before claiming would close that gap by raising one banner per
 * open window, which is what the claim is for.
 *
 * Claim first, then show, and ask whether this page may show anything before
 * either. A page that claims what it cannot show has taken the only copy away
 * from a window that could. On a Mac that has never been asked, that question
 * is the real macOS prompt, put when an offer arrives and answered before the
 * claim.
 *
 * A page that was not connected when something was offered asks for what is
 * still waiting when it connects. More than three of those is one banner,
 * "While you were away", which opens For you: a pile of banners at once is
 * the thing that gets notifications switched off.
 *
 * Only the window does this. The menu bar panel is a second client of the
 * same Host and is offered everything too, but it never imports this file and
 * its capability grants no notification command (docs/gotchas/menubar.md).
 */

import { api } from "./ipc";
import { desktop, mayNotify, notify, willNotify } from "./transport";
import type { Delivery, DeliveryId, DeliveryTarget, Reveal } from "./types";

/** How many waiting deliveries are shown one by one before they are folded. */
export const FOLD_AFTER = 3;

export const AWAY_TITLE = "While you were away";

/**
 * Shows one delivery the Host just offered, if this window wins it.
 *
 * Returns whether a notification was raised, which is false both when another
 * window claimed it and when this page may not notify.
 */
export async function deliver(delivery: Delivery, open: (target: DeliveryTarget) => void) {
  if (!(await willNotify())) return false;
  if (!(await claim(delivery.id))) return false;
  return notify(delivery.title, delivery.body, () => open(delivery.target));
}

/**
 * Shows what was offered while this page was not connected, on connect and on
 * every reconnect. Returns how many deliveries this window took.
 */
export async function catchUp(open: (target: DeliveryTarget) => void): Promise<number> {
  if (!(await mayNotify())) return 0;
  let waiting: Delivery[];
  try {
    waiting = await api.pendingDeliveries();
  } catch {
    return 0;
  }
  // Asked only once there is something waiting, so connecting with nothing to
  // show puts no question to anyone.
  if (waiting.length === 0 || !(await willNotify())) return 0;

  const won: Delivery[] = [];
  for (const delivery of waiting) {
    if (await claim(delivery.id)) won.push(delivery);
  }

  if (won.length > FOLD_AFTER) {
    const shown = won.slice(0, FOLD_AFTER).map((delivery) => delivery.title);
    const rest = won.length - shown.length;
    await notify(AWAY_TITLE, [...shown, `And ${rest} more in For you.`].join("\n"), () =>
      open({ kind: "forYou" }),
    );
  } else {
    for (const delivery of won) {
      await notify(delivery.title, delivery.body, () => open(delivery.target));
    }
  }
  return won.length;
}

/**
 * Where a click on a delivery lands.
 *
 * On desktop through the app, which is the only thing that can raise its own
 * window: `openWindow` shows and focuses it and hands the target back as the
 * same reveal the menu bar's panel sends, so both arrive at one handler. In a
 * browser the tab is already in front by the time this runs.
 */
export function land(target: DeliveryTarget, reveal: (target: Reveal) => void): void {
  const where: Reveal =
    target.kind === "forYou" ? { kind: "forYou" } : { kind: "agent", id: target.agentId };
  if (!desktop) {
    reveal(where);
    return;
  }
  api.openWindow(where).catch((error: unknown) => {
    console.warn("Could not bring the window forward for a notification", error);
    reveal(where);
  });
}

/** A refused or failed claim is a lost one: another window may still show it. */
async function claim(id: DeliveryId): Promise<boolean> {
  try {
    return (await api.claimDelivery(id)) === true;
  } catch {
    return false;
  }
}
