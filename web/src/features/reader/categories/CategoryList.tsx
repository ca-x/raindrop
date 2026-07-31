import { IconButton } from "@astryxdesign/core/IconButton"
import { StatusDot } from "@astryxdesign/core/StatusDot"
import { Tooltip } from "@astryxdesign/core/Tooltip"
import {
  TreeList,
  type TreeListDensity,
  type TreeListItemData,
} from "@astryxdesign/core/TreeList"
import { useLingui } from "@lingui/react"
import { type DragEvent, useRef, useState } from "react"

import { sourceTreeDensityStyle } from "../components/sourceDensity"
import type { ReaderSource, ReaderState } from "../model/types"
import { refreshPresentation } from "../refresh/refreshPresentation"
import { groupSubscriptions, type SubscriptionGroup } from "./groupSubscriptions"

interface CategoryListProps {
  state: ReaderState
  onSelect: (source: ReaderSource) => void
  onRequestMarkRead?: (feedId: string, title: string) => void
  onMoveSubscription?: (
    subscriptionId: string,
    categoryId: string | null,
  ) => Promise<boolean>
  isMarkingRead?: boolean
  density: TreeListDensity
  query?: string
}

interface DraggedSubscription {
  subscriptionId: string
  categoryId: string | null
  title: string
}

export function CategoryList({
  state,
  onSelect,
  onRequestMarkRead,
  onMoveSubscription,
  isMarkingRead = false,
  density,
  query = "",
}: CategoryListProps) {
  const { i18n } = useLingui()
  const draggedSubscriptionRef = useRef<DraggedSubscription | null>(null)
  const [draggedSubscriptionId, setDraggedSubscriptionId] = useState<string | null>(null)
  const [dropTargetCategoryId, setDropTargetCategoryId] = useState<string | null | undefined>()
  const [pendingCategoryId, setPendingCategoryId] = useState<string | null | undefined>()
  const [moveAnnouncement, setMoveAnnouncement] = useState("")
  const categories = state.categoryOrder.map((id) => state.categoriesById[id])
  const subscriptions = state.subscriptionOrder.map((id) => state.subscriptionsById[id])
  const groups = filterGroups(groupSubscriptions(categories, subscriptions), query)
  const dragSubscription = (
    event: DragEvent<HTMLElement>,
    subscription: DraggedSubscription,
  ) => {
    draggedSubscriptionRef.current = subscription
    setDraggedSubscriptionId(subscription.subscriptionId)
    setDropTargetCategoryId(undefined)
    setMoveAnnouncement("")
    event.dataTransfer.setData("text/plain", subscription.subscriptionId)
    event.dataTransfer.effectAllowed = "move"
  }
  const finishDragging = () => {
    draggedSubscriptionRef.current = null
    setDraggedSubscriptionId(null)
    setDropTargetCategoryId(undefined)
  }
  const canDropInto = (categoryId: string | null) => {
    const dragged = draggedSubscriptionRef.current
    return Boolean(
      dragged && dragged.categoryId !== categoryId && pendingCategoryId === undefined,
    )
  }
  const dragOverCategory = (
    event: DragEvent<HTMLElement>,
    categoryId: string | null,
  ) => {
    if (!canDropInto(categoryId)) return
    event.preventDefault()
    event.dataTransfer.dropEffect = "move"
    setDropTargetCategoryId(categoryId)
  }
  const leaveCategory = (event: DragEvent<HTMLElement>, categoryId: string | null) => {
    if (
      event.relatedTarget instanceof Node &&
      event.currentTarget.contains(event.relatedTarget)
    ) {
      return
    }
    setDropTargetCategoryId((current) => current === categoryId ? undefined : current)
  }
  const dropIntoCategory = async (
    event: DragEvent<HTMLElement>,
    categoryId: string | null,
  ) => {
    const dragged = draggedSubscriptionRef.current
    if (!dragged || !canDropInto(categoryId) || !onMoveSubscription) return
    event.preventDefault()
    finishDragging()
    setPendingCategoryId(categoryId)
    let saved = false
    try {
      saved = await onMoveSubscription(dragged.subscriptionId, categoryId)
    } catch {
      saved = false
    } finally {
      setPendingCategoryId(undefined)
      const categoryTitle = categoryId === null
        ? i18n._("reader.uncategorized")
        : state.categoriesById[categoryId]?.title ?? i18n._("reader.uncategorized")
      setMoveAnnouncement(
        saved
          ? i18n._("reader.feedMoved", {
              title: dragged.title,
              category: categoryTitle,
            })
          : i18n._("reader.feedMoveFailed", { title: dragged.title }),
      )
    }
  }
  const categoryLabel = (categoryId: string | null, title: string) => (
    <span
      className="reader-source-label reader-category-drop-label"
      data-drop-zone={onMoveSubscription ? "true" : undefined}
      data-drop-target={dropTargetCategoryId === categoryId ? "true" : undefined}
      data-drop-pending={pendingCategoryId === categoryId ? "true" : undefined}
      onDragEnter={(event) => dragOverCategory(event, categoryId)}
      onDragOver={(event) => dragOverCategory(event, categoryId)}
      onDragLeave={(event) => leaveCategory(event, categoryId)}
      onDrop={(event) => void dropIntoCategory(event, categoryId)}
    >
      {title}
    </span>
  )
  const smartItems: TreeListItemData[] = [
    ["UNREAD", "reader.unread"],
    ["ALL", "reader.all"],
    ["STARRED", "reader.starred"],
  ].map(([stateName, label]) => ({
    id: `smart:${stateName}`,
    label: <span className="reader-source-label">{i18n._(label)}</span>,
    isSelected:
      state.selectedSource.kind === "smart" &&
      state.selectedSource.state === stateName,
    onClick: () =>
      onSelect({ kind: "smart", state: stateName as "UNREAD" | "ALL" | "STARRED" }),
    startContent: (
      <span className="reader-smart-source-icon" aria-hidden="true">
        <SmartSourceIcon state={stateName as "UNREAD" | "ALL" | "STARRED"} />
      </span>
    ),
  }))

  const categoryItems = groups.categorized.map(
    (group): TreeListItemData => ({
      id: `category:${group.category!.categoryId}`,
      label: categoryLabel(group.category!.categoryId, group.category!.title),
      isSelected:
        state.selectedSource.kind === "category" &&
        state.selectedSource.categoryId === group.category!.categoryId,
      isExpanded: true,
      onClick: () =>
        onSelect({ kind: "category", categoryId: group.category!.categoryId }),
      startContent: (
        <span className="reader-smart-source-icon" aria-hidden="true">
          <CategoryIcon />
        </span>
      ),
      endContent: <UnreadCount count={group.unreadCount} />,
      children: feedItems(
        group,
        state,
        onSelect,
        onRequestMarkRead,
        isMarkingRead,
        (id, values) => i18n._(id, values),
        onMoveSubscription && pendingCategoryId === undefined
          ? dragSubscription
          : undefined,
        finishDragging,
        draggedSubscriptionId,
      ),
    }),
  )
  const uncategorized: TreeListItemData = {
    id: "uncategorized",
    label: categoryLabel(null, i18n._("reader.uncategorized")),
    isExpanded: true,
    startContent: (
      <span className="reader-smart-source-icon" aria-hidden="true">
        <CategoryIcon />
      </span>
    ),
    endContent: <UnreadCount count={groups.uncategorized.unreadCount} />,
    children: feedItems(
      groups.uncategorized,
      state,
      onSelect,
      onRequestMarkRead,
      isMarkingRead,
      (id, values) => i18n._(id, values),
      onMoveSubscription && pendingCategoryId === undefined ? dragSubscription : undefined,
      finishDragging,
      draggedSubscriptionId,
    ),
  }

  return (
    <>
      <TreeList
        className="reader-source-list"
        density={density}
        style={sourceTreeDensityStyle(density)}
        header={<span className="reader-pane-label">{i18n._("reader.sources")}</span>}
        items={[
          ...smartItems,
          ...categoryItems,
          ...(groups.uncategorized.subscriptions.length > 0 || !query.trim()
            ? [uncategorized]
            : []),
        ]}
      />
      <span className="reader-visually-hidden" role="status" aria-live="polite">
        {moveAnnouncement}
      </span>
    </>
  )
}

function feedItems(
  group: SubscriptionGroup,
  state: ReaderState,
  onSelect: (source: ReaderSource) => void,
  onRequestMarkRead: ((feedId: string, title: string) => void) | undefined,
  isMarkingRead: boolean,
  translate: (id: string, values?: Record<string, string>) => string,
  onDragStart: (
    (event: DragEvent<HTMLElement>, subscription: DraggedSubscription) => void
  ) | undefined,
  onDragEnd: () => void,
  draggedSubscriptionId: string | null,
): TreeListItemData[] {
  return group.subscriptions.map((subscription) => {
    const status = refreshPresentation(subscription.refresh)
    return {
      id: `feed:${subscription.feedId}`,
      label: (
        <span
          className="reader-source-label reader-feed-drag-label"
          draggable={Boolean(onDragStart)}
          data-dragging={
            draggedSubscriptionId === subscription.subscriptionId ? "true" : undefined
          }
          onDragStart={(event) => onDragStart?.(event, {
            subscriptionId: subscription.subscriptionId,
            categoryId: subscription.categoryId,
            title: subscription.title,
          })}
          onDragEnd={onDragEnd}
        >
          {subscription.title}
        </span>
      ),
      isSelected:
        state.selectedSource.kind === "feed" &&
        state.selectedSource.feedId === subscription.feedId,
      onClick: () => onSelect({ kind: "feed", feedId: subscription.feedId }),
      startContent: <FeedSourceIcon subscriptionId={subscription.subscriptionId} />,
      endContent: (
        <span className="reader-feed-end-content">
          {onRequestMarkRead ? (
            <Tooltip
              content={translate("reader.quickMarkFeedRead")}
              delay={180}
              hasHoverIndication={false}
            >
              <span className="reader-source-mark-read">
                <IconButton
                  label={translate("reader.quickMarkFeedReadLabel", {
                    title: subscription.title,
                  })}
                  icon={<MarkReadIcon />}
                  onClick={() => onRequestMarkRead(subscription.feedId, subscription.title)}
                  isDisabled={isMarkingRead || subscription.unreadCount === 0}
                  variant="ghost"
                  size="sm"
                />
              </span>
            </Tooltip>
          ) : null}
          <span className="reader-source-status">
            <StatusDot
              variant={status.tone}
              label={translate(status.label)}
              isPulsing={status.isPulsing}
            />
            <span>{subscription.unreadCount}</span>
          </span>
        </span>
      ),
    }
  })
}

function MarkReadIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 18 18"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="m3.5 9.2 3.1 3.1 7.9-7.9" />
    </svg>
  )
}

function filterGroups(
  groups: ReturnType<typeof groupSubscriptions>,
  query: string,
): ReturnType<typeof groupSubscriptions> {
  const normalized = query.trim().toLocaleLowerCase()
  if (!normalized) return groups
  const filter = (group: SubscriptionGroup): SubscriptionGroup => {
    const categoryMatches = group.category?.title.toLocaleLowerCase().includes(normalized)
    const subscriptions = categoryMatches
      ? group.subscriptions
      : group.subscriptions.filter((subscription) =>
          [subscription.title, subscription.feedUrl, subscription.siteUrl]
            .filter(Boolean)
            .some((value) => value!.toLocaleLowerCase().includes(normalized)),
        )
    return {
      category: group.category,
      subscriptions,
      unreadCount: subscriptions.reduce(
        (total, subscription) => total + subscription.unreadCount,
        0,
      ),
    }
  }
  return {
    categorized: groups.categorized.map(filter).filter((group) => group.subscriptions.length > 0),
    uncategorized: filter(groups.uncategorized),
  }
}

function UnreadCount({ count }: { count: number }) {
  return <span className="reader-category-unread-count">{count}</span>
}

function SmartSourceIcon({ state }: { state: "UNREAD" | "ALL" | "STARRED" }) {
  if (state === "UNREAD") {
    return (
      <svg viewBox="0 0 18 18" width="18" height="18" fill="none">
        <circle cx="9" cy="9" r="3" fill="currentColor" />
      </svg>
    )
  }
  if (state === "STARRED") {
    return (
      <svg viewBox="0 0 18 18" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round">
        <path d="m9 2.5 1.9 3.9 4.3.6-3.1 3 0.7 4.3L9 12.3l-3.8 2 0.7-4.3-3.1-3 4.3-.6L9 2.5Z" />
      </svg>
    )
  }
  return (
    <svg viewBox="0 0 18 18" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round">
      <path d="M4 5h10M4 9h10M4 13h10" />
    </svg>
  )
}

function CategoryIcon() {
  return (
    <svg viewBox="0 0 18 18" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round">
      <path d="M2.8 5.2h4l1.4 1.6h7v6.5H2.8V5.2Z" />
    </svg>
  )
}

function FeedSourceIcon({ subscriptionId }: { subscriptionId: string }) {
  return (
    <span className="reader-source-icon" aria-hidden="true">
      <svg viewBox="0 0 18 18" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
        <circle cx="5.2" cy="12.8" r="1" fill="currentColor" stroke="none" />
        <path d="M4.2 8.6a5.2 5.2 0 0 1 5.2 5.2M4.2 4.2a9.6 9.6 0 0 1 9.6 9.6" />
      </svg>
      <img
        className="reader-source-favicon"
        src={`/reader-assets/subscriptions/${subscriptionId}/favicon`}
        alt=""
        width="18"
        height="18"
        loading="lazy"
        decoding="async"
        referrerPolicy="no-referrer"
        onError={(event) => {
          event.currentTarget.hidden = true
        }}
      />
    </span>
  )
}
