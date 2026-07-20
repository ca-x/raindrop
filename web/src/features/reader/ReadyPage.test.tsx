import { render, screen, waitFor } from "@testing-library/react"
import { StrictMode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import type { SessionResponse } from "../auth/session"
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
    expect(await screen.findByRole("heading", { name: "No entries here" })).toBeVisible()

    preferenceResponse.resolve(jsonResponse({
      locale: "en",
      themeMode: "DARK",
      layoutDensity: "SPACIOUS",
      readingFontScale: 120,
      readingFontFamily: "SANS",
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

    expect(await screen.findByRole("heading", { name: "No entries here" })).toBeVisible()
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
      readingColorScheme: "AUTO",
      linkOpenMode: "NEW_TAB",
    }
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
  if (url === "/api/v1/categories") return { items: [] }
  if (url.startsWith("/api/v1/subscriptions")) {
    return { items: [], nextCursor: null }
  }
  return { items: [], nextCursor: null, snapshotGeneration: 1 }
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
