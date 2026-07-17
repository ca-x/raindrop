import { expect, it } from "vitest"

import { initialReaderState, readerReducer } from "./reducer"
import type { ReaderState } from "./types"
import { sourceKey } from "./types"
import {
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
    entries: [makeEntry({ isRead: false })],
    mode: "replace",
  })
  state = readerReducer(state, { type: "subscriptionsRequested", generation: 1 })
  state = readerReducer(state, {
    type: "subscriptionsReceived",
    generation: 1,
    subscriptions: [makeSubscription({ unreadCount: 3 })],
  })
  state = readerReducer(state, {
    type: "entryMutationFailed",
    mutationId: 30,
    error: "Request failed",
  })

  expect(state.entriesById[entryId]?.isRead).toBe(false)
  expect(state.subscriptionsById[subscriptionId]?.unreadCount).toBe(3)
})
