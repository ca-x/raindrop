import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Item } from "@astryxdesign/core/Item"
import { List } from "@astryxdesign/core/List"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { StatusDot } from "@astryxdesign/core/StatusDot"
import { useLingui } from "@lingui/react"
import { useEffect, useLayoutEffect, useRef } from "react"

import { sourceKey, type ReaderState } from "../model/types"
import { QueueToolbar } from "./ReaderToolbar"

interface EntryQueueProps {
  state: ReaderState
  showMenu: boolean
  isCompact: boolean
  onOpenSources: () => void
  onSelect: (entryId: string) => void
  cursorEntryId: string | null
  cursorFocusNonce: number
  sourceRoute: string
  savedScrollOffset: number
  onRecordScroll: (route: string, offset: number) => void
  onReload: () => Promise<void>
  onMergePending: () => void
}

export function EntryQueue({
  state,
  showMenu,
  isCompact,
  onOpenSources,
  onSelect,
  cursorEntryId,
  cursorFocusNonce,
  sourceRoute,
  savedScrollOffset,
  onRecordScroll,
  onReload,
  onMergePending,
}: EntryQueueProps) {
  const { i18n } = useLingui()
  const rootRef = useRef<HTMLDivElement>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const key = sourceKey(state.selectedSource)
  const queue = state.queueBySourceKey[key] ?? []
  const pendingCount = state.pendingNewEntryCountBySource[key] ?? 0
  useLayoutEffect(() => {
    const node = scrollRef.current
    if (!node || state.paneStatus.queue !== "ready") return
    node.scrollTop = clampOffset(node, savedScrollOffset)
    return () => onRecordScroll(sourceRoute, node.scrollTop)
  }, [sourceRoute, state.paneStatus.queue])
  useEffect(() => {
    if (!cursorEntryId || cursorFocusNonce === 0) return
    const row = [...(rootRef.current?.querySelectorAll<HTMLElement>("[data-reader-entry-id]") ?? [])]
      .find((item) => item.dataset.readerEntryId === cursorEntryId)
    const button = row?.querySelector<HTMLButtonElement>("button")
    button?.focus({ preventScroll: true })
    row?.scrollIntoView?.({ behavior: "auto", block: "nearest" })
  }, [cursorEntryId, cursorFocusNonce])
  return (
    <div ref={rootRef} className="reader-queue" aria-busy={state.paneStatus.queue === "loading"}>
      <QueueToolbar showMenu={showMenu} isCompact={isCompact} onOpenSources={onOpenSources} onReload={onReload} />
      {pendingCount > 0 ? (
        <Banner
          container="section"
          status="info"
          title={i18n._("reader.newEntriesAvailable", { count: pendingCount })}
          endContent={
            <Button
              label={i18n._("reader.showNewEntries", { count: pendingCount })}
              onClick={onMergePending}
              variant="ghost"
            />
          }
        />
      ) : null}
      {state.paneStatus.queue === "error" ? (
        <Banner
          container="section"
          status="error"
          title={i18n._("reader.queueError")}
          description={state.errors.queue ?? i18n._("reader.genericError")}
        />
      ) : state.paneStatus.queue === "loading" ? (
        <div className="reader-skeletons" role="status" aria-label={i18n._("reader.loadingEntries")}>
          {[0, 1, 2, 3].map((index) => <Skeleton key={index} height={72} radius={2} index={index} />)}
        </div>
      ) : queue.length === 0 ? (
        <EmptyState
          isCompact
          title={i18n._("reader.noEntries")}
          description={i18n._("reader.noEntriesDescription")}
        />
      ) : (
        <div
          ref={scrollRef}
          className="reader-queue-scroll"
          data-testid="entry-queue-scroll"
          onScroll={(event) => onRecordScroll(sourceRoute, event.currentTarget.scrollTop)}
        >
          <List density="compact" hasDividers data-testid="entry-list">
            {queue.map((entryId) => {
              const entry = state.entriesById[entryId]
              if (!entry) return null
              const date = new Intl.DateTimeFormat(i18n.locale, {
                month: "short",
                day: "numeric",
              }).format(new Date((entry.publishedAtUs ?? entry.sortAtUs) / 1000))
              return (
                <Item
                  as="li"
                  key={entryId}
                  className="reader-entry-item"
                  data-reader-entry-id={entryId}
                  density="balanced"
                  isSelected={cursorEntryId === entryId}
                  onClick={() => {
                    if (scrollRef.current) onRecordScroll(sourceRoute, scrollRef.current.scrollTop)
                    onSelect(entryId)
                  }}
                  label={
                    <span className="reader-entry-title">
                      {!entry.isRead ? <StatusDot variant="accent" label={i18n._("reader.unreadEntry")} /> : null}
                      <span>{entry.title ?? i18n._("reader.untitled")}</span>
                      {entry.isStarred ? <span aria-label={i18n._("reader.starredEntry")}>★</span> : null}
                    </span>
                  }
                  description={[entry.feedTitle, entry.author, entry.summary].filter(Boolean).join(" · ")}
                  descriptionLines={2}
                  labelLines={2}
                  endContent={<time dateTime={new Date((entry.publishedAtUs ?? entry.sortAtUs) / 1000).toISOString()}>{date}</time>}
                />
              )
            })}
          </List>
        </div>
      )}
    </div>
  )
}

function clampOffset(element: HTMLElement, offset: number): number {
  return Math.max(0, Math.min(offset, Math.max(0, element.scrollHeight - element.clientHeight)))
}
