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

// For the same reason nothing is ever on screen here, so anything that waits to
// be scrolled to would wait forever. Everything is reported visible, which is
// as true as anything else in a window with no dimensions.
if (!("IntersectionObserver" in globalThis)) {
  class IntersectionObserverStub {
    constructor(private readonly notify: IntersectionObserverCallback) {}
    observe(target: Element) {
      this.notify(
        [{ target, isIntersecting: true } as IntersectionObserverEntry],
        this as unknown as IntersectionObserver,
      );
    }
    unobserve() {}
    disconnect() {}
    takeRecords(): IntersectionObserverEntry[] {
      return [];
    }
  }
  (globalThis as unknown as { IntersectionObserver: unknown }).IntersectionObserver =
    IntersectionObserverStub;
}

// jsdom 27 ships no media queries at all, so every non-optional `matchMedia`
// call takes the whole tree down. A stub rather than a polyfill: nothing here
// evaluates a query, so every one of them resolves to not-matching, which is
// the light-surface, full-motion default the app is built around. A test that
// wants the other answer overrides this one for the length of that test.
if (!("matchMedia" in globalThis)) {
  (globalThis as unknown as { matchMedia: unknown }).matchMedia = (media: string) => ({
    matches: false,
    media,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  });
}

// Tauri injects this into the real webview before the app loads. Only the URL
// builder is needed: `lib/files.ts` addresses a stored file with it, and every
// command goes through a mock.
if (!("__TAURI_INTERNALS__" in globalThis)) {
  (globalThis as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    convertFileSrc: (path: string, protocol: string) =>
      `${protocol}://localhost/${encodeURIComponent(path)}`,
  };
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
