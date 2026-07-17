import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { initialReaderState } from "./model/reducer"
import type { ReaderController } from "./model/useReaderController"
import { ReaderRoutes } from "./routes/ReaderRoutes"

describe("Reader article workspace", () => {
  it("renders compact detail actions and keeps inert publisher image URLs disabled", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const controller = articleController()
    window.history.replaceState(null, "", "/reader/unread/entry/entry")

    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="compact" />
      </Providers>,
    )

    expect(screen.getByRole("heading", { name: "Reading without trackers" })).toBeVisible()
    expect(screen.getByText("Safe original article.")) .toBeVisible()
    expect(document.querySelector(".reader-article img")).not.toHaveAttribute("src")
    expect(document.body).not.toHaveTextContent("publisher.example/tracker.gif")

    await user.click(screen.getByRole("button", { name: "Mark as read" }))
    await user.click(screen.getByRole("button", { name: "Star entry" }))
    expect(controller.toggleRead).toHaveBeenCalledWith("entry")
    expect(controller.toggleStar).toHaveBeenCalledWith("entry")

    await user.click(screen.getByRole("button", { name: "Back to entry queue" }))
    expect(window.location.pathname).toBe("/reader/unread")
  })

  it("moves compact mode from queue route to detail route", async () => {
    const user = userEvent.setup()
    const controller = articleController()
    window.history.replaceState(null, "", "/reader/unread")
    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="compact" />
      </Providers>,
    )

    expect(screen.getByRole("region", { name: "Entry queue" })).toBeVisible()
    await user.click(screen.getByText("Reading without trackers"))
    expect(window.location.pathname).toBe("/reader/unread/entry/entry")
    expect(screen.getByRole("region", { name: "Article" })).toBeVisible()
    expect(screen.queryByRole("region", { name: "Entry queue" })).not.toBeInTheDocument()
  })
})

function articleController(): ReaderController {
  return {
    state: {
      ...structuredClone(initialReaderState),
      selectedEntryId: "entry",
      entriesById: {
        entry: {
          entryId: "entry",
          feedId: "feed",
          feedTitle: "Quiet Web",
          siteUrl: "https://quiet.example",
          title: "Reading without trackers",
          author: "Mara Voss",
          summary: "A safer reading path.",
          canonicalUrl: "https://quiet.example/reading",
          publishedAtUs: 1_700_000_000_000_000,
          sortAtUs: 1_700_000_000_000_000,
          isRead: false,
          isStarred: false,
        },
      },
      queueBySourceKey: { "smart:UNREAD": ["entry"] },
      paneStatus: { subscriptions: "ready", queue: "ready", detail: "ready" },
      detailsById: {
        entry: {
          entryId: "entry",
          feedId: "feed",
          feedTitle: "Quiet Web",
          siteUrl: "https://quiet.example",
          title: "Reading without trackers",
          author: "Mara Voss",
          summary: "A safer reading path.",
          canonicalUrl: "https://quiet.example/reading",
          publishedAtUs: 1_700_000_000_000_000,
          sortAtUs: 1_700_000_000_000_000,
          isRead: false,
          isStarred: false,
          contentHtml: '<p>Safe original article.</p><img data-raindrop-inert-image="0" alt="Rain">',
          inertImages: [{
            imageIndex: 0,
            sourceUrl: "https://publisher.example/tracker.gif",
            alt: "Rain",
            width: null,
            height: null,
          }],
          enclosures: [],
        },
      },
    },
    load: vi.fn().mockResolvedValue(undefined),
    selectSource: vi.fn().mockResolvedValue(undefined),
    selectEntry: vi.fn().mockResolvedValue(undefined),
    reloadEntries: vi.fn().mockResolvedValue(undefined),
    mergePendingEntries: vi.fn(),
    toggleRead: vi.fn().mockResolvedValue(undefined),
    toggleStar: vi.fn().mockResolvedValue(undefined),
    addSubscription: vi.fn().mockResolvedValue(undefined),
    deleteSubscription: vi.fn().mockResolvedValue(undefined),
    refreshSubscription: vi.fn().mockResolvedValue(undefined),
    recordScrollAnchor: vi.fn(),
    clearMutationError: vi.fn(),
  }
}
