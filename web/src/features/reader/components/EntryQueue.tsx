import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Item } from "@astryxdesign/core/Item"
import { List, type ListDensity } from "@astryxdesign/core/List"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { StatusDot } from "@astryxdesign/core/StatusDot"
import { useLingui } from "@lingui/react"
import { useEffect, useLayoutEffect, useRef } from "react"

import { MountTransition } from "../../../shared/motion/MountTransition"
import { sourceKey, type ReaderState } from "../model/types"
import { GENERIC_READER_ERROR } from "../model/controllerErrors"
import {
  entryQueueDensityMetrics,
  entryQueueDensityStyle,
} from "./entryDensity"
import { FeedSearchInput } from "./FeedSearchInput"
import { QueueToolbar, type MarkReadAvailability, type QueueSubscriptionActions } from "./QueueToolbar"
import { RelativeEntryTime } from "./RelativeEntryTime"
import { ReaderEmptyIcon } from "./ReaderEmptyIcon"
import { StarIcon } from "./StarIcon"

interface EntryQueueProps extends QueueSubscriptionActions {
  state: ReaderState
  sourceLabel: string
  showMenu: boolean
  isCompact: boolean
  onOpenSources: () => void
  onSelect: (entryId: string) => void
  onToggleStar: (entryId: string) => Promise<void>
  onViewAll?: () => void
  onAddSubscription?: () => void
  isRouteReady: boolean
  cursorEntryId: string | null
  cursorFocusNonce: number
  shouldFocusCursor?: boolean
  sourceRoute: string
  savedScrollOffset: number
  onRecordScroll: (route: string, offset: number) => void
  onReload: () => Promise<void>
  onRetry?: () => Promise<boolean>
  onLoadMore?: () => Promise<boolean>
  onSearchFeed: (query: string) => Promise<void>
  onNextUnreadSource: () => Promise<void>
  onPreviousUnreadSource: () => Promise<void>
  onRequestMarkRead: () => void
  isMarkingRead: boolean
  onMergePending: () => void
  onFocusRow: (entryId: string) => void
  density: ListDensity
}

export function EntryQueue({
  state,
  sourceLabel,
  showMenu,
  isCompact,
  onOpenSources,
  onSelect,
  onToggleStar,
  onViewAll,
  onAddSubscription,
  isRouteReady,
  cursorEntryId,
  cursorFocusNonce,
  shouldFocusCursor = true,
  sourceRoute,
  savedScrollOffset,
  onRecordScroll,
  onReload,
  onEditSubscription,
  subscriptionMenuRef,
  subscriptionRefresh,
  onRetry = async () => false,
  onLoadMore = async () => false,
  onSearchFeed,
  onNextUnreadSource,
  onPreviousUnreadSource,
  onRequestMarkRead,
  isMarkingRead,
  onMergePending,
  onFocusRow,
  density,
}: EntryQueueProps) {
  const { i18n } = useLingui()
  const rootRef = useRef<HTMLDivElement>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const smoothScrollCursorRef = useRef<string | null>(null)
  const focusedSavedRow = useRef<{ target: HTMLElement; index: number } | null>(null)
  const shouldFocusCursorRef = useRef(shouldFocusCursor)
  shouldFocusCursorRef.current = shouldFocusCursor
  const key = sourceKey(state.selectedSource)
  const queue = state.queueBySourceKey[key] ?? []
  useLayoutEffect(() => {
    const focused = focusedSavedRow.current
    if (!focused) return
    if (key === "smart:STARRED" && focused.target.isConnected) return
    focusedSavedRow.current = null
    if (key !== "smart:STARRED" || document.activeElement !== document.body) return
    const adjacent = queue[Math.min(focused.index, queue.length - 1)]
    if (adjacent) onFocusRow(adjacent)
    else rootRef.current?.querySelector<HTMLButtonElement>(".reader-queue-empty button")?.focus()
  }, [key, queue, onFocusRow])
  const hasQueueSnapshot = Object.prototype.hasOwnProperty.call(
    state.queueBySourceKey,
    key,
  )
  const nextCursor = state.nextCursorBySourceKey[key]
  const pendingCount = state.pendingNewEntryCountBySource[key] ?? 0
  const isNewLibrary = state.subscriptionsAuthoritative && state.subscriptionOrder.length === 0
  const smartFilter = state.subscriptionOrder.length && state.selectedSource.kind === "smart" ? state.selectedSource.state : null
  const emptyMessage = state.feedSearchQuery ? "noSearchResults"
    : smartFilter === "UNREAD" ? "caughtUp" : smartFilter === "STARRED" ? "noSaved" : "noEntries"
  const queueError = state.errors.queue === GENERIC_READER_ERROR
    ? i18n._("reader.queueRecoveryDescription")
    : state.errors.queue
  const markReadAvailability: MarkReadAvailability =
    state.selectedSource.kind === "smart" && state.selectedSource.state === "STARRED"
      ? "hidden"
      : state.feedSearchQuery ||
          state.snapshotGenerationBySource[key] === undefined ||
          state.paneStatus.queue !== "ready"
        ? "disabled"
        : "enabled"
  useLayoutEffect(() => {
    const node = scrollRef.current
    if (!node || state.paneStatus.queue !== "ready" || !isRouteReady) return
    node.scrollTop = clampOffset(node, savedScrollOffset)
    return () => onRecordScroll(sourceRoute, node.scrollTop)
  }, [isRouteReady, sourceRoute, state.paneStatus.queue])
  useEffect(() => {
    if (!shouldFocusCursorRef.current || !isRouteReady || !cursorEntryId || cursorFocusNonce === 0) return
    const row = [...(rootRef.current?.querySelectorAll<HTMLElement>("[data-reader-entry-id]") ?? [])]
      .find((item) => item.dataset.readerEntryId === cursorEntryId)
    const button = row?.querySelector<HTMLButtonElement>(":scope > button")
    button?.focus({ preventScroll: true })
    if (smoothScrollCursorRef.current === cursorEntryId) {
      smoothScrollCursorRef.current = null
    } else {
      row?.scrollIntoView?.({ behavior: "auto", block: "nearest" })
    }
  }, [cursorEntryId, cursorFocusNonce, isRouteReady])
  return (
    <div
      ref={rootRef}
      className="reader-queue"
      onFocusCapture={(event) => {
        const id = event.target.closest<HTMLElement>("[data-reader-entry-id]")?.dataset.readerEntryId
        focusedSavedRow.current = key === "smart:STARRED" && id
          ? { target: event.target, index: queue.indexOf(id) } : null
      }}
      aria-busy={state.requestActivity.queue || state.requestActivity.page}
      data-request-active={state.requestActivity.queue ? "true" : undefined}
      style={entryQueueDensityStyle(density)}
    >
      <div className="reader-pane-progress" aria-hidden="true" />
      <QueueToolbar
        onEditSubscription={onEditSubscription}
        subscriptionMenuRef={subscriptionMenuRef}
        subscriptionRefresh={subscriptionRefresh}
        sourceLabel={sourceLabel}
        entryCount={queue.length}
        showMenu={showMenu}
        isCompact={isCompact}
        markReadAvailability={markReadAvailability}
        isMarkingRead={isMarkingRead}
        onOpenSources={onOpenSources}
        onReload={onReload}
        onNextUnreadSource={onNextUnreadSource}
        onPreviousUnreadSource={onPreviousUnreadSource}
        onRequestMarkRead={onRequestMarkRead}
      />
      {state.selectedSource.kind === "feed" ? (
        <FeedSearchInput
          key={key}
          query={state.feedSearchQuery}
          isLoading={state.paneStatus.queue === "loading"}
          onSearch={onSearchFeed}
        />
      ) : null}
      <div className="reader-queue-update-notice" data-pending={pendingCount > 0}>
        <span role="status" className="reader-queue-update-copy">
          {i18n._(pendingCount > 0 ? "reader.newEntriesAvailable" : state.requestActivity.queue
            ? "reader.backgroundUpdating" : "reader.backgroundUpdatesHint", { count: pendingCount })}
        </span>
        {pendingCount > 0 ? (
              <Button
                label={i18n._("reader.showNewEntries", { count: pendingCount })}
                isDisabled={!isRouteReady}
                onClick={(event) => {
                  const firstPending = state.pendingNewEntriesBySource[key]?.[0]
                  smoothScrollCursorRef.current = firstPending ?? null
                  onMergePending()
                  scrollRef.current?.scrollTo({
                    top: 0,
                    behavior: event.detail === 0 || prefersReducedMotion() ? "auto" : "smooth",
                  })
                  onRecordScroll(sourceRoute, 0)
                  if (firstPending) onFocusRow(firstPending)
                }}
                variant="ghost"
              />
        ) : null}
      </div>
      {state.errors.queue && hasQueueSnapshot ? (
        <Banner
          container="section"
          status="warning"
          title={i18n._("reader.queueError")}
          description={queueError ?? i18n._("reader.genericError")}
          endContent={(
            <Button
              label={i18n._("common.retry")}
              variant="ghost"
              size="sm"
              clickAction={async () => { await onRetry() }}
            />
          )}
        />
      ) : null}
      {state.paneStatus.queue === "loading" && !hasQueueSnapshot ? (
        <div className="reader-skeletons" role="status" aria-label={i18n._("reader.loadingEntries")}>
          {[0, 1, 2, 3].map((index) => (
            <Skeleton
              key={index}
              height={entryQueueDensityMetrics[density].rowBlockSize}
              radius={2}
              index={index}
            />
          ))}
        </div>
      ) : state.paneStatus.queue === "error" && !hasQueueSnapshot ? (
        <div className="reader-queue-recovery" role="alert">
          <EmptyState
            isCompact
            icon={<ReaderEmptyIcon kind="queue" />}
            title={i18n._("reader.queueError")}
            description={queueError ?? i18n._("reader.queueRecoveryDescription")}
            actions={(
              <Button
                label={i18n._("common.retry")}
                variant="secondary"
                size="sm"
                clickAction={async () => { await onRetry() }}
              />
            )}
          />
        </div>
      ) : queue.length === 0 ? (
        <EmptyState
          className="reader-queue-empty"
          isCompact
          icon={<ReaderEmptyIcon kind="queue" />}
          title={i18n._(`reader.${emptyMessage}`)}
          description={i18n._(isNewLibrary ? "reader.noSubscriptionsDescription" : `reader.${emptyMessage}Description`)}
          actions={nextCursor && onLoadMore ? (
            <Button label={i18n._(state.errors.page ? "common.retry" : "reader.loadMoreEntries")}
              isLoading={state.requestActivity.page} variant="secondary" onClick={() => { void onLoadMore() }} />
          ) : state.feedSearchQuery ? (
            <Button
              label={i18n._("reader.clearFeedSearch")}
              variant="secondary"
              onClick={() => {
                rootRef.current?.querySelector<HTMLInputElement>(".reader-feed-search input")?.focus()
                void onSearchFeed("")
              }}
            />
          ) : isNewLibrary && onAddSubscription ? (
            <Button label={i18n._("reader.addSubscription")} variant="secondary" onClick={onAddSubscription} />
          ) : (smartFilter === "UNREAD" || smartFilter === "STARRED") && onViewAll ? (
            <Button label={i18n._("reader.viewAllArticles")} variant="secondary" onClick={onViewAll} />
          ) : undefined}
        />
      ) : (
        <div
          ref={scrollRef}
          className="reader-queue-scroll"
          data-testid="entry-queue-scroll"
          onScroll={(event) => {
            if (!isRouteReady) return
            const target = event.currentTarget
            onRecordScroll(sourceRoute, target.scrollTop)
            if (
              nextCursor &&
              !state.requestActivity.queue &&
              !state.requestActivity.page &&
              target.scrollHeight - target.scrollTop - target.clientHeight < 240
            ) {
              void onLoadMore()
            }
          }}
        >
          <List density={density} hasDividers data-testid="entry-list">
            {queue.map((entryId) => {
              const entry = state.entriesById[entryId]
              if (!entry) return null
              const summary = entryPreview(entry.summary)
              const timestampUs = entry.publishedAtUs ?? entry.sortAtUs
              return (
                <Item
                  as="li"
                  key={entryId}
                  className={`reader-entry-item${entry.isRead ? "" : " is-unread"}`}
                  data-reader-entry-id={entryId}
                  density={density}
                  isDisabled={!isRouteReady}
                  isSelected={cursorEntryId === entryId}
                  onClick={() => {
                    if (!isRouteReady) return
                    if (scrollRef.current) onRecordScroll(sourceRoute, scrollRef.current.scrollTop)
                    onSelect(entryId)
                  }}
                  label={
                    <span className="reader-entry-title">
                      <span className="reader-entry-marker">
                        {!entry.isRead ? <StatusDot variant="accent" label={i18n._("reader.unreadEntry")} /> : null}
                      </span>
                      <span id={`queue-title-${entryId}`} className="reader-entry-headline">{entry.title ?? i18n._("reader.untitled")}</span>
                    </span>
                  }
                  description={
                    <span className="reader-entry-description">
                      <span className="reader-entry-meta">
                        {[entry.feedTitle, entry.author].filter(Boolean).join(" · ")}
                      </span>
                      {summary ? (
                        <span className="reader-entry-summary">{summary}</span>
                      ) : null}
                    </span>
                  }
                  descriptionLines={2}
                  labelLines={2}
                  align="start"
                  endContent={(
                    <>
                      <RelativeEntryTime
                        timestampUs={timestampUs}
                        locale={i18n.locale}
                        justNowLabel={i18n._("reader.justNow")}
                      />
                      <button
                        type="button"
                        className="reader-queue-star"
                        aria-label={i18n._("reader.saveForLater")}
                        aria-describedby={`queue-title-${entryId}`}
                        aria-pressed={entry.isStarred}
                        title={i18n._(entry.isStarred ? "reader.unstarEntry" : "reader.saveForLater")}
                        disabled={!isRouteReady}
                        aria-disabled={state.pendingMutationByEntryId[entryId]?.isStarred ? true : undefined}
                        onClick={() => {
                          if (!state.pendingMutationByEntryId[entryId]?.isStarred) void onToggleStar(entryId)
                        }}
                      >
                        <StarIcon isFilled={entry.isStarred} />
                      </button>
                    </>
                  )}
                />
              )
            })}
          </List>
          <div className="reader-queue-pagination" role="status" aria-live="polite">
            {state.errors.page ? (
              <>
                <span>{i18n._("reader.moreEntriesError")}</span>
                <Button
                  label={i18n._("common.retry")}
                  variant="ghost"
                  size="sm"
                  clickAction={async () => { await onLoadMore() }}
                />
              </>
            ) : nextCursor ? (
              <Button
                label={i18n._("reader.loadMoreEntries")}
                variant="ghost"
                size="sm"
                isLoading={state.requestActivity.page}
                clickAction={async () => { await onLoadMore() }}
              />
            ) : (
              <span>{i18n._("reader.endOfList")}</span>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function prefersReducedMotion() {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches
}

function clampOffset(element: HTMLElement, offset: number): number {
  return Math.max(0, Math.min(offset, Math.max(0, element.scrollHeight - element.clientHeight)))
}

export function entryPreview(value: string | null): string | null {
  if (!value) return null
  const compact = value.replace(/\s+/gu, " ").trim()
  if (!compact) return null
  const characters = [...compact]
  return characters.length <= 180 ? compact : `${characters.slice(0, 179).join("")}…`
}
