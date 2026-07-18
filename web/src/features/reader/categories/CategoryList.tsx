import { StatusDot, type StatusDotVariant } from "@astryxdesign/core/StatusDot"
import {
  TreeList,
  type TreeListDensity,
  type TreeListItemData,
} from "@astryxdesign/core/TreeList"
import { useLingui } from "@lingui/react"

import type { Subscription } from "../api/subscription.generated"
import type { ReaderSource, ReaderState } from "../model/types"
import { groupSubscriptions, type SubscriptionGroup } from "./groupSubscriptions"

interface CategoryListProps {
  state: ReaderState
  onSelect: (source: ReaderSource) => void
  density: TreeListDensity
}

export function CategoryList({ state, onSelect, density }: CategoryListProps) {
  const { i18n } = useLingui()
  const categories = state.categoryOrder.map((id) => state.categoriesById[id])
  const subscriptions = state.subscriptionOrder.map((id) => state.subscriptionsById[id])
  const groups = groupSubscriptions(categories, subscriptions)
  const smartItems: TreeListItemData[] = [
    ["UNREAD", "reader.unread"],
    ["ALL", "reader.all"],
    ["STARRED", "reader.starred"],
  ].map(([stateName, label]) => ({
    id: `smart:${stateName}`,
    label: i18n._(label),
    isSelected:
      state.selectedSource.kind === "smart" &&
      state.selectedSource.state === stateName,
    onClick: () =>
      onSelect({ kind: "smart", state: stateName as "UNREAD" | "ALL" | "STARRED" }),
  }))

  const categoryItems = groups.categorized.map(
    (group): TreeListItemData => ({
      id: `category:${group.category!.categoryId}`,
      label: group.category!.title,
      isSelected:
        state.selectedSource.kind === "category" &&
        state.selectedSource.categoryId === group.category!.categoryId,
      isExpanded: true,
      onClick: () =>
        onSelect({ kind: "category", categoryId: group.category!.categoryId }),
      endContent: <UnreadCount count={group.unreadCount} />,
      children: feedItems(group, state, onSelect, (id) => i18n._(id)),
    }),
  )
  const uncategorized: TreeListItemData = {
    id: "uncategorized",
    label: i18n._("reader.uncategorized"),
    isExpanded: true,
    endContent: <UnreadCount count={groups.uncategorized.unreadCount} />,
    children: feedItems(groups.uncategorized, state, onSelect, (id) => i18n._(id)),
  }

  return (
    <TreeList
      density={density}
      header={<span className="reader-pane-label">{i18n._("reader.sources")}</span>}
      items={[...smartItems, ...categoryItems, uncategorized]}
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
    const status = refreshStatus(subscription)
    return {
      id: `feed:${subscription.feedId}`,
      label: subscription.title,
      description: subscription.siteUrl ?? undefined,
      isSelected:
        state.selectedSource.kind === "feed" &&
        state.selectedSource.feedId === subscription.feedId,
      onClick: () => onSelect({ kind: "feed", feedId: subscription.feedId }),
      endContent: (
        <span className="reader-source-status">
          <StatusDot
            variant={status.variant}
            label={translate(status.label)}
            isPulsing={status.isPulsing}
          />
          <span>{subscription.unreadCount}</span>
        </span>
      ),
    }
  })
}

function UnreadCount({ count }: { count: number }) {
  return <span className="reader-category-unread-count">{count}</span>
}

function refreshStatus(subscription: Subscription): {
  variant: StatusDotVariant
  label: string
  isPulsing?: boolean
} {
  switch (subscription.refresh?.state) {
    case "READY":
      return { variant: "success", label: "reader.refreshReady" }
    case "PENDING":
      return { variant: "warning", label: "reader.refreshPending", isPulsing: true }
    case "ERROR":
      return { variant: "error", label: "reader.refreshError" }
    case "DEGRADED":
    case "BACKING_OFF":
      return { variant: "warning", label: "reader.refreshDelayed" }
    default:
      return { variant: "neutral", label: "reader.refreshIdle" }
  }
}
