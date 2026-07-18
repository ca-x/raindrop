import { Banner } from "@astryxdesign/core/Banner"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { useLingui } from "@lingui/react"
import type { Ref } from "react"

import { CategoryList } from "../categories/CategoryList"
import type { ReaderSource, ReaderState } from "../model/types"
import { SourceToolbar } from "./ReaderToolbar"

interface SourceTreeProps {
  state: ReaderState
  onSelect: (source: ReaderSource) => void
  onAdd: () => void
  onManage: () => void
  onRefresh: (subscriptionId: string) => Promise<void>
  onLogout: () => Promise<void>
  manageButtonRef?: Ref<HTMLButtonElement>
}

export function SourceTree({
  state,
  onSelect,
  onAdd,
  onManage,
  onRefresh,
  onLogout,
  manageButtonRef,
}: SourceTreeProps) {
  const { i18n } = useLingui()
  const selectedFeedId = state.selectedSource.kind === "feed" ? state.selectedSource.feedId : null
  const selectedSubscription = selectedFeedId
    ? state.subscriptionOrder
      .map((id) => state.subscriptionsById[id])
      .find((subscription) => subscription.feedId === selectedFeedId)
    : undefined
  return (
    <div className="reader-source-tree" aria-busy={state.paneStatus.subscriptions === "loading"}>
      <SourceToolbar
        onAdd={onAdd}
        onManage={onManage}
        onLogout={onLogout}
        manageButtonRef={manageButtonRef}
        refresh={selectedSubscription ? {
          label: i18n._("reader.refreshFeed", { title: selectedSubscription.title }),
          onRefresh: () => onRefresh(selectedSubscription.subscriptionId),
        } : undefined}
      />
      <CategoryList state={state} onSelect={onSelect} />
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
