import { expect, it } from "vitest"

import { initialReaderState, readerReducer } from "./reducer"
import type { ReaderState } from "./types"
import { sourceKey } from "./types"
import {
  categoryId,
  entryId,
  makeDetail,
  makeEntry,
  makeSubscription,
  subscriptionId,
} from "./testFixtures"

it("optimistically updates read state and rolls back its bounded snapshot", () => {
  const entry = makeEntry()
  const detail = makeDetail()
  const subscription = makeSubscription()
  let state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: { [entryId]: entry },
    detailsById: { [entryId]: detail },
    queueBySourceKey: {
      [sourceKey({ kind: "smart", state: "UNREAD" })]: [entryId],
    },
  }

  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 1,
    entryId,
    field: "isRead",
    value: true,
  })

  expect(state.entriesById[entryId]?.isRead).toBe(true)
  expect(state.detailsById[entryId]?.isRead).toBe(true)
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(2)
  expect(
    state.queueBySourceKey[sourceKey({ kind: "smart", state: "UNREAD" })],
  ).toEqual([entryId])

  state = readerReducer(state, {
    type: "entryMutationFailed",
    mutationId: 1,
    error: "You cannot change this entry",
  })

  expect(state.entriesById[entryId]?.isRead).toBe(false)
  expect(state.detailsById[entryId]?.isRead).toBe(false)
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(3)
  expect(state.errors.mutation).toBe("You cannot change this entry")
})

it("commits bulk-read state while preserving entries newer than the snapshot", () => {
  const pendingEntryId = "00000000-0000-4000-8000-000000000302"
  const unreadSource = { kind: "smart", state: "UNREAD" } as const
  const allSource = { kind: "smart", state: "ALL" } as const
  const unreadKey = sourceKey(unreadSource)
  const allKey = sourceKey(allSource)
  const subscription = makeSubscription({ unreadCount: 2 })
  let state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: {
      [entryId]: makeEntry(),
      [pendingEntryId]: makeEntry({ entryId: pendingEntryId }),
    },
    detailsById: { [entryId]: makeDetail() },
    selectedEntryId: entryId,
    requestGenerationByPane: { subscriptions: 3, queue: 4, detail: 5 },
    queueBySourceKey: {
      [unreadKey]: [entryId],
      [allKey]: [entryId, pendingEntryId],
    },
    pendingNewEntriesBySource: {
      [unreadKey]: [pendingEntryId],
    },
    pendingNewEntryCountBySource: { [unreadKey]: 1 },
    snapshotGenerationBySource: { [unreadKey]: 7, [allKey]: 8 },
  }

  state = readerReducer(state, {
    type: "bulkReadCommitted",
    entryIds: [entryId],
    affectedFeedIds: [subscription.feedId],
    retainedSourceKey: unreadKey,
    retainedQueueEntryIds: [entryId],
    retainedPendingEntryIds: [pendingEntryId],
    retainedQueueGeneration: 4,
    retainedSnapshotGeneration: 7,
    retainedPendingSnapshotGeneration: null,
    invalidateAllSources: true,
  })

  expect(state.entriesById[entryId]?.isRead).toBe(true)
  expect(state.detailsById[entryId]?.isRead).toBe(true)
  expect(state.entriesById[pendingEntryId]?.isRead).toBe(false)
  expect(state.queueBySourceKey[unreadKey]).toEqual([])
  expect(state.queueBySourceKey[allKey]).toBeUndefined()
  expect(state.snapshotGenerationBySource[allKey]).toBeUndefined()
  expect(state.pendingNewEntriesBySource[unreadKey]).toEqual([pendingEntryId])
  expect(state.pendingNewEntryCountBySource[unreadKey]).toBe(1)
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(2)
  expect(state.requestGenerationByPane).toEqual({
    subscriptions: 4,
    queue: 5,
    detail: 6,
  })

  state = readerReducer(state, {
    type: "detailReceived",
    entryId,
    generation: 5,
    detail: makeDetail({ isRead: false }),
  })
  expect(state.detailsById[entryId]?.isRead).toBe(true)
})

it("makes a committed bulk read the rollback baseline for pending entry mutations", () => {
  const unreadSource = { kind: "smart", state: "UNREAD" } as const
  const unreadKey = sourceKey(unreadSource)
  const subscription = makeSubscription({ unreadCount: 1 })
  let state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: { [entryId]: makeEntry() },
    queueBySourceKey: { [unreadKey]: [entryId] },
  }
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 40,
    entryId,
    field: "isRead",
    value: true,
  })
  state = readerReducer(state, {
    type: "bulkReadCommitted",
    entryIds: [entryId],
    affectedFeedIds: [subscription.feedId],
    retainedSourceKey: unreadKey,
    retainedQueueEntryIds: [entryId],
    retainedPendingEntryIds: null,
    retainedQueueGeneration: 1,
    retainedSnapshotGeneration: null,
    retainedPendingSnapshotGeneration: null,
    invalidateAllSources: true,
  })

  expect(state.pendingMutationByEntryId[entryId]).toBeUndefined()
  expect(state.optimisticMutationsById[40]).toBeUndefined()

  state = readerReducer(state, {
    type: "entryMutationFailed",
    mutationId: 40,
    error: "Older request failed",
  })
  expect(state.entriesById[entryId]?.isRead).toBe(true)
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(0)
  expect(state.queueBySourceKey[unreadKey]).toEqual([])
})

it("commits a stable category projection while broader caches revalidate", () => {
  const subscription = makeSubscription({ categoryId })
  const categorySource = { kind: "category", categoryId } as const
  const categoryKey = sourceKey(categorySource)
  const state: ReaderState = {
    ...initialReaderState,
    selectedSource: categorySource,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: { [entryId]: makeEntry() },
    queueBySourceKey: { [categoryKey]: [entryId] },
    snapshotGenerationBySource: { [categoryKey]: 7 },
    paneStatus: { ...initialReaderState.paneStatus, queue: "ready" },
  }

  const reloading = readerReducer(state, {
    type: "bulkReadCommitted",
    entryIds: [entryId],
    affectedFeedIds: [subscription.feedId],
    retainedSourceKey: categoryKey,
    retainedQueueEntryIds: [entryId],
    retainedPendingEntryIds: null,
    retainedQueueGeneration: 0,
    retainedSnapshotGeneration: 7,
    retainedPendingSnapshotGeneration: null,
    invalidateAllSources: true,
  })

  expect(reloading.entriesById[entryId]?.isRead).toBe(true)
  expect(reloading.subscriptionsById[subscriptionId]?.unreadCount).toBe(
    subscription.unreadCount,
  )
  expect(reloading.queueBySourceKey[categoryKey]).toEqual([entryId])
  expect(reloading.snapshotGenerationBySource[categoryKey]).toBe(7)
  expect(reloading.paneStatus.queue).toBe("ready")
})

it("keeps a concurrent star mutation independent from a bulk read", () => {
  const unreadSource = { kind: "smart", state: "UNREAD" } as const
  const unreadKey = sourceKey(unreadSource)
  const subscription = makeSubscription({ unreadCount: 1 })
  let state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: { [entryId]: makeEntry() },
    queueBySourceKey: { [unreadKey]: [entryId] },
  }
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 41,
    entryId,
    field: "isStarred",
    value: true,
  })
  state = readerReducer(state, {
    type: "bulkReadCommitted",
    entryIds: [entryId],
    affectedFeedIds: [subscription.feedId],
    retainedSourceKey: unreadKey,
    retainedQueueEntryIds: [entryId],
    retainedPendingEntryIds: null,
    retainedQueueGeneration: 1,
    retainedSnapshotGeneration: null,
    retainedPendingSnapshotGeneration: null,
    invalidateAllSources: true,
  })

  expect(state.pendingMutationByEntryId[entryId]?.isStarred).toBe(41)
  expect(state.optimisticMutationsById[41]?.field).toBe("isStarred")

  state = readerReducer(state, {
    type: "entryMutationFailed",
    mutationId: 41,
    error: "Star request failed",
  })
  expect(state.entriesById[entryId]?.isStarred).toBe(false)
  expect(state.errors.mutation).toBe("Star request failed")
})

it("does not derive an authoritative unread count from a partial pending window", () => {
  const pendingEntryId = "00000000-0000-4000-8000-000000000302"
  const unreadSource = { kind: "smart", state: "UNREAD" } as const
  const unreadKey = sourceKey(unreadSource)
  const subscription = makeSubscription({ unreadCount: 1_000 })
  const state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: {
      [entryId]: makeEntry(),
      [pendingEntryId]: makeEntry({ entryId: pendingEntryId }),
    },
    queueBySourceKey: { [unreadKey]: [entryId] },
    pendingNewEntriesBySource: { [unreadKey]: [pendingEntryId] },
  }

  const committed = readerReducer(state, {
    type: "bulkReadCommitted",
    entryIds: [entryId],
    affectedFeedIds: [subscription.feedId],
    retainedSourceKey: unreadKey,
    retainedQueueEntryIds: [entryId],
    retainedPendingEntryIds: null,
    retainedQueueGeneration: 0,
    retainedSnapshotGeneration: null,
    retainedPendingSnapshotGeneration: null,
    invalidateAllSources: true,
  })

  expect(committed.subscriptionsById[subscriptionId]?.unreadCount).toBe(1_000)
})

it("invalidates a retained projection that changed while bulk read was in flight", () => {
  const replacementEntryId = "00000000-0000-4000-8000-000000000302"
  const unreadSource = { kind: "smart", state: "UNREAD" } as const
  const unreadKey = sourceKey(unreadSource)
  const subscription = makeSubscription({ unreadCount: 2 })
  const state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: {
      [entryId]: makeEntry(),
      [replacementEntryId]: makeEntry({ entryId: replacementEntryId }),
    },
    queueBySourceKey: { [unreadKey]: [replacementEntryId, entryId] },
    snapshotGenerationBySource: { [unreadKey]: 8 },
    requestGenerationByPane: { subscriptions: 0, queue: 4, detail: 0 },
    paneStatus: { ...initialReaderState.paneStatus, queue: "ready" },
  }

  const committed = readerReducer(state, {
    type: "bulkReadCommitted",
    entryIds: [entryId],
    affectedFeedIds: [subscription.feedId],
    retainedSourceKey: unreadKey,
    retainedQueueEntryIds: [entryId],
    retainedPendingEntryIds: null,
    retainedQueueGeneration: 4,
    retainedSnapshotGeneration: 8,
    retainedPendingSnapshotGeneration: null,
    invalidateAllSources: false,
  })

  expect(committed.entriesById[entryId]?.isRead).toBe(true)
  expect(committed.entriesById[replacementEntryId]?.isRead).toBe(false)
  expect(committed.queueBySourceKey[unreadKey]).toBeUndefined()
  expect(committed.snapshotGenerationBySource[unreadKey]).toBeUndefined()
  expect(committed.paneStatus.queue).toBe("idle")
})

it("invalidates a retained projection when its pending window changes", () => {
  const pendingEntryId = "00000000-0000-4000-8000-000000000302"
  const unreadKey = sourceKey({ kind: "smart", state: "UNREAD" })
  const subscription = makeSubscription({ unreadCount: 2 })
  const state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: {
      [entryId]: makeEntry(),
      [pendingEntryId]: makeEntry({ entryId: pendingEntryId }),
    },
    queueBySourceKey: { [unreadKey]: [entryId] },
    pendingNewEntriesBySource: { [unreadKey]: [pendingEntryId] },
    pendingNewEntryCountBySource: { [unreadKey]: 1 },
    snapshotGenerationBySource: { [unreadKey]: 7 },
    pendingSnapshotGenerationBySource: { [unreadKey]: 8 },
    requestGenerationByPane: { subscriptions: 0, queue: 4, detail: 0 },
    paneStatus: { ...initialReaderState.paneStatus, queue: "ready" },
  }

  const committed = readerReducer(state, {
    type: "bulkReadCommitted",
    entryIds: [entryId],
    affectedFeedIds: [subscription.feedId],
    retainedSourceKey: unreadKey,
    retainedQueueEntryIds: [entryId],
    retainedPendingEntryIds: null,
    retainedQueueGeneration: 4,
    retainedSnapshotGeneration: 7,
    retainedPendingSnapshotGeneration: null,
    invalidateAllSources: false,
  })

  expect(committed.entriesById[entryId]?.isRead).toBe(true)
  expect(committed.entriesById[pendingEntryId]?.isRead).toBe(false)
  expect(committed.queueBySourceKey[unreadKey]).toBeUndefined()
  expect(committed.pendingNewEntriesBySource[unreadKey]).toBeUndefined()
  expect(committed.pendingSnapshotGenerationBySource[unreadKey]).toBeUndefined()
  expect(committed.paneStatus.queue).toBe("idle")
})

it("invalidates every cached source after a category-scoped bulk read", () => {
  const otherFeedId = "00000000-0000-4000-8000-000000000102"
  const otherEntryId = "00000000-0000-4000-8000-000000000302"
  const categorySource = { kind: "category", categoryId } as const
  const categoryKey = sourceKey(categorySource)
  const otherFeedKey = sourceKey({ kind: "feed", feedId: otherFeedId })
  const subscription = makeSubscription({ categoryId })
  const state: ReaderState = {
    ...initialReaderState,
    selectedSource: categorySource,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: {
      [entryId]: makeEntry(),
      [otherEntryId]: makeEntry({ entryId: otherEntryId, feedId: otherFeedId }),
    },
    queueBySourceKey: {
      [categoryKey]: [entryId],
      [otherFeedKey]: [otherEntryId],
    },
    snapshotGenerationBySource: { [categoryKey]: 7, [otherFeedKey]: 8 },
  }

  const committed = readerReducer(state, {
    type: "bulkReadCommitted",
    entryIds: [entryId],
    affectedFeedIds: [subscription.feedId],
    retainedSourceKey: categoryKey,
    retainedQueueEntryIds: [entryId],
    retainedPendingEntryIds: null,
    retainedQueueGeneration: 0,
    retainedSnapshotGeneration: 7,
    retainedPendingSnapshotGeneration: null,
    invalidateAllSources: true,
  })

  expect(committed.queueBySourceKey[otherFeedKey]).toBeUndefined()
  expect(committed.snapshotGenerationBySource[otherFeedKey]).toBeUndefined()
})

it("does not let a star response overwrite a newer read projection", () => {
  let state: ReaderState = {
    ...initialReaderState,
    entriesById: { [entryId]: makeEntry({ isRead: true }) },
    detailsById: { [entryId]: makeDetail({ isRead: true }) },
  }
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 42,
    entryId,
    field: "isStarred",
    value: true,
  })
  state = readerReducer(state, {
    type: "entryMutationSucceeded",
    mutationId: 42,
    state: { entryId, isRead: false, isStarred: true },
  })

  expect(state.entriesById[entryId]).toMatchObject({ isRead: true, isStarred: true })
  expect(state.detailsById[entryId]).toMatchObject({ isRead: true, isStarred: true })
})

it("uses the server state as authoritative after an optimistic star change", () => {
  const entry = makeEntry()
  const detail = makeDetail()
  let state: ReaderState = {
    ...initialReaderState,
    entriesById: { [entryId]: entry },
    detailsById: { [entryId]: detail },
  }
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 2,
    entryId,
    field: "isStarred",
    value: true,
  })
  expect(state.entriesById[entryId]?.isStarred).toBe(true)

  state = readerReducer(state, {
    type: "entryMutationSucceeded",
    mutationId: 2,
    state: { entryId, isRead: false, isStarred: false },
  })

  expect(state.entriesById[entryId]?.isStarred).toBe(false)
  expect(state.detailsById[entryId]?.isStarred).toBe(false)
  expect(state.pendingMutationByEntryId[entryId]?.isStarred).toBeUndefined()
})

it("does not let an older rollback overwrite a newer mutation", () => {
  const subscription = makeSubscription()
  let state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: { [entryId]: makeEntry() },
  }
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 10,
    entryId,
    field: "isRead",
    value: true,
  })
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 11,
    entryId,
    field: "isRead",
    value: false,
  })
  state = readerReducer(state, {
    type: "entryMutationFailed",
    mutationId: 10,
    error: "Older request failed",
  })

  expect(state.entriesById[entryId]?.isRead).toBe(false)
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(3)
  expect(state.pendingMutationByEntryId[entryId]?.isRead).toBe(11)
  expect(state.errors.mutation).toBeNull()
})

it("keeps pending read and star state across stale source detail and subscription reloads", () => {
  const source = { kind: "smart", state: "UNREAD" } as const
  const subscription = makeSubscription()
  let state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: subscription },
    subscriptionOrder: [subscriptionId],
    entriesById: { [entryId]: makeEntry() },
    detailsById: { [entryId]: makeDetail() },
  }
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 20,
    entryId,
    field: "isRead",
    value: true,
  })
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 21,
    entryId,
    field: "isStarred",
    value: true,
  })
  state = readerReducer(state, { type: "sourceRequested", source, generation: 1 })
  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 1,
    snapshotGeneration: 1,
    entries: [makeEntry({ title: "Reloaded", isRead: false, isStarred: false })],
    mode: "replace",
  })
  state = readerReducer(state, { type: "entrySelected", entryId })
  state = readerReducer(state, { type: "detailRequested", entryId, generation: 1 })
  state = readerReducer(state, {
    type: "detailReceived",
    entryId,
    generation: 1,
    detail: makeDetail({ title: "Reloaded", isRead: false, isStarred: false }),
  })
  state = readerReducer(state, { type: "subscriptionsRequested", generation: 1 })
  state = readerReducer(state, {
    type: "subscriptionsReceived",
    generation: 1,
    subscriptions: [makeSubscription({ unreadCount: 3 })],
    categories: [],
  })
  state = readerReducer(state, {
    type: "subscriptionUpserted",
    subscription: makeSubscription({ unreadCount: 3 }),
  })

  expect(state.entriesById[entryId]).toMatchObject({
    title: "Reloaded",
    isRead: true,
    isStarred: true,
  })
  expect(state.detailsById[entryId]).toMatchObject({ isRead: true, isStarred: true })
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(2)

  state = readerReducer(state, {
    type: "entryMutationSucceeded",
    mutationId: 21,
    state: { entryId, isRead: false, isStarred: true },
  })
  state = readerReducer(state, {
    type: "entryMutationSucceeded",
    mutationId: 20,
    state: { entryId, isRead: true, isStarred: true },
  })

  expect(state.entriesById[entryId]).toMatchObject({ isRead: true, isStarred: true })
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(2)
})

it("rolls back one unread delta after a stale reload during a pending mutation", () => {
  const source = { kind: "smart", state: "UNREAD" } as const
  let state: ReaderState = {
    ...initialReaderState,
    subscriptionsById: { [subscriptionId]: makeSubscription() },
    subscriptionOrder: [subscriptionId],
    entriesById: { [entryId]: makeEntry() },
  }
  state = readerReducer(state, {
    type: "entryMutationStarted",
    mutationId: 30,
    entryId,
    field: "isRead",
    value: true,
  })
  state = readerReducer(state, { type: "sourceRequested", source, generation: 1 })
  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 1,
    snapshotGeneration: 1,
    entries: [makeEntry({ isRead: false })],
    mode: "replace",
  })
  state = readerReducer(state, { type: "subscriptionsRequested", generation: 1 })
  state = readerReducer(state, {
    type: "subscriptionsReceived",
    generation: 1,
    subscriptions: [makeSubscription({ unreadCount: 3 })],
    categories: [],
  })
  state = readerReducer(state, {
    type: "entryMutationFailed",
    mutationId: 30,
    error: "Request failed",
  })

  expect(state.entriesById[entryId]?.isRead).toBe(false)
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(3)
})

it("clears the previous feed view when a subscription changes Feed identity", () => {
  const subscription = makeSubscription()
  const previousSource = { kind: "feed", feedId: subscription.feedId } as const
  const previousKey = sourceKey(previousSource)
  const replacement = makeSubscription({
    feedId: "00000000-0000-4000-8000-000000000102",
    feedUrl: "https://publisher.example/replacement.xml",
  })
  const state = readerReducer(
    {
      ...initialReaderState,
      subscriptionsById: { [subscriptionId]: subscription },
      subscriptionOrder: [subscriptionId],
      selectedSource: previousSource,
      selectedEntryId: entryId,
      feedSearchQuery: "rust",
      queueBySourceKey: { [previousKey]: [entryId] },
      pendingNewEntriesBySource: { [previousKey]: [entryId] },
      pendingNewEntryCountBySource: { [previousKey]: 1 },
      snapshotGenerationBySource: { [previousKey]: 2 },
      pendingSnapshotGenerationBySource: { [previousKey]: 3 },
    },
    { type: "subscriptionUpserted", subscription: replacement },
  )

  expect(state.subscriptionsById[subscriptionId]).toEqual(replacement)
  expect(state.queueBySourceKey[previousKey]).toBeUndefined()
  expect(state.pendingNewEntriesBySource[previousKey]).toBeUndefined()
  expect(state.pendingNewEntryCountBySource[previousKey]).toBeUndefined()
  expect(state.snapshotGenerationBySource[previousKey]).toBeUndefined()
  expect(state.pendingSnapshotGenerationBySource[previousKey]).toBeUndefined()
  expect(state.retiredFeedIds[subscription.feedId]).toBe(true)
  expect(state.selectedSource).toEqual({ kind: "smart", state: "UNREAD" })
  expect(state.selectedEntryId).toBeNull()
  expect(state.feedSearchQuery).toBe("")
})
