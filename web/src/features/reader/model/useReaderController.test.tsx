import { act, renderHook } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"

import type { ReaderCache, ReaderCacheSnapshot } from "../cache/readerCache"
import type { ListSubscriptionsOptions } from "../api/subscriptions"
import type { ReaderApi } from "./controllerApi"
import {
  categoryId,
  entryId,
  makeCategory,
  makeDetail,
  makeEntry,
  makeSubscription,
} from "./testFixtures"
import { sourceKey } from "./types"
import { useReaderController } from "./useReaderController"

const userId = "11111111-1111-4111-8111-111111111111"

afterEach(() => vi.useRealTimers())

it("hydrates cached Reader rows before delayed requests and reconciles to the server result", async () => {
  const categories = deferred<{ items: ReturnType<typeof makeCategory>[] }>()
  const subscriptions = deferred<{
    items: ReturnType<typeof makeSubscription>[]
    nextCursor: null
  }>()
  const entries = deferred<{
    items: ReturnType<typeof makeEntry>[]
    nextCursor: null
    snapshotGeneration: number
  }>()
  const cachedSubscription = makeSubscription({ title: "Cached feed", unreadCount: 9 })
  const cachedEntry = makeEntry({ title: "Cached entry" })
  const cache: ReaderCache = {
    load: vi.fn(async () => cacheSnapshot(cachedSubscription, cachedEntry)),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const api = makeApi({
    listCategories: vi.fn(() => categories.promise),
    listSubscriptions: vi.fn(() => subscriptions.promise),
    listEntries: vi.fn(() => entries.promise),
  })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated: vi.fn(),
      api,
    }),
  )

  let load!: Promise<void>
  act(() => { load = result.current.load() })
  await act(async () => { await Promise.resolve() })

  const unreadKey = sourceKey({ kind: "smart", state: "UNREAD" })
  expect(result.current.state.paneStatus).toMatchObject({
    subscriptions: "ready",
    queue: "ready",
  })
  expect(result.current.state.subscriptionOrder).toEqual([
    cachedSubscription.subscriptionId,
  ])
  expect(result.current.state.queueBySourceKey[unreadKey]).toEqual([cachedEntry.entryId])

  const freshSubscriptionId = "00000000-0000-4000-8000-000000000202"
  const freshEntryId = "00000000-0000-4000-8000-000000000302"
  categories.resolve({ items: [] })
  subscriptions.resolve({
    items: [makeSubscription({
      subscriptionId: freshSubscriptionId,
      title: "Fresh feed",
      unreadCount: 1,
    })],
    nextCursor: null,
  })
  entries.resolve({
    items: [makeEntry({ entryId: freshEntryId, title: "Fresh entry", isRead: true })],
    nextCursor: null,
    snapshotGeneration: 8,
  })
  await act(async () => load)

  expect(result.current.state.subscriptionOrder).toEqual([freshSubscriptionId])
  expect(result.current.state.subscriptionsById[cachedSubscription.subscriptionId]).toBeUndefined()
  expect(result.current.state.queueBySourceKey[unreadKey]).toEqual([freshEntryId])
  expect(result.current.state.entriesById[freshEntryId]).toMatchObject({
    title: "Fresh entry",
    isRead: true,
  })
})

it("serializes best-effort cache writes so a newer state cannot be overwritten", async () => {
  vi.useFakeTimers()
  const firstSave = deferred<void>()
  const save = vi
    .fn<(userId: string, snapshot: ReaderCacheSnapshot) => Promise<void>>()
    .mockImplementationOnce(() => firstSave.promise)
    .mockResolvedValue(undefined)
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save,
    clear: vi.fn(async () => undefined),
  }
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated: vi.fn(),
      api: makeApi({
        listCategories: vi.fn(async () => ({ items: [makeCategory()] })),
        listSubscriptions: vi.fn(async () => ({
          items: [makeSubscription()],
          nextCursor: null,
        })),
        listEntries: vi.fn(async () => ({
          items: [makeEntry()],
          nextCursor: null,
          snapshotGeneration: 1,
        })),
      }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => vi.advanceTimersByTimeAsync(100))
  expect(save).toHaveBeenCalledTimes(1)
  expect(JSON.stringify(save.mock.calls[0])).not.toContain("csrf-memory")

  act(() => result.current.recordScrollAnchor("/reader/unread", 256))
  await act(async () => vi.advanceTimersByTimeAsync(100))
  expect(save).toHaveBeenCalledTimes(1)

  firstSave.resolve()
  await act(async () => { await Promise.resolve() })
  expect(save).toHaveBeenCalledTimes(2)
  expect(save.mock.calls[1]?.[1].scrollAnchorByRoute).toEqual({
    "/reader/unread": 256,
  })
})

it("loads every subscription page and the selected source through injected clients", async () => {
  const subscription = makeSubscription()
  const category = makeCategory()
  const entry = makeEntry()
  const listCategories = vi.fn(async () => ({ items: [category] }))
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
  const api = makeApi({ listCategories, listSubscriptions, listEntries })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api,
    }),
  )

  await act(async () => result.current.load())

  expect(listSubscriptions).toHaveBeenCalledTimes(2)
  expect(listCategories).toHaveBeenCalledWith(expect.any(AbortSignal))
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
  expect(result.current.state.categoryOrder).toEqual([category.categoryId])
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
    .mockResolvedValueOnce({
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 4,
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
  expect(result.current.state.snapshotGenerationBySource[sourceKey(unread)]).toBe(1)
  expect(result.current.state.pendingSnapshotGenerationBySource[sourceKey(unread)]).toBe(2)

  act(() => result.current.mergePendingEntries())
  expect(result.current.state.queueBySourceKey[sourceKey(unread)]).toEqual([
    newEntryId,
    entryId,
  ])
  expect(result.current.state.snapshotGenerationBySource[sourceKey(unread)]).toBe(2)

  const feed = { kind: "feed", feedId: makeEntry().feedId } as const
  await act(async () => result.current.selectSource(feed))
  expect(listEntries).toHaveBeenLastCalledWith(
    expect.objectContaining({ feedId: feed.feedId, state: "ALL" }),
  )
  expect(result.current.state.selectedSource).toEqual(feed)

  const category = { kind: "category", categoryId } as const
  await act(async () => result.current.selectSource(category))
  expect(listEntries).toHaveBeenLastCalledWith(
    expect.objectContaining({ categoryId, state: "ALL" }),
  )
  expect(result.current.state.selectedSource).toEqual(category)
})

it("searches only the selected Feed and clears search on source change", async () => {
  const listEntries = vi.fn(async () => ({
    items: [makeEntry()],
    nextCursor: null,
    snapshotGeneration: 5,
  }))
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listEntries }),
    }),
  )
  const feed = { kind: "feed", feedId: makeEntry().feedId } as const
  await act(async () => result.current.selectSource(feed))
  await act(async () => result.current.searchFeed("  Rust storage  "))
  expect(result.current.state.feedSearchQuery).toBe("Rust storage")
  expect(listEntries).toHaveBeenLastCalledWith(
    expect.objectContaining({
      feedId: feed.feedId,
      search: "Rust storage",
      signal: expect.any(AbortSignal),
    }),
  )

  await act(async () =>
    result.current.selectSource({ kind: "smart", state: "UNREAD" }),
  )
  expect(result.current.state.feedSearchQuery).toBe("")
  expect(listEntries).toHaveBeenLastCalledWith(
    expect.not.objectContaining({ search: expect.anything() }),
  )
})

it("marks the visible snapshot read then reloads subscriptions and entries", async () => {
  const subscription = makeSubscription({ unreadCount: 3 })
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({ items: [subscription], nextCursor: null })
    .mockResolvedValueOnce({
      items: [{ ...subscription, unreadCount: 0 }],
      nextCursor: null,
    })
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 7,
    })
    .mockResolvedValueOnce({
      items: [],
      nextCursor: null,
      snapshotGeneration: 7,
    })
  const markEntriesRead = vi.fn(async () => undefined)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listSubscriptions, listEntries, markEntriesRead }),
    }),
  )

  await act(async () => result.current.load())
  await act(async () => {
    await expect(result.current.markCurrentSourceRead()).resolves.toBe(true)
  })

  expect(markEntriesRead).toHaveBeenCalledWith(
    { snapshotGeneration: 7 },
    "csrf-memory",
    expect.any(AbortSignal),
  )
  expect(listSubscriptions).toHaveBeenCalledTimes(2)
  expect(listEntries).toHaveBeenCalledTimes(2)
  expect(
    result.current.state.queueBySourceKey[
      sourceKey({ kind: "smart", state: "UNREAD" })
    ],
  ).toEqual([])
  expect(
    result.current.state.subscriptionsById[subscription.subscriptionId]?.unreadCount,
  ).toBe(0)
})

it("marks an unselected feed read from a fresh feed snapshot", async () => {
  const subscription = makeSubscription({ unreadCount: 3 })
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({ items: [subscription], nextCursor: null })
    .mockResolvedValueOnce({
      items: [{ ...subscription, unreadCount: 0 }],
      nextCursor: null,
    })
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 5,
    })
    .mockResolvedValueOnce({
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 9,
    })
    .mockResolvedValueOnce({
      items: [],
      nextCursor: null,
      snapshotGeneration: 9,
    })
  const markEntriesRead = vi.fn(async () => undefined)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listSubscriptions, listEntries, markEntriesRead }),
    }),
  )

  await act(async () => result.current.load())
  await act(async () => {
    await expect(result.current.markFeedRead(subscription.feedId)).resolves.toBe(true)
  })

  expect(listEntries).toHaveBeenNthCalledWith(2, {
    feedId: subscription.feedId,
    state: "ALL",
    limit: 1,
    signal: expect.any(AbortSignal),
  })
  expect(markEntriesRead).toHaveBeenCalledWith(
    { snapshotGeneration: 9, feedId: subscription.feedId },
    "csrf-memory",
    expect.any(AbortSignal),
  )
})

function makeApi(overrides: Partial<ReaderApi> = {}): ReaderApi {
  return {
    listCategories: vi.fn(async () => ({ items: [] })),
    createCategory: vi.fn(),
    updateCategory: vi.fn(),
    deleteCategory: vi.fn(),
    listSubscriptions: vi.fn(async () => ({ items: [], nextCursor: null })),
    getSubscription: vi.fn(),
    createSubscription: vi.fn(),
    deleteSubscription: vi.fn(),
    refreshSubscription: vi.fn(),
    updateSubscription: vi.fn(),
    listEntries: vi.fn(async () => ({
      items: [],
      nextCursor: null,
      snapshotGeneration: 0,
    })),
    getEntry: vi.fn(),
    patchEntryState: vi.fn(),
    markEntriesRead: vi.fn(),
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

function cacheSnapshot(
  subscription = makeSubscription(),
  entry = makeEntry(),
): ReaderCacheSnapshot {
  return {
    categories: [makeCategory()],
    subscriptions: [subscription],
    source: { kind: "smart", state: "UNREAD" },
    entries: [entry],
    queue: [entry.entryId],
    snapshotGeneration: 7,
    scrollAnchorByRoute: { "/reader/unread": 128 },
  }
}
