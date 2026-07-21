import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { EntryQueue, entryPreview } from "./components/EntryQueue"
import { MarkReadDialog } from "./components/MarkReadDialog"
import { initialReaderState } from "./model/reducer"
import type { ReaderState } from "./model/types"

afterEach(() => vi.restoreAllMocks())

it("bounds and normalizes publisher summaries for queue rows", () => {
  expect(entryPreview("  first\n\nsecond  ")).toBe("first second")
  const preview = entryPreview("界".repeat(200))
  expect([...(preview ?? "")]).toHaveLength(180)
  expect(preview?.endsWith("…")).toBe(true)
})

describe("Reader queue actions", () => {
  it("shows Feed search only for Feed sources and submits or clears it", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const searchFeed = vi.fn().mockResolvedValue(undefined)
    const { rerender } = renderQueue(feedState(), { onSearchFeed: searchFeed })

    const search = screen.getByRole("textbox", { name: "Search this feed" })
    await user.type(search, "Rust storage{Enter}")
    expect(searchFeed).toHaveBeenLastCalledWith("Rust storage")

    rerender(queue(feedState({ feedSearchQuery: "Rust storage" }), {
      onSearchFeed: searchFeed,
    }))
    await user.click(screen.getByRole("button", { name: "Clear Search this feed" }))
    expect(searchFeed).toHaveBeenLastCalledWith("")
    expect(screen.getByRole("textbox", { name: "Search this feed" })).toHaveFocus()

    rerender(queue(smartState("UNREAD")))
    expect(screen.queryByRole("textbox", { name: "Search this feed" })).not.toBeInTheDocument()
  })

  it("rejects Feed searches beyond 128 UTF-8 bytes before requesting", async () => {
    activateLocale("en")
    const searchFeed = vi.fn().mockResolvedValue(undefined)
    renderQueue(feedState(), { onSearchFeed: searchFeed })

    const search = screen.getByRole("textbox", { name: "Search this feed" })
    fireEvent.change(search, { target: { value: "界".repeat(43) } })
    fireEvent.keyDown(search, { key: "Enter" })

    expect(await screen.findByText("Keep the search within 128 UTF-8 bytes.")).toBeVisible()
    expect(search).toHaveAttribute("aria-invalid", "true")
    expect(searchFeed).not.toHaveBeenCalled()
  })

  it("hides bulk mark-read for Starred and disables it for Feed search", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const { rerender } = renderQueue(smartState("STARRED"))

    await user.click(screen.getByRole("button", { name: "Queue menu" }))
    expect(screen.queryByRole("menuitem", { name: "Mark current source read", hidden: true }))
      .not.toBeInTheDocument()
    expect(screen.queryByRole("menuitem", { name: "Mark all is unavailable in this view", hidden: true }))
      .not.toBeInTheDocument()

    rerender(queue(feedState({ feedSearchQuery: "database" })))
    await user.click(screen.getByRole("button", { name: "Queue menu" }))
    expect(screen.getByRole("menuitem", {
      name: "Mark all is unavailable in this view",
      hidden: true,
    })).toHaveAttribute("aria-disabled", "true")
  })

  it("keeps unread navigation and mark-read available from the compact menu", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const nextUnread = vi.fn().mockResolvedValue(undefined)
    const previousUnread = vi.fn().mockResolvedValue(undefined)
    const requestMarkRead = vi.fn()
    renderQueue(smartState("UNREAD"), {
      showMenu: true,
      isCompact: true,
      onNextUnreadSource: nextUnread,
      onPreviousUnreadSource: previousUnread,
      onRequestMarkRead: requestMarkRead,
    })

    const toolbar = screen.getByRole("toolbar", { name: "Queue actions" })
    expect(toolbar.closest(".reader-compact-navigation")).not.toBeNull()

    await user.click(screen.getByRole("button", { name: "Queue menu" }))
    await user.click(screen.getByRole("menuitem", { name: "Next unread source", hidden: true }))
    expect(nextUnread).toHaveBeenCalledOnce()

    await user.click(screen.getByRole("button", { name: "Queue menu" }))
    await user.click(screen.getByRole("menuitem", { name: "Previous unread source", hidden: true }))
    expect(previousUnread).toHaveBeenCalledOnce()

    await user.click(screen.getByRole("button", { name: "Queue menu" }))
    await user.click(screen.getByRole("menuitem", { name: "Mark current source read", hidden: true }))
    expect(requestMarkRead).toHaveBeenCalledOnce()
  })
})

describe("Mark read confirmation", () => {
  it("describes the stable snapshot, focuses cancel, and exposes loading", async () => {
    activateLocale("en")
    vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) {
      this.setAttribute("open", "")
      this.querySelector<HTMLButtonElement>("button")?.focus()
    })
    const onOpenChange = vi.fn()
    const onConfirm = vi.fn().mockResolvedValue(true)
    const { rerender } = render(
      dialog({ isLoading: true, onOpenChange, onConfirm }),
    )

    expect(screen.getByRole("alertdialog", { name: "Mark “Planet Rust” read?" }))
      .toHaveTextContent("Entries received after the list loaded will stay unread.")
    await waitFor(() => expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus())
    expect(screen.getByRole("button", { name: "Mark all read" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "Mark all read" })).toHaveAttribute("aria-busy", "true")

    rerender(dialog({ isLoading: false, onOpenChange, onConfirm }))
    await userEvent.click(screen.getByRole("button", { name: "Mark all read" }))
    expect(onConfirm).toHaveBeenCalledOnce()
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })
})

function renderQueue(
  state: ReaderState,
  overrides: Partial<QueueProps> = {},
) {
  return render(queue(state, overrides))
}

type QueueProps = React.ComponentProps<typeof EntryQueue>

function queue(state: ReaderState, overrides: Partial<QueueProps> = {}) {
  return (
    <Providers>
      <EntryQueue
        state={state}
        showMenu={false}
        isCompact={false}
        onOpenSources={vi.fn()}
        onSelect={vi.fn()}
        isRouteReady
        cursorEntryId={null}
        cursorFocusNonce={0}
        sourceRoute="/reader/unread"
        savedScrollOffset={0}
        onRecordScroll={vi.fn()}
        onReload={vi.fn().mockResolvedValue(undefined)}
        onSearchFeed={vi.fn().mockResolvedValue(undefined)}
        onNextUnreadSource={vi.fn().mockResolvedValue(undefined)}
        onPreviousUnreadSource={vi.fn().mockResolvedValue(undefined)}
        onRequestMarkRead={vi.fn()}
        isMarkingRead={false}
        onMergePending={vi.fn()}
        onMergedEntryFocus={vi.fn()}
        density="balanced"
        {...overrides}
      />
    </Providers>
  )
}

function feedState(overrides: Partial<ReaderState> = {}): ReaderState {
  return {
    ...structuredClone(initialReaderState),
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
    snapshotGenerationBySource: { "feed:feed-rust": 42 },
    paneStatus: { subscriptions: "ready", queue: "ready", detail: "idle" },
    ...overrides,
  }
}

function smartState(state: "UNREAD" | "STARRED"): ReaderState {
  const key = `smart:${state}` as const
  return {
    ...structuredClone(initialReaderState),
    selectedSource: { kind: "smart", state },
    snapshotGenerationBySource: { [key]: 42 },
    paneStatus: { subscriptions: "ready", queue: "ready", detail: "idle" },
  }
}

interface DialogOverrides {
  isLoading: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: () => Promise<boolean>
}

function dialog(overrides: DialogOverrides) {
  return (
    <Providers>
      <MarkReadDialog
        isOpen
        sourceLabel="Planet Rust"
        {...overrides}
      />
    </Providers>
  )
}
