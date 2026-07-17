import { expect, it } from "vitest"

import { initialReaderState, readerReducer } from "./reducer"
import { entryId, makeDetail, makeEntry } from "./testFixtures"
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
