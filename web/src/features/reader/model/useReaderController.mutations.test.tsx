import { act, renderHook } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import type { EntryStateResponse } from "../api/reader.generated"
import { ApiClientError } from "../../../shared/api/client"
import type { ReaderApi } from "./controllerApi"
import {
  entryId,
  makeDetail,
  makeEntry,
  makeSubscription,
  subscriptionId,
} from "./testFixtures"
import { useReaderController } from "./useReaderController"

it("optimistically toggles read and lets the validated server response win", async () => {
  const response = deferred<EntryStateResponse>()
  const patchEntryState = vi.fn(() => response.promise)
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ patchEntryState }),
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
  const refresh = {
    operationId: "00000000-0000-4000-8000-000000000801",
    state: "PENDING" as const,
    newCount: 0,
    updatedCount: 0,
    droppedCount: 0,
    generation: null,
    errorCode: null,
    retryAt: null,
    queuedAt: "2026-07-18T02:00:00.000000Z",
    startedAt: null,
    completedAt: null,
  }
  const createSubscription = vi.fn(async () => ({
    created: true,
    subscription: created,
  }))
  const refreshSubscription = vi.fn(async () => refresh)
  const deleteSubscription = vi.fn(async () => undefined)
  const requestId = "00000000-0000-4000-8000-000000000901"
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      createRequestId: () => requestId,
      api: makeApi({
        createSubscription,
        refreshSubscription,
        deleteSubscription,
      }),
    }),
  )
  await act(async () => result.current.load())

  await act(async () => result.current.addSubscription("https://created.example/feed"))
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
  expect(result.current.state.subscriptionsById[created.subscriptionId]?.refresh).toEqual(
    refresh,
  )

  await act(async () => result.current.deleteSubscription(created.subscriptionId))
  expect(deleteSubscription).toHaveBeenCalledWith(
    created.subscriptionId,
    "csrf-memory",
    expect.any(AbortSignal),
  )
  expect(result.current.state.subscriptionsById[created.subscriptionId]).toBeUndefined()
})

function makeApi(overrides: Partial<ReaderApi> = {}): ReaderApi {
  return {
    listSubscriptions: vi.fn(async () => ({
      items: [makeSubscription()],
      nextCursor: null,
    })),
    getSubscription: vi.fn(),
    createSubscription: vi.fn(),
    deleteSubscription: vi.fn(),
    refreshSubscription: vi.fn(),
    listEntries: vi.fn(async () => ({
      items: [makeEntry()],
      nextCursor: null,
      snapshotGeneration: 1,
    })),
    getEntry: vi.fn(async () => makeDetail()),
    patchEntryState: vi.fn(),
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
