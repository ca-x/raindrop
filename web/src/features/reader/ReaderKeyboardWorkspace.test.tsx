import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"
import type { ComponentProps } from "react"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { fakePreferencesController } from "../preferences/model/testFixtures"
import { initialReaderState } from "./model/reducer"
import type { ReaderController } from "./model/useReaderController"
import { ReaderRoutes as ProductionReaderRoutes } from "./routes/ReaderRoutes"
import "./reader.css"

describe("Reader keyboard workspace", () => {
  it("keeps N/P cursor movement separate from the open article", async () => {
    activateLocale("en")
    const controller = keyboardController()
    window.history.replaceState(null, "", "/reader/unread/entry/first")
    render(workspace(controller))
    await screen.findByRole("heading", { name: "First article" })
    vi.clearAllMocks()

    fireEvent.keyDown(window, { key: "n" })

    const second = queueRow("Second article")
    expect(second).toHaveAttribute("aria-selected", "true")
    expect(second.querySelector("button")).toHaveFocus()
    expect(window.location.pathname).toBe("/reader/unread/entry/first")
    expect(controller.selectEntry).not.toHaveBeenCalled()
    expect(controller.toggleRead).not.toHaveBeenCalled()
  })

  it("opens J/K targets immediately and replaces same-source detail history", async () => {
    const controller = keyboardController()
    window.history.replaceState(null, "", "/reader/unread")
    window.history.pushState({ readerQueuePath: "/reader/unread" }, "", "/reader/unread/entry/first")
    render(workspace(controller))
    await screen.findByRole("heading", { name: "First article" })

    fireEvent.keyDown(window, { key: "j" })
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread/entry/second"))
    expect(controller.toggleRead).toHaveBeenCalledWith("second")

    fireEvent.keyDown(window, { key: "j" })
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread/entry/third"))
    expect(controller.toggleRead).toHaveBeenCalledWith("third")

    act(() => window.history.back())
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))
  })

  it("toggles the cursor target once and exposes visible ASTRYX hints", async () => {
    const controller = keyboardController()
    window.history.replaceState(null, "", "/reader/unread/entry/first")
    render(workspace(controller))
    await screen.findByRole("heading", { name: "First article" })
    fireEvent.keyDown(window, { key: "n" })

    fireEvent.keyDown(window, { key: "m" })
    fireEvent.keyDown(window, { key: "s" })
    fireEvent.keyDown(window, { key: "m", repeat: true })
    fireEvent.keyDown(window, { key: "s", repeat: true })

    expect(controller.toggleRead).toHaveBeenCalledOnce()
    expect(controller.toggleRead).toHaveBeenCalledWith("second")
    expect(controller.toggleStar).toHaveBeenCalledOnce()
    expect(controller.toggleStar).toHaveBeenCalledWith("second")
    for (const key of ["J", "K", "N", "P", "M", "S"]) {
      expect(screen.getByRole("img", { name: key })).toBeVisible()
    }
    for (const label of ["Open", "Cursor", "Read state", "Star state"]) {
      expect(screen.getByText(label)).toBeVisible()
    }
  })

  it("restores the originating queue row focus after compact Back", async () => {
    const controller = keyboardController()
    window.history.replaceState(null, "", "/reader/unread")
    render(workspace(controller, "compact"))

    fireEvent.click(screen.getByText("Second article"))
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread/entry/second"))
    fireEvent.click(screen.getByRole("button", { name: "Back to entry queue" }))
    await waitFor(() => expect(window.location.pathname).toBe("/reader/unread"))

    await waitFor(() => expect(queueRow("Second article").querySelector("button")).toHaveFocus())
  })

  it("does not use stale queue entries while a new source route is settling", async () => {
    const controller = keyboardController()
    window.history.replaceState(null, "", "/reader/unread/entry/first")
    render(workspace(controller))
    await screen.findByRole("heading", { name: "First article" })

    act(() => {
      window.history.pushState(null, "", "/reader/feed/feed-b")
      window.dispatchEvent(new PopStateEvent("popstate"))
    })
    await waitFor(() => expect(controller.selectSource).toHaveBeenCalledWith({ kind: "feed", feedId: "feed-b" }))
    vi.clearAllMocks()

    for (const key of ["j", "k", "m", "s"]) {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })
      window.dispatchEvent(event)
      expect(event.defaultPrevented).toBe(false)
    }
    const scroller = screen.getByTestId("entry-queue-scroll")
    scroller.scrollTop = 120
    fireEvent.scroll(scroller)

    expect(window.location.pathname).toBe("/reader/feed/feed-b")
    expect(controller.selectEntry).not.toHaveBeenCalled()
    expect(controller.toggleRead).not.toHaveBeenCalled()
    expect(controller.toggleStar).not.toHaveBeenCalled()
    expect(controller.recordScrollAnchor).not.toHaveBeenCalled()
  })

  it("merges pending entries at the top without replacing the open article", async () => {
    const controller = keyboardController()
    const pending = { ...controller.state.entriesById.second, entryId: "pending", title: "Pending article" }
    controller.state.entriesById.pending = pending
    controller.state.pendingNewEntriesBySource["smart:UNREAD"] = ["pending"]
    controller.state.pendingNewEntryCountBySource["smart:UNREAD"] = 1
    vi.mocked(controller.mergePendingEntries).mockImplementation(() => {
      controller.state.queueBySourceKey["smart:UNREAD"] = ["pending", "first", "second", "third"]
      controller.state.pendingNewEntriesBySource["smart:UNREAD"] = []
      controller.state.pendingNewEntryCountBySource["smart:UNREAD"] = 0
    })
    window.history.replaceState(null, "", "/reader/unread/entry/first")
    render(workspace(controller))
    await screen.findByRole("heading", { name: "First article" })
    const scroller = screen.getByTestId("entry-queue-scroll")
    scroller.scrollTop = 260
    fireEvent.scroll(scroller)
    vi.clearAllMocks()

    fireEvent.click(screen.getByRole("button", { name: "Show 1 new entries" }))

    const pendingRow = await screen.findByText("Pending article")
    expect(window.location.pathname).toBe("/reader/unread/entry/first")
    expect(screen.getByRole("heading", { name: "First article" })).toBeVisible()
    expect(scroller.scrollTop).toBe(0)
    expect(pendingRow.closest("li")).toHaveAttribute("aria-selected", "true")
    expect(pendingRow.closest("li")?.querySelector("button")).toHaveFocus()
    expect(controller.recordScrollAnchor).toHaveBeenCalledWith("/reader/unread", 0)
    expect(controller.selectEntry).not.toHaveBeenCalled()
  })
})

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

function workspace(controller: ReaderController, viewportMode: "wide" | "compact" = "wide") {
  return (
    <Providers>
      <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode={viewportMode} />
    </Providers>
  )
}

function queueRow(name: string) {
  return within(screen.getByRole("region", { name: "Entry queue" }))
    .getByText(name)
    .closest("li") as HTMLElement
}

function keyboardController(): ReaderController {
  const entries = Object.fromEntries(["first", "second", "third"].map((entryId, index) => [
    entryId,
    {
      entryId,
      feedId: "feed",
      feedTitle: "Quiet Web",
      siteUrl: null,
      title: `${entryId[0].toUpperCase()}${entryId.slice(1)} article`,
      author: null,
      summary: `Summary ${index + 1}`,
      canonicalUrl: `https://quiet.example/${entryId}`,
      publishedAtUs: 1_700_000_000_000_000 + index,
      sortAtUs: 1_700_000_000_000_000 + index,
      isRead: entryId === "first",
      isStarred: false,
    },
  ]))
  const details = Object.fromEntries(Object.values(entries).map((entry) => [
    entry.entryId,
    { ...entry, contentHtml: `<p>${entry.title}</p>`, inertImages: [], enclosures: [] },
  ]))
  return {
    state: {
      ...structuredClone(initialReaderState),
      selectedEntryId: "first",
      entriesById: entries,
      detailsById: details,
      queueBySourceKey: { "smart:UNREAD": ["first", "second", "third"] },
      paneStatus: { subscriptions: "ready", queue: "ready", detail: "ready" },
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
