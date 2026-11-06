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
import type { UiEvent } from "./types";

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
 *
 * The connection is the other thing this keeps, in both directions. Nothing
 * arriving is a page that is not live, and the socket is the only thing that
 * knows it without being asked, so its close marks the window as showing the
 * state of a moment that has passed — and its open clears that, because a host
 * that answers again has answered. What the mark costs is a line the operator
 * reads; a page that shows an hour-old transcript as if it were this minute is
 * worse than one that says so.
 */
export function useRuntime(onEvent?: (event: UiEvent) => void): boolean {
  const [ready, setReady] = useState(false);
  const [handler] = useState(() => onEvent);

  useEffect(() => {
    const { applyEvent, linkDown, linkUp, showFailure } = useStore.getState();
    let unlisten: (() => void) | undefined;
    // Subscribing is async, so a teardown can arrive before it resolves. Without
    // this flag the listener leaks: StrictMode mounts twice in development, the
    // first cleanup finds `unlisten` still undefined, and every stream delta is
    // then applied by two listeners. That renders as text interleaved with
    // itself, which looks like a model bug rather than a subscription bug.
    let canceled = false;
    // When the host last said anything: every event applied, and every read
    // that came back. Kept here rather than in the store because an event is
    // exactly the wrong moment to write a field the status bar draws — a
    // streaming turn would redraw it forty times a second — and the only moment
    // it is read is the one where nothing is arriving.
    let heardAt = Date.now();

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
          heardAt = Date.now();
        }
      } catch (error) {
        if (!canceled) showFailure(error);
      } finally {
        refreshing = false;
      }
    };

    void (async () => {
      // Subscribe before the first read so nothing that happens during startup
      // is missed.
      const stop = await onRuntimeEvent(
        (event) => {
          heardAt = Date.now();
          applyEvent(event);
          handler?.(event);
          if (refreshing && DURABLE.includes(event.type)) requested = true;
        },
        () => {
          // The line is back, so the sentence about it goes before the read that
          // fills the gap behind it.
          linkUp();
          heardAt = Date.now();
          void refresh();
        },
        (error) => {
          // Only if this page is still the one watching: a page torn down a
          // moment ago cannot speak for the next one.
          if (!canceled) linkDown(error, heardAt);
        },
      );
      if (canceled) {
        stop();
        return;
      }
      unlisten = stop;

      try {
        await useStore.getState().bootstrap();
        heardAt = Date.now();
      } catch (error) {
        showFailure(error);
      } finally {
        initialReadDone = true;
        if (requested && !canceled) await refresh();
        if (!canceled) setReady(true);
      }
    })();

    return () => {
      canceled = true;
      unlisten?.();
      // This page stops watching, so what it knew about the line is not a fact
      // about the next one. Left in place, a window that unmounted for a token
      // screen would open again on a mark the page before it left behind.
      linkUp();
    };
  }, [handler]);

  return ready;
}
