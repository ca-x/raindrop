import type { Page, Route } from "@playwright/test"

import type {
  EntryDetailResponse,
  EntryListItemResponse,
  EntryStateResponse,
  MarkEntriesReadRequest,
  PatchEntryStateRequest,
} from "../../src/features/reader/api/reader.generated"
import type {
  PatchUserPreferencesRequest,
  UserPreferences,
} from "../../src/features/preferences/api/preferences.generated"
import {
  installReaderOrganizationFixture,
  readerOrganizationIds,
  type ReaderOrganizationFixture,
} from "./readerOrganizationFixture"

export const readerIds = {
  ...readerOrganizationIds,
  feedA: "00000000-0000-4000-8000-000000000101",
  feedB: "00000000-0000-4000-8000-000000000102",
  subscriptionA: "00000000-0000-4000-8000-000000000201",
  subscriptionB: "00000000-0000-4000-8000-000000000202",
  firstEntry: "00000000-0000-4000-8000-000000000301",
  secondEntry: "00000000-0000-4000-8000-000000000302",
  thirdEntry: "00000000-0000-4000-8000-000000000303",
  fourthEntry: "00000000-0000-4000-8000-000000000304",
  seventhEntry: "00000000-0000-4000-8000-000000000307",
  pendingEntry: "00000000-0000-4000-8000-00000000030c",
  deepOnlyEntry: "00000000-0000-4000-8000-00000000030d",
  latePendingEntry: "00000000-0000-4000-8000-00000000030e",
} as const

interface PatchCall {
  entryId: string
  body: PatchEntryStateRequest
  csrf: string | undefined
}

interface PreferencePatchCall {
  body: PatchUserPreferencesRequest
  csrf: string
}

interface EntryListCall {
  state: string | null
  feedId: string | null
  categoryId: string | null
  search: string | null
  snapshotGeneration: number
}

interface MarkReadCall {
  body: MarkEntriesReadRequest
  csrf: string | undefined
}

export interface ReaderPreferenceFixture {
  current: () => UserPreferences
  failNextPatch: () => void
  patches: PreferencePatchCall[]
}

export interface ReaderApiFixture {
  discoverPending: () => void
  discoverLatePending: () => void
  entryState: (entryId: string) => EntryStateResponse
  entryLists: EntryListCall[]
  markReadCalls: MarkReadCall[]
  organization: ReaderOrganizationFixture
  patches: PatchCall[]
  preferences: ReaderPreferenceFixture
}

export async function installReaderApiFixture(page: Page): Promise<ReaderApiFixture> {
  const entries = createEntries()
  const details = new Map(entries.map((entry) => [entry.entryId, detailFor(entry)]))
  const deepOnly = createEntry(readerIds.deepOnlyEntry, readerIds.feedA, 13, "Hostile deep-link article")
  details.set(deepOnly.entryId, hostileDetail(deepOnly))
  const pending = createEntry(readerIds.pendingEntry, readerIds.feedA, 12, "Newly discovered entry")
  details.set(pending.entryId, detailFor(pending))
  const latePending = createEntry(
    readerIds.latePendingEntry,
    readerIds.feedA,
    14,
    "Later snapshot entry",
  )
  details.set(latePending.entryId, detailFor(latePending))
  const patches: PatchCall[] = []
  const entryLists: EntryListCall[] = []
  const markReadCalls: MarkReadCall[] = []
  const preferences = await installPreferenceFixture(page)
  let hasPending = false
  let hasLatePending = false
  const organization = await installReaderOrganizationFixture(page, {
    feedA: readerIds.feedA,
    feedB: readerIds.feedB,
    subscriptionA: readerIds.subscriptionA,
    subscriptionB: readerIds.subscriptionB,
  })
  const availableEntries = () => [
    ...(hasLatePending ? [latePending] : []),
    ...(hasPending ? [pending] : []),
    ...entries,
  ]
  const currentSnapshot = () => hasLatePending ? 3 : hasPending ? 2 : 1
  const entryGeneration = new Map([
    ...entries.map((entry) => [entry.entryId, 1] as const),
    [pending.entryId, 2] as const,
    [latePending.entryId, 3] as const,
  ])
  const syncUnreadCounts = () => {
    for (const subscription of organization.subscriptions) {
      subscription.unreadCount = availableEntries().filter(
        (entry) => entry.feedId === subscription.feedId && !entry.isRead,
      ).length
    }
  }

  await page.route("**/api/v1/entries**", async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    if (url.pathname === "/api/v1/entries/mark-read" && request.method() === "POST") {
      const body = request.postDataJSON() as MarkEntriesReadRequest
      markReadCalls.push({ body, csrf: request.headers()["x-csrf-token"] })
      const categoryFeedIds = body.categoryId
        ? organization.feedIdsForCategory(body.categoryId)
        : null
      for (const entry of availableEntries()) {
        const generation = entryGeneration.get(entry.entryId)
        const inScope = body.feedId
          ? entry.feedId === body.feedId
          : categoryFeedIds
            ? categoryFeedIds.has(entry.feedId)
            : true
        if (generation !== undefined && generation <= body.snapshotGeneration && inScope) {
          entry.isRead = true
          const detail = details.get(entry.entryId)
          if (detail) detail.isRead = true
        }
      }
      syncUnreadCounts()
      await route.fulfill({ status: 204, body: "" })
      return
    }
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
      syncUnreadCounts()
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
      const categoryId = url.searchParams.get("categoryId")
      const search = url.searchParams.get("search")
      if (feedId && categoryId) {
        throw new Error("Reader fixture received mutually exclusive feed/category filters")
      }
      let items = availableEntries()
      if (feedId) items = items.filter((entry) => entry.feedId === feedId)
      if (categoryId) {
        const feedIds = organization.feedIdsForCategory(categoryId)
        items = items.filter((entry) => feedIds.has(entry.feedId))
      }
      if (state === "UNREAD") items = items.filter((entry) => !entry.isRead)
      if (state === "STARRED") items = items.filter((entry) => entry.isStarred)
      if (search) items = items.filter((entry) => matchesSearch(entry, details, search))
      const snapshotGeneration = currentSnapshot()
      entryLists.push({ state, feedId, categoryId, search, snapshotGeneration })
      await json(route, { items, nextCursor: null, snapshotGeneration })
      return
    }
    throw new Error(`unexpected Reader request: ${request.method()} ${url.pathname}${url.search}`)
  })

  return {
    discoverPending: () => {
      hasPending = true
      syncUnreadCounts()
    },
    discoverLatePending: () => {
      if (!hasPending) throw new Error("discover the first pending Entry before the late Entry")
      hasLatePending = true
      syncUnreadCounts()
    },
    entryState: (entryId) => {
      const detail = details.get(entryId)
      if (!detail) throw new Error(`unknown fixture entry ${entryId}`)
      return { entryId, isRead: detail.isRead, isStarred: detail.isStarred }
    },
    entryLists,
    markReadCalls,
    organization,
    patches,
    preferences,
  }
}

function matchesSearch(
  entry: EntryListItemResponse,
  details: Map<string, EntryDetailResponse>,
  rawSearch: string,
): boolean {
  const terms = rawSearch.trim().toLocaleLowerCase().split(/\s+/u).filter(Boolean)
  const content = details.get(entry.entryId)?.contentHtml.replace(/<[^>]*>/gu, " ") ?? ""
  const projection = [entry.title, entry.author, entry.summary, content]
    .filter((value): value is string => Boolean(value))
    .join(" ")
    .toLocaleLowerCase()
  return terms.every((term) => projection.includes(term))
}

async function installPreferenceFixture(page: Page): Promise<ReaderPreferenceFixture> {
  let current: UserPreferences = {
    locale: "en",
    themeMode: "SYSTEM",
    layoutDensity: "BALANCED",
    readingFontScale: 100,
  }
  let shouldFailNextPatch = false
  const patches: PreferencePatchCall[] = []

  await page.route("**/api/v1/preferences", async (route) => {
    const request = route.request()
    const method = request.method()
    if (method === "GET") {
      await json(route, current)
      return
    }
    if (method !== "PATCH") {
      throw new Error(`unexpected Preferences request: ${method}`)
    }
    const csrf = request.headers()["x-csrf-token"]
    if (!csrf) throw new Error("preference mutation omitted CSRF")
    const body = request.postDataJSON() as PatchUserPreferencesRequest
    patches.push({ body, csrf })
    if (shouldFailNextPatch) {
      shouldFailNextPatch = false
      await json(route, {
        error: {
          code: "INTERNAL_ERROR",
          message: "Preferences unavailable",
          requestId: "00000000-0000-4000-8000-000000000902",
        },
      }, 500)
      return
    }
    current = { ...current, ...body }
    await json(route, current)
  })

  return {
    current: () => structuredClone(current),
    failNextPatch: () => { shouldFailNextPatch = true },
    patches,
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

async function json(route: Route, body: unknown, status = 200): Promise<void> {
  await route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) })
}
