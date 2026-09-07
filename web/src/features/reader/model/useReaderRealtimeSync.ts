import { useEffect, type RefObject } from "react"

import type { ReaderSession } from "./controllerSession"
import type { ReaderState } from "./types"

const backgroundSyncIntervalMs = 60_000
const eventSyncDebounceMs = 150
const busySyncRetryMs = 500
const disconnectedSyncDelayMs = 1_000
const minimumSyncIntervalMs = 5_000
const readerEventKinds = new Set([
  "SYNC_REQUIRED",
  "FEED_REFRESHED",
  "SUBSCRIPTIONS_CHANGED",
  "ENTRIES_CHANGED",
])

export type ReaderEventSourceFactory = (url: string) => EventSource

interface ReaderRealtimeSyncOptions {
  session: ReaderSession
  stateRef: RefObject<ReaderState>
  reloadSubscriptions: () => Promise<boolean>
  reloadEntries: () => Promise<boolean>
  eventSourceFactory?: ReaderEventSourceFactory
}

export function useReaderRealtimeSync({
  session,
  stateRef,
  reloadSubscriptions,
  reloadEntries,
  eventSourceFactory,
}: ReaderRealtimeSyncOptions): void {
  useEffect(() => {
    let syncTimer: number | null = null
    let syncInFlight = false
    let syncPending = false
    let eventStreamOpen = false
    let eventSource: EventSource | null = null
    let nextSyncAt = 0
    let lastSyncFailed = false
    let stopped = false

    const scheduleSync = (delayMs = eventSyncDebounceMs) => {
      if (stopped) return
      syncPending = true
      if (syncTimer !== null) return
      syncTimer = window.setTimeout(() => {
        syncTimer = null
        void sync()
      }, Math.max(delayMs, nextSyncAt - Date.now()))
    }
    const sync = async () => {
      if (
        stopped || !session.active() ||
        document.visibilityState === "hidden" ||
        navigator.onLine === false
      ) {
        return
      }
      if (Date.now() < nextSyncAt) {
        scheduleSync(0)
        return
      }
      if (
        syncInFlight ||
        stateRef.current.requestActivity.subscriptions ||
        stateRef.current.requestActivity.queue ||
        stateRef.current.requestActivity.page
      ) {
        scheduleSync(busySyncRetryMs)
        return
      }
      syncPending = false
      syncInFlight = true
      nextSyncAt = Date.now() + minimumSyncIntervalMs
      try {
        const results = await Promise.allSettled([reloadSubscriptions(), reloadEntries()])
        lastSyncFailed = results.some((result) =>
          result.status === "rejected" || !result.value)
      } finally {
        syncInFlight = false
        if (syncPending) scheduleSync()
      }
    }
    const onReaderEvent = (event: Event) => {
      if (isReaderEvent(event)) scheduleSync()
    }
    const onEventStreamOpen = () => {
      eventStreamOpen = true
    }
    const onEventStreamError = () => {
      eventStreamOpen = false
      scheduleSync(disconnectedSyncDelayMs)
    }
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") scheduleSync(0)
    }
    const onOnline = () => scheduleSync(0)
    const timer = window.setInterval(() => {
      if (
        !eventStreamOpen || lastSyncFailed ||
        stateRef.current.errors.subscriptions || stateRef.current.errors.queue
      ) void sync()
    }, backgroundSyncIntervalMs)

    const canCreateEventSource =
      eventSourceFactory !== undefined || typeof EventSource !== "undefined"
    if (canCreateEventSource) {
      const createEventSource = eventSourceFactory ?? ((url: string) =>
        new EventSource(url, { withCredentials: true }))
      eventSource = createEventSource("/api/v1/events")
      eventSource.addEventListener("reader", onReaderEvent)
      eventSource.addEventListener("open", onEventStreamOpen)
      eventSource.addEventListener("error", onEventStreamError)
    }
    window.addEventListener("online", onOnline)
    document.addEventListener("visibilitychange", onVisibilityChange)
    return () => {
      stopped = true
      if (syncTimer !== null) window.clearTimeout(syncTimer)
      window.clearInterval(timer)
      window.removeEventListener("online", onOnline)
      document.removeEventListener("visibilitychange", onVisibilityChange)
      eventSource?.removeEventListener("reader", onReaderEvent)
      eventSource?.removeEventListener("open", onEventStreamOpen)
      eventSource?.removeEventListener("error", onEventStreamError)
      eventSource?.close()
    }
  }, [eventSourceFactory, reloadEntries, reloadSubscriptions, session, stateRef])
}

export function isReaderEvent(event: Event): boolean {
  if (!(event instanceof MessageEvent) || typeof event.data !== "string") return false
  try {
    const value: unknown = JSON.parse(event.data)
    return Boolean(
      value &&
      typeof value === "object" &&
      "version" in value &&
      value.version === 1 &&
      "kind" in value &&
      typeof value.kind === "string" &&
      readerEventKinds.has(value.kind),
    )
  } catch {
    return false
  }
}
