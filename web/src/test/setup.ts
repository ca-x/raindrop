import "@testing-library/jest-dom/vitest"

Object.defineProperty(window, "matchMedia", {
  configurable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  }),
})

class ResizeObserverStub implements ResizeObserver {
  disconnect() {}
  observe() {}
  unobserve() {}
}

globalThis.ResizeObserver = ResizeObserverStub

Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
  configurable: true,
  value: () => null,
})
