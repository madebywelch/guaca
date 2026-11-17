if (!("ResizeObserver" in globalThis)) {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = ResizeObserverStub;
}

if (typeof Element.prototype.scrollIntoView !== "function") {
  Element.prototype.scrollIntoView = function scrollIntoView() {};
}

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

if (!("__TAURI_INTERNALS__" in globalThis)) {
  (globalThis as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    convertFileSrc: (path: string, protocol: string) =>
      `${protocol}://localhost/${encodeURIComponent(path)}`,
  };
}

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

{
  const complain = console.error;
  console.error = (...args: unknown[]) => {
    const said = args.map((arg) => (typeof arg === "string" ? arg : "")).join(" ");
    if (said.includes("the same key") || said.includes('unique "key"')) {
      throw new Error(`React key defect: ${said}`);
    }
    complain(...args);
  };
}
