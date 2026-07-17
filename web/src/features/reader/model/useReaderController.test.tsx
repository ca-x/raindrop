import { act, renderHook } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import type { ListSubscriptionsOptions } from "../api/subscriptions"
import type { ReaderApi } from "./controllerApi"
import { entryId, makeDetail, makeEntry, makeSubscription } from "./testFixtures"
import { sourceKey } from "./types"
import { useReaderController } from "./useReaderController"

it("loads every subscription page and the selected source through injected clients", async () => {
  const subscription = makeSubscription()
  const entry = makeEntry()
  const listSubscriptions = vi.fn(async ({ cursor }: ListSubscriptionsOptions = {}) =>
    cursor === undefined
      ? { items: [subscription], nextCursor: "next" }
      : { items: [], nextCursor: null },
  )
  const listEntries = vi.fn(async () => ({
    items: [entry],
    nextCursor: null,
    snapshotGeneration: 1,
  }))
  const api = makeApi({ listSubscriptions, listEntries })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api,
    }),
  )

  await act(async () => result.current.load())

  expect(listSubscriptions).toHaveBeenCalledTimes(2)
  expect(listSubscriptions.mock.calls.map(([options]) => options?.cursor)).toEqual([
    undefined,
    "next",
  ])
  expect(listSubscriptions.mock.calls.every(([options]) => options?.signal instanceof AbortSignal)).toBe(
    true,
  )
  expect(listEntries).toHaveBeenCalledWith(
    expect.objectContaining({ state: "UNREAD", signal: expect.any(AbortSignal) }),
  )
  expect(result.current.state.subscriptionOrder).toEqual([
    subscription.subscriptionId,
  ])
  expect(
    result.current.state.queueBySourceKey[
      sourceKey({ kind: "smart", state: "UNREAD" })
    ],
  ).toEqual([entry.entryId])
})

it("aborts the previous detail request and ignores its late response", async () => {
  const secondEntryId = "00000000-0000-4000-8000-000000000302"
  const first = deferred<ReturnType<typeof makeDetail>>()
  const second = deferred<ReturnType<typeof makeDetail>>()
  const getEntry = vi.fn((requestedEntryId: string, _signal?: AbortSignal) =>
    requestedEntryId === entryId ? first.promise : second.promise,
  )
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ getEntry }),
    }),
  )

  let firstLoad!: Promise<void>
  let secondLoad!: Promise<void>
  act(() => {
    firstLoad = result.current.selectEntry(entryId)
  })
  act(() => {
    secondLoad = result.current.selectEntry(secondEntryId)
  })
  expect(getEntry.mock.calls[0]?.[1]?.aborted).toBe(true)

  first.resolve(makeDetail({ title: "Late detail" }))
  await act(async () => firstLoad)
  expect(result.current.state.detailsById[entryId]).toBeUndefined()

  second.resolve(makeDetail({ entryId: secondEntryId, title: "Winning detail" }))
  await act(async () => secondLoad)
  expect(result.current.state.selectedEntryId).toBe(secondEntryId)
  expect(result.current.state.detailsById[secondEntryId]?.title).toBe("Winning detail")
})

it("discovers stored entries without reordering until merge and selects feed sources", async () => {
  const newEntryId = "00000000-0000-4000-8000-000000000302"
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 1,
    })
    .mockResolvedValueOnce({
      items: [
        makeEntry({ entryId: newEntryId, sortAtUs: 2 }),
        makeEntry({ title: "Updated stored entity" }),
      ],
      nextCursor: null,
      snapshotGeneration: 2,
    })
    .mockResolvedValueOnce({
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 3,
    })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listEntries }),
    }),
  )
  const unread = { kind: "smart", state: "UNREAD" } as const

  await act(async () => result.current.load())
  await act(async () => result.current.reloadEntries())
  expect(result.current.state.queueBySourceKey[sourceKey(unread)]).toEqual([entryId])
  expect(result.current.state.pendingNewEntriesBySource[sourceKey(unread)]).toEqual([
    newEntryId,
  ])

  act(() => result.current.mergePendingEntries())
  expect(result.current.state.queueBySourceKey[sourceKey(unread)]).toEqual([
    newEntryId,
    entryId,
  ])

  const feed = { kind: "feed", feedId: makeEntry().feedId } as const
  await act(async () => result.current.selectSource(feed))
  expect(listEntries).toHaveBeenLastCalledWith(
    expect.objectContaining({ feedId: feed.feedId, state: "ALL" }),
  )
  expect(result.current.state.selectedSource).toEqual(feed)
})

function makeApi(overrides: Partial<ReaderApi> = {}): ReaderApi {
  return {
    listSubscriptions: vi.fn(async () => ({ items: [], nextCursor: null })),
    getSubscription: vi.fn(),
    createSubscription: vi.fn(),
    deleteSubscription: vi.fn(),
    refreshSubscription: vi.fn(),
    listEntries: vi.fn(async () => ({
      items: [],
      nextCursor: null,
      snapshotGeneration: 0,
    })),
    getEntry: vi.fn(),
    patchEntryState: vi.fn(),
    ...overrides,
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}
