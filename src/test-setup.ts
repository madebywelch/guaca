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
