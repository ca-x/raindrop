import { act, renderHook } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"

import type { ReaderSession } from "./controllerSession"
import { initialReaderState } from "./reducer"
import {
  isReaderEvent,
  useReaderRealtimeSync,
} from "./useReaderRealtimeSync"

afterEach(() => vi.useRealTimers())

it("coalesces SSE notifications and uses polling only while disconnected", async () => {
  vi.useFakeTimers()
  const source = new FakeEventSource()
  const reloadSubscriptions = vi.fn(async () => true)
  const reloadEntries = vi.fn(async () => true)
  const { unmount } = renderHook(() =>
    useReaderRealtimeSync({
      session: activeSession(),
      stateRef: { current: initialReaderState },
      reloadSubscriptions,
      reloadEntries,
      eventSourceFactory: vi.fn(() => source as unknown as EventSource),
    }),
  )

  act(() => {
    source.emit("open", new Event("open"))
    source.emit("reader", readerEvent("FEED_REFRESHED"))
    source.emit("reader", readerEvent("ENTRIES_CHANGED"))
  })
  await act(async () => vi.advanceTimersByTimeAsync(150))
  expect(reloadSubscriptions).toHaveBeenCalledOnce()
  expect(reloadEntries).toHaveBeenCalledOnce()

  await act(async () => vi.advanceTimersByTimeAsync(60_000))
  expect(reloadSubscriptions).toHaveBeenCalledOnce()

  act(() => source.emit("error", new Event("error")))
  await act(async () => vi.advanceTimersByTimeAsync(1_000))
  expect(reloadSubscriptions).toHaveBeenCalledTimes(2)
  expect(reloadEntries).toHaveBeenCalledTimes(2)

  unmount()
  expect(source.close).toHaveBeenCalledOnce()
})

it("accepts only the versioned Reader event contract", () => {
  expect(isReaderEvent(readerEvent("SYNC_REQUIRED"))).toBe(true)
  expect(isReaderEvent(readerEvent("UNKNOWN"))).toBe(false)
  expect(isReaderEvent(new MessageEvent("reader", { data: "not-json" }))).toBe(false)
  expect(isReaderEvent(new Event("reader"))).toBe(false)
})

it("bounds revalidation during a sustained refresh burst and reconciles the trailing event", async () => {
  vi.useFakeTimers()
  const source = new FakeEventSource()
  const reloadSubscriptions = vi.fn(async () => true)
  const reloadEntries = vi.fn(async () => true)
  renderHook(() => useReaderRealtimeSync({
    session: activeSession(),
    stateRef: { current: initialReaderState },
    reloadSubscriptions,
    reloadEntries,
    eventSourceFactory: () => source as unknown as EventSource,
  }))
  act(() => source.emit("open", new Event("open")))
  for (let index = 0; index < 100; index++) {
    act(() => source.emit("reader", readerEvent("FEED_REFRESHED")))
    await act(async () => vi.advanceTimersByTimeAsync(200))
  }
  const beforeTrailing = reloadSubscriptions.mock.calls.length
  expect(beforeTrailing).toBeLessThanOrEqual(5)
  await act(async () => vi.advanceTimersByTimeAsync(5_000))
  expect(reloadSubscriptions).toHaveBeenCalledTimes(beforeTrailing + 1)
  expect(reloadEntries).toHaveBeenCalledTimes(beforeTrailing + 1)
})

it("recovers a failed read even while the event stream remains connected", async () => {
  vi.useFakeTimers()
  const source = new FakeEventSource()
  const reloadSubscriptions = vi.fn(async () => true)
  const reloadEntries = vi.fn().mockResolvedValueOnce(false).mockResolvedValue(true)
  renderHook(() => useReaderRealtimeSync({
    session: activeSession(),
    stateRef: { current: initialReaderState },
    reloadSubscriptions,
    reloadEntries,
    eventSourceFactory: () => source as unknown as EventSource,
  }))
  act(() => {
    source.emit("open", new Event("open"))
    source.emit("reader", readerEvent("FEED_REFRESHED"))
  })
  await act(async () => vi.advanceTimersByTimeAsync(150))
  expect(reloadEntries).toHaveBeenCalledOnce()
  await act(async () => vi.advanceTimersByTimeAsync(60_000))
  expect(reloadEntries).toHaveBeenCalledTimes(2)
})

it("keeps polling recovery and trailing events behind the same deadline", async () => {
  vi.useFakeTimers()
  const source = new FakeEventSource()
  const reloadSubscriptions = vi.fn(async () => true)
  const reloadEntries = vi.fn().mockResolvedValueOnce(false).mockResolvedValue(true)
  renderHook(() => useReaderRealtimeSync({
    session: activeSession(), stateRef: { current: initialReaderState },
    reloadSubscriptions, reloadEntries,
    eventSourceFactory: () => source as unknown as EventSource,
  }))
  act(() => source.emit("open", new Event("open")))
  await act(async () => vi.advanceTimersByTimeAsync(59_000))
  act(() => source.emit("reader", readerEvent("FEED_REFRESHED")))
  await act(async () => vi.advanceTimersByTimeAsync(1_000))
  expect(reloadEntries).toHaveBeenCalledOnce()
  act(() => source.emit("reader", readerEvent("FEED_REFRESHED")))
  await act(async () => vi.advanceTimersByTimeAsync(4_150))
  expect(reloadEntries).toHaveBeenCalledTimes(2)
  await act(async () => vi.advanceTimersByTimeAsync(5_000))
  expect(reloadEntries).toHaveBeenCalledTimes(2)
})

function activeSession(): ReaderSession {
  return {
    active: () => true,
    begin: () => null,
    isCurrent: () => false,
    finish: vi.fn(),
    expire: vi.fn(),
  }
}

function readerEvent(kind: string): MessageEvent<string> {
  return new MessageEvent("reader", {
    data: JSON.stringify({ version: 1, kind }),
  })
}

class FakeEventSource {
  readonly close = vi.fn()
  private readonly listeners = new Map<string, Set<(event: Event) => void>>()

  addEventListener(type: string, listener: (event: Event) => void): void {
    const listeners = this.listeners.get(type) ?? new Set()
    listeners.add(listener)
    this.listeners.set(type, listeners)
  }

  removeEventListener(type: string, listener: (event: Event) => void): void {
    this.listeners.get(type)?.delete(listener)
  }

  emit(type: string, event: Event): void {
    for (const listener of this.listeners.get(type) ?? []) listener(event)
  }
}
