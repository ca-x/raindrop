import { Banner } from "@astryxdesign/core/Banner"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { StatusDot, type StatusDotVariant } from "@astryxdesign/core/StatusDot"
import { TreeList, type TreeListItemData } from "@astryxdesign/core/TreeList"
import { useLingui } from "@lingui/react"

import type { ReaderSource, ReaderState } from "../model/types"
import { SourceToolbar } from "./ReaderToolbar"

interface SourceTreeProps {
  state: ReaderState
  onSelect: (source: ReaderSource) => void
  onAdd: () => void
  onRefresh: (subscriptionId: string) => Promise<void>
  onLogout: () => Promise<void>
}

export function SourceTree({ state, onSelect, onAdd, onRefresh, onLogout }: SourceTreeProps) {
  const { i18n } = useLingui()
  const selectedFeedId = state.selectedSource.kind === "feed" ? state.selectedSource.feedId : null
  const selectedSubscription = selectedFeedId
    ? state.subscriptionOrder
      .map((id) => state.subscriptionsById[id])
      .find((subscription) => subscription.feedId === selectedFeedId)
    : undefined
  const smartItems: TreeListItemData[] = [
    ["UNREAD", "reader.unread"],
    ["ALL", "reader.all"],
    ["STARRED", "reader.starred"],
  ].map(([stateName, label]) => ({
    id: `smart:${stateName}`,
    label: i18n._(label),
    isSelected: state.selectedSource.kind === "smart" && state.selectedSource.state === stateName,
    onClick: () => onSelect({ kind: "smart", state: stateName as "UNREAD" | "ALL" | "STARRED" }),
  }))
  const feedItems = state.subscriptionOrder.map((subscriptionId): TreeListItemData => {
    const subscription = state.subscriptionsById[subscriptionId]
    const status = refreshStatus(subscription.refresh?.state)
    return {
      id: `feed:${subscription.feedId}`,
      label: subscription.title,
      description: subscription.siteUrl ?? undefined,
      isSelected: state.selectedSource.kind === "feed" && state.selectedSource.feedId === subscription.feedId,
      onClick: () => onSelect({ kind: "feed", feedId: subscription.feedId }),
      endContent: (
        <span className="reader-source-status">
          <StatusDot variant={status.variant} label={i18n._(status.label)} isPulsing={status.isPulsing} />
          <span>{subscription.unreadCount}</span>
        </span>
      ),
    }
  })

  return (
    <div className="reader-source-tree" aria-busy={state.paneStatus.subscriptions === "loading"}>
      <SourceToolbar
        onAdd={onAdd}
        onLogout={onLogout}
        refresh={selectedSubscription ? {
          label: i18n._("reader.refreshFeed", { title: selectedSubscription.title }),
          onRefresh: () => onRefresh(selectedSubscription.subscriptionId),
        } : undefined}
      />
      <TreeList
        density="compact"
        header={<span className="reader-pane-label">{i18n._("reader.sources")}</span>}
        items={[
          ...smartItems,
          {
            id: "subscriptions",
            label: i18n._("reader.subscriptions"),
            isExpanded: true,
            children: feedItems,
          },
        ]}
      />
      {state.paneStatus.subscriptions === "error" ? (
        <Banner
          container="section"
          status="error"
          title={i18n._("reader.subscriptionsError")}
          description={state.errors.subscriptions ?? i18n._("reader.genericError")}
        />
      ) : state.paneStatus.subscriptions === "loading" ? (
        <div className="reader-skeletons" role="status" aria-label={i18n._("reader.loadingSubscriptions")}>
          {[0, 1, 2].map((index) => <Skeleton key={index} height={44} radius={2} index={index} />)}
        </div>
      ) : null}
      {state.subscriptionOrder.length === 0 &&
      (state.paneStatus.subscriptions === "idle" || state.paneStatus.subscriptions === "ready") ? (
        <EmptyState
          isCompact
          title={i18n._("reader.noSubscriptions")}
          description={i18n._("reader.noSubscriptionsDescription")}
        />
      ) : null}
    </div>
  )
}

function refreshStatus(state?: string): { variant: StatusDotVariant; label: string; isPulsing?: boolean } {
  switch (state) {
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
