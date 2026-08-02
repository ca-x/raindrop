import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { StrictMode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import type { SessionResponse } from "../auth/session"
import type { ReaderCache, ReaderCacheSnapshot } from "./cache/readerCache"
import { makeCategory, makeEntry, makeSubscription } from "./model/testFixtures"
import { ReadyPage } from "./ReadyPage"

const session: SessionResponse = {
  user: {
    id: "11111111-1111-4111-8111-111111111111",
    username: "reader",
    email: null,
    isDisabled: false,
    roles: [],
  },
  csrfToken: "csrf",
  expiresAt: "2027-01-01T00:00:00Z",
}
const entryId = "00000000-0000-4000-8000-000000000301"

describe("ReadyPage lifecycle", () => {
  beforeEach(() => {
    localStorage.clear()
    document.documentElement.removeAttribute("data-theme")
    document.documentElement.removeAttribute("data-raindrop-density")
    document.documentElement.style.removeProperty("--raindrop-reading-scale")
  })
  afterEach(() => vi.unstubAllGlobals())

  it("restores a deep-linked source from the session-scoped cache before Reader requests settle", async () => {
    activateLocale("en")
    const feedId = makeSubscription().feedId
    window.history.replaceState(null, "", `/reader/feed/${feedId}`)
    const cache = makeReaderCache(cachedFeedSnapshot())
    const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url.startsWith("/api/v1/entries")) {
        return pendingResponse(init?.signal)
      }
      if (url.startsWith("/api/v1/subscriptions")) {
        return Promise.resolve(jsonResponse({
          ownerUserId: session.user.id,
          items: [makeSubscription()],
          nextCursor: null,
        }))
      }
      return Promise.resolve(jsonResponse(responseBody(url)))
    })
    vi.stubGlobal("fetch", fetchMock)

    render(
      <Providers>
        <ReadyPage
          session={session}
          onLoggedOut={vi.fn()}
          readerCache={cache}
        />
      </Providers>,
    )

    expect(await screen.findByText("Cached article")).toBeVisible()
    expect(cache.load).toHaveBeenCalledWith(session.user.id)
    await waitFor(() => {
      expect(fetchMock.mock.calls.some(([input]) => {
        const url = new URL(String(input), "https://raindrop.test")
        return url.pathname === "/api/v1/entries" &&
          url.searchParams.get("feedId") === feedId
      })).toBe(true)
    })
  })

  it("falls back without restarting the request when validation removes a cached deep link", async () => {
    activateLocale("en")
    const feedId = makeSubscription().feedId
    window.history.replaceState(null, "", `/reader/feed/${feedId}`)
    const fetchMock = vi.fn((input: RequestInfo | URL) =>
      Promise.resolve(jsonResponse(responseBody(String(input)))),
    )
    vi.stubGlobal("fetch", fetchMock)

    render(
      <Providers>
        <ReadyPage
          session={session}
          onLoggedOut={vi.fn()}
          readerCache={makeReaderCache(cachedFeedSnapshot())}
        />
      </Providers>,
    )

    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))
    expect(await screen.findByRole("heading", { name: "No entries here" })).toBeVisible()
    expect(fetchMock.mock.calls.filter(([input]) =>
      String(input).startsWith("/api/v1/entries"),
    ).length).toBeLessThanOrEqual(2)
  })

  it("waits for Reader cache removal before completing explicit logout", async () => {
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/unread")
    const cacheClear = deferred<void>()
    const cache = makeReaderCache(null, () => cacheClear.promise)
    const onLoggedOut = vi.fn()
    vi.stubGlobal("fetch", vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      if (url === "/api/v1/auth/logout") {
        return Promise.resolve(new Response(null, { status: 204 }))
      }
      return Promise.resolve(jsonResponse(responseBody(url)))
    }))
    const user = userEvent.setup()
    render(
      <Providers>
        <ReadyPage
          session={session}
          onLoggedOut={onLoggedOut}
          readerCache={cache}
        />
      </Providers>,
    )
    await screen.findByRole("heading", { name: "No entries here" })

    await user.click(screen.getByRole("button", { name: "Open menu" }))
    await user.click(await screen.findByText("Sign out"))
    await waitFor(() => expect(cache.clear).toHaveBeenCalledOnce())
    expect(onLoggedOut).not.toHaveBeenCalled()

    cacheClear.resolve()
    await waitFor(() => expect(onLoggedOut).toHaveBeenCalledOnce())
  })

  it("clears Reader cache before another ready-page request reports session expiry", async () => {
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/unread")
    const cacheClear = deferred<void>()
    const cache = makeReaderCache(null, () => cacheClear.promise)
    const onLoggedOut = vi.fn()
    vi.stubGlobal("fetch", vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      if (url === "/api/v2/preferences") {
        return Promise.resolve(new Response(JSON.stringify({
          error: { code: "AUTHENTICATION_REQUIRED", message: "Sign in again" },
        }), {
          status: 401,
          headers: { "content-type": "application/json" },
        }))
      }
      return Promise.resolve(jsonResponse(responseBody(url)))
    }))

    render(
      <Providers>
        <ReadyPage
          session={session}
          onLoggedOut={onLoggedOut}
          readerCache={cache}
        />
      </Providers>,
    )

    await waitFor(() => expect(cache.clear).toHaveBeenCalledOnce())
    expect(onLoggedOut).not.toHaveBeenCalled()
    cacheClear.resolve()
    await waitFor(() => expect(onLoggedOut).toHaveBeenCalledOnce())
  })

  it("starts Reader and preference loading together without blocking the workspace", async () => {
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/unread")
    const preferenceResponse = deferred<Response>()
    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      return url === "/api/v2/preferences"
        ? preferenceResponse.promise
        : Promise.resolve(jsonResponse(responseBody(url)))
    })
    vi.stubGlobal("fetch", fetchMock)

    render(
      <Providers>
        <ReadyPage session={session} onLoggedOut={vi.fn()} />
      </Providers>,
    )

    await waitFor(() => {
      const urls = fetchMock.mock.calls.map(([input]) => String(input))
      expect(urls).toContain("/api/v2/preferences")
      expect(urls).toContain("/api/v1/categories")
      expect(urls.some((url) => url.startsWith("/api/v1/subscriptions"))).toBe(true)
      expect(urls.some((url) => url.startsWith("/api/v1/entries"))).toBe(true)
    })
    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "No entries here" })).toBeVisible()
    })

    preferenceResponse.resolve(jsonResponse({
      locale: "en",
      themeMode: "DARK",
      layoutDensity: "SPACIOUS",
      readingFontScale: 120,
      readingFontFamily: "SANS",
      readingCustomFontId: null,
      readingColorScheme: "SEPIA",
      linkOpenMode: "CURRENT_TAB",
    }))
    await waitFor(() => {
      expect(document.documentElement).toHaveAttribute("data-theme", "dark")
      expect(document.documentElement).toHaveAttribute(
        "data-raindrop-density",
        "spacious",
      )
      expect(document.documentElement.style.getPropertyValue(
        "--raindrop-reading-scale",
      )).toBe("120%")
    })
  })

  it("keeps Reader usable when preference loading fails", async () => {
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/unread")
    vi.stubGlobal("fetch", vi.fn((input: RequestInfo | URL) => {
      const url = String(input)
      if (url === "/api/v2/preferences") {
        return Promise.resolve(new Response(JSON.stringify({
          error: {
            code: "INTERNAL_ERROR",
            message: "Preferences unavailable",
            requestId: "00000000-0000-4000-8000-000000000901",
          },
        }), {
          status: 500,
          headers: { "content-type": "application/json" },
        }))
      }
      return Promise.resolve(jsonResponse(responseBody(url)))
    }))

    render(
      <Providers>
        <ReadyPage session={session} onLoggedOut={vi.fn()} />
      </Providers>,
    )

    expect(await screen.findByRole("heading", { name: "No entries here" })).toBeVisible()
  })

  it("replaces StrictMode-aborted initial requests instead of leaving panes busy", async () => {
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/unread")
    vi.stubGlobal("fetch", vi.fn(delayedReaderResponse))

    render(
      <StrictMode>
        <Providers>
          <ReadyPage session={session} onLoggedOut={vi.fn()} />
        </Providers>
      </StrictMode>,
    )

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "No entries here" })).toBeVisible()
    })
  })

  it("replaces a StrictMode-aborted deep-linked detail request", async () => {
    activateLocale("en")
    window.history.replaceState(null, "", `/reader/unread/entry/${entryId}`)
    const fetchMock = vi.fn(delayedReaderResponse)
    vi.stubGlobal("fetch", fetchMock)

    render(
      <StrictMode>
        <Providers>
          <ReadyPage session={session} onLoggedOut={vi.fn()} />
        </Providers>
      </StrictMode>,
    )

    expect(fetchMock.mock.calls.filter(([input]) => String(input).endsWith(entryId))).toHaveLength(2)
    expect(await screen.findByRole("heading", { name: "StrictMode detail" })).toBeVisible()
  })
})

function delayedReaderResponse(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input)
  const body = responseBody(url)
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => resolve(jsonResponse(body)), 20)
    init?.signal?.addEventListener("abort", () => {
      window.clearTimeout(timer)
      reject(new DOMException("Aborted", "AbortError"))
    }, { once: true })
  })
}

function responseBody(url: string): unknown {
  if (url === "/api/v2/preferences") {
    return {
      locale: "en",
      themeMode: "SYSTEM",
      layoutDensity: "BALANCED",
      readingFontScale: 100,
      readingFontFamily: "SERIF",
      readingCustomFontId: null,
      readingColorScheme: "AUTO",
      linkOpenMode: "NEW_TAB",
    }
  }
  if (url === "/api/v2/preferences/fonts") {
    return { items: [], maximumCount: 8, maximumBytes: 5_242_880 }
  }
  if (url === `/api/v1/entries/${entryId}`) {
    return {
      entryId,
      feedId: "00000000-0000-4000-8000-000000000101",
      feedTitle: "Quiet Web",
      siteUrl: "https://quiet.example",
      title: "StrictMode detail",
      author: "Reader",
      summary: "Recovered detail request.",
      canonicalUrl: "https://quiet.example/detail",
      publishedAtUs: 1_700_000_000_000_000,
      sortAtUs: 1_700_000_000_000_000,
      isRead: false,
      isStarred: false,
      contentHtml: "<p>Safe article.</p>",
      inertImages: [],
      enclosures: [],
    }
  }
  if (url === "/api/v1/categories") return { ownerUserId: session.user.id, items: [] }
  if (url.startsWith("/api/v1/subscriptions")) {
    return { ownerUserId: session.user.id, items: [], nextCursor: null }
  }
  return {
    ownerUserId: session.user.id,
    items: [],
    nextCursor: null,
    snapshotGeneration: 1,
  }
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
  })
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

function makeReaderCache(
  snapshot: ReaderCacheSnapshot | null,
  clear: () => Promise<void> = async () => undefined,
): ReaderCache {
  return {
    load: vi.fn(async () => snapshot),
    save: vi.fn(async () => undefined),
    clear: vi.fn(clear),
  }
}

function cachedFeedSnapshot(): ReaderCacheSnapshot {
  const entry = makeEntry({ title: "Cached article" })
  return {
    categories: [makeCategory()],
    subscriptions: [makeSubscription({ title: "Cached feed" })],
    source: { kind: "feed", feedId: entry.feedId },
    entries: [entry],
    queue: [entry.entryId],
    snapshotGeneration: 7,
    scrollAnchorByRoute: {},
  }
}

function pendingResponse(signal?: AbortSignal | null): Promise<Response> {
  return new Promise((_resolve, reject) => {
    signal?.addEventListener(
      "abort",
      () => reject(new DOMException("Aborted", "AbortError")),
      { once: true },
    )
  })
}
