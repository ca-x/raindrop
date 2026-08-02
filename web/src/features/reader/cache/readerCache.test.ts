import { describe, expect, it, vi } from "vitest"

import {
  createReaderCache,
  readerCacheSnapshot,
  type ReaderCacheSnapshot,
  type ReaderCacheStorage,
} from "./readerCache"
import { initialReaderState } from "../model/reducer"
import { readerReducer } from "../model/reducer"
import {
  entryId,
  makeCategory,
  makeEntry,
  makeSubscription,
} from "../model/testFixtures"
import { sourceKey, type ReaderState } from "../model/types"

const nowMs = Date.UTC(2026, 7, 2, 12)
const userId = "11111111-1111-4111-8111-111111111111"

describe("Reader cache", () => {
  it("round-trips a bounded same-user projection without authentication material", async () => {
    const storage = new MemoryStorage()
    const cache = createReaderCache(storage, () => nowMs)
    const snapshot = makeSnapshot()

    await cache.save(userId, snapshot)

    await expect(cache.load(userId)).resolves.toEqual(projectedSnapshot(snapshot))
    expect(JSON.stringify(storage.value)).not.toContain("csrf-memory")
    expect(storage.value).toMatchObject({
      schemaVersion: 2,
      ownerUserId: userId,
      validatedAtMs: nowMs,
    })
    expect(storage.clear).not.toHaveBeenCalled()
  })

  it("never persists URL-bearing feed credentials in the Reader projection", async () => {
    const storage = new MemoryStorage()
    const cache = createReaderCache(storage, () => nowMs)
    const secret = "reader-cache-query-secret"
    const snapshot = makeSnapshot()
    snapshot.subscriptions[0] = makeSubscription({
      feedUrl: `https://feeds.example/private.xml?token=${secret}`,
      siteUrl: `https://publisher.example/private?token=${secret}`,
    })
    snapshot.entries[0] = makeEntry({
      siteUrl: `https://publisher.example/private?token=${secret}`,
      canonicalUrl: `https://publisher.example/article?token=${secret}`,
    })

    await cache.save(userId, snapshot)

    expect(JSON.stringify(storage.value)).not.toContain(secret)
  })

  it("does not turn cache access into a sliding seven-day freshness window", async () => {
    const storage = new MemoryStorage()
    let currentMs = nowMs
    const cache = createReaderCache(storage, () => currentMs)
    const snapshot = makeSnapshot()

    await cache.save(userId, snapshot)
    currentMs += 6 * 24 * 60 * 60 * 1_000
    await expect(cache.load(userId)).resolves.toEqual(projectedSnapshot(snapshot))
    await cache.save(userId, snapshot)
    currentMs += 2 * 24 * 60 * 60 * 1_000

    await expect(cache.load(userId)).resolves.toBeNull()
    expect(storage.clear).toHaveBeenCalledOnce()
  })

  it("advances freshness only after an authoritative validation", async () => {
    const storage = new MemoryStorage()
    let currentMs = nowMs
    const cache = createReaderCache(storage, () => currentMs)
    const snapshot = makeSnapshot()

    await cache.save(userId, snapshot)
    currentMs += 6 * 24 * 60 * 60 * 1_000
    await cache.load(userId)
    await cache.save(userId, snapshot, { markValidated: true })
    currentMs += 2 * 24 * 60 * 60 * 1_000

    await expect(cache.load(userId)).resolves.toEqual(projectedSnapshot(snapshot))
  })

  it.each([
    ["another owner", { ownerUserId: "22222222-2222-4222-8222-222222222222" }],
    ["an expired snapshot", { validatedAtMs: nowMs - 7 * 24 * 60 * 60 * 1_000 - 1 }],
    ["an obsolete schema", { schemaVersion: 1 }],
    ["a future timestamp", { validatedAtMs: nowMs + 5 * 60 * 1_000 + 1 }],
  ])("clears %s instead of hydrating it", async (_label, override) => {
    const storage = new MemoryStorage({ ...makeEnvelope(), ...override })
    const cache = createReaderCache(storage, () => nowMs)

    await expect(cache.load(userId)).resolves.toBeNull()
    expect(storage.clear).toHaveBeenCalledOnce()
  })

  it("rejects malformed, oversized, and source-inconsistent projections", async () => {
    const malformed = new MemoryStorage({
      ...makeEnvelope(),
      snapshot: { ...makeSnapshot(), subscriptions: [{ unexpected: true }] },
    })
    await expect(createReaderCache(malformed, () => nowMs).load(userId)).resolves.toBeNull()
    expect(malformed.clear).toHaveBeenCalledOnce()

    const tooManyEntries = Array.from({ length: 101 }, (_, index) =>
      makeEntry({
        entryId: `00000000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
      }),
    )
    const oversized = new MemoryStorage({
      ...makeEnvelope(),
      snapshot: {
        ...makeSnapshot(),
        entries: tooManyEntries,
        queue: tooManyEntries.map((entry) => entry.entryId),
      },
    })
    await expect(createReaderCache(oversized, () => nowMs).load(userId)).resolves.toBeNull()
    expect(oversized.clear).toHaveBeenCalledOnce()

    const inconsistent = new MemoryStorage({
      ...makeEnvelope(),
      snapshot: { ...makeSnapshot(), queue: [entryId, "00000000-0000-4000-8000-000000000399"] },
    })
    await expect(createReaderCache(inconsistent, () => nowMs).load(userId)).resolves.toBeNull()
    expect(inconsistent.clear).toHaveBeenCalledOnce()

    const tooLarge = new MemoryStorage({
      ...makeEnvelope(),
      snapshot: {
        ...makeSnapshot(),
        subscriptions: [makeSubscription({
          titleOverride: "x".repeat(2 * 1024 * 1024),
        })],
      },
    })
    await expect(createReaderCache(tooLarge, () => nowMs).load(userId)).resolves.toBeNull()
    expect(tooLarge.clear).toHaveBeenCalledOnce()
  })

  it("treats storage failures as cache misses and no-op writes", async () => {
    const storage: ReaderCacheStorage = {
      read: vi.fn(async () => { throw new DOMException("denied", "SecurityError") }),
      write: vi.fn(async () => { throw new DOMException("full", "QuotaExceededError") }),
      clear: vi.fn(async () => { throw new DOMException("denied", "SecurityError") }),
    }
    const cache = createReaderCache(storage, () => nowMs)

    await expect(cache.load(userId)).resolves.toBeNull()
    await expect(cache.save(userId, makeSnapshot())).resolves.toBeUndefined()
    await expect(cache.clear()).resolves.toBeUndefined()
  })

  it("removes an older snapshot when its replacement write is rejected", async () => {
    const storage = new MemoryStorage(makeEnvelope())
    storage.write = vi.fn(async () => {
      throw new DOMException("full", "QuotaExceededError")
    })
    const cache = createReaderCache(storage, () => nowMs)

    await expect(cache.load(userId)).resolves.not.toBeNull()
    await expect(cache.save(userId, {
      ...makeSnapshot(),
      entries: [],
      queue: [],
      snapshotGeneration: 8,
    })).resolves.toBeUndefined()

    expect(storage.value).toBeNull()
    expect(storage.clear).toHaveBeenCalledOnce()
  })

  it("does not refresh cache age when an ordinary write retries after failure", async () => {
    const storage = new MemoryStorage(makeEnvelope())
    let rejectNextWrite = true
    storage.write = vi.fn(async (value: unknown) => {
      if (rejectNextWrite) {
        rejectNextWrite = false
        throw new DOMException("full", "QuotaExceededError")
      }
      storage.value = value
    })
    let currentMs = nowMs
    const cache = createReaderCache(storage, () => currentMs)
    await cache.load(userId)

    currentMs += 60 * 60 * 1_000
    await cache.save(userId, makeSnapshot())
    currentMs += 60 * 60 * 1_000
    await cache.save(userId, makeSnapshot())

    expect(storage.value).toMatchObject({ validatedAtMs: nowMs })
  })

  it("rejects non-serializable cache records without throwing", async () => {
    const cyclic: Record<string, unknown> = makeEnvelope()
    cyclic.self = cyclic
    const storage = new MemoryStorage(cyclic)

    await expect(createReaderCache(storage, () => nowMs).load(userId)).resolves.toBeNull()
    expect(storage.clear).toHaveBeenCalledOnce()
  })

  it("replaces the one active owner and cannot restore the previous owner", async () => {
    const storage = new MemoryStorage()
    const cache = createReaderCache(storage, () => nowMs)
    const otherUserId = "22222222-2222-4222-8222-222222222222"

    await cache.save(userId, makeSnapshot())
    await cache.save(otherUserId, {
      ...makeSnapshot(),
      entries: [],
      queue: [],
      snapshotGeneration: 8,
    })

    await expect(cache.load(otherUserId)).resolves.toMatchObject({
      entries: [],
      snapshotGeneration: 8,
    })
    await expect(cache.load(userId)).resolves.toBeNull()
    expect(storage.clear).toHaveBeenCalledOnce()
  })

  it("removes the previous snapshot when a replacement fails validation", async () => {
    const storage = new MemoryStorage(makeEnvelope())
    const cache = createReaderCache(storage, () => nowMs)
    const invalid = {
      ...makeSnapshot(),
      queue: ["00000000-0000-4000-8000-000000000399"],
    }

    await cache.save(userId, invalid)

    expect(storage.value).toBeNull()
    expect(storage.clear).toHaveBeenCalledOnce()
  })

  it("quarantines stale writers after a coordinated clear until a new load", async () => {
    const storage = new MemoryStorage()
    const coordination = new MemoryCoordination()
    const staleCache = createReaderCache(storage, () => nowMs, coordination)
    const clearingCache = createReaderCache(storage, () => nowMs, coordination)

    await staleCache.load(userId)
    await staleCache.save(userId, makeSnapshot())
    await clearingCache.load(userId)
    await clearingCache.clear()
    await staleCache.save(userId, makeSnapshot())

    expect(storage.value).toBeNull()

    await staleCache.load(userId)
    await staleCache.save(userId, makeSnapshot())
    expect(storage.value).toMatchObject({ ownerUserId: userId })
  })

  it("projects only the first 100 current rows and bounds summaries by Unicode scalar", () => {
    const source = { kind: "smart", state: "UNREAD" } as const
    const entries = Array.from({ length: 101 }, (_, index) => makeEntry({
      entryId: `00000000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
      summary: index === 0 ? `${"😀".repeat(512)}truncated` : `Summary ${index}`,
    }))
    const state: ReaderState = {
      ...initialReaderState,
      categoriesById: { [makeCategory().categoryId]: makeCategory() },
      categoryOrder: [makeCategory().categoryId],
      subscriptionsById: {
        [makeSubscription().subscriptionId]: makeSubscription(),
      },
      subscriptionOrder: [makeSubscription().subscriptionId],
      entriesById: Object.fromEntries(entries.map((entry) => [entry.entryId, entry])),
      queueBySourceKey: { [sourceKey(source)]: entries.map((entry) => entry.entryId) },
      snapshotGenerationBySource: { [sourceKey(source)]: 9 },
      selectedSource: source,
      paneStatus: { ...initialReaderState.paneStatus, subscriptions: "ready", queue: "ready" },
    }

    const snapshot = readerCacheSnapshot(state)

    expect(snapshot?.entries).toHaveLength(100)
    expect(snapshot?.queue).toHaveLength(100)
    expect(snapshot?.entries[0]?.summary).toBe("😀".repeat(512))
    expect([...(snapshot?.entries[0]?.summary ?? "")]).toHaveLength(512)
    expect(snapshot?.snapshotGeneration).toBe(9)
  })

  it("never persists a feed search subset as the complete feed queue", () => {
    const subscription = makeSubscription()
    const source = { kind: "feed", feedId: subscription.feedId } as const
    const entry = makeEntry()
    const state: ReaderState = {
      ...initialReaderState,
      subscriptionsById: { [subscription.subscriptionId]: subscription },
      subscriptionOrder: [subscription.subscriptionId],
      entriesById: { [entry.entryId]: entry },
      queueBySourceKey: { [sourceKey(source)]: [entry.entryId] },
      snapshotGenerationBySource: { [sourceKey(source)]: 9 },
      selectedSource: source,
      feedSearchQuery: "security",
      paneStatus: { ...initialReaderState.paneStatus, subscriptions: "ready", queue: "ready" },
    }

    expect(readerCacheSnapshot(state)).toBeNull()
  })

  it("keeps a refreshed scroll anchor inside the 32-route LRU projection", () => {
    const subscription = makeSubscription()
    let state: ReaderState = {
      ...initialReaderState,
      subscriptionsById: { [subscription.subscriptionId]: subscription },
      subscriptionOrder: [subscription.subscriptionId],
      queueBySourceKey: { "smart:UNREAD": [] },
      snapshotGenerationBySource: { "smart:UNREAD": 1 },
      paneStatus: {
        ...initialReaderState.paneStatus,
        subscriptions: "ready",
        queue: "ready",
      },
    }
    for (let index = 0; index < 33; index += 1) {
      state = readerReducer(state, {
        type: "scrollAnchorRecorded",
        route: `/reader/feed/${index}`,
        offset: index,
      })
    }
    state = readerReducer(state, {
      type: "scrollAnchorRecorded",
      route: "/reader/feed/0",
      offset: 999,
    })

    const anchors = readerCacheSnapshot(state)?.scrollAnchorByRoute
    expect(Object.keys(anchors ?? {})).toHaveLength(32)
    expect(anchors?.["/reader/feed/0"]).toBe(999)
    expect(anchors?.["/reader/feed/1"]).toBeUndefined()
  })
})

class MemoryStorage implements ReaderCacheStorage {
  value: unknown
  read = vi.fn(async () => this.value)
  write = vi.fn(async (value: unknown) => { this.value = value })
  clear = vi.fn(async () => { this.value = null })

  constructor(value: unknown = null) {
    this.value = value
  }
}

class MemoryCoordination {
  private readonly listeners = new Set<() => void>()

  subscribe(onCleared: () => void) {
    this.listeners.add(onCleared)
  }

  notifyCleared() {
    for (const listener of this.listeners) listener()
  }
}

function makeSnapshot(): ReaderCacheSnapshot {
  const entry = makeEntry({ summary: "Cached summary" })
  return {
    categories: [makeCategory()],
    subscriptions: [makeSubscription()],
    source: { kind: "smart", state: "UNREAD" },
    entries: [entry],
    queue: [entry.entryId],
    snapshotGeneration: 7,
    scrollAnchorByRoute: { "/reader/unread": 128 },
  }
}

function makeEnvelope(): Record<string, unknown> {
  return {
    schemaVersion: 2,
    ownerUserId: userId,
    validatedAtMs: nowMs,
    snapshot: projectedSnapshot(makeSnapshot()),
  }
}

function projectedSnapshot(snapshot: ReaderCacheSnapshot): ReaderCacheSnapshot {
  return {
    ...snapshot,
    subscriptions: snapshot.subscriptions.map((subscription) => {
      const { feedUrl: _feedUrl, siteUrl: _siteUrl, ...projected } = subscription as
        typeof subscription & { feedUrl?: string; siteUrl?: string | null }
      return projected
    }),
    entries: snapshot.entries.map((entry) => {
      const { siteUrl: _siteUrl, canonicalUrl: _canonicalUrl, ...projected } = entry as
        typeof entry & { siteUrl?: string | null; canonicalUrl?: string | null }
      return projected
    }),
  }
}
