import { act, renderHook } from "@testing-library/react"
import { StrictMode, type PropsWithChildren } from "react"
import { expect, it, vi } from "vitest"

import { ApiClientError } from "../../../shared/api/client"
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
  let create!: Promise<void>
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
    listCategories: vi.fn(async () => ({ items: [] })),
    createCategory: vi.fn(),
    updateCategory: vi.fn(),
    deleteCategory: vi.fn(),
    listSubscriptions: vi.fn(async () => ({ items: [makeSubscription()], nextCursor: null })),
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
  return { items: [entry], nextCursor: null, snapshotGeneration: 1 }
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
