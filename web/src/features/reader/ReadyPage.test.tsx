import { render, screen } from "@testing-library/react"
import { StrictMode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

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
  afterEach(() => vi.unstubAllGlobals())

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
  return url.startsWith("/api/v1/subscriptions")
    ? { items: [], nextCursor: null }
    : { items: [], nextCursor: null, snapshotGeneration: 1 }
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
  })
}
