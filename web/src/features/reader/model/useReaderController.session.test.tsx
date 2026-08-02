import { act, renderHook, waitFor } from "@testing-library/react"
import { StrictMode, type PropsWithChildren } from "react"
import { expect, it, vi } from "vitest"

import { ApiClientError } from "../../../shared/api/client"
import type { ReaderCache } from "../cache/readerCache"
import type { EntryDetailResponse, EntryPageResponse } from "../api/reader.generated"
import type { CreateSubscriptionResponse } from "../api/subscription.generated"
import type { ReaderApi } from "./controllerApi"
import { entryId, makeDetail, makeEntry, makeSubscription } from "./testFixtures"
import { sourceKey } from "./types"
import { useReaderController } from "./useReaderController"

const unauthorized = () =>
  new ApiClientError(401, {
    code: "AUTHENTICATION_REQUIRED",
    message: "Sign in again",
  })

const userId = "11111111-1111-4111-8111-111111111111"
const otherUserId = "22222222-2222-4222-8222-222222222222"

it("clears cached Reader data before reporting a current-session 401", async () => {
  const cacheClear = deferred<void>()
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save: vi.fn(async () => undefined),
    clear: vi.fn(() => cacheClear.promise),
  }
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated,
      api: makeApi({ listEntries: vi.fn(async () => { throw unauthorized() }) }),
    }),
  )

  let load!: Promise<void>
  act(() => { load = result.current.load() })
  await waitFor(() => expect(cache.clear).toHaveBeenCalledOnce())
  expect(onUnauthenticated).not.toHaveBeenCalled()

  cacheClear.resolve()
  await act(async () => load)
  expect(onUnauthenticated).toHaveBeenCalledOnce()
})

it("rejects Reader responses authenticated as a different browser session user", async () => {
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated,
      api: makeApi({
        listCategories: vi.fn(async () => ({ ownerUserId: otherUserId, items: [] })),
        listSubscriptions: vi.fn(async () => ({
          ownerUserId: otherUserId,
          items: [makeSubscription({ title: "Other user's feed" })],
          nextCursor: null,
        })),
        listEntries: vi.fn(async () => ({
          ownerUserId: otherUserId,
          items: [makeEntry({ title: "Other user's entry" })],
          nextCursor: null,
          snapshotGeneration: 1,
        })),
      }),
    }),
  )

  await act(async () => result.current.load())

  expect(cache.clear).toHaveBeenCalledOnce()
  expect(onUnauthenticated).toHaveBeenCalledOnce()
  expect(result.current.state.subscriptionsById).toEqual({})
  expect(result.current.state.entriesById).toEqual({})
  expect(cache.save).not.toHaveBeenCalled()
})

it("stops subscription pagination at the first wrong-owner page", async () => {
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const listSubscriptions = vi.fn(async () => ({
    ownerUserId: otherUserId,
    items: [makeSubscription({ title: "Other user's feed" })],
    nextCursor: "must-not-be-requested",
  }))
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated,
      api: makeApi({ listSubscriptions }),
    }),
  )

  await act(async () => result.current.load())

  expect(listSubscriptions).toHaveBeenCalledOnce()
  expect(cache.clear).toHaveBeenCalledOnce()
  expect(onUnauthenticated).toHaveBeenCalledOnce()
  expect(result.current.state.subscriptionsById).toEqual({})
})

it.each([
  [
    "category list",
    {
      listCategories: vi.fn(async () => ({ ownerUserId: otherUserId, items: [] })),
    },
  ],
  [
    "subscription page",
    {
      listSubscriptions: vi.fn(async () => ({
        ownerUserId: otherUserId,
        items: [makeSubscription()],
        nextCursor: null,
      })),
    },
  ],
  [
    "entry page",
    {
      listEntries: vi.fn(async () => ({
        ownerUserId: otherUserId,
        items: [makeEntry()],
        nextCursor: null,
        snapshotGeneration: 1,
      })),
    },
  ],
])("expires when only the %s belongs to another owner", async (_label, overrides) => {
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated,
      api: makeApi(overrides),
    }),
  )

  await act(async () => result.current.load())

  expect(cache.clear).toHaveBeenCalledOnce()
  expect(onUnauthenticated).toHaveBeenCalledOnce()
  expect(cache.save).not.toHaveBeenCalled()
})

it("expires when subscription pagination changes owner", async () => {
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const listSubscriptions = vi
    .fn()
    .mockResolvedValueOnce({
      ownerUserId: userId,
      items: [makeSubscription()],
      nextCursor: "next",
    })
    .mockResolvedValueOnce({
      ownerUserId: otherUserId,
      items: [],
      nextCursor: null,
    })
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated,
      api: makeApi({ listSubscriptions }),
    }),
  )

  await act(async () => result.current.load())

  expect(listSubscriptions).toHaveBeenCalledTimes(2)
  expect(cache.clear).toHaveBeenCalledOnce()
  expect(onUnauthenticated).toHaveBeenCalledOnce()
})

it("rejects a mark-read snapshot authenticated as another owner", async () => {
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce(page(makeEntry()))
    .mockResolvedValueOnce({
      ownerUserId: otherUserId,
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 2,
    })
  const markEntriesRead = vi.fn(async () => undefined)
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated,
      api: makeApi({ listEntries, markEntriesRead }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => {
    await expect(result.current.markFeedRead(makeSubscription().feedId)).resolves.toBe(false)
  })

  expect(markEntriesRead).not.toHaveBeenCalled()
  expect(cache.clear).toHaveBeenCalledOnce()
  expect(onUnauthenticated).toHaveBeenCalledOnce()
})

it("does not clear cached Reader data for a superseded 401", async () => {
  const stale = deferred<EntryPageResponse>()
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce(page(makeEntry()))
    .mockImplementationOnce(() => stale.promise)
    .mockResolvedValueOnce(page(makeEntry({ title: "Current source" })))
  const cache: ReaderCache = {
    load: vi.fn(async () => null),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
  }
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      userId,
      cache,
      onUnauthenticated,
      api: makeApi({ listEntries }),
    }),
  )
  await act(async () => result.current.load())

  let staleLoad!: Promise<void>
  act(() => { staleLoad = result.current.reloadEntries() })
  await act(async () => {
    await result.current.selectSource({ kind: "smart", state: "ALL" })
  })
  stale.reject(unauthorized())
  await act(async () => staleLoad)

  expect(cache.clear).not.toHaveBeenCalled()
  expect(onUnauthenticated).not.toHaveBeenCalled()
})

it("ignores late source and detail 401 responses from superseded requests", async () => {
  const staleSource = deferred<EntryPageResponse>()
  const staleDetail = deferred<EntryDetailResponse>()
  const listEntries = vi
    .fn()
    .mockImplementationOnce(() => staleSource.promise)
    .mockResolvedValueOnce(page(makeEntry({ title: "Current source" })))
  const getEntry = vi
    .fn()
    .mockImplementationOnce(() => staleDetail.promise)
    .mockResolvedValueOnce(makeDetail({ title: "Current detail" }))
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated,
      api: makeApi({ listEntries, getEntry }),
    }),
    { wrapper: StrictWrapper },
  )

  let staleLoad!: Promise<void>
  act(() => {
    staleLoad = result.current.load()
  })
  await waitFor(() => expect(listEntries).toHaveBeenCalledTimes(1))
  const all = { kind: "smart", state: "ALL" } as const
  await act(async () => result.current.selectSource(all))
  staleSource.reject(unauthorized())
  await act(async () => staleLoad)

  let staleSelection!: Promise<void>
  act(() => {
    staleSelection = result.current.selectEntry(entryId)
  })
  await act(async () => result.current.selectEntry(entryId))
  staleDetail.reject(unauthorized())
  await act(async () => staleSelection)

  expect(onUnauthenticated).not.toHaveBeenCalled()
  expect(result.current.state.queueBySourceKey[sourceKey(all)]).toEqual([entryId])
  expect(result.current.state.detailsById[entryId]?.title).toBe("Current detail")
})

it("expires once and quarantines concurrent 401 and ignored-abort completions", async () => {
  const staleReload = deferred<EntryPageResponse>()
  const staleMutation = deferred<never>()
  const staleCreate = deferred<CreateSubscriptionResponse>()
  const listEntries = vi
    .fn()
    .mockResolvedValueOnce(page(makeEntry()))
    .mockImplementationOnce(() => staleReload.promise)
  const patchEntryState = vi.fn(() => staleMutation.promise)
  const createSubscription = vi.fn(() => staleCreate.promise)
  const onUnauthenticated = vi.fn()
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated,
      api: makeApi({ listEntries, patchEntryState, createSubscription }),
    }),
    { wrapper: StrictWrapper },
  )
  await act(async () => result.current.load())

  let reload!: Promise<void>
  let mutation!: Promise<void>
  let create!: Promise<CreateSubscriptionResponse | null>
  act(() => {
    reload = result.current.reloadEntries()
    mutation = result.current.toggleRead(entryId)
    create = result.current.addSubscription("https://late.example/feed")
  })
  const preExpiryGeneration = result.current.state.requestGenerationByPane.queue
  staleReload.reject(unauthorized())
  staleMutation.reject(unauthorized())
  await act(async () => Promise.all([reload, mutation]))

  staleCreate.resolve({
    created: true,
    subscription: makeSubscription({
      subscriptionId: "00000000-0000-4000-8000-000000000299",
    }),
  })
  await act(async () => create)
  await act(async () => result.current.addSubscription("https://blocked.example/feed"))

  expect(onUnauthenticated).toHaveBeenCalledTimes(1)
  expect(result.current.state.entriesById).toEqual({})
  expect(result.current.state.subscriptionsById).toEqual({})
  expect(result.current.state.requestGenerationByPane.queue).toBe(preExpiryGeneration)
  expect(createSubscription).toHaveBeenCalledTimes(1)
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
    listEntries: vi.fn(async () => page(makeEntry())),
    getEntry: vi.fn(async () => makeDetail()),
    patchEntryState: vi.fn(),
    markEntriesRead: vi.fn(),
    ...overrides,
  }
}

function page(entry: ReturnType<typeof makeEntry>): EntryPageResponse {
  return { ownerUserId: userId, items: [entry], nextCursor: null, snapshotGeneration: 1 }
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

function StrictWrapper({ children }: PropsWithChildren) {
  return <StrictMode>{children}</StrictMode>
}
