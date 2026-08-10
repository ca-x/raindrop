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
  const categories = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeCategory>[]
  }>()
  const subscriptions = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeSubscription>[]
    nextCursor: null
  }>()
  const entries = deferred<{
    ownerUserId: string
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
  categories.resolve({ ownerUserId: userId, items: [] })
  subscriptions.resolve({
    ownerUserId: userId,
    items: [makeSubscription({
      subscriptionId: freshSubscriptionId,
      title: "Fresh feed",
      unreadCount: 1,
    })],
    nextCursor: null,
  })
  entries.resolve({
    ownerUserId: userId,
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

it("does not let a pre-mutation revalidation response undo a committed cached entry change", async () => {
  const entries = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeEntry>[]
    nextCursor: null
    snapshotGeneration: number
  }>()
  const cachedEntry = makeEntry({ title: "Cached entry", isRead: false })
  const cache: ReaderCache = {
    load: vi.fn(async () => cacheSnapshot(makeSubscription(), cachedEntry)),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const listEntries = vi
    .fn()
    .mockImplementationOnce(() => entries.promise)
    .mockResolvedValue({
      ownerUserId: userId,
      items: [makeEntry({ title: "Cached entry", isRead: true })],
      nextCursor: null,
      snapshotGeneration: 8,
    })
  const api = makeApi({
    listEntries,
    patchEntryState: vi.fn(async () => ({
      entryId: cachedEntry.entryId,
      isRead: true,
      isStarred: false,
    })),
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
  await act(async () => result.current.toggleRead(cachedEntry.entryId))
  entries.resolve({
    ownerUserId: userId,
    items: [makeEntry({ title: "Stale server entry", isRead: false })],
    nextCursor: null,
    snapshotGeneration: 7,
  })
  await act(async () => load)

  expect(result.current.state.entriesById[cachedEntry.entryId]?.isRead).toBe(true)
  expect(result.current.state.entriesById[cachedEntry.entryId]?.title).toBe("Cached entry")
})

it("serializes cache writes and coalesces pending updates to the latest state", async () => {
  vi.useFakeTimers()
  const firstSave = deferred<void>()
  const save = vi
    .fn<(
      userId: string,
      snapshot: ReaderCacheSnapshot,
      options?: { markValidated?: boolean },
    ) => Promise<void>>()
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
        listCategories: vi.fn(async () => ({ ownerUserId: userId, items: [makeCategory()] })),
        listSubscriptions: vi.fn(async () => ({
          ownerUserId: userId,
          items: [makeSubscription()],
          nextCursor: null,
        })),
        listEntries: vi.fn(async () => ({
          ownerUserId: userId,
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
  expect(save.mock.calls[0]?.[2]).toEqual({ markValidated: true })

  act(() => result.current.recordScrollAnchor("/reader/unread", 128))
  await act(async () => vi.advanceTimersByTimeAsync(100))
  act(() => result.current.recordScrollAnchor("/reader/unread", 256))
  await act(async () => vi.advanceTimersByTimeAsync(100))
  expect(save).toHaveBeenCalledTimes(1)

  firstSave.resolve()
  await act(async () => { await Promise.resolve() })
  expect(save).toHaveBeenCalledTimes(2)
  expect(save.mock.calls[1]?.[1].scrollAnchorByRoute).toEqual({
    "/reader/unread": 256,
  })
  expect(save.mock.calls[1]?.[2]).toBeUndefined()
})

it("clears cache without waiting for an in-flight save and drops queued writes", async () => {
  vi.useFakeTimers()
  const firstSave = deferred<void>()
  const save = vi.fn<ReaderCache["save"]>(() => firstSave.promise)
  const clear = vi.fn(async () => undefined)
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save,
    clear,
  }
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated: vi.fn(),
      api: makeApi(),
    }),
  )
  await act(async () => result.current.load())
  expect(save).toHaveBeenCalledOnce()

  act(() => result.current.recordScrollAnchor("/reader/unread", 256))
  await act(async () => vi.advanceTimersByTimeAsync(100))
  expect(save).toHaveBeenCalledOnce()

  await act(async () => result.current.clearCache())
  expect(clear).toHaveBeenCalledOnce()

  firstSave.resolve()
  await act(async () => { await Promise.resolve() })
  expect(save).toHaveBeenCalledOnce()
})

it("does not create a cold cache until both authoritative panes validate", async () => {
  vi.useFakeTimers()
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated: vi.fn(),
      api: makeApi({
        listCategories: vi.fn(async () => ({ ownerUserId: userId, items: [] })),
        listSubscriptions: vi.fn(async () => ({
          ownerUserId: userId,
          items: [makeSubscription()],
          nextCursor: null,
        })),
        listEntries: vi.fn(async () => { throw new Error("offline") }),
      }),
    }),
  )

  await act(async () => result.current.load())
  await act(async () => vi.advanceTimersByTimeAsync(200))

  expect(cache.save).not.toHaveBeenCalled()
})

it("revalidates visible reader data in the background without reordering the queue", async () => {
  vi.useFakeTimers()
  const storedEntry = makeEntry()
  const discoveredEntry = makeEntry({
    entryId: "00000000-0000-4000-8000-000000000302",
    title: "Background entry",
    sortAtUs: storedEntry.sortAtUs + 1,
  })
  const listSubscriptions = vi.fn(async () => ({
    ownerUserId: userId,
    items: [makeSubscription()],
    nextCursor: null,
  }))
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [storedEntry],
      nextCursor: null,
      snapshotGeneration: 1,
    })
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [discoveredEntry, storedEntry],
      nextCursor: null,
      snapshotGeneration: 2,
    })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listSubscriptions, listEntries }),
    }),
  )

  await act(async () => result.current.load())
  await act(async () => vi.advanceTimersByTimeAsync(60_000))

  const unreadKey = sourceKey({ kind: "smart", state: "UNREAD" })
  expect(listSubscriptions).toHaveBeenCalledTimes(2)
  expect(listEntries).toHaveBeenCalledTimes(2)
  expect(result.current.state.queueBySourceKey[unreadKey]).toEqual([
    storedEntry.entryId,
  ])
  expect(result.current.state.pendingNewEntriesBySource[unreadKey]).toEqual([
    discoveredEntry.entryId,
  ])
})

it("does not advance cached freshness when background validation fails", async () => {
  vi.useFakeTimers()
  const save = vi.fn<ReaderCache["save"]>(async () => undefined)
  const cache: ReaderCache = {
    load: vi.fn(async () => cacheSnapshot()),
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
        listCategories: vi.fn(async () => ({ ownerUserId: userId, items: [makeCategory()] })),
        listSubscriptions: vi.fn(async () => ({
          ownerUserId: userId,
          items: [makeSubscription()],
          nextCursor: null,
        })),
        listEntries: vi.fn(async () => { throw new Error("offline") }),
      }),
    }),
  )

  await act(async () => result.current.load())
  await act(async () => vi.advanceTimersByTimeAsync(200))

  expect(save).toHaveBeenCalled()
  expect(save.mock.calls.every((call) => call[2] === undefined)).toBe(true)
})

it("does not persist an optimistic entry mutation before confirmation", async () => {
  vi.useFakeTimers()
  const mutation = deferred<{
    entryId: string
    isRead: boolean
    isStarred: boolean
  }>()
  const save = vi.fn(async () => undefined)
  const cache: ReaderCache = {
    load: vi.fn(async () => cacheSnapshot()),
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
        listCategories: vi.fn(async () => ({ ownerUserId: userId, items: [makeCategory()] })),
        listSubscriptions: vi.fn(async () => ({
          ownerUserId: userId,
          items: [makeSubscription()],
          nextCursor: null,
        })),
        listEntries: vi.fn(async () => ({
          ownerUserId: userId,
          items: [makeEntry()],
          nextCursor: null,
          snapshotGeneration: 1,
        })),
        patchEntryState: vi.fn(() => mutation.promise),
      }),
    }),
  )
  await act(async () => result.current.load())
  await act(async () => vi.advanceTimersByTimeAsync(100))
  save.mockClear()

  let pending!: Promise<void>
  act(() => { pending = result.current.toggleRead(entryId) })
  await act(async () => vi.advanceTimersByTimeAsync(200))
  expect(save).not.toHaveBeenCalled()

  mutation.resolve({ entryId, isRead: true, isStarred: false })
  await act(async () => pending)
})

it("loads every subscription page and the selected source through injected clients", async () => {
  const subscription = makeSubscription()
  const category = makeCategory()
  const entry = makeEntry()
  const listCategories = vi.fn(async () => ({ ownerUserId: userId, items: [category] }))
  const listSubscriptions = vi.fn(async ({ cursor }: ListSubscriptionsOptions = {}) =>
    cursor === undefined
      ? { ownerUserId: userId, items: [subscription], nextCursor: "next" }
      : { ownerUserId: userId, items: [], nextCursor: null },
  )
  const listEntries = vi.fn(async () => ({
    ownerUserId: userId,
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

it("reveals the first subscription page while later pages are still loading", async () => {
  const laterPage = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeSubscription>[]
    nextCursor: null
  }>()
  const first = makeSubscription({ title: "First page feed" })
  const second = makeSubscription({
    subscriptionId: "00000000-0000-4000-8000-000000000202",
    feedId: "00000000-0000-4000-8000-000000000102",
    title: "Later page feed",
  })
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({ ownerUserId: userId, items: [first], nextCursor: "next" })
    .mockImplementationOnce(() => laterPage.promise)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listSubscriptions }),
    }),
  )

  let loading!: Promise<void>
  act(() => { loading = result.current.load() })
  await vi.waitFor(() => {
    expect(result.current.state.subscriptionOrder).toEqual([first.subscriptionId])
  })
  expect(result.current.state.paneStatus.subscriptions).toBe("ready")
  expect(result.current.state.requestActivity.subscriptions).toBe(true)
  expect(result.current.state.subscriptionsAuthoritative).toBe(false)

  laterPage.resolve({ ownerUserId: userId, items: [second], nextCursor: null })
  await act(async () => loading)

  expect(result.current.state.subscriptionOrder).toEqual([
    first.subscriptionId,
    second.subscriptionId,
  ])
  expect(result.current.state.requestActivity.subscriptions).toBe(false)
  expect(result.current.state.subscriptionsAuthoritative).toBe(true)
})

it("keeps the complete source tree visible during a progressive background refresh", async () => {
  const laterPage = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeSubscription>[]
    nextCursor: null
  }>()
  const first = makeSubscription({ title: "First page feed" })
  const second = makeSubscription({
    subscriptionId: "00000000-0000-4000-8000-000000000202",
    feedId: "00000000-0000-4000-8000-000000000102",
    title: "Later page feed",
  })
  const refreshedFirst = { ...first, title: "Refreshed first page feed" }
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [first, second],
      nextCursor: null,
    })
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [refreshedFirst],
      nextCursor: "next",
    })
    .mockImplementationOnce(() => laterPage.promise)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listSubscriptions }),
    }),
  )

  await act(async () => result.current.load())

  let refreshing!: Promise<boolean>
  act(() => { refreshing = result.current.reloadSubscriptions() })
  await vi.waitFor(() => {
    expect(result.current.state.subscriptionsById[first.subscriptionId]?.title)
      .toBe("Refreshed first page feed")
  })
  expect(result.current.state.subscriptionOrder).toEqual([
    first.subscriptionId,
    second.subscriptionId,
  ])
  expect(result.current.state.requestActivity.subscriptions).toBe(true)

  laterPage.resolve({ ownerUserId: userId, items: [second], nextCursor: null })
  await act(async () => refreshing)

  expect(result.current.state.subscriptionOrder).toEqual([
    first.subscriptionId,
    second.subscriptionId,
  ])
  expect(result.current.state.subscriptionsAuthoritative).toBe(true)
})

it("keeps the entry list independent when subscription loading fails", async () => {
  const entry = makeEntry({ title: "Available entry" })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({
        listCategories: vi.fn(async () => { throw new Error("offline") }),
        listEntries: vi.fn(async () => ({
          ownerUserId: userId,
          items: [entry],
          nextCursor: null,
          snapshotGeneration: 1,
        })),
      }),
    }),
  )

  await act(async () => result.current.load())

  expect(result.current.state.errors.subscriptions).toBeTruthy()
  expect(
    result.current.state.queueBySourceKey[
      sourceKey({ kind: "smart", state: "UNREAD" })
    ],
  ).toEqual([entry.entryId])
  expect(result.current.state.paneStatus.queue).toBe("ready")
})

it("appends the next entry page without duplicating existing rows", async () => {
  const nextEntry = makeEntry({
    entryId: "00000000-0000-4000-8000-000000000302",
    title: "Older entry",
    sortAtUs: 1,
  })
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: "next-page",
      snapshotGeneration: 4,
    })
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry(), nextEntry],
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

  await act(async () => result.current.load())
  await act(async () => {
    await expect(result.current.loadMoreEntries()).resolves.toBe(true)
  })

  expect(listEntries).toHaveBeenLastCalledWith(
    expect.objectContaining({ cursor: "next-page" }),
  )
  expect(
    result.current.state.queueBySourceKey[
      sourceKey({ kind: "smart", state: "UNREAD" })
    ],
  ).toEqual([entryId, nextEntry.entryId])
  expect(result.current.state.nextCursorBySourceKey["smart:UNREAD"]).toBeNull()
})

it("does not let pagination interrupt a background queue refresh", async () => {
  const refreshedPage = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeEntry>[]
    nextCursor: string | null
    snapshotGeneration: number
  }>()
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: "next-page",
      snapshotGeneration: 4,
    })
    .mockImplementationOnce(() => refreshedPage.promise)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listEntries }),
    }),
  )

  await act(async () => result.current.load())
  let refresh!: Promise<void>
  act(() => {
    refresh = result.current.reloadEntries()
  })

  expect(result.current.state.requestActivity.queue).toBe(true)
  await act(async () => {
    await expect(result.current.loadMoreEntries()).resolves.toBe(false)
  })
  expect(listEntries).toHaveBeenCalledTimes(2)
  expect(listEntries.mock.calls[1]?.[0].signal.aborted).toBe(false)

  refreshedPage.resolve({
    ownerUserId: userId,
    items: [makeEntry()],
    nextCursor: "next-page",
    snapshotGeneration: 5,
  })
  await act(async () => refresh)
  expect(result.current.state.requestActivity.queue).toBe(false)
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
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 1,
    })
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [
        makeEntry({ entryId: newEntryId, sortAtUs: 2 }),
        makeEntry({ title: "Updated stored entity" }),
      ],
      nextCursor: null,
      snapshotGeneration: 2,
    })
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 3,
    })
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 4,
    })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({
        listEntries,
        listSubscriptions: vi.fn(async () => ({
          ownerUserId: userId,
          items: [makeSubscription()],
          nextCursor: null,
        })),
      }),
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
    ownerUserId: userId,
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
    .mockResolvedValueOnce({ ownerUserId: userId, items: [subscription], nextCursor: null })
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [{ ...subscription, unreadCount: 0 }],
      nextCursor: null,
    })
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 7,
    })
    .mockResolvedValueOnce({
      ownerUserId: userId,
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

it("commits a bulk-read response before slow background reconciliation", async () => {
  const entry = makeEntry()
  const subscription = makeSubscription({ unreadCount: 1 })
  const reconciliation = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeEntry>[]
    nextCursor: null
    snapshotGeneration: number
  }>()
  const subscriptionReconciliation = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeSubscription>[]
    nextCursor: null
  }>()
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [subscription],
      nextCursor: null,
    })
    .mockImplementationOnce(() => subscriptionReconciliation.promise)
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [entry],
      nextCursor: null,
      snapshotGeneration: 7,
    })
    .mockImplementationOnce(() => reconciliation.promise)
  const markEntriesRead = vi.fn(async () => undefined)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listSubscriptions, listEntries, markEntriesRead }),
    }),
  )

  await act(async () => result.current.load())
  let markRead!: Promise<boolean>
  act(() => { markRead = result.current.markCurrentSourceRead() })
  await vi.waitFor(() => expect(markEntriesRead).toHaveBeenCalledOnce())
  await act(async () => { await Promise.resolve() })

  const unreadKey = sourceKey({ kind: "smart", state: "UNREAD" })
  const queueBeforeReconciliation = result.current.state.queueBySourceKey[unreadKey]
  const outcomeBeforeReconciliation = await Promise.race([
    markRead,
    Promise.resolve("pending" as const),
  ])

  await act(async () => { await markRead })
  expect(queueBeforeReconciliation).toEqual([])
  expect(outcomeBeforeReconciliation).toBe(true)
  expect(result.current.isMarkingRead).toBe(false)
  expect(result.current.state.entriesById[entry.entryId]?.isRead).toBe(true)
  expect(
    result.current.state.subscriptionsById[subscription.subscriptionId]?.unreadCount,
  ).toBe(1)
  expect(listEntries).toHaveBeenCalledTimes(1)

  await act(async () => {
    subscriptionReconciliation.resolve({
      ownerUserId: userId,
      items: [{ ...subscription, unreadCount: 0 }],
      nextCursor: null,
    })
    await vi.waitFor(() => expect(listEntries).toHaveBeenCalledTimes(2))
    reconciliation.resolve({
      ownerUserId: userId,
      items: [],
      nextCursor: null,
      snapshotGeneration: 7,
    })
    await Promise.resolve()
  })
})

it("serializes a pending single-entry read before a bulk read", async () => {
  const entry = makeEntry()
  const subscription = makeSubscription({ unreadCount: 1 })
  const patch = deferred<{
    entryId: string
    isRead: boolean
    isStarred: boolean
  }>()
  const patchEntryState = vi.fn(() => patch.promise)
  const markEntriesRead = vi.fn(async () => undefined)
  const listEntries = vi.fn(async () => ({
    ownerUserId: userId,
    items: [entry],
    nextCursor: null,
    snapshotGeneration: 7,
  }))
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({
        listSubscriptions: vi.fn(async () => ({
          ownerUserId: userId,
          items: [subscription],
          nextCursor: null,
        })),
        listEntries,
        patchEntryState,
        markEntriesRead,
      }),
    }),
  )

  await act(async () => result.current.load())
  let singleRead!: Promise<void>
  let bulkRead!: Promise<boolean>
  act(() => {
    singleRead = result.current.toggleRead(entry.entryId)
    bulkRead = result.current.markCurrentSourceRead()
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })

  expect(patchEntryState).toHaveBeenCalledOnce()
  expect(markEntriesRead).not.toHaveBeenCalled()

  await act(async () => {
    patch.resolve({ entryId: entry.entryId, isRead: true, isStarred: false })
    await singleRead
  })
  await vi.waitFor(() => expect(markEntriesRead).toHaveBeenCalledOnce())
  await act(async () => { await bulkRead })
})

it("does not cache provisional bulk-read counts before reconciliation", async () => {
  vi.useFakeTimers()
  const entry = makeEntry()
  const subscription = makeSubscription({ unreadCount: 1 })
  const subscriptionReconciliation = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeSubscription>[]
    nextCursor: null
  }>()
  const entryReconciliation = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeEntry>[]
    nextCursor: null
    snapshotGeneration: number
  }>()
  const save = vi.fn<ReaderCache["save"]>(async () => undefined)
  const invalidate = vi.fn(async () => undefined)
  const cache: ReaderCache = {
    load: vi.fn(async () => cacheSnapshot(subscription, entry)),
    save,
    invalidate,
    clear: vi.fn(async () => undefined),
  }
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [subscription],
      nextCursor: null,
    })
    .mockImplementationOnce(() => subscriptionReconciliation.promise)
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [entry],
      nextCursor: null,
      snapshotGeneration: 7,
    })
    .mockImplementationOnce(() => entryReconciliation.promise)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated: vi.fn(),
      api: makeApi({
        listSubscriptions,
        listEntries,
        markEntriesRead: vi.fn(async () => undefined),
      }),
    }),
  )

  await act(async () => result.current.load())
  const validatedSaveCount = save.mock.calls.length
  await act(async () => {
    await expect(result.current.markCurrentSourceRead()).resolves.toBe(true)
  })
  expect(invalidate).toHaveBeenCalledOnce()
  await act(async () => vi.advanceTimersByTimeAsync(200))

  expect(result.current.state.subscriptionsById[subscription.subscriptionId]?.unreadCount)
    .toBe(1)
  expect(save).toHaveBeenCalledTimes(validatedSaveCount)

  await act(async () => {
    subscriptionReconciliation.resolve({
      ownerUserId: userId,
      items: [{ ...subscription, unreadCount: 0 }],
      nextCursor: null,
    })
    entryReconciliation.resolve({
      ownerUserId: userId,
      items: [],
      nextCursor: null,
      snapshotGeneration: 7,
    })
    await Promise.resolve()
  })

  expect(save.mock.calls.length).toBeGreaterThan(validatedSaveCount)
  expect(save.mock.lastCall?.[1]).toMatchObject({
    subscriptions: [expect.objectContaining({ unreadCount: 0 })],
    queue: [],
  })
})

it("does not let unrelated source validation complete bulk cache reconciliation", async () => {
  vi.useFakeTimers()
  const entry = makeEntry()
  const pendingEntry = makeEntry({
    entryId: "00000000-0000-4000-8000-000000000302",
  })
  const subscription = makeSubscription({ unreadCount: 1 })
  const subscriptionReconciliation = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeSubscription>[]
    nextCursor: null
  }>()
  const bulkEntryReconciliation = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeEntry>[]
    nextCursor: null
    snapshotGeneration: number
  }>()
  const save = vi.fn<ReaderCache["save"]>(async () => undefined)
  const cache: ReaderCache = {
    load: vi.fn(async () => cacheSnapshot(subscription, entry)),
    save,
    clear: vi.fn(async () => undefined),
  }
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [subscription],
      nextCursor: null,
    })
    .mockImplementationOnce(() => subscriptionReconciliation.promise)
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [entry],
      nextCursor: null,
      snapshotGeneration: 7,
    })
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [pendingEntry],
      nextCursor: null,
      snapshotGeneration: 8,
    })
    .mockImplementationOnce(() => bulkEntryReconciliation.promise)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated: vi.fn(),
      api: makeApi({
        listSubscriptions,
        listEntries,
        markEntriesRead: vi.fn(async () => undefined),
      }),
    }),
  )

  await act(async () => result.current.load())
  await act(async () => {
    await expect(result.current.markCurrentSourceRead()).resolves.toBe(true)
  })
  const validatedSaveCount = save.mock.calls.length

  await act(async () => result.current.reloadEntries())
  await act(async () => {
    subscriptionReconciliation.resolve({
      ownerUserId: userId,
      items: [{ ...subscription, unreadCount: 1 }],
      nextCursor: null,
    })
    await Promise.resolve()
    await Promise.resolve()
  })

  expect(listEntries).toHaveBeenCalledTimes(3)
  expect(save).toHaveBeenCalledTimes(validatedSaveCount)

  await act(async () => {
    bulkEntryReconciliation.resolve({
      ownerUserId: userId,
      items: [pendingEntry],
      nextCursor: null,
      snapshotGeneration: 8,
    })
    await Promise.resolve()
    await Promise.resolve()
  })
  expect(save.mock.calls.length).toBeGreaterThan(validatedSaveCount)
  expect(save.mock.lastCall?.[1]).toMatchObject({
    queue: [pendingEntry.entryId],
  })
})

it("marks an unselected feed read from a fresh feed snapshot", async () => {
  const subscription = makeSubscription({ unreadCount: 2 })
  const pendingEntry = makeEntry({
    entryId: "00000000-0000-4000-8000-000000000302",
    title: "Arrived after quick snapshot",
  })
  const quickSnapshot = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeEntry>[]
    nextCursor: null
    snapshotGeneration: number
  }>()
  const subscriptionReconciliation = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeSubscription>[]
    nextCursor: null
  }>()
  const entryReconciliation = deferred<{
    ownerUserId: string
    items: ReturnType<typeof makeEntry>[]
    nextCursor: null
    snapshotGeneration: number
  }>()
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({ ownerUserId: userId, items: [subscription], nextCursor: null })
    .mockImplementationOnce(() => subscriptionReconciliation.promise)
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 5,
    })
    .mockImplementationOnce(() => quickSnapshot.promise)
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry(), pendingEntry],
      nextCursor: null,
      snapshotGeneration: 10,
    })
    .mockImplementationOnce(() => entryReconciliation.promise)
  const markEntriesRead = vi.fn(async () => undefined)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listSubscriptions, listEntries, markEntriesRead }),
    }),
  )

  await act(async () => result.current.load())
  let markRead!: Promise<boolean>
  act(() => { markRead = result.current.markFeedRead(subscription.feedId) })
  await vi.waitFor(() => expect(listEntries).toHaveBeenCalledTimes(2))
  await act(async () => result.current.reloadEntries())
  quickSnapshot.resolve({
    ownerUserId: userId,
    items: [makeEntry()],
    nextCursor: null,
    snapshotGeneration: 9,
  })
  await vi.waitFor(() => expect(markEntriesRead).toHaveBeenCalledOnce())
  await act(async () => { await markRead })

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
  expect(result.current.isMarkingRead).toBe(false)
  expect(
    result.current.state.queueBySourceKey[
      sourceKey({ kind: "smart", state: "UNREAD" })
    ],
  ).toBeUndefined()
  expect(result.current.state.entriesById[pendingEntry.entryId]?.isRead).toBe(false)
  expect(
    result.current.state.pendingNewEntriesBySource[
      sourceKey({ kind: "smart", state: "UNREAD" })
    ],
  ).toBeUndefined()
  expect(
    result.current.state.subscriptionsById[subscription.subscriptionId]?.unreadCount,
  ).toBe(2)

  await act(async () => {
    subscriptionReconciliation.resolve({
      ownerUserId: userId,
      items: [{ ...subscription, unreadCount: 1 }],
      nextCursor: null,
    })
    entryReconciliation.resolve({
      ownerUserId: userId,
      items: [pendingEntry],
      nextCursor: null,
      snapshotGeneration: 10,
    })
    await Promise.resolve()
  })
})

function makeApi(overrides: Partial<ReaderApi> = {}): ReaderApi {
  return {
    listCategories: vi.fn(async () => ({ ownerUserId: userId, items: [] })),
    createCategory: vi.fn(),
    updateCategory: vi.fn(),
    deleteCategory: vi.fn(),
    listSubscriptions: vi.fn(async () => ({
      ownerUserId: userId,
      items: [],
      nextCursor: null,
    })),
    getSubscription: vi.fn(),
    createSubscription: vi.fn(),
    deleteSubscription: vi.fn(),
    refreshSubscription: vi.fn(),
    updateSubscription: vi.fn(),
    listEntries: vi.fn(async () => ({
      ownerUserId: userId,
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
