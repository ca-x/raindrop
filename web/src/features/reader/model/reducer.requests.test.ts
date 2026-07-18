import { expect, it } from "vitest"

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
    entries: [makeEntry({ title: "Pre-expiry response" })],
    mode: "replace",
  })
  expect(state.entriesById[entryId]).toBeUndefined()

  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 8,
    entries: [makeEntry({ title: "Post-expiry response" })],
    mode: "replace",
  })
  expect(state.entriesById[entryId]?.title).toBe("Post-expiry response")
})
