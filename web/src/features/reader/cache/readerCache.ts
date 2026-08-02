import {
  isEntryListItemResponse,
  isEntryListState,
  type EntryListItemResponse,
} from "../api/reader.generated"
import { isCategory, type Category } from "../api/organization.generated"
import { isSubscription, type Subscription } from "../api/subscription.generated"
import { sourceKey, type ReaderSource, type ReaderState } from "../model/types"

const CACHE_SCHEMA_VERSION = 1
const CACHE_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1_000
const CACHE_FUTURE_SKEW_MS = 5 * 60 * 1_000
const CACHE_MAX_SERIALIZED_BYTES = 2 * 1024 * 1024
const CACHE_MAX_CATEGORIES = 250
const CACHE_MAX_SUBSCRIPTIONS = 1_000
const CACHE_MAX_ENTRIES = 100
const CACHE_MAX_SCROLL_ANCHORS = 32
const CACHE_MAX_SUMMARY_CHARACTERS = 512
const CACHE_DATABASE_NAME = "raindrop-reader-cache"
const CACHE_DATABASE_VERSION = 1
const CACHE_STORE_NAME = "snapshots"
const CACHE_ACTIVE_KEY = "active"

export interface ReaderCacheSnapshot {
  categories: Category[]
  subscriptions: Subscription[]
  source: ReaderSource
  entries: EntryListItemResponse[]
  queue: string[]
  snapshotGeneration: number | null
  scrollAnchorByRoute: Record<string, number>
}

export interface ReaderCacheStorage {
  read(): Promise<unknown>
  write(value: unknown): Promise<void>
  clear(): Promise<void>
}

export interface ReaderCache {
  load(userId: string): Promise<ReaderCacheSnapshot | null>
  save(userId: string, snapshot: ReaderCacheSnapshot): Promise<void>
  clear(): Promise<void>
}

interface ReaderCacheEnvelope {
  schemaVersion: typeof CACHE_SCHEMA_VERSION
  ownerUserId: string
  savedAtMs: number
  snapshot: ReaderCacheSnapshot
}

interface RawReaderCacheSnapshot {
  categories: unknown[]
  subscriptions: unknown[]
  source: unknown
  entries: unknown[]
  queue: unknown[]
  snapshotGeneration: unknown
  scrollAnchorByRoute: Record<string, unknown>
}

export function createReaderCache(
  storage: ReaderCacheStorage,
  now: () => number = Date.now,
): ReaderCache {
  const clear = async () => {
    try {
      await storage.clear()
    } catch {
      // Reader cache is an optional acceleration path.
    }
  }

  return {
    async load(userId) {
      let stored: unknown
      try {
        stored = await storage.read()
      } catch {
        return null
      }
      if (stored === null || stored === undefined) return null
      const snapshot = decodeEnvelope(stored, userId, now())
      if (snapshot) return snapshot
      await clear()
      return null
    },

    async save(userId, snapshot) {
      const envelope: ReaderCacheEnvelope = {
        schemaVersion: CACHE_SCHEMA_VERSION,
        ownerUserId: userId,
        savedAtMs: now(),
        snapshot,
      }
      if (!decodeEnvelope(envelope, userId, envelope.savedAtMs)) return
      try {
        await storage.write(envelope)
      } catch {
        // Quota and browser policy failures must not affect reading.
      }
    },

    clear,
  }
}

export const browserReaderCache = createReaderCache(createIndexedDbStorage())

export function readerCacheSnapshot(state: ReaderState): ReaderCacheSnapshot | null {
  if (state.paneStatus.subscriptions !== "ready") return null
  const categories = state.categoryOrder
    .map((id) => state.categoriesById[id])
    .filter((category): category is Category => category !== undefined)
  const subscriptions = state.subscriptionOrder
    .map((id) => state.subscriptionsById[id])
    .filter((subscription): subscription is Subscription => subscription !== undefined)
  if (
    categories.length !== state.categoryOrder.length ||
    subscriptions.length !== state.subscriptionOrder.length
  ) {
    return null
  }

  const key = sourceKey(state.selectedSource)
  const storedQueue = state.queueBySourceKey[key]
  const storedGeneration = state.snapshotGenerationBySource[key]
  let entries: EntryListItemResponse[] = []
  let queue: string[] = []
  let snapshotGeneration: number | null = null
  if (storedQueue !== undefined && storedGeneration !== undefined) {
    queue = storedQueue.slice(0, CACHE_MAX_ENTRIES)
    const projected = queue.map((entryId) => state.entriesById[entryId])
    if (projected.some((entry) => entry === undefined)) return null
    entries = projected.map((entry) => ({
      ...entry!,
      summary: boundedSummary(entry!.summary),
    }))
    snapshotGeneration = storedGeneration
  }

  return {
    categories,
    subscriptions,
    source: state.selectedSource,
    entries,
    queue,
    snapshotGeneration,
    scrollAnchorByRoute: Object.fromEntries(
      Object.entries(state.scrollAnchorByRoute).slice(-CACHE_MAX_SCROLL_ANCHORS),
    ),
  }
}

function decodeEnvelope(
  value: unknown,
  expectedOwnerUserId: string,
  nowMs: number,
): ReaderCacheSnapshot | null {
  try {
    if (
      !isRecord(value) ||
      !hasExactKeys(value, ["schemaVersion", "ownerUserId", "savedAtMs", "snapshot"]) ||
      value.schemaVersion !== CACHE_SCHEMA_VERSION ||
      value.ownerUserId !== expectedOwnerUserId ||
      !isUuid(value.ownerUserId) ||
      !isNonNegativeInteger(value.savedAtMs) ||
      value.savedAtMs > nowMs + CACHE_FUTURE_SKEW_MS ||
      nowMs - value.savedAtMs > CACHE_MAX_AGE_MS ||
      !isShallowSnapshot(value.snapshot)
    ) {
      return null
    }
    const serialized = JSON.stringify(value)
    if (new TextEncoder().encode(serialized).byteLength > CACHE_MAX_SERIALIZED_BYTES) {
      return null
    }
    return isReaderCacheSnapshot(value.snapshot) ? value.snapshot : null
  } catch {
    return null
  }
}

function isShallowSnapshot(value: unknown): value is RawReaderCacheSnapshot {
  return (
    isRecord(value) &&
    hasExactKeys(value, [
      "categories",
      "subscriptions",
      "source",
      "entries",
      "queue",
      "snapshotGeneration",
      "scrollAnchorByRoute",
    ]) &&
    Array.isArray(value.categories) &&
    value.categories.length <= CACHE_MAX_CATEGORIES &&
    Array.isArray(value.subscriptions) &&
    value.subscriptions.length <= CACHE_MAX_SUBSCRIPTIONS &&
    Array.isArray(value.entries) &&
    value.entries.length <= CACHE_MAX_ENTRIES &&
    Array.isArray(value.queue) &&
    value.queue.length <= CACHE_MAX_ENTRIES &&
    isRecord(value.scrollAnchorByRoute) &&
    Object.keys(value.scrollAnchorByRoute).length <= CACHE_MAX_SCROLL_ANCHORS
  )
}

function isReaderCacheSnapshot(
  value: RawReaderCacheSnapshot,
): value is ReaderCacheSnapshot {
  if (
    !value.categories.every(isCategory) ||
    !value.subscriptions.every(isSubscription) ||
    !isReaderSource(value.source) ||
    !value.entries.every(isEntryListItemResponse) ||
    !value.queue.every((entryId) => typeof entryId === "string" && isUuid(entryId)) ||
    !isSnapshotGeneration(value.snapshotGeneration) ||
    !isScrollAnchors(value.scrollAnchorByRoute)
  ) {
    return false
  }

  const categories = value.categories as Category[]
  const subscriptions = value.subscriptions as Subscription[]
  const source = value.source as ReaderSource
  const entries = value.entries as EntryListItemResponse[]
  const queue = value.queue as string[]

  if (
    hasDuplicates(categories.map((category) => category.categoryId)) ||
    hasDuplicates(subscriptions.map((subscription) => subscription.subscriptionId)) ||
    hasDuplicates(entries.map((entry) => entry.entryId)) ||
    hasDuplicates(queue)
  ) {
    return false
  }
  const entryIds = entries.map((entry) => entry.entryId)
  if (
    entryIds.length !== queue.length ||
    entryIds.some((entryId, index) => entryId !== queue[index]) ||
    (value.snapshotGeneration === null && queue.length > 0)
  ) {
    return false
  }

  const categoryIds = new Set(categories.map((category) => category.categoryId))
  const subscribedFeedIds = new Set(subscriptions.map((subscription) => subscription.feedId))
  if (
    source.kind === "feed" &&
    !subscribedFeedIds.has(source.feedId)
  ) {
    return false
  }
  if (
    source.kind === "category" &&
    !categoryIds.has(source.categoryId)
  ) {
    return false
  }
  const visibleFeedIds = source.kind === "category"
    ? new Set(
        subscriptions
          .filter((subscription) => subscription.categoryId === source.categoryId)
          .map((subscription) => subscription.feedId),
      )
    : subscribedFeedIds
  return entries.every((entry) => {
    if (source.kind === "feed") return entry.feedId === source.feedId
    return visibleFeedIds.has(entry.feedId)
  })
}

function isReaderSource(value: unknown): value is ReaderSource {
  if (!isRecord(value) || typeof value.kind !== "string") return false
  if (value.kind === "smart") {
    return hasExactKeys(value, ["kind", "state"]) && isEntryListState(value.state)
  }
  if (value.kind === "feed") {
    return hasExactKeys(value, ["kind", "feedId"]) &&
      typeof value.feedId === "string" && isUuid(value.feedId)
  }
  return value.kind === "category" && hasExactKeys(value, ["kind", "categoryId"]) &&
    typeof value.categoryId === "string" && isUuid(value.categoryId)
}

function isSnapshotGeneration(value: unknown): value is number | null {
  return value === null || isNonNegativeInteger(value)
}

function isScrollAnchors(value: Record<string, unknown>): value is Record<string, number> {
  return Object.entries(value).every(
    ([route, offset]) =>
      route.startsWith("/reader/") &&
      route.length <= 4_096 &&
      typeof offset === "number" &&
      Number.isFinite(offset) &&
      offset >= 0 &&
      offset <= 10_000_000,
  )
}

function boundedSummary(summary: string | null): string | null {
  if (summary === null) return null
  let result = ""
  let count = 0
  for (const character of summary) {
    if (count >= CACHE_MAX_SUMMARY_CHARACTERS) break
    result += character
    count += 1
  }
  return result
}

function hasDuplicates(values: string[]): boolean {
  return new Set(values).size !== values.length
}

function isNonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
}

function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(value)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function hasExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value)
  return actual.length === keys.length && actual.every((key) => keys.includes(key))
}

function createIndexedDbStorage(): ReaderCacheStorage {
  return {
    read: () => withIndexedDbStore("readonly", (store) => store.get(CACHE_ACTIVE_KEY)),
    write: async (value) => {
      await withIndexedDbStore("readwrite", (store) => store.put(value, CACHE_ACTIVE_KEY))
    },
    clear: async () => {
      await withIndexedDbStore("readwrite", (store) => store.delete(CACHE_ACTIVE_KEY))
    },
  }
}

async function withIndexedDbStore(
  mode: IDBTransactionMode,
  request: (store: IDBObjectStore) => IDBRequest,
): Promise<unknown> {
  const database = await openCacheDatabase()
  return new Promise((resolve, reject) => {
    let result: unknown
    const transaction = database.transaction(CACHE_STORE_NAME, mode)
    const operation = request(transaction.objectStore(CACHE_STORE_NAME))
    operation.onsuccess = () => { result = operation.result }
    operation.onerror = () => transaction.abort()
    transaction.oncomplete = () => {
      database.close()
      resolve(result)
    }
    transaction.onabort = () => {
      database.close()
      reject(transaction.error ?? operation.error ?? new Error("Reader cache transaction aborted"))
    }
    transaction.onerror = () => {
      // The abort handler owns rejection and cleanup.
    }
  })
}

function openCacheDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    if (!("indexedDB" in globalThis) || !globalThis.indexedDB) {
      reject(new Error("IndexedDB is unavailable"))
      return
    }
    const request = globalThis.indexedDB.open(CACHE_DATABASE_NAME, CACHE_DATABASE_VERSION)
    request.onupgradeneeded = () => {
      if (!request.result.objectStoreNames.contains(CACHE_STORE_NAME)) {
        request.result.createObjectStore(CACHE_STORE_NAME)
      }
    }
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error ?? new Error("Reader cache database failed"))
    request.onblocked = () => reject(new Error("Reader cache database is blocked"))
  })
}
