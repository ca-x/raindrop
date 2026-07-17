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
