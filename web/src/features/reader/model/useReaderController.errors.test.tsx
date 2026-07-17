import { act, renderHook } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"

import { getEntry, listEntries, patchEntryState } from "../api/entries"
import type { ReaderApi } from "./controllerApi"
import { GENERIC_READER_ERROR } from "./controllerErrors"
import { entryId, makeDetail, makeEntry } from "./testFixtures"
import { useReaderController } from "./useReaderController"

afterEach(() => vi.unstubAllGlobals())

it("surfaces a malformed entry page as a presentation-safe queue error", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      jsonResponse({ items: {}, nextCursor: null, snapshotGeneration: 1 }),
    ),
  )
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listEntries }),
    }),
  )

  await act(async () => result.current.load())

  expect(result.current.state.errors.queue).toBe(GENERIC_READER_ERROR)
})

it("surfaces malformed entry detail without exposing response internals", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(jsonResponse({ ...makeDetail(), contentHtml: 7 })),
  )
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ getEntry }),
    }),
  )

  await act(async () => result.current.selectEntry(entryId))

  expect(result.current.state.detailsById[entryId]).toBeUndefined()
  expect(result.current.state.errors.detail).toBe(GENERIC_READER_ERROR)
})

it("rejects a mismatched state response and rolls back the optimistic value", async () => {
  const otherEntryId = "00000000-0000-4000-8000-000000000302"
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      jsonResponse({ entryId: otherEntryId, isRead: true, isStarred: false }),
    ),
  )
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ patchEntryState }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.toggleRead(entryId))

  expect(result.current.state.entriesById[entryId]?.isRead).toBe(false)
  expect(result.current.state.errors.mutation).toBe(GENERIC_READER_ERROR)
})

it("rejects a malformed state response through the Task 1 PATCH validator", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(jsonResponse({ entryId, isRead: true })),
  )
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ patchEntryState }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.toggleRead(entryId))

  expect(result.current.state.entriesById[entryId]?.isRead).toBe(false)
  expect(result.current.state.errors.mutation).toBe(GENERIC_READER_ERROR)
})

function makeApi(overrides: Partial<ReaderApi> = {}): ReaderApi {
  return {
    listSubscriptions: vi.fn(async () => ({ items: [], nextCursor: null })),
    getSubscription: vi.fn(),
    createSubscription: vi.fn(),
    deleteSubscription: vi.fn(),
    refreshSubscription: vi.fn(),
    listEntries: vi.fn(async () => ({
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 1,
    })),
    getEntry: vi.fn(),
    patchEntryState: vi.fn(),
    ...overrides,
  }
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}
