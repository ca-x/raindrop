import { StatusDot } from "@astryxdesign/core/StatusDot"
import {
  TreeList,
  type TreeListDensity,
  type TreeListItemData,
} from "@astryxdesign/core/TreeList"
import { useLingui } from "@lingui/react"

import { sourceTreeDensityStyle } from "../components/sourceDensity"
import type { ReaderSource, ReaderState } from "../model/types"
import { refreshPresentation } from "../refresh/refreshPresentation"
import { groupSubscriptions, type SubscriptionGroup } from "./groupSubscriptions"

interface CategoryListProps {
  state: ReaderState
  onSelect: (source: ReaderSource) => void
  density: TreeListDensity
  query?: string
}

export function CategoryList({ state, onSelect, density, query = "" }: CategoryListProps) {
  const { i18n } = useLingui()
  const categories = state.categoryOrder.map((id) => state.categoriesById[id])
  const subscriptions = state.subscriptionOrder.map((id) => state.subscriptionsById[id])
  const groups = filterGroups(groupSubscriptions(categories, subscriptions), query)
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
      label: <span className="reader-source-label">{group.category!.title}</span>,
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
      children: feedItems(group, state, onSelect, (id) => i18n._(id)),
    }),
  )
  const uncategorized: TreeListItemData = {
    id: "uncategorized",
    label: <span className="reader-source-label">{i18n._("reader.uncategorized")}</span>,
    isExpanded: true,
    startContent: (
      <span className="reader-smart-source-icon" aria-hidden="true">
        <CategoryIcon />
      </span>
    ),
    endContent: <UnreadCount count={groups.uncategorized.unreadCount} />,
    children: feedItems(groups.uncategorized, state, onSelect, (id) => i18n._(id)),
  }

  return (
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
  )
}

function feedItems(
  group: SubscriptionGroup,
  state: ReaderState,
  onSelect: (source: ReaderSource) => void,
  translate: (id: string) => string,
): TreeListItemData[] {
  return group.subscriptions.map((subscription) => {
    const status = refreshPresentation(subscription.refresh)
    return {
      id: `feed:${subscription.feedId}`,
      label: <span className="reader-source-label">{subscription.title}</span>,
      isSelected:
        state.selectedSource.kind === "feed" &&
        state.selectedSource.feedId === subscription.feedId,
      onClick: () => onSelect({ kind: "feed", feedId: subscription.feedId }),
      startContent: <FeedSourceIcon subscriptionId={subscription.subscriptionId} />,
      endContent: (
        <span className="reader-source-status">
          <StatusDot
            variant={status.tone}
            label={translate(status.label)}
            isPulsing={status.isPulsing}
          />
          <span>{subscription.unreadCount}</span>
        </span>
      ),
    }
  })
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
