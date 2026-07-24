import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { ComponentProps } from "react"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { fakeAiSettingsController } from "../ai/model/testFixtures"
import { fakePreferencesController } from "../preferences/model/testFixtures"
import { initialReaderState } from "./model/reducer"
import type { ReaderController } from "./model/useReaderController"
import { ReaderRoutes as ProductionReaderRoutes } from "./routes/ReaderRoutes"
import "./reader.css"

describe("Reader article workspace", () => {
  afterEach(() => vi.unstubAllGlobals())

  it("restores sanitized publisher images and preserves a failed image frame", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const controller = articleController()
    window.history.replaceState(null, "", "/reader/unread/entry/entry")

    const view = render(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="compact" />
      </Providers>,
    )

    expect(screen.getByRole("heading", { name: "Reading without trackers" })).toBeVisible()
    expect(screen.getByText("Mara Voss")).toBeVisible()
    expect(document.querySelector(".reader-article-meta time")).toHaveAttribute(
      "datetime",
      "2023-11-14T22:13:20.000Z",
    )
    expect(screen.getByText("Safe original article.")) .toBeVisible()
    const image = document.querySelector<HTMLImageElement>(".reader-article img")!
    const frame = document.querySelector<HTMLElement>(".reader-article-image-frame")!
    expect(image).toHaveAttribute("src", "/reader-assets/entries/entry/images/0")
    expect(image).toHaveAttribute("loading", "lazy")
    expect(image).toHaveAttribute("decoding", "async")
    expect(image).toHaveAttribute("referrerpolicy", "no-referrer")
    expect(document.body).not.toHaveTextContent("publisher.example/tracker.gif")
    fireEvent.load(image)
    expect(image).toHaveAttribute("data-raindrop-image-state", "loaded")
    expect(frame).toHaveAttribute("data-raindrop-image-state", "loaded")
    controller.state = {
      ...controller.state,
      scrollAnchorByRoute: { "/reader/unread/entry/entry": 180 },
    }
    view.rerender(
      <Providers>
        <ReaderRoutes controller={controller} username="reader" onLogout={vi.fn()} viewportMode="compact" />
      </Providers>,
    )
    expect(document.querySelector(".reader-article img")).toBe(image)
    expect(document.querySelector(".reader-article-image-frame")).toBe(frame)
    expect(image).toHaveAttribute("data-raindrop-image-state", "loaded")
    expect(frame).toHaveAttribute("data-raindrop-image-state", "loaded")
    fireEvent.error(image)
    expect(image).not.toHaveAttribute("src")
    expect(image).toHaveAttribute("data-raindrop-image-state", "error")
    expect(image).toHaveAttribute("hidden")
    expect(frame).toHaveAttribute("data-raindrop-image-state", "error")
    expect(frame).toHaveAttribute("role", "img")
    expect(frame).toHaveAttribute("aria-label", "Rain")
    const navigation = screen.getByRole("toolbar", { name: "Article navigation" })
    expect(navigation.closest(".reader-compact-navigation")).toBeInTheDocument()
    expect(screen.queryByRole("button", {
      name: "Reset text size, currently 100%",
    })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", {
      name: "Open article display controls",
    }))
    const resetSize = screen.getByRole("button", {
      name: "Reset text size, currently 100%",
    })
    expect(resetSize).toHaveTextContent("100%")
    expect(resetSize.closest(".reader-reading-float")).toBeInTheDocument()

    const original = screen
      .getAllByRole("link", { name: "Open original article" })
      .find((link) => link.classList.contains("reader-open-original"))!
    expect(getComputedStyle(original).minInlineSize).toBe("44px")
    expect(getComputedStyle(original).minBlockSize).toBe("44px")

    await user.click(screen.getByRole("button", { name: "Mark as read" }))
    await user.click(screen.getByRole("button", { name: "Star entry" }))
    expect(controller.toggleRead).toHaveBeenCalledWith("entry")
    expect(controller.toggleStar).toHaveBeenCalledWith("entry")

    await user.click(screen.getByRole("button", { name: "Back to entry queue" }))
    expect(window.location.pathname).toBe("/reader/unread")
  })

  it("opens the compact reading dock and saves font and article-theme choices", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const save = vi.fn().mockResolvedValue(true)
    const preferencesController = fakePreferencesController({ save })
    window.history.replaceState(null, "", "/reader/unread/entry/entry")

    render(
      <Providers>
        <ProductionReaderRoutes
          controller={articleController()}
          preferencesController={preferencesController}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="compact"
        />
      </Providers>,
    )

    const dock = document.querySelector<HTMLElement>(".reader-reading-dock")!
    expect(dock).not.toHaveAttribute("data-expanded")
    await user.click(screen.getByRole("button", { name: "Open article display controls" }))
    expect(dock).toHaveAttribute("data-expanded", "true")

    const fontTrigger = screen.getByRole("button", { name: "Choose article font" })
    fireEvent.click(fontTrigger)
    expect(fontTrigger).toHaveAttribute("aria-expanded", "true")
    const fontDialog = screen.getByRole("dialog", {
      name: "Choose article font",
      hidden: true,
    })
    fireEvent.click(within(fontDialog).getByRole("button", {
      name: "Sans serif",
      hidden: true,
    }))
    expect(save).toHaveBeenCalledWith({
      ...preferencesController.preferences,
      readingFontFamily: "SANS",
      readingCustomFontId: null,
    })

    fireEvent.click(screen.getByRole("button", { name: "Choose article theme" }))
    const themeDialog = screen.getByRole("dialog", {
      name: "Choose article theme",
      hidden: true,
    })
    fireEvent.click(within(themeDialog).getByRole("button", { name: "Sepia", hidden: true }))
    expect(save).toHaveBeenCalledWith({
      ...preferencesController.preferences,
      readingColorScheme: "SEPIA",
    })
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

  it("keeps the original article node and scroll position while using the AI sidecar", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const controller = articleController()
    const baseAiSettingsController = fakeAiSettingsController()
    const providerId = baseAiSettingsController.providers[0]!.providerId
    const aiSettingsController = fakeAiSettingsController({
      configEnvelope: {
        pluginState: "READY",
        mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
        config: {
          revision: 1,
          isEnabled: true,
          summary: {
            enabled: true,
            providerId,
            style: "BALANCED",
            maxOutputTokens: 1024,
          },
          translation: {
            enabled: true,
            providerId,
            defaultTargetLocale: "zh-CN",
            maxOutputTokens: 4096,
          },
        },
      },
    })
    window.history.replaceState(null, "", "/reader/unread/entry/entry")
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        jsonResponse({
          availability: "READY",
          mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
          summary: {
            operation: "SUMMARIZE",
            state: "SUCCEEDED",
            job: null,
            artifact: {
              artifactId: "00000000-0000-4000-8000-000000000501",
              kind: "AI_SUMMARY",
              providerLabel: "Primary model",
              createdAt: "2026-07-20T10:00:00Z",
              sourceLanguage: "en",
              summary: "A stable summary.",
              bullets: ["Original content stays present."],
              conclusion: null,
            },
          },
          translation: {
            operation: "TRANSLATE",
            targetLocale: "zh-CN",
            state: "SUCCEEDED",
            job: null,
            artifact: {
              artifactId: "00000000-0000-4000-8000-000000000502",
              kind: "AI_TRANSLATION",
              providerLabel: "Primary model",
              createdAt: "2026-07-20T10:00:00Z",
              detectedSourceLanguage: "en",
              targetLocale: "zh-CN",
              title: "不受追踪地阅读",
              bodyMarkdown: "原文仍然保留。",
            },
          },
        }),
      ),
    )

    render(
      <Providers>
        <ReaderRoutes
          controller={controller}
          aiSettingsController={aiSettingsController}
          username="reader"
          onLogout={vi.fn()}
          viewportMode="medium"
        />
      </Providers>,
    )

    const article = document.querySelector<HTMLElement>(".reader-article")!
    const title = screen.getByRole("heading", { name: "Reading without trackers" })
    const body = document.querySelector(".reader-article-body")!
    article.scrollTop = 180

    const summaryTrigger = screen.getByRole("button", { name: "Summary" })
    await user.click(summaryTrigger)
    expect(await screen.findByText("A stable summary.")).toBeVisible()
    expect(document.querySelector(".reader-article")).toBe(article)
    expect(screen.getByRole("heading", { name: "Reading without trackers" })).toBe(title)
    expect(document.querySelector(".reader-article-body")).toBe(body)
    expect(body).toHaveTextContent("Safe original article.")
    expect(article.scrollTop).toBe(180)
    expect(screen.getByRole("button", { name: "Mark as read" })).toHaveAttribute(
      "aria-pressed",
      "false",
    )

    await user.click(screen.getByRole("button", { name: "Close AI sidecar" }))
    expect(screen.queryByText("A stable summary.")).not.toBeInTheDocument()
    expect(document.querySelector(".reader-article")).toBe(article)
    expect(article.scrollTop).toBe(180)
    await waitFor(() => expect(summaryTrigger).toHaveFocus())
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

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
  })
}
