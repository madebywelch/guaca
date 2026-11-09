import { useEffect, useState } from "react";

import { onRuntimeEvent } from "./ipc";
import { restartQuietRemaining } from "./link";
import { useStore } from "./store";
import type { UiEvent } from "./types";

const DURABLE: UiEvent["type"][] = [
  "messageAppended",
  "agentsChanged",
  "activityChanged",
  "liveSnapshot",
  "channelsCleared",
  "settingsChanged",
  "approvalRequested",
  "approvalSettled",
  "escalationRaised",
  "escalationCleared",
  "decisionsChanged",
  "runSettled",
];

type Phase =
  | "connecting"
  | "grace"
  | "down"
  | "reconnecting"
  | "reconciling"
  | "connected"
  | "stopped";

export function useRuntime(onEvent?: (event: UiEvent) => void): boolean {
  const [ready, setReady] = useState(false);
  const [handler] = useState(() => onEvent);

  useEffect(() => {
    const { applyEvent, linkDown, linkReconciling, linkUp, showFailure } = useStore.getState();
    let phase: Phase = "connecting";
    let unlisten: (() => void) | undefined;
    let generation = 0;
    let heardAt = Date.now();
    let initial = true;
    let reading = false;
    let requested = true;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let quiet: ReturnType<typeof setTimeout> | undefined;
    const stopped = () => phase === "stopped";

    const refresh = async () => {
      if (reading || phase === "stopped") return;
      reading = true;
      while (requested && (initial || phase === "reconciling" || phase === "connected")) {
        requested = false;
        const current = generation;
        const valid = () => phase !== "stopped" && generation === current;
        try {
          if (initial) await useStore.getState().bootstrap(valid);
          else await useStore.getState().resynchronize(valid);
          if (!valid()) continue;
          heardAt = Date.now();
          if (!requested && phase === "reconciling") {
            phase = "connected";
            linkUp();
          }
        } catch (error) {
          if (valid()) {
            requested = true;
            showFailure(error);
            if (phase === "connected" || phase === "reconciling") {
              phase = "reconciling";
              linkDown(error, heardAt);
              retry = setTimeout(() => {
                retry = undefined;
                void refresh();
              }, 1_000);
            }
          }
          break;
        } finally {
          if (initial) {
            initial = false;
            if (!stopped()) setReady(true);
          }
        }
      }
      reading = false;
      if (requested && !retry && phase === "reconciling") void refresh();
    };

    void (async () => {
      const stop = await onRuntimeEvent(
        (event) => {
          if (phase === "stopped") return;
          heardAt = Date.now();
          const current = generation;
          applyEvent(event, () => phase !== "stopped" && generation === current);
          handler?.(event);
          if (reading && DURABLE.includes(event.type)) requested = true;
        },
        () => {
          if (phase === "stopped") return;
          linkReconciling(heardAt);
          if (quiet) clearTimeout(quiet);
          quiet = undefined;
          if (retry) clearTimeout(retry);
          retry = undefined;
          phase = "reconciling";
          requested = true;
          void refresh();
        },
        (error) => {
          if (phase === "stopped") return;
          phase = "down";
          const report = () => {
            if (phase !== "down" && phase !== "reconnecting") return;
            const remaining = restartQuietRemaining();
            if (remaining > 0) quiet = setTimeout(report, remaining);
            else linkDown(error, heardAt);
          };
          report();
        },
        () => {
          if (phase === "stopped") return;
          generation++;
          phase = phase === "down" || phase === "reconnecting" ? "reconnecting" : "grace";
          requested = true;
          if (retry) clearTimeout(retry);
          retry = undefined;
        },
      );
      if (stopped()) {
        stop();
        return;
      }
      unlisten = stop;
      void refresh();
    })();

    return () => {
      phase = "stopped";
      generation++;
      if (retry) clearTimeout(retry);
      if (quiet) clearTimeout(quiet);
      unlisten?.();
      linkUp(false);
    };
  }, [handler]);

  return ready;
}
