import type { Page, Route } from "@playwright/test"

import type {
  EntryDetailResponse,
  EntryListItemResponse,
  EntryStateResponse,
  PatchEntryStateRequest,
} from "../../src/features/reader/api/reader.generated"
import type {
  Refresh,
  Subscription,
} from "../../src/features/reader/api/subscription.generated"

export const readerIds = {
  feedA: "00000000-0000-4000-8000-000000000101",
  feedB: "00000000-0000-4000-8000-000000000102",
  subscriptionA: "00000000-0000-4000-8000-000000000201",
  subscriptionB: "00000000-0000-4000-8000-000000000202",
  firstEntry: "00000000-0000-4000-8000-000000000301",
  secondEntry: "00000000-0000-4000-8000-000000000302",
  thirdEntry: "00000000-0000-4000-8000-000000000303",
  fourthEntry: "00000000-0000-4000-8000-000000000304",
  pendingEntry: "00000000-0000-4000-8000-00000000030c",
  deepOnlyEntry: "00000000-0000-4000-8000-00000000030d",
} as const

interface PatchCall {
  entryId: string
  body: PatchEntryStateRequest
  csrf: string | undefined
}

export interface ReaderApiFixture {
  discoverPending: () => void
  entryState: (entryId: string) => EntryStateResponse
  patches: PatchCall[]
}

export async function installReaderApiFixture(page: Page): Promise<ReaderApiFixture> {
  const entries = createEntries()
  const details = new Map(entries.map((entry) => [entry.entryId, detailFor(entry)]))
  const deepOnly = createEntry(readerIds.deepOnlyEntry, readerIds.feedA, 13, "Hostile deep-link article")
  details.set(deepOnly.entryId, hostileDetail(deepOnly))
  const pending = createEntry(readerIds.pendingEntry, readerIds.feedA, 12, "Newly discovered entry")
  details.set(pending.entryId, detailFor(pending))
  const patches: PatchCall[] = []
  let hasPending = false

  await page.route("**/api/v1/subscriptions**", async (route) => {
    await handleSubscriptions(route)
  })
  await page.route("**/api/v1/categories**", async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    if (url.pathname === "/api/v1/categories" && request.method() === "GET") {
      await json(route, { items: [] })
      return
    }
    throw new Error(`unexpected Category request: ${request.method()} ${url.pathname}`)
  })
  await page.route("**/api/v1/entries**", async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    const stateMatch = /^\/api\/v1\/entries\/([^/]+)\/state$/u.exec(url.pathname)
    if (stateMatch && request.method() === "PATCH") {
      const entryId = decodeURIComponent(stateMatch[1])
      const body = request.postDataJSON() as PatchEntryStateRequest
      patches.push({ entryId, body, csrf: request.headers()["x-csrf-token"] })
      const item = entries.find((entry) => entry.entryId === entryId) ?? (entryId === pending.entryId ? pending : deepOnly)
      if (body.isRead !== undefined) item.isRead = body.isRead
      if (body.isStarred !== undefined) item.isStarred = body.isStarred
      const detail = details.get(entryId)
      if (detail) {
        detail.isRead = item.isRead
        detail.isStarred = item.isStarred
      }
      await json(route, { entryId, isRead: item.isRead, isStarred: item.isStarred })
      return
    }
    const detailMatch = /^\/api\/v1\/entries\/([^/]+)$/u.exec(url.pathname)
    if (detailMatch && request.method() === "GET") {
      const detail = details.get(decodeURIComponent(detailMatch[1]))
      if (!detail) throw new Error(`unexpected Reader detail: ${url.pathname}`)
      await json(route, detail)
      return
    }
    if (url.pathname === "/api/v1/entries" && request.method() === "GET") {
      const state = url.searchParams.get("state")
      const feedId = url.searchParams.get("feedId")
      let items = entries
      if (feedId) items = items.filter((entry) => entry.feedId === feedId)
      if (state === "UNREAD") items = items.filter((entry) => !entry.isRead)
      if (state === "STARRED") items = items.filter((entry) => entry.isStarred)
      if (hasPending && state === "UNREAD" && (!feedId || feedId === pending.feedId)) {
        items = [pending, ...items]
      }
      await json(route, { items, nextCursor: null, snapshotGeneration: 1 })
      return
    }
    throw new Error(`unexpected Reader request: ${request.method()} ${url.pathname}${url.search}`)
  })

  return {
    discoverPending: () => { hasPending = true },
    entryState: (entryId) => {
      const detail = details.get(entryId)
      if (!detail) throw new Error(`unknown fixture entry ${entryId}`)
      return { entryId, isRead: detail.isRead, isStarred: detail.isStarred }
    },
    patches,
  }
}

async function handleSubscriptions(route: Route): Promise<void> {
  const request = route.request()
  const url = new URL(request.url())
  if (url.pathname === "/api/v1/subscriptions" && request.method() === "GET") {
    await json(route, { items: subscriptions, nextCursor: null })
    return
  }
  const refreshMatch = /^\/api\/v1\/subscriptions\/([^/]+)\/refresh$/u.exec(url.pathname)
  if (refreshMatch && request.method() === "POST") {
    await json(route, pendingRefresh())
    return
  }
  throw new Error(`unexpected Subscription request: ${request.method()} ${url.pathname}`)
}

const subscriptions: Subscription[] = [
  subscription(readerIds.subscriptionA, readerIds.feedA, "Quiet Web", "https://quiet.example/"),
  subscription(readerIds.subscriptionB, readerIds.feedB, "Rust Dispatch", "https://rust.example/"),
]

function subscription(subscriptionId: string, feedId: string, title: string, siteUrl: string): Subscription {
  return {
    subscriptionId,
    feedId,
    categoryId: null,
    titleOverride: null,
    position: 0,
    title,
    siteUrl,
    unreadCount: 6,
    refresh: null,
  }
}

function createEntries(): EntryListItemResponse[] {
  return Array.from({ length: 12 }, (_, index) => {
    const ordinal = index + 1
    const entryId = `00000000-0000-4000-8000-${String(300 + ordinal).padStart(12, "0")}`
    const feedId = ordinal <= 6 ? readerIds.feedA : readerIds.feedB
    const titles = ["First quiet article", "Second quiet article", "Third quiet article"]
    return createEntry(entryId, feedId, ordinal, titles[index] ?? `Fixture entry ${String(ordinal).padStart(2, "0")}`)
  })
}

function createEntry(entryId: string, feedId: string, index: number, title: string): EntryListItemResponse {
  const feedTitle = feedId === readerIds.feedA ? "Quiet Web" : "Rust Dispatch"
  return {
    entryId,
    feedId,
    feedTitle,
    siteUrl: feedId === readerIds.feedA ? "https://quiet.example/" : "https://rust.example/",
    title,
    author: "Reader Fixture",
    summary: `Deterministic Reader entry ${index}`,
    canonicalUrl: `https://quiet.example/articles/${entryId}`,
    publishedAtUs: 1_700_000_000_000_000 + index,
    sortAtUs: 1_700_000_000_000_000 + index,
    isRead: false,
    isStarred: false,
  }
}

function detailFor(entry: EntryListItemResponse): EntryDetailResponse {
  const paragraphs = Array.from({ length: 32 }, (_, index) => `<p>Scrollable paragraph ${index + 1}</p>`).join("")
  return { ...entry, contentHtml: `<p>${entry.summary}</p>${paragraphs}`, inertImages: [], enclosures: [] }
}

function hostileDetail(entry: EntryListItemResponse): EntryDetailResponse {
  const longToken = `https://overflow.invalid/${"x".repeat(700)}`
  const longCode = "0123456789abcdef".repeat(120)
  const cells = Array.from({ length: 18 }, (_, index) => `<td>column-${index}-${"w".repeat(28)}</td>`).join("")
  const paragraphs = Array.from({ length: 32 }, (_, index) => `<p>Hostile paragraph ${index + 1}</p>`).join("")
  return {
    ...entry,
    contentHtml: `<p data-fixture="long-token">${longToken}</p><img data-raindrop-inert-image="0" alt="Inert wide image" width="1600" height="900"><table data-fixture="wide-table"><tbody><tr>${cells}</tr></tbody></table><pre data-fixture="wide-pre"><code>${longCode}</code></pre><iframe data-fixture="wide-iframe" title="Contained frame" width="1600" height="240"></iframe><video data-fixture="wide-video" controls width="1600"></video>${paragraphs}`,
    inertImages: [{
      imageIndex: 0,
      sourceUrl: "https://publisher.invalid/tracker.gif",
      alt: "Inert wide image",
      width: 1600,
      height: 900,
    }],
    enclosures: [],
  }
}

function pendingRefresh(): Refresh {
  return {
    operationId: "00000000-0000-4000-8000-000000000401",
    state: "PENDING",
    newCount: 0,
    updatedCount: 0,
    droppedCount: 0,
    generation: null,
    errorCode: null,
    retryAt: null,
    queuedAt: "2026-07-18T00:00:00.000000Z",
    startedAt: null,
    completedAt: null,
  }
}

async function json(route: Route, body: unknown): Promise<void> {
  await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) })
}
