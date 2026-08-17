/**
 * Fills in the browser APIs jsdom lacks but WKWebView has.
 *
 * These are stubs, not polyfills: the tests care that a component mounts and
 * renders, not that layout measurement produces real numbers.
 */

if (!("ResizeObserver" in globalThis)) {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = ResizeObserverStub;
}

// jsdom does no layout, so it has no way to scroll an element into view and
// does not pretend to. Anything that brings a row into the window calls this.
if (typeof Element.prototype.scrollIntoView !== "function") {
  Element.prototype.scrollIntoView = function scrollIntoView() {};
}

// jsdom hands back a `localStorage` object with no methods on it here, so a
// component that stores a preference throws rather than storing one. Real
// WKWebView has the whole thing; this is enough of it to assert against.
if (typeof (globalThis as { localStorage?: Storage }).localStorage?.getItem !== "function") {
  const held = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => held.get(key) ?? null,
      setItem: (key: string, value: string) => void held.set(key, String(value)),
      removeItem: (key: string) => void held.delete(key),
      clear: () => held.clear(),
      key: (index: number) => [...held.keys()][index] ?? null,
      get length() {
        return held.size;
      },
    },
  });
}
