import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Item } from "@astryxdesign/core/Item"
import { List } from "@astryxdesign/core/List"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { StatusDot } from "@astryxdesign/core/StatusDot"
import { useLingui } from "@lingui/react"

import { sourceKey, type ReaderState } from "../model/types"
import { QueueToolbar } from "./ReaderToolbar"

interface EntryQueueProps {
  state: ReaderState
  showMenu: boolean
  isCompact: boolean
  onOpenSources: () => void
  onSelect: (entryId: string) => void
  onReload: () => Promise<void>
  onMergePending: () => void
}

export function EntryQueue({ state, showMenu, isCompact, onOpenSources, onSelect, onReload, onMergePending }: EntryQueueProps) {
  const { i18n } = useLingui()
  const key = sourceKey(state.selectedSource)
  const queue = state.queueBySourceKey[key] ?? []
  const pendingCount = state.pendingNewEntryCountBySource[key] ?? 0
  return (
    <div className="reader-queue" aria-busy={state.paneStatus.queue === "loading"}>
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
                density="balanced"
                isSelected={state.selectedEntryId === entryId}
                onClick={() => onSelect(entryId)}
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
      )}
    </div>
  )
}
