import { act, renderHook } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import type { EntryStateResponse } from "../api/reader.generated"
import type { ReaderApi } from "./controllerApi"
import {
  entryId,
  makeDetail,
  makeEntry,
  makeSubscription,
  subscriptionId,
} from "./testFixtures"
import { useReaderController } from "./useReaderController"

const userId = "11111111-1111-4111-8111-111111111111"

it("keeps optimistic state through stale controller reloads without double-counting success", async () => {
  const response = deferred<EntryStateResponse>()
  let committed = false
  const patchEntryState = vi.fn(() => response.promise.then((state) => {
    committed = true
    return state
  }))
  const listSubscriptions = vi.fn(async () => ({
    ownerUserId: userId,
    items: [makeSubscription({ unreadCount: committed ? 2 : 3 })],
    nextCursor: null,
  }))
  const listEntries = vi.fn(async () => ({
    ownerUserId: userId,
    items: [makeEntry({ isRead: committed })],
    nextCursor: null,
    snapshotGeneration: 1,
  }))
  const getEntry = vi.fn(async () => makeDetail({ isRead: committed }))
  const { result } = renderHook(() =>
    useReaderController({
      csrfToken: "csrf-memory",
      onUnauthenticated: vi.fn(),
      api: makeApi({ listSubscriptions, listEntries, getEntry, patchEntryState }),
    }),
  )
  await act(async () => result.current.load())
  await act(async () => result.current.selectEntry(entryId))

  let mutation!: Promise<void>
  act(() => {
    mutation = result.current.toggleRead(entryId)
  })
  await act(async () => result.current.load())
  await act(async () => result.current.selectEntry(entryId))

  expect(result.current.state.entriesById[entryId]?.isRead).toBe(true)
  expect(result.current.state.detailsById[entryId]?.isRead).toBe(true)
  expect(result.current.state.subscriptionsById[subscriptionId]?.unreadCount).toBe(2)

  response.resolve({ entryId, isRead: true, isStarred: false })
  await act(async () => mutation)

  expect(result.current.state.entriesById[entryId]?.isRead).toBe(true)
  expect(result.current.state.subscriptionsById[subscriptionId]?.unreadCount).toBe(2)
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
