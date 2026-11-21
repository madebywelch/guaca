import { useEffect, useRef, useState } from "react";
import { api } from "./ipc";
import { useStore } from "./store";
import type { SearchHits } from "./types";

const DEBOUNCE_MS = 120;

const LIMIT = 25;

type Request =
  | { phase: "inactive" }
  | { phase: "debouncing" | "searching" | "settled"; generation: number };

type RequestEvent =
  | { type: "begin"; generation: number }
  | { type: "start" | "finish" | "cancel"; generation: number };

export function searchRequest(state: Request, event: RequestEvent): Request {
  if (event.type === "begin") return { phase: "debouncing", generation: event.generation };
  if (state.phase === "inactive" || state.generation !== event.generation) return state;
  if (event.type === "cancel") return { phase: "inactive" };
  if (event.type === "start" && state.phase === "debouncing") {
    return { phase: "searching", generation: event.generation };
  }
  if (event.type === "finish" && state.phase === "searching") {
    return { phase: "settled", generation: event.generation };
  }
  return state;
}

type Result = { kind: "ready"; hits: SearchHits } | { kind: "failed" };

export function useSearchHits(query: string | null): Result | null {
  const showFailure = useStore((state) => state.showFailure);
  const [result, setResult] = useState<{
    generation: number;
    query: string;
    result: Result;
  } | null>(null);
  const issued = useRef(0);
  const request = useRef<Request>({ phase: "inactive" });

  useEffect(() => {
    if (query === null) return;
    const generation = ++issued.current;
    request.current = searchRequest(request.current, { type: "begin", generation });
    const timer = window.setTimeout(() => {
      request.current = searchRequest(request.current, { type: "start", generation });
      const finish = () => {
        const previous = request.current;
        request.current = searchRequest(previous, { type: "finish", generation });
        return request.current !== previous;
      };
      void api.search(query, LIMIT).then(
        (hits) => {
          if (finish()) setResult({ generation, query, result: { kind: "ready", hits } });
        },
        (error: unknown) => {
          if (!finish()) return;
          setResult({ generation, query, result: { kind: "failed" } });
          showFailure(error);
        },
      );
    }, DEBOUNCE_MS);
    return () => {
      window.clearTimeout(timer);
      request.current = searchRequest(request.current, { type: "cancel", generation });
    };
  }, [query, showFailure]);

  return result?.query === query && result.generation === issued.current ? result.result : null;
}
