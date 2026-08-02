import { expect, it } from "vitest"

import type { ReaderCacheSnapshot } from "../cache/readerCache"
import { initialReaderState, readerReducer } from "./reducer"
import {
  categoryId,
  entryId,
  makeCategory,
  makeDetail,
  makeEntry,
  makeSubscription,
} from "./testFixtures"
import type { ReaderState } from "./types"
import { sourceKey, type ReaderSource } from "./types"

it("hydrates a matching cached queue and keeps it visible through revalidation failure", () => {
  const source: ReaderSource = { kind: "smart", state: "UNREAD" }
  const cachedEntry = makeEntry({ title: "Cached entry" })
  const cached = cacheSnapshot({ source, entries: [cachedEntry], snapshotGeneration: 7 })

  let state = readerReducer(initialReaderState, { type: "readerCacheHydrated", cached })

  expect(state.paneStatus).toMatchObject({ subscriptions: "ready", queue: "ready" })
  expect(state.queueBySourceKey[sourceKey(source)]).toEqual([cachedEntry.entryId])
  expect(state.entriesById[cachedEntry.entryId]?.title).toBe("Cached entry")
  expect(state.snapshotGenerationBySource[sourceKey(source)]).toBe(7)

  state = readerReducer(state, { type: "sourceRequested", source, generation: 1 })
  expect(state.paneStatus.queue).toBe("ready")
  state = readerReducer(state, {
    type: "sourceFailed",
    source,
    generation: 1,
    error: "Temporarily offline",
  })
  expect(state.paneStatus.queue).toBe("ready")
  expect(state.queueBySourceKey[sourceKey(source)]).toEqual([cachedEntry.entryId])
  expect(state.errors.queue).toBe("Temporarily offline")

  const freshEntryId = "00000000-0000-4000-8000-000000000302"
  state = readerReducer(state, { type: "sourceRequested", source, generation: 2 })
  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 2,
    entries: [makeEntry({ entryId: freshEntryId, title: "Fresh entry", isRead: true })],
    snapshotGeneration: 8,
    mode: "replace",
  })
  expect(state.queueBySourceKey[sourceKey(source)]).toEqual([freshEntryId])
  expect(state.entriesById[freshEntryId]).toMatchObject({ title: "Fresh entry", isRead: true })
  expect(state.snapshotGenerationBySource[sourceKey(source)]).toBe(8)
  expect(state.errors.queue).toBeNull()
})

it("hydrates cached organization without installing a queue for another route source", () => {
  const selectedSource: ReaderSource = { kind: "smart", state: "ALL" }
  const cachedSource: ReaderSource = { kind: "smart", state: "UNREAD" }
  const selectedState = { ...initialReaderState, selectedSource }

  const state = readerReducer(selectedState, {
    type: "readerCacheHydrated",
    cached: cacheSnapshot({ source: cachedSource }),
  })

  expect(state.paneStatus.subscriptions).toBe("ready")
  expect(state.subscriptionOrder).toEqual([makeSubscription().subscriptionId])
  expect(state.paneStatus.queue).toBe("idle")
  expect(state.queueBySourceKey[sourceKey(cachedSource)]).toBeUndefined()
})

it("keeps cached subscriptions ready until an authoritative organization response wins", () => {
  const cachedSubscription = makeSubscription({ title: "Cached feed", unreadCount: 9 })
  let state = readerReducer(initialReaderState, {
    type: "readerCacheHydrated",
    cached: cacheSnapshot({ subscriptions: [cachedSubscription] }),
  })

  state = readerReducer(state, { type: "subscriptionsRequested", generation: 1 })
  expect(state.paneStatus.subscriptions).toBe("ready")
  state = readerReducer(state, {
    type: "subscriptionsFailed",
    generation: 1,
    error: "Temporarily offline",
  })
  expect(state.paneStatus.subscriptions).toBe("ready")
  expect(state.subscriptionsById[cachedSubscription.subscriptionId]?.unreadCount).toBe(9)

  const freshSubscriptionId = "00000000-0000-4000-8000-000000000202"
  state = readerReducer(state, { type: "subscriptionsRequested", generation: 2 })
  state = readerReducer(state, {
    type: "subscriptionsReceived",
    generation: 2,
    subscriptions: [
      makeSubscription({
        subscriptionId: freshSubscriptionId,
        title: "Fresh feed",
        unreadCount: 1,
      }),
    ],
    categories: [],
  })
  expect(state.subscriptionOrder).toEqual([freshSubscriptionId])
  expect(state.subscriptionsById[cachedSubscription.subscriptionId]).toBeUndefined()
  expect(state.subscriptionsById[freshSubscriptionId]?.unreadCount).toBe(1)
  expect(state.errors.subscriptions).toBeNull()
})

it("rejects late detail responses and updates the shared entity from the winner", () => {
  let state: ReaderState = {
    ...initialReaderState,
    entriesById: { [entryId]: makeEntry({ title: "List title" }) },
  }
  state = readerReducer(state, { type: "entrySelected", entryId })
  state = readerReducer(state, {
    type: "detailRequested",
    entryId,
    generation: 1,
  })
  state = readerReducer(state, {
    type: "detailRequested",
    entryId,
    generation: 2,
  })
  state = readerReducer(state, {
    type: "detailReceived",
    entryId,
    generation: 1,
    detail: makeDetail({ title: "Late title", isStarred: true }),
  })
  state = readerReducer(state, {
    type: "detailFailed",
    entryId,
    generation: 1,
    error: "Late failure",
  })

  expect(state.detailsById[entryId]).toBeUndefined()
  expect(state.errors.detail).toBeNull()

  state = readerReducer(state, {
    type: "detailReceived",
    entryId,
    generation: 2,
    detail: makeDetail({ title: "Winning title", isStarred: true }),
  })

  expect(state.detailsById[entryId]?.title).toBe("Winning title")
  expect(state.entriesById[entryId]).toMatchObject({
    title: "Winning title",
    isStarred: true,
  })
})

it("rejects late source responses and errors after a newer generation starts", () => {
  const source: ReaderSource = { kind: "smart", state: "UNREAD" }
  let state = readerReducer(initialReaderState, {
    type: "sourceRequested",
    source,
    generation: 1,
  })
  state = readerReducer(state, { type: "sourceRequested", source, generation: 2 })
  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 1,
    snapshotGeneration: 1,
    entries: [makeEntry({ title: "Late entry" })],
    mode: "replace",
  })
  state = readerReducer(state, {
    type: "sourceFailed",
    source,
    generation: 1,
    error: "Late failure",
  })

  expect(state.queueBySourceKey[sourceKey(source)]).toBeUndefined()
  expect(state.errors.queue).toBeNull()

  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 2,
    snapshotGeneration: 2,
    entries: [makeEntry({ title: "Winning entry" })],
    mode: "replace",
  })
  expect(state.entriesById[entryId]?.title).toBe("Winning entry")
})

it("loads categories and subscriptions atomically and rejects late organization pages", () => {
  const selectedSource: ReaderSource = { kind: "category", categoryId }
  let state = readerReducer(initialReaderState, {
    type: "sourceSelected",
    source: selectedSource,
  })
  state = readerReducer(state, { type: "subscriptionsRequested", generation: 1 })
  state = readerReducer(state, { type: "subscriptionsRequested", generation: 2 })
  state = readerReducer(state, {
    type: "subscriptionsReceived",
    generation: 1,
    subscriptions: [makeSubscription({ title: "Late subscription" })],
    categories: [makeCategory({ title: "Late category" })],
  })
  expect(state.subscriptionOrder).toEqual([])
  expect(state.categoryOrder).toEqual([])

  const secondCategory = makeCategory({
    categoryId: "00000000-0000-4000-8000-000000000502",
    title: "Science",
    position: 512,
  })
  state = readerReducer(state, {
    type: "subscriptionsReceived",
    generation: 2,
    subscriptions: [makeSubscription({ categoryId })],
    categories: [makeCategory(), secondCategory],
  })

  expect(state.categoryOrder).toEqual([secondCategory.categoryId, categoryId])
  expect(state.subscriptionsById[makeSubscription().subscriptionId]?.categoryId).toBe(
    categoryId,
  )
  expect(state.selectedSource).toEqual(selectedSource)
})

it("deletes a category without a second store and clears affected subscription projections", () => {
  const source: ReaderSource = { kind: "category", categoryId }
  const key = sourceKey(source)
  let state: ReaderState = {
    ...initialReaderState,
    categoriesById: { [categoryId]: makeCategory() },
    categoryOrder: [categoryId],
    subscriptionsById: {
      [makeSubscription().subscriptionId]: makeSubscription({ categoryId }),
    },
    subscriptionOrder: [makeSubscription().subscriptionId],
    selectedSource: source,
    selectedEntryId: entryId,
    queueBySourceKey: { [key]: [entryId] },
  }

  state = readerReducer(state, { type: "categoryDeleted", categoryId })

  expect(state.categoriesById).toEqual({})
  expect(state.categoryOrder).toEqual([])
  expect(state.subscriptionsById[makeSubscription().subscriptionId]?.categoryId).toBeNull()
  expect(state.queueBySourceKey[key]).toBeUndefined()
  expect(state.selectedSource).toEqual({ kind: "smart", state: "UNREAD" })
  expect(state.selectedEntryId).toBeNull()
})

it("keeps request generations monotonic across session expiry", () => {
  const source: ReaderSource = { kind: "smart", state: "UNREAD" }
  let state = readerReducer(initialReaderState, {
    type: "sourceRequested",
    source,
    generation: 7,
  })
  state = readerReducer(state, { type: "sessionExpired" })
  expect(state.requestGenerationByPane.queue).toBe(7)

  state = readerReducer(state, { type: "sourceRequested", source, generation: 8 })
  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 7,
    snapshotGeneration: 7,
    entries: [makeEntry({ title: "Pre-expiry response" })],
    mode: "replace",
  })
  expect(state.entriesById[entryId]).toBeUndefined()

  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 8,
    snapshotGeneration: 8,
    entries: [makeEntry({ title: "Post-expiry response" })],
    mode: "replace",
  })
  expect(state.entriesById[entryId]?.title).toBe("Post-expiry response")
})

function cacheSnapshot(
  overrides: Partial<ReaderCacheSnapshot> = {},
): ReaderCacheSnapshot {
  const entries = overrides.entries ?? [makeEntry({ title: "Cached entry" })]
  return {
    categories: [makeCategory()],
    subscriptions: [makeSubscription()],
    source: { kind: "smart", state: "UNREAD" },
    entries,
    queue: entries.map((entry) => entry.entryId),
    snapshotGeneration: entries.length > 0 ? 7 : null,
    scrollAnchorByRoute: { "/reader/unread": 128 },
    ...overrides,
  }
}
