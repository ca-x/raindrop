import "@testing-library/jest-dom/vitest"
import { cleanup } from "@testing-library/react"
import { afterEach } from "vitest"

afterEach(cleanup)

let viewportWidth = 1280
let viewportHeight = 800
const mediaQueries = new Map<string, MatchMediaMock>()

class MatchMediaMock {
  readonly listeners = new Set<EventListenerOrEventListenerObject>()
  onchange: ((this: MediaQueryList, event: MediaQueryListEvent) => void) | null = null

  constructor(readonly media: string) {}

  get matches() {
    return matchesMediaQuery(this.media)
  }

  addEventListener(_type: string, listener: EventListenerOrEventListenerObject) {
    this.listeners.add(listener)
  }

  removeEventListener(_type: string, listener: EventListenerOrEventListenerObject) {
    this.listeners.delete(listener)
  }

  addListener(listener: (event: MediaQueryListEvent) => void) {
    this.listeners.add(listener as EventListener)
  }

  removeListener(listener: (event: MediaQueryListEvent) => void) {
    this.listeners.delete(listener as EventListener)
  }

  dispatchEvent(event: Event) {
    for (const listener of this.listeners) {
      if (typeof listener === "function") listener.call(this, event)
      else listener.handleEvent(event)
    }
    this.onchange?.call(this as unknown as MediaQueryList, event as MediaQueryListEvent)
    return true
  }
}

Object.defineProperty(window, "matchMedia", {
  configurable: true,
  value: (query: string) => {
    const existing = mediaQueries.get(query)
    if (existing) return existing
    const media = new MatchMediaMock(query)
    mediaQueries.set(query, media)
    return media as unknown as MediaQueryList
  },
})

export function setTestViewport(width: number, height = 800) {
  const previousMatches = new Map(
    [...mediaQueries].map(([query, media]) => [query, media.matches]),
  )
  viewportWidth = width
  viewportHeight = height
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width })
  Object.defineProperty(window, "innerHeight", { configurable: true, value: height })
  for (const [query, media] of mediaQueries) {
    if (previousMatches.get(query) !== media.matches) {
      media.dispatchEvent(new Event("change"))
    }
  }
  window.dispatchEvent(new Event("resize"))
}

function matchesMediaQuery(query: string) {
  if (/prefers-reduced-motion:\s*reduce/.test(query)) return false
  if (/hover:\s*hover|pointer:\s*fine/.test(query)) return false
  if (/pointer:\s*coarse/.test(query)) return true
  const minWidth = query.match(/min-width:\s*(\d+)px/)
  const maxWidth = query.match(/max-width:\s*(\d+)px/)
  const minHeight = query.match(/min-height:\s*(\d+)px/)
  const maxHeight = query.match(/max-height:\s*(\d+)px/)
  return (
    (!minWidth || viewportWidth >= Number(minWidth[1])) &&
    (!maxWidth || viewportWidth <= Number(maxWidth[1])) &&
    (!minHeight || viewportHeight >= Number(minHeight[1])) &&
    (!maxHeight || viewportHeight <= Number(maxHeight[1]))
  )
}

if (typeof HTMLDialogElement !== "undefined") {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value(this: HTMLDialogElement) {
      this.setAttribute("open", "")
    },
  })
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value(this: HTMLDialogElement) {
      this.removeAttribute("open")
    },
  })
}

class StorageMock implements Storage {
  private values = new Map<string, string>()

  get length() {
    return this.values.size
  }

  clear() {
    this.values.clear()
  }

  getItem(key: string) {
    return this.values.get(key) ?? null
  }

  key(index: number) {
    return [...this.values.keys()][index] ?? null
  }

  removeItem(key: string) {
    this.values.delete(key)
  }

  setItem(key: string, value: string) {
    this.values.set(key, value)
  }
}

const localStorageMock = new StorageMock()
Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: localStorageMock,
})
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: localStorageMock,
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

setTestViewport(1280, 800)
