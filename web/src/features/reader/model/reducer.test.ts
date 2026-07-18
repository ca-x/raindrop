import { expect, it } from "vitest"

import type { EntryListItemResponse } from "../api/reader.generated"
import { initialReaderState, readerReducer } from "./reducer"
import { makeDetail } from "./testFixtures"
import { sourceKey, type ReaderSource } from "./types"
import type { ReaderState } from "./types"

const feedId = "00000000-0000-4000-8000-000000000101"
const entryId = "00000000-0000-4000-8000-000000000301"
const newEntryId = "00000000-0000-4000-8000-000000000302"
const allSource: ReaderSource = { kind: "smart", state: "ALL" }
const feedSource: ReaderSource = { kind: "feed", feedId }

it("shares one normalized entry entity across source queues", () => {
  const original = entry({ title: "Original title" })
  const updated = entry({ title: "Updated title", isStarred: true })

  let state = readerReducer(initialReaderState, {
    type: "sourceSelected",
    source: allSource,
  })
  state = readerReducer(state, {
    type: "sourceRequested",
    source: allSource,
    generation: 1,
  })
  state = readerReducer(state, {
    type: "sourceReceived",
    source: allSource,
    generation: 1,
    snapshotGeneration: 1,
    entries: [original],
    mode: "replace",
  })
  state = readerReducer(state, { type: "sourceSelected", source: feedSource })
  state = readerReducer(state, {
    type: "sourceRequested",
    source: feedSource,
    generation: 2,
  })
  state = readerReducer(state, {
    type: "sourceReceived",
    source: feedSource,
    generation: 2,
    snapshotGeneration: 2,
    entries: [updated],
    mode: "replace",
  })

  expect(state.queueBySourceKey[sourceKey(allSource)]).toEqual([entryId])
  expect(state.queueBySourceKey[sourceKey(feedSource)]).toEqual([entryId])
  expect(state.entriesById[entryId]).toMatchObject({
    title: "Updated title",
    isStarred: true,
  })
})

it("keeps the queue stable until pending new entries are explicitly merged", () => {
  const source: ReaderSource = { kind: "smart", state: "UNREAD" }
  let state = readerReducer(initialReaderState, {
    type: "sourceRequested",
    source,
    generation: 1,
  })
  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 1,
    snapshotGeneration: 1,
    entries: [entry()],
    mode: "replace",
  })
  state = readerReducer(state, {
    type: "sourceRequested",
    source,
    generation: 2,
  })
  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 2,
    snapshotGeneration: 2,
    entries: [
      entry({ entryId: newEntryId, title: "New entry", sortAtUs: 2 }),
      entry({ title: "Updated in place" }),
    ],
    mode: "discover",
  })

  expect(state.queueBySourceKey[sourceKey(source)]).toEqual([entryId])
  expect(state.pendingNewEntriesBySource[sourceKey(source)]).toEqual([newEntryId])
  expect(state.pendingNewEntryCountBySource[sourceKey(source)]).toBe(1)
  expect(state.snapshotGenerationBySource[sourceKey(source)]).toBe(1)
  expect(state.pendingSnapshotGenerationBySource[sourceKey(source)]).toBe(2)
  expect(state.entriesById[entryId]?.title).toBe("Updated in place")

  state = readerReducer(state, { type: "pendingEntriesMerged", source })
  expect(state.queueBySourceKey[sourceKey(source)]).toEqual([newEntryId, entryId])
  expect(state.pendingNewEntriesBySource[sourceKey(source)]).toEqual([])
  expect(state.pendingNewEntryCountBySource[sourceKey(source)]).toBe(0)
  expect(state.snapshotGenerationBySource[sourceKey(source)]).toBe(2)
  expect(state.pendingSnapshotGenerationBySource[sourceKey(source)]).toBeUndefined()
})

it("clears Feed search when selecting another source", () => {
  let state = readerReducer(initialReaderState, {
    type: "feedSearchChanged",
    query: "rust",
  })
  state = readerReducer(state, {
    type: "sourceSelected",
    source: { kind: "smart", state: "ALL" },
  })
  expect(state.feedSearchQuery).toBe("")
})

it("records route scroll anchors with the workspace state", () => {
  const state = readerReducer(initialReaderState, {
    type: "scrollAnchorRecorded",
    route: "/reader/unread",
    offset: 420,
  })

  expect(state.scrollAnchorByRoute["/reader/unread"]).toBe(420)
})

it("updates loaded detail metadata when a stored entity changes", () => {
  const source: ReaderSource = { kind: "smart", state: "UNREAD" }
  const key = sourceKey(source)
  let state: ReaderState = {
    ...initialReaderState,
    entriesById: { [entryId]: entry() },
    queueBySourceKey: { [key]: [entryId] },
    detailsById: { [entryId]: makeDetail({ title: "Old detail title" }) },
  }
  state = readerReducer(state, { type: "sourceRequested", source, generation: 1 })
  state = readerReducer(state, {
    type: "sourceReceived",
    source,
    generation: 1,
    snapshotGeneration: 2,
    entries: [entry({ title: "Server title", isRead: true })],
    mode: "discover",
  })

  expect(state.detailsById[entryId]).toMatchObject({
    title: "Server title",
    isRead: true,
    contentHtml: "<p>Safe article</p>",
  })
})

function entry(
  overrides: Partial<EntryListItemResponse> = {},
): EntryListItemResponse {
  return {
    entryId,
    feedId,
    feedTitle: "Example Feed",
    siteUrl: "https://example.com/",
    title: "Entry title",
    author: "Reader",
    summary: "Summary",
    canonicalUrl: "https://example.com/article",
    publishedAtUs: 1_784_246_400_000_000,
    sortAtUs: 1_784_246_400_000_000,
    isRead: false,
    isStarred: false,
    ...overrides,
  }
}
