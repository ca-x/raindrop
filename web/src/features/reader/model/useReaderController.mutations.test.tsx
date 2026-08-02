import { act, renderHook, waitFor } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import type { EntryStateResponse } from "../api/reader.generated"
import type {
  CreateSubscriptionResponse,
  Refresh,
  RefreshState,
  Subscription,
} from "../api/subscription.generated"
import { ApiClientError } from "../../../shared/api/client"
import type { ReaderApi } from "./controllerApi"
import {
  categoryId,
  entryId,
  makeCategory,
  makeDetail,
  makeEntry,
  makeSubscription,
  subscriptionId,
} from "./testFixtures"
import { useReaderController } from "./useReaderController"

const userId = "11111111-1111-4111-8111-111111111111"

it("optimistically toggles read and lets the validated server response win", async () => {
  const response = deferred<EntryStateResponse>()
  let serverEntry = makeEntry()
  let serverDetail = makeDetail()
  const patchEntryState = vi.fn(() => response.promise.then((state) => {
    serverEntry = { ...serverEntry, ...state }
    serverDetail = { ...serverDetail, ...state }
    return state
  }))
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({
        patchEntryState,
        listEntries: vi.fn(async () => ({
          ownerUserId: userId,
          items: [serverEntry],
          nextCursor: null,
          snapshotGeneration: 1,
        })),
        getEntry: vi.fn(async () => serverDetail),
      }),
    }),
  )
  await act(async () => result.current.load())
  await act(async () => result.current.selectEntry(entryId))

  let mutation!: Promise<void>
  act(() => {
    mutation = result.current.toggleRead(entryId)
  })
  expect(result.current.state.entriesById[entryId]?.isRead).toBe(true)
  expect(result.current.state.detailsById[entryId]?.isRead).toBe(true)
  expect(result.current.state.subscriptionsById[subscriptionId]?.unreadCount).toBe(2)
  expect(patchEntryState).toHaveBeenCalledWith(
    entryId,
    { isRead: true },
    "csrf-memory",
    expect.any(AbortSignal),
  )

  response.resolve({ entryId, isRead: false, isStarred: true })
  await act(async () => mutation)
  expect(result.current.state.entriesById[entryId]).toMatchObject({
    isRead: false,
    isStarred: true,
  })
  expect(result.current.state.detailsById[entryId]).toMatchObject({
    isRead: false,
    isStarred: true,
  })
  expect(result.current.state.subscriptionsById[subscriptionId]?.unreadCount).toBe(3)
})

it("reconciles an entry mutation incrementally without collapsing the reading queue", async () => {
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 1,
    })
    .mockResolvedValue({
      ownerUserId: userId,
      items: [],
      nextCursor: null,
      snapshotGeneration: 1,
    })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({
        listEntries,
        patchEntryState: vi.fn(async () => ({
          entryId,
          isRead: true,
          isStarred: false,
        })),
      }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.toggleRead(entryId))
  await waitFor(() => expect(listEntries).toHaveBeenCalledTimes(2))

  expect(result.current.state.queueBySourceKey["smart:UNREAD"]).toEqual([entryId])
  expect(result.current.state.entriesById[entryId]?.isRead).toBe(true)
  expect(result.current.state.paneStatus.queue).toBe("ready")
})

it("rolls back a forbidden mutation and preserves its stable API message", async () => {
  const onUnauthenticated = vi.fn()
  const patchEntryState = vi.fn(async () => {
    throw new ApiClientError(403, {
      code: "FORBIDDEN",
      message: "You cannot change this entry",
    })
  })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated,
      api: makeApi({ patchEntryState }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.toggleRead(entryId))

  expect(result.current.state.entriesById[entryId]?.isRead).toBe(false)
  expect(result.current.state.subscriptionsById[subscriptionId]?.unreadCount).toBe(3)
  expect(result.current.state.errors.mutation).toBe("You cannot change this entry")
  expect(onUnauthenticated).not.toHaveBeenCalled()
})

it("clears Reader-owned state and ends the session on a 401", async () => {
  const onUnauthenticated = vi.fn()
  const patchEntryState = vi.fn(async () => {
    throw new ApiClientError(401, {
      code: "AUTHENTICATION_REQUIRED",
      message: "Sign in again",
    })
  })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated,
      api: makeApi({ patchEntryState }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.toggleStar(entryId))

  expect(onUnauthenticated).toHaveBeenCalledTimes(1)
  expect(result.current.state.entriesById).toEqual({})
  expect(result.current.state.subscriptionsById).toEqual({})
  expect(result.current.state.pendingNewEntriesBySource).toEqual({})
  expect(result.current.state.errors.mutation).toBeNull()
})

it("adds, refreshes, and deletes subscriptions through CSRF-aware actions", async () => {
  const created = makeSubscription({
    subscriptionId: "00000000-0000-4000-8000-000000000202",
    title: "Created Feed",
  })
  const refresh = makeRefresh("PENDING")
  const completedRefresh = makeRefresh("READY")
  let subscriptions = [makeSubscription()]
  const listSubscriptions = vi.fn(async () => ({
    ownerUserId: userId,
    items: [...subscriptions],
    nextCursor: null,
  }))
  const createSubscription = vi.fn(async () => {
    subscriptions = [...subscriptions, created]
    return { created: true, subscription: created }
  })
  const refreshSubscription = vi.fn(async () => {
    subscriptions = subscriptions.map((subscription) =>
      subscription.subscriptionId === created.subscriptionId
        ? { ...subscription, refresh }
        : subscription,
    )
    return refresh
  })
  const getSubscription = vi.fn(async () => {
    const completed = makeSubscription({
      ...created,
      unreadCount: 4,
      refresh: completedRefresh,
    })
    subscriptions = subscriptions.map((subscription) =>
      subscription.subscriptionId === created.subscriptionId
        ? completed
        : subscription,
    )
    return completed
  })
  const deleteSubscription = vi.fn(async () => {
    subscriptions = subscriptions.filter(
      (subscription) => subscription.subscriptionId !== created.subscriptionId,
    )
  })
  const requestId = "00000000-0000-4000-8000-000000000901"
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      createRequestId: () => requestId,
      api: makeApi({
        createSubscription,
        getSubscription,
        listSubscriptions,
        refreshSubscription,
        deleteSubscription,
      }),
    }),
  )
  await act(async () => result.current.load())

  let addResult: CreateSubscriptionResponse | null = null
  await act(async () => {
    addResult = await result.current.addSubscription("https://created.example/feed")
  })
  expect(addResult).toEqual({ created: true, subscription: created })
  expect(createSubscription).toHaveBeenCalledWith(
    { url: "https://created.example/feed" },
    "csrf-memory",
    expect.any(AbortSignal),
  )
  expect(result.current.state.subscriptionOrder).toContain(created.subscriptionId)

  await act(async () => result.current.refreshSubscription(created.subscriptionId))
  expect(refreshSubscription).toHaveBeenCalledWith(
    created.subscriptionId,
    { requestId },
    "csrf-memory",
    expect.any(AbortSignal),
  )
  await waitFor(() => {
    expect(result.current.state.subscriptionsById[created.subscriptionId]).toMatchObject({
      unreadCount: 4,
      refresh: completedRefresh,
    })
  })
  expect(getSubscription).toHaveBeenCalledWith(
    created.subscriptionId,
    expect.any(AbortSignal),
  )

  await act(async () => result.current.deleteSubscription(created.subscriptionId))
  expect(deleteSubscription).toHaveBeenCalledWith(
    created.subscriptionId,
    "csrf-memory",
    expect.any(AbortSignal),
  )
  expect(result.current.state.subscriptionsById[created.subscriptionId]).toBeUndefined()
})

it("creates, updates, deletes, and assigns categories through the shared Reader state", async () => {
  const createdCategory = makeCategory()
  const renamedCategory = makeCategory({ title: "Science", position: 512 })
  const categorizedSubscription = makeSubscription({ categoryId })
  let categories: ReturnType<typeof makeCategory>[] = []
  let subscriptions = [makeSubscription()]
  const listCategories = vi.fn(async () => ({
    ownerUserId: userId,
    items: [...categories],
  }))
  const listSubscriptions = vi.fn(async () => ({
    ownerUserId: userId,
    items: [...subscriptions],
    nextCursor: null,
  }))
  const createCategory = vi.fn(async () => {
    categories = [...categories, createdCategory]
    return createdCategory
  })
  const updateCategory = vi.fn(async () => {
    categories = categories.map((category) =>
      category.categoryId === categoryId ? renamedCategory : category,
    )
    return renamedCategory
  })
  const deleteCategory = vi.fn(async () => {
    categories = categories.filter((category) => category.categoryId !== categoryId)
    subscriptions = subscriptions.map((subscription) =>
      subscription.categoryId === categoryId
        ? { ...subscription, categoryId: null }
        : subscription,
    )
  })
  const updateSubscription = vi.fn(async () => {
    subscriptions = subscriptions.map((subscription) =>
      subscription.subscriptionId === subscriptionId
        ? categorizedSubscription
        : subscription,
    )
    return categorizedSubscription
  })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({
        createCategory,
        listCategories,
        listSubscriptions,
        updateCategory,
        deleteCategory,
        updateSubscription,
      }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.createCategory("Technology"))
  expect(createCategory).toHaveBeenCalledWith(
    { title: "Technology" },
    "csrf-memory",
    expect.any(AbortSignal),
  )
  expect(result.current.state.categoriesById[categoryId]).toEqual(createdCategory)

  await act(async () => result.current.updateCategory(categoryId, { title: "Science" }))
  expect(updateCategory).toHaveBeenCalledWith(
    categoryId,
    { title: "Science" },
    "csrf-memory",
    expect.any(AbortSignal),
  )
  expect(result.current.state.categoriesById[categoryId]).toEqual(renamedCategory)

  await act(async () =>
    result.current.updateSubscription(subscriptionId, { categoryId }),
  )
  expect(updateSubscription).toHaveBeenCalledWith(
    subscriptionId,
    { categoryId },
    "csrf-memory",
    expect.any(AbortSignal),
  )
  expect(result.current.state.subscriptionsById[subscriptionId]?.categoryId).toBe(
    categoryId,
  )

  await act(async () => result.current.deleteCategory(categoryId))
  expect(deleteCategory).toHaveBeenCalledWith(
    categoryId,
    "csrf-memory",
    expect.any(AbortSignal),
  )
  expect(result.current.state.categoriesById[categoryId]).toBeUndefined()
  expect(result.current.state.subscriptionsById[subscriptionId]?.categoryId).toBeNull()
})

it("reconciles provisional subscription metadata after create refresh completes", async () => {
  const pending = makeRefresh("PENDING")
  const ready = makeRefresh("READY")
  const provisional = makeSubscription({ title: "www.ithome.com", unreadCount: 0, refresh: pending })
  const resolved = makeSubscription({ title: "IT之家", unreadCount: 60, refresh: ready })
  const poll = deferred<Subscription>()
  let subscriptions = [makeSubscription()]
  const listSubscriptions = vi.fn(async () => ({
    ownerUserId: userId,
    items: [...subscriptions],
    nextCursor: null,
  }))
  const getSubscription = vi.fn(() => poll.promise.then((subscription) => {
    subscriptions = subscriptions.map((current) =>
      current.subscriptionId === subscription.subscriptionId
        ? subscription
        : current,
    )
    return subscription
  }))
  const createSubscription = vi.fn(async () => {
    subscriptions = [...subscriptions, provisional]
    return { created: true, subscription: provisional }
  })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({
        createSubscription,
        getSubscription,
        listSubscriptions,
      }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.addSubscription("https://www.ithome.com/rss/"))

  expect(result.current.state.subscriptionsById[subscriptionId]).toMatchObject({
    title: "www.ithome.com",
    refresh: pending,
  })
  await act(async () => poll.resolve(resolved))
  await waitFor(() => {
    expect(result.current.state.subscriptionsById[subscriptionId]).toMatchObject({
      title: "IT之家",
      unreadCount: 60,
      refresh: ready,
    })
  })
  expect(getSubscription).toHaveBeenCalledWith(subscriptionId, expect.any(AbortSignal))
})

it("aborts an in-flight refresh poll when its subscription is deleted", async () => {
  const pending = makeRefresh("PENDING")
  const poll = deferred<Subscription>()
  let subscriptions = [makeSubscription()]
  let pollSignal: AbortSignal | undefined
  const getSubscription = vi.fn((_subscriptionId: string, signal?: AbortSignal) => {
    pollSignal = signal
    return poll.promise
  })
  const listSubscriptions = vi.fn(async () => ({
    ownerUserId: userId,
    items: [...subscriptions],
    nextCursor: null,
  }))
  const deleteSubscription = vi.fn(async () => {
    subscriptions = []
  })
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({
        getSubscription,
        listSubscriptions,
        refreshSubscription: vi.fn(async () => pending),
        deleteSubscription,
      }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.refreshSubscription(subscriptionId))
  await waitFor(() => expect(getSubscription).toHaveBeenCalledOnce())
  await act(async () => result.current.deleteSubscription(subscriptionId))

  expect(pollSignal?.aborted).toBe(true)
  await act(async () => poll.resolve(makeSubscription({ title: "Stale poll result" })))
  expect(result.current.state.subscriptionsById[subscriptionId]).toBeUndefined()
  expect(result.current.state.errors.mutation).toBeNull()
})

it("continues polling pending refreshes and stops at the first terminal subscription", async () => {
  vi.useFakeTimers()
  try {
    const pending = makeRefresh("PENDING")
    const ready = makeRefresh("READY")
    const getSubscription = vi
      .fn()
      .mockResolvedValueOnce(makeSubscription({ refresh: pending }))
      .mockResolvedValueOnce(makeSubscription({ title: "Resolved Feed", refresh: ready }))
    const { result } = renderHook(() =>
      useReaderController({
        csrfToken: "csrf-memory",
        onUnauthenticated: vi.fn(),
        api: makeApi({
          getSubscription,
          refreshSubscription: vi.fn(async () => pending),
        }),
      }),
    )
    await act(async () => result.current.load())

    await act(async () => result.current.refreshSubscription(subscriptionId))
    expect(getSubscription).toHaveBeenCalledTimes(1)
    expect(result.current.state.subscriptionsById[subscriptionId]?.refresh).toEqual(pending)

    await act(async () => vi.advanceTimersByTimeAsync(1_000))

    expect(getSubscription).toHaveBeenCalledTimes(2)
    expect(result.current.state.subscriptionsById[subscriptionId]).toMatchObject({
      title: "Resolved Feed",
      refresh: ready,
    })
    await act(async () => vi.advanceTimersByTimeAsync(5_000))
    expect(getSubscription).toHaveBeenCalledTimes(2)
  } finally {
    vi.useRealTimers()
  }
})

function makeApi(overrides: Partial<ReaderApi> = {}): ReaderApi {
  return {
    listCategories: vi.fn(async () => ({ ownerUserId: userId, items: [] })),
    createCategory: vi.fn(),
    updateCategory: vi.fn(),
    deleteCategory: vi.fn(),
    listSubscriptions: vi.fn(async () => ({
      ownerUserId: userId,
      items: [makeSubscription()],
      nextCursor: null,
    })),
    getSubscription: vi.fn(),
    createSubscription: vi.fn(),
    deleteSubscription: vi.fn(),
    refreshSubscription: vi.fn(),
    updateSubscription: vi.fn(),
    listEntries: vi.fn(async () => ({
      ownerUserId: userId,
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 1,
    })),
    getEntry: vi.fn(async () => makeDetail()),
    patchEntryState: vi.fn(),
    markEntriesRead: vi.fn(),
    ...overrides,
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise
  })
  return { promise, resolve }
}

function makeRefresh(state: RefreshState): Refresh {
  const isPending = state === "PENDING"
  return {
    operationId: "00000000-0000-4000-8000-000000000801",
    state,
    pendingState: isPending ? "QUEUED" : null,
    newCount: isPending ? 0 : 1,
    updatedCount: 0,
    droppedCount: 0,
    entryIssues: [],
    generation: isPending ? null : 2,
    errorCode: null,
    retryAt: null,
    lastSuccessAt: isPending ? null : "2026-07-18T02:00:00.200000Z",
    queuedAt: "2026-07-18T02:00:00.000000Z",
    startedAt: isPending ? null : "2026-07-18T02:00:00.100000Z",
    completedAt: isPending ? null : "2026-07-18T02:00:00.200000Z",
  }
}
