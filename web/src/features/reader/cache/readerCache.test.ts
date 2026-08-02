import { describe, expect, it, vi } from "vitest"

import {
  createReaderCache,
  readerCacheSnapshot,
  type ReaderCacheSnapshot,
  type ReaderCacheStorage,
} from "./readerCache"
import { initialReaderState } from "../model/reducer"
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

    await expect(cache.load(userId)).resolves.toEqual(snapshot)
    expect(JSON.stringify(storage.value)).not.toContain("csrf-memory")
    expect(storage.value).toMatchObject({
      schemaVersion: 1,
      ownerUserId: userId,
      savedAtMs: nowMs,
    })
    expect(storage.clear).not.toHaveBeenCalled()
  })

  it.each([
    ["another owner", { ownerUserId: "22222222-2222-4222-8222-222222222222" }],
    ["an expired snapshot", { savedAtMs: nowMs - 7 * 24 * 60 * 60 * 1_000 - 1 }],
    ["a future schema", { schemaVersion: 2 }],
    ["a future timestamp", { savedAtMs: nowMs + 5 * 60 * 1_000 + 1 }],
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
    schemaVersion: 1,
    ownerUserId: userId,
    savedAtMs: nowMs,
    snapshot: makeSnapshot(),
  }
}
