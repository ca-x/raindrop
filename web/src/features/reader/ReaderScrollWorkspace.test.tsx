import { fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { Providers } from "../../app/Providers"
import { ArticleReader } from "./components/ArticleReader"
import { EntryQueue } from "./components/EntryQueue"
import { initialReaderState } from "./model/reducer"
import type { ReaderState } from "./model/types"
import "./reader.css"

describe("Reader scroll anchors", () => {
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(1_000)
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(200)
  })
  afterEach(() => vi.restoreAllMocks())

  it("restores, clamps, records, and focuses the route-keyed queue anchor", () => {
    const record = vi.fn()
    render(
      <Providers>
        <EntryQueue
          state={readerState("first")}
          showMenu={false}
          isCompact={false}
          onOpenSources={vi.fn()}
          onSelect={vi.fn()}
          cursorEntryId="second"
          cursorFocusNonce={1}
          sourceRoute="/reader/unread"
          savedScrollOffset={1_400}
          onRecordScroll={record}
          onReload={vi.fn().mockResolvedValue(undefined)}
          onMergePending={vi.fn()}
        />
      </Providers>,
    )

    const scroller = screen.getByTestId("entry-queue-scroll")
    expect(scroller.scrollTop).toBe(800)
    expect(screen.getByText("Second article").closest("li")?.querySelector("button")).toHaveFocus()
    scroller.scrollTop = 360
    fireEvent.scroll(scroller)
    expect(record).toHaveBeenLastCalledWith("/reader/unread", 360)
  })

  it("keeps article offsets separate by entry route and starts new content at top", () => {
    const anchors: Record<string, number> = { "/reader/unread/entry/first": 320 }
    const record = vi.fn((route: string, offset: number) => { anchors[route] = offset })
    const { rerender } = render(article("first", anchors, record))
    const first = screen.getByRole("article")
    expect(first.scrollTop).toBe(320)
    first.scrollTop = 540
    fireEvent.scroll(first)

    rerender(article("second", anchors, record))
    const second = screen.getByRole("article")
    expect(second.scrollTop).toBe(0)
    second.scrollTop = 210
    fireEvent.scroll(second)

    rerender(article("first", anchors, record))
    expect(screen.getByRole("article").scrollTop).toBe(540)
    expect(record).toHaveBeenCalledWith("/reader/unread/entry/second", 210)
  })
})

function article(entryId: "first" | "second", anchors: Record<string, number>, record: (route: string, offset: number) => void) {
  const route = `/reader/unread/entry/${entryId}`
  return (
    <Providers>
      <ArticleReader
        state={readerState(entryId)}
        entryRoute={route}
        savedScrollOffset={anchors[route] ?? 0}
        shouldFocusArticle
        onRecordScroll={record}
        onToggleRead={vi.fn().mockResolvedValue(undefined)}
        onToggleStar={vi.fn().mockResolvedValue(undefined)}
      />
    </Providers>
  )
}

function readerState(selectedEntryId: "first" | "second"): ReaderState {
  const entries = Object.fromEntries(["first", "second"].map((entryId, index) => [
    entryId,
    {
      entryId,
      feedId: "feed",
      feedTitle: "Quiet Web",
      siteUrl: null,
      title: `${entryId === "first" ? "First" : "Second"} article`,
      author: null,
      summary: `Summary ${index}`,
      canonicalUrl: null,
      publishedAtUs: 1_700_000_000_000_000 + index,
      sortAtUs: 1_700_000_000_000_000 + index,
      isRead: false,
      isStarred: false,
    },
  ]))
  const details = Object.fromEntries(Object.values(entries).map((entry) => [
    entry.entryId,
    { ...entry, contentHtml: `<p>${entry.title}</p>`, inertImages: [], enclosures: [] },
  ]))
  return {
    ...structuredClone(initialReaderState),
    selectedEntryId,
    entriesById: entries,
    detailsById: details,
    queueBySourceKey: { "smart:UNREAD": ["first", "second"] },
    paneStatus: { subscriptions: "ready", queue: "ready", detail: "ready" },
  }
}
