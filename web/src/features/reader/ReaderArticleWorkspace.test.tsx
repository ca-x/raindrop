import { act, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { initialReaderState } from "./model/reducer"
import type { ReaderController } from "./model/useReaderController"
import { ReaderRoutes } from "./routes/ReaderRoutes"
import "./reader.css"

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
    const navigation = screen.getByRole("toolbar", { name: "Article navigation" })
    expect(navigation.closest(".reader-compact-navigation")).toBeInTheDocument()

    const original = screen.getByRole("link", { name: "Open original article" })
    expect(getComputedStyle(original).minInlineSize).toBe("44px")
    expect(getComputedStyle(original).minBlockSize).toBe("44px")

    await user.click(screen.getByRole("button", { name: "Mark as read" }))
    await user.click(screen.getByRole("button", { name: "Star entry" }))
    expect(controller.toggleRead).toHaveBeenCalledWith("entry")
    expect(controller.toggleStar).toHaveBeenCalledWith("entry")

    await user.click(screen.getByRole("button", { name: "Back to entry queue" }))
    expect(window.location.pathname).toBe("/reader/unread")
  })

  it.each(["loading", "error"] as const)(
    "keeps compact source and Back navigation available while detail is %s",
    async (status) => {
      const user = userEvent.setup()
      const controller = articleController()
      controller.state.paneStatus.detail = status
      controller.state.errors.detail = status === "error" ? "Detail unavailable." : null
      window.history.replaceState(null, "", "/reader/unread/entry/entry")

      render(
        <Providers>
          <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="compact" />
        </Providers>,
      )

      expect(screen.getByRole("button", { name: "Back to entry queue" })).toBeVisible()
      await user.click(screen.getByRole("button", { name: "Open sources" }))
      expect(screen.getByRole("dialog", { name: "Sources" })).toHaveAttribute("open")
      expect(within(screen.getByRole("dialog", { name: "Sources" })).getByRole("tree", { name: "Sources" })).toBeVisible()
    },
  )

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
    expect(screen.getByRole("toolbar", { name: "Queue actions" }).closest(".reader-compact-navigation")).toBeInTheDocument()
    await user.click(screen.getByText("Reading without trackers"))
    expect(window.location.pathname).toBe("/reader/unread/entry/entry")
    expect(screen.getByRole("region", { name: "Article" })).toBeVisible()
    expect(screen.queryByRole("region", { name: "Entry queue" })).not.toBeInTheDocument()
  })

  it("uses history Back for an internally opened detail without reopening it", async () => {
    const user = userEvent.setup()
    window.history.replaceState(null, "", "/reader/all")
    window.history.pushState(null, "", "/reader/unread")
    render(
      <Providers>
        <ReaderRoutes controller={articleController()} username="reader" onLogout={vi.fn()} viewportMode="compact" />
      </Providers>,
    )

    await user.click(screen.getByText("Reading without trackers"))
    await user.click(screen.getByRole("button", { name: "Back to entry queue" }))
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))

    act(() => window.history.back())
    await waitFor(() => expect(window.location.pathname).toBe("/reader/all"))
  })

  it("replaces a direct-linked detail with its queue before browser Back", async () => {
    const user = userEvent.setup()
    window.history.replaceState(null, "", "/reader/all")
    window.history.pushState(null, "", "/reader/unread/entry/entry")
    render(
      <Providers>
        <ReaderRoutes controller={articleController()} username="reader" onLogout={vi.fn()} viewportMode="compact" />
      </Providers>,
    )

    await user.click(screen.getByRole("button", { name: "Back to entry queue" }))
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))

    act(() => window.history.back())
    await waitFor(() => expect(window.location.pathname).toBe("/reader/all"))
  })

  it("replaces A with B while preserving the genuine queue history origin", async () => {
    const user = userEvent.setup()
    const controller = twoArticleController()
    window.history.replaceState(null, "", "/reader/all")
    window.history.pushState(null, "", "/reader/unread")
    const { rerender } = render(readerWorkspace(controller, "medium"))

    const queue = screen.getByRole("region", { name: "Entry queue" })
    await user.click(within(queue).getByText("Reading without trackers"))
    await user.click(within(queue).getByText("Second quiet article"))
    rerender(readerWorkspace(controller, "compact"))
    await user.click(screen.getByRole("button", { name: "Back to entry queue" }))

    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))
  })

  it("keeps direct-link A to B markerless for compact fallback Back", async () => {
    const user = userEvent.setup()
    const controller = twoArticleController()
    window.history.replaceState(null, "", "/reader/all")
    window.history.pushState(null, "", "/reader/unread/entry/entry")
    const { rerender } = render(readerWorkspace(controller, "medium"))

    await user.click(within(screen.getByRole("region", { name: "Entry queue" })).getByText("Second quiet article"))
    rerender(readerWorkspace(controller, "compact"))
    await user.click(screen.getByRole("button", { name: "Back to entry queue" }))
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))

    act(() => window.history.back())
    await waitFor(() => expect(window.location.pathname).toBe("/reader/all"))
  })
})

function readerWorkspace(controller: ReaderController, viewportMode: "compact" | "medium") {
  return (
    <Providers>
      <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode={viewportMode} />
    </Providers>
  )
}

function twoArticleController(): ReaderController {
  const controller = articleController()
  const entry = controller.state.entriesById.entry
  const detail = controller.state.detailsById.entry
  controller.state.entriesById.second = { ...entry, entryId: "second", title: "Second quiet article" }
  controller.state.detailsById.second = { ...detail, entryId: "second", title: "Second quiet article" }
  controller.state.queueBySourceKey["smart:UNREAD"] = ["entry", "second"]
  return controller
}

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
    createCategory: vi.fn().mockResolvedValue(true),
    updateCategory: vi.fn().mockResolvedValue(true),
    deleteCategory: vi.fn().mockResolvedValue(true),
    updateSubscription: vi.fn().mockResolvedValue(true),
    recordScrollAnchor: vi.fn(),
    clearMutationError: vi.fn(),
  }
}
