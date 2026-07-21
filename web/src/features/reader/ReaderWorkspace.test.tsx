import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"
import type { ComponentProps } from "react"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { useViewportMode } from "../../shared/responsive/useViewportMode"
import { setTestViewport } from "../../test/setup"
import { fakePreferencesController } from "../preferences/model/testFixtures"
import type { ReaderController } from "./model/useReaderController"
import { initialReaderState } from "./model/reducer"
import type { ReaderState } from "./model/types"
import { ReaderRoutes as ProductionReaderRoutes } from "./routes/ReaderRoutes"
import "./reader.css"

describe("Reader workspace", () => {
  it("shows source, queue, and article panes on a wide canonical route", async () => {
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/unread")

    render(
      <Providers>
        <ReaderRoutes
          controller={fakeController()}
          username="reader"
          onLogout={vi.fn().mockResolvedValue(undefined)}
          viewportMode="wide"
        />
      </Providers>,
    )

    expect(await screen.findByRole("navigation", { name: "Sources" })).toBeVisible()
    expect(screen.getByRole("region", { name: "Entry queue" })).toBeVisible()
    expect(screen.getByRole("complementary", { name: "Article" })).toBeVisible()
  })

  it.each([
    [719, "compact"],
    [720, "medium"],
    [1099, "medium"],
    [1100, "wide"],
  ] as const)("maps %ipx to %s", (width, expected) => {
    act(() => setTestViewport(width))
    const { result } = renderHook(() => useViewportMode())
    expect(result.current).toBe(expected)
  })

  it("navigates to a feed route before synchronizing the selected source", async () => {
    const user = userEvent.setup()
    const controller = fakeController({
      subscriptionsById: {
        subscription: {
          subscriptionId: "subscription",
          feedId: "feed-rust",
          categoryId: null,
          titleOverride: null,
          position: 0,
          title: "Planet Rust",
          feedUrl: "https://planet-rust.example/feed.xml",
          siteUrl: "https://planet-rust.example",
          unreadCount: 7,
          refresh: null,
        },
      },
      subscriptionOrder: ["subscription"],
      paneStatus: { ...initialReaderState.paneStatus, subscriptions: "ready" },
    })
    window.history.replaceState(null, "", "/reader/unread")

    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )

    await user.click(screen.getByText("Planet Rust"))

    expect(window.location.pathname).toBe("/reader/feed/feed-rust")
    await waitFor(() => expect(controller.selectSource).toHaveBeenCalledWith({ kind: "feed", feedId: "feed-rust" }))
  })

  it("selects an entry through its canonical detail route", async () => {
    const user = userEvent.setup()
    const controller = fakeController({
      entriesById: {
        entry: {
          entryId: "entry",
          feedId: "feed",
          feedTitle: "Planet Rust",
          siteUrl: null,
          title: "Borrowing without noise",
          author: "林岚",
          summary: "A practical ownership note.",
          canonicalUrl: "https://example.com/entry",
          publishedAtUs: 1_700_000_000_000_000,
          sortAtUs: 1_700_000_000_000_000,
          isRead: false,
          isStarred: false,
        },
      },
      queueBySourceKey: { "smart:UNREAD": ["entry"] },
      paneStatus: { ...initialReaderState.paneStatus, queue: "ready" },
    })
    window.history.replaceState(null, "", "/reader/unread")

    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )

    await user.click(screen.getByText("Borrowing without noise"))

    expect(window.location.pathname).toBe("/reader/unread/entry/entry")
    await waitFor(() => expect(controller.selectEntry).toHaveBeenCalledWith("entry"))
  })

  it("keeps a queue failure visible and reloads stored entries on request", async () => {
    const user = userEvent.setup()
    const controller = fakeController({
      paneStatus: { ...initialReaderState.paneStatus, queue: "error" },
      errors: { ...initialReaderState.errors, queue: "Stored entries are unavailable." },
    })
    window.history.replaceState(null, "", "/reader/unread")

    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )

    expect(screen.getByRole("alert")).toHaveTextContent("Stored entries are unavailable.")
    await user.click(screen.getByRole("button", { name: "Reload stored entries" }))
    expect(controller.reloadEntries).toHaveBeenCalledOnce()
  })

  it("keeps feed network refresh separate from stored-entry reload", async () => {
    const user = userEvent.setup()
    const controller = fakeController({
      selectedSource: { kind: "feed", feedId: "feed-rust" },
      subscriptionsById: {
        subscription: {
          subscriptionId: "subscription",
          feedId: "feed-rust",
          categoryId: null,
          titleOverride: null,
          position: 0,
          title: "Planet Rust",
          feedUrl: "https://planet-rust.example/feed.xml",
          siteUrl: null,
          unreadCount: 7,
          refresh: null,
        },
      },
      subscriptionOrder: ["subscription"],
      paneStatus: { subscriptions: "ready", queue: "ready", detail: "idle" },
    })
    window.history.replaceState(null, "", "/reader/feed/feed-rust")

    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )

    await user.click(screen.getByRole("button", { name: "Refresh Planet Rust" }))
    await user.click(screen.getByRole("button", { name: "Reload stored entries" }))

    expect(controller.refreshSubscription).toHaveBeenCalledWith("subscription")
    expect(controller.reloadEntries).toHaveBeenCalledOnce()
  })

  it("opens management for the selected feed from the source toolbar", async () => {
    const user = userEvent.setup()
    const controller = fakeController({
      selectedSource: { kind: "feed", feedId: "feed-rust" },
      subscriptionsById: {
        subscription: {
          subscriptionId: "subscription",
          feedId: "feed-rust",
          categoryId: null,
          titleOverride: null,
          position: 0,
          title: "Planet Rust",
          feedUrl: "https://planet-rust.example/feed.xml",
          siteUrl: "https://planet-rust.example",
          unreadCount: 7,
          refresh: null,
        },
      },
      subscriptionOrder: ["subscription"],
      paneStatus: { subscriptions: "ready", queue: "ready", detail: "idle" },
    })
    window.history.replaceState(null, "", "/reader/feed/feed-rust")

    render(
      <Providers>
        <ReaderRoutes
          controller={controller}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="wide"
        />
      </Providers>,
    )

    await user.click(screen.getByRole("button", { name: "Manage subscriptions" }))
    expect(
      screen.getByRole("dialog", { name: "Manage subscriptions" }),
    ).toBeVisible()
    expect(screen.getByRole("link", { name: "https://planet-rust.example/feed.xml" })).toHaveAttribute(
      "href",
      "https://planet-rust.example/feed.xml",
    )
    expect(screen.getByRole("link", { name: "https://planet-rust.example" })).toHaveAttribute(
      "href",
      "https://planet-rust.example",
    )
  })

  it("offers newly discovered entries without reordering the active queue", async () => {
    const user = userEvent.setup()
    const controller = fakeController({
      pendingNewEntryCountBySource: { "smart:UNREAD": 3 },
      paneStatus: { ...initialReaderState.paneStatus, queue: "ready" },
    })
    window.history.replaceState(null, "", "/reader/unread")

    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )

    await user.click(screen.getByRole("button", { name: "Show 3 new entries" }))
    expect(controller.mergePendingEntries).toHaveBeenCalledOnce()
  })

  it("opens ASTRYX source navigation throughout medium mode", async () => {
    const user = userEvent.setup()
    window.history.replaceState(null, "", "/reader/unread")

    render(
      <Providers>
        <ReaderRoutes controller={fakeController()} username="reader" onLogout={vi.fn()} viewportMode="medium" />
      </Providers>,
    )

    await user.click(screen.getByRole("button", { name: "Open sources" }))
    const dialog = screen.getByRole("dialog", { name: "Sources" })
    expect(dialog).toHaveAttribute("open")
    expect(screen.getByRole("tree", { name: "Sources" })).toBeVisible()
    const close = within(dialog).getByRole("button", { name: "Close navigation" })
    expect(getComputedStyle(close).minInlineSize).toBe("44px")
    expect(getComputedStyle(close).minBlockSize).toBe("44px")
    expect(screen.getByRole("toolbar", { name: "Queue actions" }).closest(".reader-compact-navigation")).toBeNull()
  })

  it("redirects unknown ready-state paths to unread", async () => {
    window.history.replaceState(null, "", "/reader/not-a-source")
    render(
      <Providers>
        <ReaderRoutes controller={fakeController()} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))
  })

  it("returns focus to the management trigger after the desktop dialog closes", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    window.history.replaceState(null, "", "/reader/unread")
    render(
      <Providers>
        <ReaderRoutes
          controller={fakeController()}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="wide"
        />
      </Providers>,
    )

    const trigger = await screen.findByRole("button", { name: "Manage subscriptions" })
    await user.click(trigger)
    const dialog = await screen.findByRole("dialog", { name: "Manage subscriptions" })
    await user.click(within(dialog).getByRole("button", { name: "Close" }))
    await waitFor(() => expect(trigger).toHaveFocus())
  })

  it("opens settings from the ASTRYX MoreMenu and restores trigger focus", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    window.history.replaceState(null, "", "/reader/unread")
    render(
      <Providers>
        <ReaderRoutes
          controller={fakeController()}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="wide"
        />
      </Providers>,
    )

    const menuTrigger = await screen.findByRole("button", { name: "Open menu" })
    await user.click(menuTrigger)
    await user.click(
      await screen.findByRole("menuitem", { name: "Settings", hidden: true }),
    )
    const dialog = await screen.findByRole("dialog", { name: "Settings" })
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))

    await waitFor(() => expect(menuTrigger).toHaveFocus())
  })

  it("maps the saved density to ASTRYX source and entry lists", () => {
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/unread")
    const controller = fakeController({
      entriesById: {
        entry: {
          entryId: "entry",
          feedId: "feed",
          feedTitle: "Quiet Web",
          siteUrl: null,
          title: "A spacious queue",
          author: null,
          summary: null,
          canonicalUrl: null,
          publishedAtUs: null,
          sortAtUs: 1_700_000_000_000_000,
          isRead: false,
          isStarred: false,
        },
      },
      queueBySourceKey: { "smart:UNREAD": ["entry"] },
      paneStatus: { ...initialReaderState.paneStatus, queue: "ready" },
    })

    render(
      <Providers>
        <ProductionReaderRoutes
          controller={controller}
          preferencesController={fakePreferencesController({
            preferences: {
              locale: "en",
              themeMode: "SYSTEM",
              layoutDensity: "SPACIOUS",
              readingFontScale: 100,
              readingFontFamily: "SERIF",
              readingCustomFontId: null,
              readingColorScheme: "AUTO",
              linkOpenMode: "NEW_TAB",
            },
          })}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="wide"
        />
      </Providers>,
    )

    expect(screen.getByRole("tree").parentElement).toHaveAttribute(
      "data-density",
      "spacious",
    )
    expect(screen.getByTestId("entry-list")).toHaveAttribute(
      "data-density",
      "spacious",
    )
    expect(document.querySelector("[data-reader-entry-id='entry']")).toHaveAttribute(
      "data-density",
      "spacious",
    )
  })

  it("reopens mobile sources and restores focus after subscription management", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    window.history.replaceState(null, "", "/reader/unread")
    render(
      <Providers>
        <ReaderRoutes
          controller={fakeController()}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="compact"
        />
      </Providers>,
    )

    await user.click(await screen.findByRole("button", { name: "Open sources" }))
    const sources = await screen.findByRole("dialog", { name: "Sources" })
    await user.click(within(sources).getByRole("button", { name: "Manage subscriptions" }))
    const managementDialog = await screen.findByRole("dialog", {
      name: "Manage subscriptions",
    })
    await user.click(within(managementDialog).getByRole("button", { name: "Close" }))

    const reopenedSources = await screen.findByRole("dialog", { name: "Sources" })
    const restoredTrigger = within(reopenedSources).getByRole("button", {
      name: "Manage subscriptions",
    })
    await waitFor(() => expect(restoredTrigger).toHaveFocus())
  })

  it("reopens mobile sources and restores MoreMenu focus after settings", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    window.history.replaceState(null, "", "/reader/unread")
    render(
      <Providers>
        <ReaderRoutes
          controller={fakeController()}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="compact"
        />
      </Providers>,
    )

    await user.click(await screen.findByRole("button", { name: "Open sources" }))
    const sources = await screen.findByRole("dialog", { name: "Sources" })
    await user.click(within(sources).getByRole("button", { name: "Open menu" }))
    await user.click(
      await screen.findByRole("menuitem", { name: "Settings", hidden: true }),
    )
    const preferences = await screen.findByRole("dialog", { name: "Settings" })
    await user.click(within(preferences).getByRole("button", { name: "Cancel" }))

    const reopenedSources = await screen.findByRole("dialog", { name: "Sources" })
    const restoredTrigger = within(reopenedSources).getByRole("button", {
      name: "Open menu",
    })
    await waitFor(() => expect(restoredTrigger).toHaveFocus())
  })

  it("navigates an actively deleted category back to unread", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const categoryId = "00000000-0000-4000-8000-000000000501"
    const controller = fakeController({
      categoriesById: {
        [categoryId]: { categoryId, title: "Technology", position: 1024 },
      },
      categoryOrder: [categoryId],
      selectedSource: { kind: "category", categoryId },
      paneStatus: { ...initialReaderState.paneStatus, queue: "ready" },
    })
    window.history.replaceState(null, "", `/reader/category/${categoryId}`)
    render(
      <Providers>
        <ReaderRoutes
          controller={controller}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="wide"
        />
      </Providers>,
    )

    await user.click(await screen.findByRole("button", { name: "Manage subscriptions" }))
    await user.click(screen.getByRole("button", { name: "Add category" }))
    await user.click(screen.getByRole("button", { name: "Delete category" }))
    const alert = await screen.findByRole("alertdialog", {
      name: "Delete this category?",
    })
    await user.click(within(alert).getByRole("button", { name: "Delete category" }))

    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))
    expect(controller.deleteCategory).toHaveBeenCalledWith(categoryId)
  })

  it.each([
    "/reader/feed/%E0%A4%A",
    "/reader/unread/entry/%E0%A4%A",
  ])("redirects malformed encoded route %s to unread", async (path) => {
    window.history.replaceState(null, "", path)
    render(
      <Providers>
        <ReaderRoutes controller={fakeController()} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))
  })
})

function fakeController(state: Partial<ReaderState> = {}): ReaderController {
  return {
    state: { ...structuredClone(initialReaderState), ...state },
    load: vi.fn().mockResolvedValue(undefined),
    selectSource: vi.fn().mockResolvedValue(undefined),
    selectEntry: vi.fn().mockResolvedValue(undefined),
    reloadEntries: vi.fn().mockResolvedValue(undefined),
    searchFeed: vi.fn().mockResolvedValue(undefined),
    mergePendingEntries: vi.fn(),
    isMarkingRead: false,
    markCurrentSourceRead: vi.fn().mockResolvedValue(true),
    nextUnreadSource: vi.fn().mockResolvedValue(undefined),
    previousUnreadSource: vi.fn().mockResolvedValue(undefined),
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

function ReaderRoutes(
  props: Omit<ComponentProps<typeof ProductionReaderRoutes>, "preferencesController">,
) {
  return (
    <ProductionReaderRoutes
      {...props}
      preferencesController={fakePreferencesController()}
    />
  )
}
