import { render, screen, within } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"
import type { ComponentProps } from "react"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { fakePreferencesController } from "../preferences/model/testFixtures"
import { initialReaderState } from "./model/reducer"
import type { ReaderController } from "./model/useReaderController"
import { ReaderRoutes as ProductionReaderRoutes } from "./routes/ReaderRoutes"

describe("Reader workspace pane states", () => {
  it("keeps subscription and article failures visible while the queue is busy", () => {
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/unread/entry/entry")
    const controller = fakeController()

    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )

    expect(screen.getByText("Subscriptions are unavailable.")).toBeVisible()
    expect(screen.getByText("Article detail is unavailable.")).toBeVisible()
    const queue = screen.getByRole("region", { name: "Entry queue" })
    expect(within(queue).getByRole("status", { name: "Loading entries" })).toBeVisible()
    expect(queue).toHaveAttribute("aria-busy", "true")
  })

  it("announces transient mutation failures in the root toast viewport", async () => {
    const controller = fakeController()
    controller.state = {
      ...controller.state,
      paneStatus: { subscriptions: "ready", queue: "ready", detail: "idle" },
      selectedEntryId: null,
      errors: { ...controller.state.errors, mutation: "The entry could not be updated." },
    }
    window.history.replaceState(null, "", "/reader/unread")
    render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="wide" />
      </Providers>,
    )

    await vi.waitFor(() => expect(controller.clearMutationError).toHaveBeenCalledOnce())
    const dismiss = await screen.findByRole("button", {
      name: "Dismiss notification",
      hidden: true,
    })
    expect(dismiss.closest('[role="alert"]')).toHaveTextContent("The entry could not be updated.")
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

function fakeController(): ReaderController {
  return {
    state: {
      ...structuredClone(initialReaderState),
      selectedEntryId: "entry",
      paneStatus: { subscriptions: "error", queue: "loading", detail: "error" },
      errors: {
        subscriptions: "Subscriptions are unavailable.",
        queue: null,
        detail: "Article detail is unavailable.",
        mutation: null,
      },
    },
    load: vi.fn().mockResolvedValue(undefined),
    selectSource: vi.fn().mockResolvedValue(undefined),
    selectEntry: vi.fn().mockResolvedValue(undefined),
    reloadEntries: vi.fn().mockResolvedValue(undefined),
    searchFeed: vi.fn().mockResolvedValue(undefined),
    mergePendingEntries: vi.fn(),
    isMarkingRead: false,
    markCurrentSourceRead: vi.fn().mockResolvedValue(true),
    markFeedRead: vi.fn().mockResolvedValue(true),
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
