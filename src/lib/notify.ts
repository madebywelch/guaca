import { api } from "./ipc";
import { desktop, mayNotify, notify, willNotify } from "./transport";
import type { Delivery, DeliveryId, DeliveryTarget, Reveal } from "./types";

export const FOLD_AFTER = 3;

export const AWAY_TITLE = "While you were away";

export async function deliver(delivery: Delivery, open: (target: DeliveryTarget) => void) {
  if (!(await willNotify())) return false;
  if (!(await claim(delivery.id))) return false;
  const shown = await notify(delivery.title, delivery.body, () => open(delivery.target));
  if (!shown) await release(delivery.id);
  return shown;
}

export async function catchUp(open: (target: DeliveryTarget) => void): Promise<number> {
  if (!(await mayNotify())) return 0;
  let waiting: Delivery[];
  try {
    waiting = await api.pendingDeliveries();
  } catch {
    return 0;
  }
  if (waiting.length === 0 || !(await willNotify())) return 0;

  const won: Delivery[] = [];
  for (const delivery of waiting) {
    if (await claim(delivery.id)) won.push(delivery);
  }

  if (won.length > FOLD_AFTER) {
    const shown = won.slice(0, FOLD_AFTER).map((delivery) => delivery.title);
    const rest = won.length - shown.length;
    const folded = await notify(
      AWAY_TITLE,
      [...shown, `And ${rest} more in For you.`].join("\n"),
      () => open({ kind: "forYou" }),
    );
    if (!folded) for (const delivery of won) await release(delivery.id);
  } else {
    for (const delivery of won) {
      if (!(await notify(delivery.title, delivery.body, () => open(delivery.target)))) {
        await release(delivery.id);
      }
    }
  }
  return won.length;
}

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

async function release(id: DeliveryId): Promise<void> {
  try {
    await api.releaseDelivery(id);
  } catch {}
}

async function claim(id: DeliveryId): Promise<boolean> {
  try {
    return (await api.claimDelivery(id)) === true;
  } catch {
    return false;
  }
}
