import { type RefCallback, useCallback, useRef } from "react";

const NEAR_END_PX = 24;

const AT_END_PX = 1;

export interface FollowBottom {
  ref: RefCallback<HTMLElement>;
  node: () => HTMLElement | null;
  follow: () => void;
  pin: () => void;
}

export function useFollowBottom(): FollowBottom {
  const node = useRef<HTMLElement | null>(null);
  const following = useRef(true);
  const written = useRef<number | null>(null);
  const lastTop = useRef(0);
  const frame = useRef(0);

  const ours = useCallback((box: HTMLElement) => {
    const put = written.current;
    if (put === null || box.scrollTop >= put - AT_END_PX) return true;
    if (box.scrollHeight - box.scrollTop - box.clientHeight > AT_END_PX) return false;
    written.current = box.scrollTop;
    return true;
  }, []);

  const release = useCallback(() => {
    following.current = false;
    written.current = null;
  }, []);

  const toEnd = useCallback(() => {
    const box = node.current;
    if (!box || !following.current) return;
    if (!ours(box)) return release();
    box.scrollTop = box.scrollHeight;
    written.current = box.scrollTop;
  }, [ours, release]);

  const ref = useCallback<RefCallback<HTMLElement>>(
    (box) => {
      node.current = box;
      if (!box) return;
      written.current = null;
      lastTop.current = box.scrollTop;
      const onScroll = () => {
        const top = box.scrollTop;
        const down = top >= lastTop.current;
        lastTop.current = top;
        if (following.current) {
          if (!ours(box)) release();
          return;
        }
        if (down && box.scrollHeight - top - box.clientHeight <= NEAR_END_PX) {
          following.current = true;
          written.current = top;
        }
      };
      box.addEventListener("scroll", onScroll, { passive: true });

      const watch = new ResizeObserver(() => toEnd());
      watch.observe(box);

      return () => {
        node.current = null;
        box.removeEventListener("scroll", onScroll);
        watch.disconnect();
      };
    },
    [ours, release, toEnd],
  );

  const follow = useCallback(() => {
    if (frame.current) return;
    toEnd();
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      toEnd();
    });
  }, [toEnd]);

  const pin = useCallback(() => {
    following.current = true;
    written.current = null;
    toEnd();
  }, [toEnd]);

  const at = useCallback(() => node.current, []);

  return { ref, node: at, follow, pin };
}
