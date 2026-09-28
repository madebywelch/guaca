/**
 * The one subscription a page keeps to the runtime.
 *
 * Two pages draw a workspace: the window, and the panel under the menu bar
 * icon. The panel is a second client of the same host rather than a view the
 * window feeds, so it needs exactly what the window needs and nothing less:
 * events before the first read, and the read done again after anything that
 * could have been missed. That is a few lines with a race in each of them, and
 * it is written here once.
 */

import { useEffect, useState } from "react";

import { onRuntimeEvent } from "./ipc";
import { useStore } from "./store";
import { errorMessage, type UiEvent } from "./types";

/**
 * Events a read that overlapped them may have missed.
 *
 * A read that was in flight while one of these landed can hand back rows older
 * than the event, so it is read again once it completes. Token deltas need no
 * database read and are not here.
 */
const DURABLE: UiEvent["type"][] = [
  "messageAppended",
  "agentsChanged",
  "approvalRequested",
  "approvalSettled",
  "escalationRaised",
  "escalationCleared",
  "decisionsChanged",
  "runSettled",
];

/**
 * Subscribes, reads the workspace, and keeps it read. True once the first read
 * has landed, whether or not it succeeded: a failure is on the banner.
 *
 * `onEvent` sees every event after the store has applied it. It is read once,
 * when the subscription is made, and has to read anything else it needs at the
 * moment of the event rather than close over it.
 */
export function useRuntime(onEvent?: (event: UiEvent) => void): boolean {
  const [ready, setReady] = useState(false);
  const [handler] = useState(() => onEvent);

  useEffect(() => {
    const { applyEvent, setBanner } = useStore.getState();
    let unlisten: (() => void) | undefined;
    // Subscribing is async, so a teardown can arrive before it resolves. Without
    // this flag the listener leaks: StrictMode mounts twice in development, the
    // first cleanup finds `unlisten` still undefined, and every stream delta is
    // then applied by two listeners. That renders as text interleaved with
    // itself, which looks like a model bug rather than a subscription bug.
    let canceled = false;

    let initialReadDone = false;
    let refreshing = false;
    let requested = false;
    const refresh = async () => {
      requested = true;
      if (!initialReadDone || refreshing) return;
      refreshing = true;
      try {
        while (requested && !canceled) {
          requested = false;
          await useStore.getState().resynchronize();
        }
      } catch (error) {
        if (!canceled) setBanner({ tone: "error", text: errorMessage(error) });
      } finally {
        refreshing = false;
      }
    };

    void (async () => {
      // Subscribe before the first read so nothing that happens during startup
      // is missed.
      const stop = await onRuntimeEvent(
        (event) => {
          applyEvent(event);
          handler?.(event);
          if (refreshing && DURABLE.includes(event.type)) requested = true;
        },
        () => {
          void refresh();
        },
      );
      if (canceled) {
        stop();
        return;
      }
      unlisten = stop;

      try {
        await useStore.getState().bootstrap();
      } catch (error) {
        setBanner({ tone: "error", text: errorMessage(error) });
      } finally {
        initialReadDone = true;
        if (requested && !canceled) await refresh();
        if (!canceled) setReady(true);
      }
    })();

    return () => {
      canceled = true;
      unlisten?.();
    };
  }, [handler]);

  return ready;
}
